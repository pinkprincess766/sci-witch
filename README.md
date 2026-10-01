# sci-witch

[Русская версия](README.ru.md)

**sci-witch** studies how spoken Russian scientific language — chemistry, mathematics, physics — can be compiled into a typed structure, and when that compilation should refuse. The object of study is the path from an utterance to one tree (or to the original words). The crates and the `sciwhisper` binary are internal names; the public name of the project is sci-witch.

The compiler's contract is [docs/compiler/COMPILER_CONTRACT_RU.md](docs/compiler/COMPILER_CONTRACT_RU.md). The research programme, including both experimental questions and the live-speech critical path, is [docs/research/sci-witch-plan.md](docs/research/sci-witch-plan.md). Related fields, the mathematical formulation and where to look for prior work: [research/LITERATURE_MAP_RU.md](research/LITERATURE_MAP_RU.md).

## Research questions

| # | Question | Status | Still untested on |
|---|---|---|---|
| 1 | Does selecting among *K* Whisper hypotheses by a combined ASR + grammar score raise AST exact match on live speakers, relative to top-1 Whisper plus the current parser (`interpret_utterance`)? | Open. The recording protocol is preregistered; this comparison is not. | Live speakers, Whisper *N*-best lists, word error rate, a PCFG score. The plan requires a dedicated preregistration in `research/` before a frozen-test run, on the model of [research/reranker/program.md](research/reranker/program.md). |
| 2 | Does a fitted confidence model (ASR and grammar features) give lower risk at the same coverage than the four hand-set parse levels 1.0 / 0.95 / 0.7 / 0.0 with insert threshold 0.9? | Open. | Live speakers, and any corpus with at least `MIN_ERRORS_TO_FIT` (20) errors among answered items ([`crates/sciwhisper-eval/src/selective.rs`](crates/sciwhisper-eval/src/selective.rs)). On `dev-seed-v3`, `selective_prediction.calibration.errors` is 1 ([research/results/deterministic-v6.json](research/results/deterministic-v6.json)). |
| 3 | Can it be guaranteed that ordinary speech is left as words — that a substance name inside a sentence is not replaced by a formula (severity S4 in [research/schema/severity-v1.json](research/schema/severity-v1.json))? | Preliminary answer on author-written text: observed S4 count 0 and false-rewrite numerator 0 on the four benchmark corpora, with the Wilson upper bounds in the table below. | Live speakers, other authors' phrasing, recognizer output. The prose corpus is a floor on the author's own traps ([research/data/prose-negatives-v1.manifest.json](research/data/prose-negatives-v1.manifest.json)). |

Question 1 and question 2 are stated in [docs/research/sci-witch-plan.md](docs/research/sci-witch-plan.md) (постановка №1, постановка №2). Question 3 is the safety claim the compiler gates already judge on text ([research/schema/compiler-gates-v1.json](research/schema/compiler-gates-v1.json)).

The voice-collection protocol [research/protocol/voice-v1.md](research/protocol/voice-v1.md) is a preregistration of *how* the first live recordings will be taken (five speakers `spk01`–`spk05`, 220 clips, test held out by speaker). It is not a preregistration of questions 1 or 2. Status there: **preregistration, no recordings yet.**

`confidence` in this repository is a parse level. The compiler emits the four values 1.0 (exact shortcut), 0.95 (parse without warnings), 0.7 (parse with warnings) and 0.0 (failure), in [`crates/sciwhisper-core/src/interpret.rs`](crates/sciwhisper-core/src/interpret.rs). The insert threshold for a span inside a sentence is `INLINE_MIN_CONFIDENCE = 0.9` in [`crates/sciwhisper-core/src/utterance.rs`](crates/sciwhisper-core/src/utterance.rs).

## Approach

```text
audio → log-Mel spectrogram → Whisper → text
      → candidate readings → typed AST
      → chemistry / syntax checks → Unicode / LaTeX / Word (OMML)
```

The measured artefact is the text compiler. [`sciwhisper-core`](crates/sciwhisper-core) has no audio dependency. Input is a string (recognizer, keyboard, or paste). Output is one tree and three views of that tree, or the original words.

