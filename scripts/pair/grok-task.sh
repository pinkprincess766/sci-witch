#!/usr/bin/env bash
# Delegates one task to Grok and brings the result back for review.
#
# Claude is the chef: it writes the task, reads what comes back, runs the
# checks, and either accepts the work or returns it with findings. Grok is
# the intern: it works headlessly in its own copy of the repository and
# answers with a report in a fixed JSON shape. Nobody relays text by hand,
# and nothing drives a screen.
#
# What keeps the intern inside the lines. Each point was checked on a
# throwaway repository, on disk, before it was relied on here:
#
#   * **The intern has no terminal.** Grok checks an *allowed* shell command
#     only by its first word: with `cat` allowed, `cat x; touch ../y` and
#     `cat x && touch ../y` both ran in full. An allowlist of commands is
#     therefore not a boundary, and the terminal is denied outright
#     (`--deny Bash`). The intern reads, searches and edits files; the chef
#     builds, tests and gates, and sends the output back. The test counts in
#     a review are the chef's own run, never the intern's account of one.
#   * File tools are allowed only under the working directory. A write to
#     `../x` and to an absolute path outside were both refused.
#   * Explicit denials hold even inside a chain (`cat x; git commit` was
#     refused), so git history and the network are denied by name as well,
#     in case somebody loosens the terminal rule later.
#   * The working directory is a git worktree outside the project.
#   * A tripwire: the real working tree is fingerprinted before a round and
#     after it. If it changed, the round is reported as tripped and `apply`
#     refuses. Grok's own `--sandbox workspace` is not relied on — on macOS
#     it let a shell command write to a sibling directory and reach the
#     network.
#
# Nothing here commits. The worktree is detached, its baseline is a tree
# object (not a commit), and `apply` leaves the change uncommitted in the
# real working tree for the owner to review.
#
# Usage:
#   grok-task.sh start  TASK.md           new task, round 1
#   grok-task.sh check  RUN [quick]       the chef's gates in Grok's copy
#   grok-task.sh resume RUN FEEDBACK.md   same Grok session, next round
#   grok-task.sh diff   RUN               Grok's change against the start
#   grok-task.sh apply  RUN               put the change into the real tree
#   grok-task.sh clean  RUN               remove the worktree and run files
#   grok-task.sh list                     runs on disk
#
# Limits, all overridable from the environment and all explicit:
#   PAIR_MAX_TURNS  (80)  agent turns per round
#   PAIR_MAX_ROUNDS (3)   rounds per task, counting the first
#   SCIWITCH_PAIR_HOME    where runs live (default: $TMPDIR/sciwitch-pair)
#   GROK_BIN              the grok executable (default: ~/.grok/bin/grok)
#   PAIR_ROUND_TIMEOUT    seconds one round may run before grok is stopped (3600)

set -euo pipefail

GROK="${GROK_BIN:-$HOME/.grok/bin/grok}"
ROUND_TIMEOUT="${PAIR_ROUND_TIMEOUT:-3600}"
GROK_PID=""
WATCHDOG_PID=""
MAX_TURNS="${PAIR_MAX_TURNS:-80}"
MAX_ROUNDS="${PAIR_MAX_ROUNDS:-3}"
PAIR_HOME="${SCIWITCH_PAIR_HOME:-${TMPDIR:-/tmp}/sciwitch-pair}"
REPO="$(git rev-parse --show-toplevel)"
HERE="$REPO/scripts/pair"

die() {
    printf 'grok-task: %s\n' "$*" >&2
    exit 1
}

# What the intern may do: read, search, and edit files inside its copy.
# Nothing else. Anything not listed ends the run.
ALLOW=(
    'Read'
    'Grep'
    'Glob'
    'LS'
    'Edit(./**)'
    'Write(./**)'
)

# The terminal as a whole, and — named separately, in case the first rule is
# ever loosened — the things a mistake could not take back: git history and
# the network. A denial is soft: the intern is told no and carries on with
# the file tools, which is what happened in the test.
DENY=(
    'Bash'
    'Bash(git commit:*)'
    'Bash(git push:*)'
    'Bash(git add:*)'
    'Bash(git reset:*)'
    'Bash(git checkout:*)'
    'Bash(git stash:*)'
    'Bash(git worktree:*)'
    'Bash(rm:*)'
    'Bash(curl:*)'
    'Bash(wget:*)'
)