| Call | File | Role |
|---|---|---|
| `interpret` | [`crates/sciwhisper-core/src/interpret.rs`](crates/sciwhisper-core/src/interpret.rs) | The whole string as one construct, or refusal. |
| `interpret_utterance` | [`crates/sciwhisper-core/src/utterance.rs`](crates/sciwhisper-core/src/utterance.rs) | Ordinary words stay; proven spans are replaced. |
| `render` | [`crates/sciwhisper-core/src/render/mod.rs`](crates/sciwhisper-core/src/render/mod.rs) | Unicode, LaTeX, or OMML from the same AST. Renderers do not parse the transcript again. |
| lattice | [`crates/sciwhisper-core/src/lattice.rs`](crates/sciwhisper-core/src/lattice.rs) | Candidate readings with origin; no winner. |
| `choose_hypothesis` | [`crates/sciwhisper-core/src/nbest.rs`](crates/sciwhisper-core/src/nbest.rs) | One text hypothesis from an *n*-best list. |

Refusal is a result:

- A span is inserted only at parse level ≥ `INLINE_MIN_CONFIDENCE` (0.9).
- A substance name inside ordinary prose is treated as a mention, and so is an amount (a number with a unit: «один моль бензола»). The rule lives in `is_bare_substance`, `is_bare_quantity` and `MIN_ENUMERATION_SPANS` in [`utterance.rs`](crates/sciwhisper-core/src/utterance.rs).
- `balance_equation` ([`crates/sciwhisper-core/src/balance.rs`](crates/sciwhisper-core/src/balance.rs)) may propose coefficients from *Aν* = 0 as a warning. The dictated reaction is left as said.
- `choose_hypothesis` ([`crates/sciwhisper-core/src/nbest.rs`](crates/sciwhisper-core/src/nbest.rs)) reads at most `MAX_HYPOTHESES` (16) entries, counting the first. The first hypothesis is kept when it is already a whole-utterance reading (`TopAlreadyParsed`); that check runs before any other. A later hypothesis replaces it only when the first did not parse as a whole utterance and exactly one near hypothesis (within `MAX_HYPOTHESIS_EDITS` (2) characters) did parse as a whole utterance and shares a scientific word with the first (`NearHypothesisParsed`). The first is also kept on `NearHypothesesDisagree`, `NoScientificAnchor`, `NothingNearParsed`, and `Empty`. Strings longer than `lattice::MAX_INPUT_BYTES` (16 KiB) are not compared. There is no acoustic score in this choice.

Chemical nomenclature is the documented subset in [docs/compiler/CHEMISTRY_NOMENCLATURE_RU.md](docs/compiler/CHEMISTRY_NOMENCLATURE_RU.md). The formulas of the implemented algorithms are in [docs/compiler/MATHEMATICS_RU.md](docs/compiler/MATHEMATICS_RU.md).

## What is measured / what is not

**Live speakers recorded: 0.** Each of the four benchmark reports sets `split_audit.speakers` to 0.

**WER: unmeasured.** [`crates/sciwhisper-eval`](crates/sciwhisper-eval) has no word-error metric.

**Authorship.** Every benchmark record is `handcrafted_text`, written by the author of the parser. Gold AST labels were written by hand. Passing these corpora does not measure live speech, other people's phrasing, noise, accent, or latency.

All numbers below are text in, AST out. Intervals are the `ci95_low` / `ci95_high` fields (`ci_method`: `wilson`), shown to three decimal places (lower bound rounded down, upper bound rounded up). For a zero numerator the reports also store `zero_count_upper95`, rounded up the same way. Each of the four reports names `program.git_commit` `f6b6d8104643d6d074eb7bb34c92f90a95bde8cc` and `program.git_dirty` `true`.

In each report, `user_path.false_scientific_rewrite_rate` has the same numerator and denominator as `metrics.false_scientific_rewrite_rate`.

| Corpus | Records | Metric | Value | 95% Wilson interval | Source |
|---|---:|---|---|---|---|
| `dev-seed-v3` | 113 | `metrics.ast_exact_match` | 112/113 | [0.951, 0.999] | [research/results/deterministic-v6.json](research/results/deterministic-v6.json) |
| `dev-seed-v3` | 113 | `metrics.false_scientific_rewrite_rate` | 0/29 | [0.0, 0.117]; `zero_count_upper95` 0.099 | same |
| `dev-seed-v3` | 113 | `severity.count_by_severity.S4` | 0 | | same |
| `dev-seed-v3` | 113 | `split_audit.speakers` | 0 | | same |
| `ambiguous-v2` | 58 | `metrics.ast_exact_match` | 36/58 | [0.492, 0.735] | [research/results/ambiguous-auto-v2.json](research/results/ambiguous-auto-v2.json) |
| `ambiguous-v2` | 58 | `metrics.false_scientific_rewrite_rate` | 0/20 | [0.0, 0.162]; `zero_count_upper95` 0.140 | same |
| `ambiguous-v2` | 58 | `severity.count_by_severity.S4` | 0 | | same |
| `ambiguous-v2` | 58 | `split_audit.speakers` | 0 | | same |
| `nomenclature-v1` | 53 | `metrics.ast_exact_match` | 53/53 | [0.932, 1.0] | [research/results/nomenclature-v1.json](research/results/nomenclature-v1.json) |
| `nomenclature-v1` | 53 | `metrics.false_scientific_rewrite_rate` | 0/27 | [0.0, 0.125]; `zero_count_upper95` 0.106 | same |
| `nomenclature-v1` | 53 | `severity.count_by_severity.S4` | 0 | | same |
| `nomenclature-v1` | 53 | `split_audit.speakers` | 0 | | same |
| `prose-negatives-v2` | 180 | `metrics.ast_exact_match` | 180/180 | [0.979, 1.0] | [research/results/prose-negatives-v2.json](research/results/prose-negatives-v2.json) |
| `prose-negatives-v2` | 180 | `metrics.false_scientific_rewrite_rate` | 0/180 | [0.0, 0.021]; `zero_count_upper95` 0.017 | same |
| `prose-negatives-v2` | 180 | `severity.count_by_severity.S4` | 0 | | same |
| `prose-negatives-v2` | 180 | `split_audit.speakers` | 0 | | same |

Exact values are in the JSON report linked in the last column.

The compiler-gate profile `compiler-v2` ([research/schema/compiler-gates-v2.json](research/schema/compiler-gates-v2.json)) pools the four reports into one sample before judging: counts are summed, proportions re-divided from the sums. Ordinary-speech denominators of `metrics.false_scientific_rewrite_rate` are 29 + 20 + 27 + 180 = 256 ([research/results/deterministic-v6.json](research/results/deterministic-v6.json), [research/results/ambiguous-auto-v2.json](research/results/ambiguous-auto-v2.json), [research/results/nomenclature-v1.json](research/results/nomenclature-v1.json), [research/results/prose-negatives-v2.json](research/results/prose-negatives-v2.json)); false scientific rewrites 0/256. The one-sided 95% upper bound at zero observations is 0.0116 (`1 − 0.05^(1/n)` with n = 256). All six checks passed: the five gates declared in the `gates` array of `compiler-gates-v2.json`, and the benchmark-coverage check that the code adds (`BENCHMARK_GATE_ID` in [`crates/sciwhisper-eval/src/gate.rs`](crates/sciwhisper-eval/src/gate.rs)). These 256 sentences were written for this project, by the author of the parser and the agents working on it; the bound applies to this set, not to live speech. The previous profile, `compiler-v1` ([research/schema/compiler-gates-v1.json](research/schema/compiler-gates-v1.json)), pinned `prose-negatives-v1` instead and is kept unchanged.

`dev-seed-v3` is the main development seed (chemistry, mathematics, physics, and ordinary-speech RAW items). `ambiguous-v2` is a deliberately adversarial set; its rate is not comparable with `dev-seed-v3` as an overall figure ([research/data/ambiguous-v2.manifest.json](research/data/ambiguous-v2.manifest.json)). `nomenclature-v1` covers the documented nomenclature subset. `prose-negatives-v1` is ordinary scientific prose whose gold is the sentence itself; AST exact match 146/146 on that corpus is the sentences being left alone.

The one miss on `dev-seed-v3` is S1 (`severity.errors` 1, `count_by_severity.S1` 1): `math-root-atom-001-a`, transcript `корень из икс плюс один`. On `ambiguous-v2`, `severity.errors` is 24, all S1; `metrics.false_abstention_rate` is 23/38.

These four reports are the compiler benchmark pinned in [research/schema/compiler-gates-v2.json](research/schema/compiler-gates-v2.json). Config in every report: `domain_policy` `auto`, `auto_insert_threshold` 0.9, `evaluated_split` `all`, `bootstrap_seed` 20260904, `bootstrap_resamples` 2000.

Three synthesized clips live under [`research/voice-samples/synthetic/`](research/voice-samples/synthetic/). They are pipeline fixtures, not a speaker study ([`research/voice-samples/README.md`](research/voice-samples/README.md)).