# A round stopped with Ctrl-C leaves its worktree behind. It is marked, so
# `list` shows it and nobody mistakes it for a finished round; the first one
# found this way had been reported as "nothing ran" when a worktree existed.
#
# grok runs in the background and the round waits for it: bash runs a trap
# only between commands, so a grok in the foreground that hangs (it did, when
# its usage limit ran out) kept Ctrl-C from doing anything until it returned.
# `wait` is interrupted by the signal at once, and the trap stops grok first.
stop_grok() {
    if [ -n "$WATCHDOG_PID" ]; then
        pkill -TERM -P "$WATCHDOG_PID" 2>/dev/null || true
        kill -TERM "$WATCHDOG_PID" 2>/dev/null || true
    fi
    if [ -n "$GROK_PID" ]; then
        pkill -TERM -P "$GROK_PID" 2>/dev/null || true
        kill -TERM "$GROK_PID" 2>/dev/null || true
    fi
}

on_interrupt() {
    stop_grok
    printf 'interrupted\n' >"$1/interrupted"
    printf '\ngrok-task: запуск %s прерван; копия осталась. Убрать: %s clean %s\n' \
        "$(basename "$1")" "$0" "$(basename "$1")" >&2
    exit 130
}

run_dir() {
    local id="${1:?укажите RUN}"
    local dir="$PAIR_HOME/$id"
    [ -d "$dir" ] || die "нет такого запуска: $id (см. list)"
    printf '%s\n' "$dir"
}

# Fingerprint of the real working tree: HEAD, every uncommitted change and
# every untracked file that is not ignored.
main_state() {
    {
        git -C "$REPO" rev-parse HEAD
        git -C "$REPO" diff HEAD --binary
        (
            cd "$REPO"
            git ls-files --others --exclude-standard -z |
                while IFS= read -r -d '' file; do
                    shasum -a 256 "$file"
                done
        )
    } | shasum -a 256 | cut -d' ' -f1
}

# A tree object for the whole worktree, ignored files excluded. A tree is
# not a commit: nothing points at it and no history changes.
tree_of() {
    local worktree="$1" index="$2"
    rm -f "${index:?}"
    GIT_INDEX_FILE="$index" git -C "$worktree" add -A
    GIT_INDEX_FILE="$index" git -C "$worktree" write-tree
}

prompt_header() {
    cat <<'EOF'
Ты стажёр в проекте sci-witch. Задачу ставит шеф (Claude), он же принимает работу.

Как устроена работа:
- Текущий каталог — отдельная копия репозитория. Вне него ничего не меняй.
- Правила проекта переданы тебе целиком (AGENTS.md). Следуй им.
- Терминала у тебя нет: только чтение, поиск и правка файлов в этом каталоге.
  Сборку, тесты, rustfmt и ворота запускает шеф и присылает дословный вывод.
  Не пытайся их запускать и не обходи запрет.
- Поэтому в checks_needed перечисли, какие проверки шефу запустить, чтобы
  принять твою работу, — команды целиком.
- Правила из раздела «Прежде чем сказать „готово“» выполняй поиском по коду:
  найди всех, кто опирается на то, что ты меняешь.
- Всё, что не сделано, — в not_done.

Отчёт. Ровно один, в самом конце, когда все файлы уже записаны. До этого —
никаких промежуточных отчётов, никаких JSON-объектов в сообщениях: пока работаешь,
пиши обычный текст или вызывай инструменты. Отчёт — последнее сообщение: один
JSON-объект по схеме ниже, без пояснений до и после него. Сообщение с отчётом
завершает работу; всё, что сделано после него, никто не увидит. Если отчёта нет
или в нём не хватает обязательного поля, работа не принимается.

Схема отчёта (JSON Schema):
EOF
    cat "$HERE/report-schema.json"
}