## Data

Published corpora are frozen by SHA-256. A published file is not rewritten under the same name; a new version gets a new id and a new manifest. Records live in `research/data/<corpus_id>.jsonl` next to each manifest.

| Corpus | Records | Origin | SHA-256 | Manifest |
|---|---:|---|---|---|
| [`dev-seed-v3`](research/data/dev-seed-v3.jsonl) | 113 | Derived from `dev-seed-v2` by [research/data/apply_prose_policy.py](research/data/apply_prose_policy.py). Gold written by hand; the builder does not call `sciwhisper-core`. | `590f0c3d17d998dd6f2df32ee07c56f01a8a2866f68953f289883b553779c88d` | [research/data/dev-seed-v3.manifest.json](research/data/dev-seed-v3.manifest.json) |
| [`ambiguous-v2`](research/data/ambiguous-v2.jsonl) | 58 | Derived from `ambiguous-v1` by the same script. Gold written by hand before the candidate lattice existed. | `d10f8d5aa11e55abf787c8ca01b4df19c01cb929ed034fbc7559849fb74684bc` | [research/data/ambiguous-v2.manifest.json](research/data/ambiguous-v2.manifest.json) |
| [`nomenclature-v1`](research/data/nomenclature-v1.jsonl) | 53 | Written directly as JSONL; no builder script. Manifest added on 2026-09-28; the data file was left unchanged. | `a2e265d7c97a69858d662c4a99c9f1e9e31e89f61e0737a4c75303736a14e528` | [research/data/nomenclature-v1.manifest.json](research/data/nomenclature-v1.manifest.json) |
| [`prose-negatives-v1`](research/data/prose-negatives-v1.jsonl) | 146 | Built by [research/data/build_prose_negatives.py](research/data/build_prose_negatives.py). Gold is the transcript. | `185155162b0750b3246872387e9fad1eb71984dde10bc7267635682a6298c451` | [research/data/prose-negatives-v1.manifest.json](research/data/prose-negatives-v1.manifest.json) |
| [`prose-negatives-v2`](research/data/prose-negatives-v2.jsonl) | 180 | v1 unchanged plus 34 sentences for the traps found on 2026-09-30 (amounts in prose, new units, a conjunction between operands, letter names); built by [research/data/build_prose_negatives_v2.py](research/data/build_prose_negatives_v2.py). Gold is the transcript. | `47caffedb258c2207c3ffe8604092a16cb9144f4889e503b09b13c09b77b621b` | [research/data/prose-negatives-v2.manifest.json](research/data/prose-negatives-v2.manifest.json) |

Earlier published snapshots, kept byte-for-byte:

| Corpus | Records | SHA-256 | Manifest |
|---|---:|---|---|
| [`dev-seed-v1`](research/data/dev-seed-v1.jsonl) | 113 | `70ee77e24e3b17980b33d894c564bb306b37392f9dca7ca97ae276e77417f490` | [research/data/dev-seed-v1.manifest.json](research/data/dev-seed-v1.manifest.json) |
| [`dev-seed-v2`](research/data/dev-seed-v2.jsonl) | 113 | `4a8bfa082134a8093d6bbf50afaa5181bf5ed25c56bd3b6a6db2b575611e6b24` | [research/data/dev-seed-v2.manifest.json](research/data/dev-seed-v2.manifest.json) |
| [`ambiguous-v1`](research/data/ambiguous-v1.jsonl) | 58 | `29683beb85f7055633e1cc7ba8756fa95d1c5f2d2a6237b4c5c496a4af689e2e` | [research/data/ambiguous-v1.manifest.json](research/data/ambiguous-v1.manifest.json) |

`asr_hypotheses` are empty and `speaker_id` is null on these text corpora. Records tagged `asr-error` in `ambiguous-v2` are hand-written spellings of mistakes observed in Russian Whisper output, not recognizer samples.