run_round() {
    local run="$1" body="$2" session="$3"
    local round
    round=$(($(cat "$run/round") + 1))
    [ "$round" -le "$MAX_ROUNDS" ] ||
        die "предел раундов $MAX_ROUNDS исчерпан; задача возвращается шефу без приёма"

    local prompt="$run/prompt-$round.md"
    {
        prompt_header
        printf '\n'
        cat "$body"
    } >"$prompt"

    local args=(
        --prompt-file "$prompt"
        --max-turns "$MAX_TURNS"
        --permission-mode dontAsk
        --rules "$(cat "$run/rules.md")"
        --disable-web-search
        --no-subagents
        --cwd "$run/worktree"
    )
    local rule
    for rule in "${ALLOW[@]}"; do args+=(--allow "$rule"); done
    for rule in "${DENY[@]}"; do args+=(--deny "$rule"); done
    if [ -n "$session" ]; then args+=(--resume "$session"); fi

    local before rc=0
    before="$(main_state)"
    (
        cd "$run/worktree"
        exec "$GROK" "${args[@]}"
    ) >"$run/round-$round.json" 2>"$run/round-$round.err" &
    GROK_PID=$!
    # A round that outlives ROUND_TIMEOUT is stopped and reported like any
    # failed round; the watchdog is a plain sleep, killed when grok finishes.
    (
        sleep "$ROUND_TIMEOUT"
        printf 'timeout after %ss\n' "$ROUND_TIMEOUT" >>"$run/round-$round.err"
        kill -TERM "$GROK_PID" 2>/dev/null
    ) >/dev/null 2>&1 &
    WATCHDOG_PID=$!
    wait "$GROK_PID" || rc=$?
    pkill -TERM -P "$WATCHDOG_PID" 2>/dev/null || true
    kill "$WATCHDOG_PID" 2>/dev/null || true
    wait "$WATCHDOG_PID" 2>/dev/null || true
    GROK_PID=""
    WATCHDOG_PID=""
    printf '%s\n' "$round" >"$run/round"

    tree_of "$run/worktree" "$run/after.idx" >"$run/after-$round.tree"
    git -C "$run/worktree" diff --binary \
        "$(cat "$run/base.tree")" "$(cat "$run/after-$round.tree")" >"$run/round-$round.diff"

    local tripped=no
    if [ "$(main_state)" != "$before" ]; then
        tripped=yes
        printf 'yes\n' >"$run/tripped"
    fi

    python3 - "$run" "$round" "$rc" "$tripped" "$HERE/report-schema.json" <<'PY'
import json, pathlib, sys
run, round_, rc, tripped = pathlib.Path(sys.argv[1]), sys.argv[2], int(sys.argv[3]), sys.argv[4]
schema = json.loads(pathlib.Path(sys.argv[5]).read_text(encoding="utf-8"))


def check(value, node, where="отчёт"):
    """Stdlib-only check of the subset of JSON Schema that report-schema.json
    uses: type, required, properties, additionalProperties, items. Returns a
    list of problems; empty means the value fits."""
    kind = node.get("type")
    types = {"object": dict, "array": list, "string": str}
    # A keyword this checker does not know would be skipped silently and
    # let a wrong report through; stop instead.
    unknown = set(node) - {"type", "required", "properties", "additionalProperties", "items", "description"}
    if unknown or kind not in types:
        sys.exit(f"report-schema.json: checker does not support {sorted(unknown) or kind!r}; extend check()")
    if kind in types and not isinstance(value, types[kind]):
        return [f"{where}: ожидался {kind}, получен {type(value).__name__}"]
    problems = []
    if kind == "object":
        props = node.get("properties", {})
        for key in node.get("required", []):
            if key not in value:
                problems.append(f"{where}: нет обязательного поля {key}")
        for key, item in value.items():
            if key in props:
                problems += check(item, props[key], f"{where}.{key}")
            elif node.get("additionalProperties") is False:
                problems.append(f"{where}: лишнее поле {key}")
    elif kind == "array" and "items" in node:
        for number, item in enumerate(value):
            problems += check(item, node["items"], f"{where}[{number}]")
    return problems


raw = (run / f"round-{round_}.json").read_text(encoding="utf-8")
print(f"запуск {run.name}, раунд {round_}, код выхода grok {rc}")
try:
    out = json.loads(raw)
except json.JSONDecodeError:
    print("ответ не JSON; первые строки:")
    print(raw[:800])
    err = (run / f"round-{round_}.err").read_text(encoding="utf-8", errors="replace")
    print("stderr:", err[:800])
    sys.exit(0)
session = out.get("sessionId")
if session:
    (run / "session").write_text(session + "\n", encoding="utf-8")
usage = out.get("usage") or {}
print(f"остановка: {out.get('stopReason')}  сессия: {session}")
print(f"токены: вход {usage.get('input_tokens')}, из кэша {usage.get('cache_read_input_tokens')}, выход {usage.get('output_tokens')}")
if tripped == "yes":
    print("РАСТЯЖКА: настоящее рабочее дерево изменилось за время раунда. Раунд не принимается.")
if out.get("stopReason") == "cancelled":
    print("ЗАПУСК ОБОРВАН: стажёр попытался сделать то, что ему запрещено.")
# The schema is not passed to grok (the CLI applies it to every assistant
# message and ends the run after the first one), so the text is free prose and
# the report is asked for in the prompt, once, at the very end. The report is
# the last JSON object in the text that fits the schema. An object that does
# not fit (an early {"status": ...}, a report missing a field) is not a
# report, even if it is the only JSON there is. Only top-level objects are
# examined: after an object parses, scanning resumes after it.
text = out.get("text") or ""
found, rejected, index, decoder = [], [], 0, json.JSONDecoder()
while True:
    index = text.find("{", index)
    if index < 0:
        break
    try:
        value, end = decoder.raw_decode(text, index)
    except json.JSONDecodeError:
        index += 1
        continue
    problems = check(value, schema)
    if problems:
        rejected.append(problems)
    else:
        found.append(value)
    index = end
if found:
    report = found[-1]
    (run / f"report-{round_}.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    label = "отчёт стажёра" if out.get("stopReason") == "end_turn" else "отчёт найден, но запуск не завершён (stopReason не end_turn) — не принимать"
    print(f"{label}:")
    print(json.dumps(report, ensure_ascii=False, indent=2))
else:
    print("ОТЧЁТА НЕТ: в тексте нет JSON-объекта по схеме report-schema.json. Запуск не принят.")
    for problems in rejected:
        print("  отклонён объект:", "; ".join(problems[:3]))
    print("текст:", text[:800])
PY
    printf '\nизменения стажёра:\n'
    git -C "$run/worktree" diff --stat \
        "$(cat "$run/base.tree")" "$(cat "$run/after-$round.tree")" || true
}

cmd_start() {
    local task="${1:?укажите файл задачи}"
    [ -f "$task" ] || die "нет файла задачи: $task"
    [ -x "$GROK" ] || die "grok не найден: $GROK"

    local id run
    id="$(date +%Y%m%d-%H%M%S)-$$"
    run="$PAIR_HOME/$id"
    mkdir -p "$run"
    trap 'on_interrupt "$run"' INT TERM

    # The copy starts where the real tree is now, uncommitted work included:
    # HEAD, then the uncommitted diff, then untracked files that are not
    # ignored. Grok works on what the owner sees, not on the last commit.
    git -C "$REPO" worktree add --detach --quiet "$run/worktree" HEAD
    git -C "$REPO" diff HEAD --binary >"$run/uncommitted.patch"
    if [ -s "$run/uncommitted.patch" ]; then
        git -C "$run/worktree" apply --binary "$run/uncommitted.patch"
    fi
    (
        cd "$REPO"
        git ls-files --others --exclude-standard -z | tar --null -T - -cf -
    ) | tar -xf - -C "$run/worktree"

    tree_of "$run/worktree" "$run/base.idx" >"$run/base.tree"
    main_state >"$run/main.state"
    cp "$task" "$run/task.md"
    cp "$REPO/AGENTS.md" "$run/rules.md"
    printf '0\n' >"$run/round"

    run_round "$run" "$run/task.md" ""
    printf '\nRUN=%s\n' "$id"
}

cmd_resume() {
    local run feedback="${2:?укажите файл замечаний}"
    run="$(run_dir "${1:-}")"
    [ -f "$feedback" ] || die "нет файла замечаний: $feedback"
    [ -f "$run/session" ] || die "у запуска нет сессии grok — возвращать некуда"
    trap 'on_interrupt "$run"' INT TERM
    local body="$run/feedback-$(($(cat "$run/round") + 1)).md"
    {
        printf '## Замечания шефа\n\n'
        cat "$feedback"
    } >"$body"
    run_round "$run" "$body" "$(cat "$run/session")"
}

# The chef's gates, run in the intern's copy. `quick` runs the tests of the
# crates the change touched instead of the whole workspace; the full run is
# what a change is accepted on.
cmd_check() {
    local run scope="${2:-full}"
    run="$(run_dir "${1:-}")"
    local round out
    round="$(cat "$run/round")"
    out="$run/check-$round.txt"
    local wt="$run/worktree"
    export CARGO_TARGET_DIR="$run/target"
    : >"$out"
    step() {
        local label="$1"
        shift
        local rc=0
        printf '$ %s\n' "$*" >>"$out"
        (cd "$wt" && "$@") >>"$out.log" 2>&1 || rc=$?
        printf '  %-44s код %s\n' "$label" "$rc" | tee -a "$out"
        return 0
    }
    count_tests() {
        python3 - "$out.log" <<'PY' | tee -a "$out"
import re, sys
ok = fail = bins = bad = 0
for line in open(sys.argv[1], encoding="utf-8", errors="replace"):
    m = re.match(r"test result: (ok|FAILED)\. (\d+) passed; (\d+) failed", line)
    if m:
        bins += 1
        ok += int(m.group(2))
        fail += int(m.group(3))
        bad += m.group(1) == "FAILED"
print(f"  тесты: бинарей {bins}, упавших бинарей {bad}, прошло {ok}, упало {fail}")
PY
    }
    : >"$out.log"
    step "cargo fmt --all -- --check" cargo fmt --all -- --check
    step "git diff --check" git diff --check
    if [ "$scope" = quick ]; then
        local crates
        crates="$(git -C "$wt" diff --name-only "$(cat "$run/base.tree")" -- crates |
            cut -d/ -f2 | sort -u)"
        local crate
        for crate in $crates; do
            step "cargo test -p $crate --locked" cargo test -p "$crate" --locked
        done
    else
        step "cargo clippy … -D warnings" cargo clippy --workspace --all-targets --locked -- -D warnings
        step "cargo test --workspace --locked" cargo test --workspace --locked
        step "cargo test -p sciwhisper-core --release" cargo test -p sciwhisper-core --release --locked
    fi
    count_tests
    grep -E '^(---- |error(\[|:)|test result: FAILED)' "$out.log" | head -40 >>"$out" || true
    printf 'полный вывод: %s\n' "$out.log" | tee -a "$out"
    du -sh "$CARGO_TARGET_DIR" 2>/dev/null | sed 's/^/  сборка во временном каталоге: /' | tee -a "$out"
}