**Voice protocol.** [research/protocol/voice-v1.md](research/protocol/voice-v1.md) and the frozen family list [research/protocol/voice-v1-families.json](research/protocol/voice-v1-families.json) (built from [research/data/dev-seed-v2.jsonl](research/data/dev-seed-v2.jsonl), [research/data/ambiguous-v1.jsonl](research/data/ambiguous-v1.jsonl), [research/data/nomenclature-v1.jsonl](research/data/nomenclature-v1.jsonl) at commit `f6b6d8104643d6d074eb7bb34c92f90a95bde8cc`). Five speakers, two family blocks, test = speakers `spk04`–`spk05` in `dev_holdout`. WAV 16-bit mono 16 kHz. Collection CLI: `sciwhisper collect-voice` ([`crates/sciwhisper-cli/src/collect_voice.rs`](crates/sciwhisper-cli/src/collect_voice.rs)). Notes on how a live corpus would be built: [research/data/VOICE_CORPUS_RU.md](research/data/VOICE_CORPUS_RU.md), [docs/user/VOICE_COLLECTION_RU.md](docs/user/VOICE_COLLECTION_RU.md).

**Consent drafts** (engineer-written; a lawyer has not reviewed them):

- [research/data/consent-ru-v1.md](research/data/consent-ru-v1.md) — local recording, no publication.
- [research/data/consent-ru-v2.md](research/data/consent-ru-v2.md) — recording and publication (CC BY 4.0). Required for an open voice set.

## Reproducing

Rust 1.98.0, pinned in [`rust-toolchain.toml`](rust-toolchain.toml). No model and no microphone. The commands below use the same settings as the published reports (`domain_policy` `auto`, `auto_insert_threshold` 0.9, `evaluated_split` `all`, `bootstrap_seed` 20260904, `bootstrap_resamples` 2000).

```bash
cargo run -p sciwhisper-eval --locked -- evaluate \
  --dataset research/data/dev-seed-v3.jsonl \
  --policy auto --split all --threshold 0.9 --k 4 \
  --bootstrap-seed 20260904 --bootstrap-resamples 2000 \
  --quiet --output /tmp/dev-seed-v3.json

cargo run -p sciwhisper-eval --locked -- evaluate \
  --dataset research/data/ambiguous-v2.jsonl \
  --policy auto --split all --threshold 0.9 --k 4 \
  --bootstrap-seed 20260904 --bootstrap-resamples 2000 \
  --quiet --output /tmp/ambiguous-v2.json

cargo run -p sciwhisper-eval --locked -- evaluate \
  --dataset research/data/nomenclature-v1.jsonl \
  --policy auto --split all --threshold 0.9 --k 4 \
  --bootstrap-seed 20260904 --bootstrap-resamples 2000 \
  --quiet --output /tmp/nomenclature-v1.json

cargo run -p sciwhisper-eval --locked -- evaluate \
  --dataset research/data/prose-negatives-v2.jsonl \
  --policy auto --split all --threshold 0.9 --k 4 \
  --bootstrap-seed 20260904 --bootstrap-resamples 2000 \
  --quiet --output /tmp/prose-negatives-v2.json
```

Compare `metrics.ast_exact_match`, `metrics.false_scientific_rewrite_rate`, `severity.count_by_severity.S4`, `split_audit.speakers`, and `dataset.sha256` with the table and the manifests. Timing fields may differ.

Compiler gates, the default profile of `gate`:

```bash
cargo run -p sciwhisper-eval --locked -- gate \
  --report research/results/deterministic-v6.json \
  --report research/results/ambiguous-auto-v2.json \
  --report research/results/nomenclature-v1.json \
  --report research/results/prose-negatives-v2.json
```

The same check on the published reports runs as the test `the_published_benchmark_reports_pass` in [`crates/sciwhisper-eval/src/gate.rs`](crates/sciwhisper-eval/src/gate.rs). The pinned digests are checked against the files on disk by `the_pinned_digests_match_the_corpora_on_disk` in the same file.

The voice-application profile `release-0.5` ([research/schema/release-gates-v1.json](research/schema/release-gates-v1.json)) still requires `real_audio` and a sealed frozen test. There is no `research/results/voice-v1.json` in this tree.

## Roadmap

From [docs/research/sci-witch-plan.md](docs/research/sci-witch-plan.md):

| Stage | What |
|---|---|
| 1 | Hygiene and storefront: toolchain pin, English README, one public name. |
| 2 | Live recordings under `voice-v1`, WER/CER, speaker-cluster intervals, oracle transcript vs Whisper. |
| 3А | Explicit grammar (EBNF from the parser code), Earley reference, PCFG. |
| 3Б | Rescoring Whisper *N*-best with grammar (question 1). |
| 3В | Fitted parse-level calibration (question 2). |
| 3Г | Balance-equation properties (`proptest`, overflow bound). |
| 4 | Technical report, voice dataset with datasheet, one-command reproduction. |
| 5 | Mathematical background, in parallel with the stages that use it. |