cmd_diff() {
    local run
    run="$(run_dir "${1:-}")"
    local round
    round="$(cat "$run/round")"
    cat "$run/round-$round.diff"
}

# Puts the accepted change into the real working tree, uncommitted.
#
# Two refusals. A round that tripped the wire is never applied: something
# wrote to the real tree while Grok was working, and nobody can say it was
# not Grok. And every file the change touches must still be what it was when
# the task started — other files may have moved on, but applying on top of
# an edit to the same file would be merging over somebody's work unseen.
cmd_apply() {
    local run
    run="$(run_dir "${1:-}")"
    local round
    round="$(cat "$run/round")"
    [ ! -f "$run/tripped" ] ||
        die "в одном из раундов сработала растяжка; такую работу не применяют"
    local patch="$run/round-$round.diff"
    [ -s "$patch" ] || die "стажёр ничего не изменил — применять нечего"

    local base after path moved=""
    base="$(cat "$run/base.tree")"
    after="$(cat "$run/after-$round.tree")"
    while IFS= read -r -d '' path; do
        local was now
        was="$(git -C "$run/worktree" rev-parse --quiet --verify "$base:$path" 2>/dev/null || printf 'absent')"
        if [ -e "$REPO/$path" ]; then
            now="$(git -C "$REPO" hash-object -- "$path")"
        else
            now=absent
        fi
        [ "$was" = "$now" ] || moved="$moved  $path"$'\n'
    done < <(git -C "$run/worktree" diff --name-only -z "$base" "$after")
    [ -z "$moved" ] ||
        die "эти файлы изменились в рабочем дереве после старта задачи, поверх них применять нельзя:"$'\n'"$moved"

    git -C "$REPO" apply --check --binary "$patch"
    git -C "$REPO" apply --binary "$patch"
    printf 'применено в рабочее дерево без коммита:\n'
    git -C "$REPO" apply --stat "$patch"
}