**Critical path: live speakers.** Questions 1 and 2 wait on recordings. Stages 3А and 3Г can proceed on text while recording is arranged. A logistic lattice reranker is preregistered in [research/reranker/program.md](research/reranker/program.md) and has not started: each of the four benchmark reports sets `metrics.reranker_readiness.verdict` to `insufficient`.

## Using the software

```bash
cargo build --release
cargo run -p sciwhisper-cli -- format --domain chemistry \
  "гидроксид меди два превращается в оксид меди два плюс вода"
cargo run -p sciwhisper-cli -- nbest "карбанат кальция" "карбонат кальция"
```

`format` compiles text already in hand (Whisper is not invoked). `nbest` chooses one hypothesis, then compiles it. A phrase that does not parse is left as said; `format` on `предел терпения` prints the original words and then exits with code 1 (`could not parse input; raw transcript preserved`).

The tray application (double-tap Control to record, insert into the focused window, optional Word equation) ships in this repository as a **reference frontend**. Compiler gates do not judge it. Click-by-click use and settings: [README.ru.md](README.ru.md). Windows portable-archive notes: [packaging/windows/README-WINDOWS.txt](packaging/windows/README-WINDOWS.txt). Known frontend limits: [docs/user/KNOWN_LIMITATIONS_RU.md](docs/user/KNOWN_LIMITATIONS_RU.md).

```text
crates/sciwhisper-core     AST, lexicons, parser, renderers
crates/sciwhisper-eval     corpora, metrics, compiler gates
crates/sciwhisper-cli      format / nbest / rec / collect-voice
crates/sciwhisper-asr      speech recognition, resident model, microphone
crates/sciwhisper-shell    tray, hotkey, clipboard, Word
crates/sciwhisper-update   install and replace an existing build
```

Stable surface of the compiler: re-exports in [`crates/sciwhisper-core/src/lib.rs`](crates/sciwhisper-core/src/lib.rs), pinned by [`crates/sciwhisper-core/tests/public_surface.rs`](crates/sciwhisper-core/tests/public_surface.rs).

## Limitations

- Live speakers: 0. WER: unmeasured. Gold and transcripts: author-written. The compiler-gate zeros are counts on this benchmark, with the Wilson upper bounds above.
- Questions 1 and 2 are open. `choose_hypothesis` has been exercised on synthetic text lists without acoustic scores.
- `confidence` is a four-level parse flag. [`crates/sciwhisper-eval/src/selective.rs`](crates/sciwhisper-eval/src/selective.rs) reports risk–coverage and refuses to fit a threshold below `MIN_ERRORS_TO_FIT` (20).
- Chemical nomenclature is the subset in [docs/compiler/CHEMISTRY_NOMENCLATURE_RU.md](docs/compiler/CHEMISTRY_NOMENCLATURE_RU.md). Organic names are not searched for inside a sentence (so that «декан факультета» is not read as a hydrocarbon).
- The four published reports were written from a dirty worktree (`git_dirty`: `true`).
- OMML generation ([`crates/sciwhisper-core/tests/acceptance.rs`](crates/sciwhisper-core/tests/acceptance.rs), [`crates/sciwhisper-core/tests/functions.rs`](crates/sciwhisper-core/tests/functions.rs)) and insert-mode selection (`resolve_mode` in [`crates/sciwhisper-shell/src/insert.rs`](crates/sciwhisper-shell/src/insert.rs)) are covered by tests. Word insertion through COM ([`crates/sciwhisper-shell/src/word_win.rs`](crates/sciwhisper-shell/src/word_win.rs), `#[cfg(windows)]`) is not covered by automatic tests and has not been checked by hand.
- Windows builds of the reference frontend are unsigned. macOS builds are ad-hoc signed, not notarized.
- Speech models and third-party recognizers are outside this repository.

## Licenses

- Code: [Apache-2.0](LICENSE). Notices: [NOTICE](NOTICE).
- Project lexicons and teaching sets in [`crates/sciwhisper-core/data/`](crates/sciwhisper-core/data/): [CC BY 4.0](DATA_LICENSE.md). That file does not cover the corpora in `research/data/` or the clips in `research/voice-samples/`; a license for those is not declared separately.
- Speech models and third-party recognizers are not in this repository and keep their own licenses.