cmd_clean() {
    local run
    run="$(run_dir "${1:-}")"
    git -C "$REPO" worktree remove --force "$run/worktree" 2>/dev/null || true
    git -C "$REPO" worktree prune
    rm -rf "${run:?}"
    printf 'удалено: %s\n' "$run"
}

cmd_list() {
    [ -d "$PAIR_HOME" ] || {
        printf 'запусков нет\n'
        return
    }
    local dir
    for dir in "$PAIR_HOME"/*/; do
        [ -d "$dir" ] || continue
        printf '%s  раундов: %s%s%s\n' "$(basename "$dir")" "$(cat "$dir/round" 2>/dev/null || printf '?')" \
            "$([ -f "$dir/tripped" ] && printf '  РАСТЯЖКА')" \
            "$([ -f "$dir/interrupted" ] && printf '  ПРЕРВАН')"
    done
}

case "${1:-}" in
start) cmd_start "${2:-}" ;;
check) cmd_check "${2:-}" "${3:-full}" ;;
resume) cmd_resume "${2:-}" "${3:-}" ;;
diff) cmd_diff "${2:-}" ;;
apply) cmd_apply "${2:-}" ;;
clean) cmd_clean "${2:-}" ;;
list) cmd_list ;;
*)
    # The comment block at the top of this file, and nothing after it.
    awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$0"
    exit 2
    ;;
esac
