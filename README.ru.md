# sci-witch

[Английская версия](README.md)

**sci-witch** изучает, как устная русская научная речь — химия, математика, физика — компилируется в типизированную структуру и когда эта компиляция должна отказаться. Предмет изучения — путь от высказывания к одному дереву (или к исходным словам). Крейты и бинарник `sciwhisper` — внутренние имена; публичное имя проекта — sci-witch.

Контракт компилятора — [docs/compiler/COMPILER_CONTRACT_RU.md](docs/compiler/COMPILER_CONTRACT_RU.md). Исследовательская программа, включая обе экспериментальные постановки и критический путь живой речи, — [docs/research/sci-witch-plan.md](docs/research/sci-witch-plan.md). Смежные области, математическая формулировка и где искать предшествующие работы: [research/LITERATURE_MAP_RU.md](research/LITERATURE_MAP_RU.md).

## Исследовательские вопросы

| # | Вопрос | Статус | Ещё не проверено на |
|---|---|---|---|
| 1 | Повышает ли выбор среди *K* гипотез Whisper по совместной оценке ASR и грамматики точное совпадение AST на живых дикторах относительно top-1 Whisper плюс текущий разборщик (`interpret_utterance`)? | Открыт. Процедура записана в [research/protocol/question-1-nbest.md](research/protocol/question-1-nbest.md); прогона нет. Температура, seed и сетка λ в файле пустые, поэтому прогон на замороженном test запрещён. | Живые дикторы, списки Whisper *N*-best, доля ошибок слов, оценка PCFG. |
| 2 | Даёт ли подогнанная модель уверенности (признаки ASR и грамматики) меньший риск при том же покрытии, чем четыре ручных уровня разбора 1.0 / 0.95 / 0.7 / 0.0 с порогом вставки 0.9? | Открыт. Процедура записана в [research/protocol/question-2-calibration.md](research/protocol/question-2-calibration.md); прогона нет. | Живые дикторы и любой корпус, где среди отвеченных пунктов не меньше `MIN_ERRORS_TO_FIT` (20) ошибок ([`crates/sciwhisper-eval/src/selective.rs`](crates/sciwhisper-eval/src/selective.rs)). На `dev-seed-v3` поле `selective_prediction.calibration.errors` равно 1 ([research/results/deterministic-v6.json](research/results/deterministic-v6.json)). |
| 3 | Можно ли гарантировать, что обычная речь остаётся словами — что название вещества внутри предложения не заменяется формулой (серьёзность S4 в [research/schema/severity-v1.json](research/schema/severity-v1.json))? | Предварительный ответ на тексте, написанном автором: наблюдаемое число S4 равно 0 и числитель ложных переписываний равен 0 на четырёх бенчмарк-корпусах, с верхними границами Уилсона в таблице ниже. | Живые дикторы, формулировки других авторов, вывод распознавателя. Корпус прозы — нижняя планка на собственных ловушках автора ([research/data/prose-negatives-v1.manifest.json](research/data/prose-negatives-v1.manifest.json)). |

Вопрос 1 и вопрос 2 сформулированы в [docs/research/sci-witch-plan.md](docs/research/sci-witch-plan.md) (постановка №1, постановка №2). Вопрос 3 — утверждение о безопасности, которое ворота компилятора уже судят на тексте ([research/schema/compiler-gates-v1.json](research/schema/compiler-gates-v1.json)).

Протокол сбора голоса [research/protocol/voice-v1.md](research/protocol/voice-v1.md) — предрегистрация того, *как* будут взяты первые живые записи (пять дикторов `spk01`–`spk05`, 220 клипов, test отложен по дикторам). Это не предрегистрация вопросов 1 или 2. Статус там: **предрегистрация, записей ещё нет.** Сами сравнения записаны в [research/protocol/question-1-nbest.md](research/protocol/question-1-nbest.md) и [research/protocol/question-2-calibration.md](research/protocol/question-2-calibration.md). Прогонов по ним нет.

В этом репозитории `confidence` — уровень разбора. Компилятор выдаёт четыре значения: 1.0 (точное сокращение), 0.95 (разбор без предупреждений), 0.7 (разбор с предупреждениями) и 0.0 (неудача), в [`crates/sciwhisper-core/src/interpret.rs`](crates/sciwhisper-core/src/interpret.rs). Порог вставки фрагмента внутри предложения — `INLINE_MIN_CONFIDENCE = 0.9` в [`crates/sciwhisper-core/src/utterance.rs`](crates/sciwhisper-core/src/utterance.rs).

## Подход

```text
аудио → log-Mel спектрограмма → Whisper → текст
      → кандидаты прочтений → типизированное AST
      → проверки химии и синтаксиса → Unicode / LaTeX / Word (OMML)
```

Измеряемый артефакт — текстовый компилятор. У [`sciwhisper-core`](crates/sciwhisper-core) нет зависимости от аудио. Вход — строка (распознаватель, клавиатура или вставка). Выход — одно дерево и три представления этого дерева, либо исходные слова.

| Вызов | Файл | Роль |
|---|---|---|
| `interpret` | [`crates/sciwhisper-core/src/interpret.rs`](crates/sciwhisper-core/src/interpret.rs) | Вся строка как одна конструкция, либо отказ. |
| `interpret_utterance` | [`crates/sciwhisper-core/src/utterance.rs`](crates/sciwhisper-core/src/utterance.rs) | Обычные слова остаются; доказанные фрагменты заменяются. |
| `render` | [`crates/sciwhisper-core/src/render/mod.rs`](crates/sciwhisper-core/src/render/mod.rs) | Unicode, LaTeX или OMML из того же AST. Рендереры не разбирают транскрипт заново. |
| решётка | [`crates/sciwhisper-core/src/lattice.rs`](crates/sciwhisper-core/src/lattice.rs) | Кандидаты прочтений с происхождением; победителя нет. |
| `choose_hypothesis` | [`crates/sciwhisper-core/src/nbest.rs`](crates/sciwhisper-core/src/nbest.rs) | Одна текстовая гипотеза из списка *n*-best. |

Отказ — это результат:

- Фрагмент вставляется только при уровне разбора ≥ `INLINE_MIN_CONFIDENCE` (0.9).
- Название вещества внутри обычной прозы считается упоминанием, и количество тоже (число с единицей: «один моль бензола»). Ряд названий — список и потому переписывается — только после двоеточия («Примеры: …»), если до конца предложения нет других слов, кроме названий и «и»/«а», или когда предложение состоит из одних названий; глагол в предложении делает его прозой. Правило живёт в `is_bare_substance`, `is_bare_quantity` и `holds_an_enumeration` (с `MIN_ENUMERATION_SPANS`) в [`utterance.rs`](crates/sciwhisper-core/src/utterance.rs).
- `balance_equation` ([`crates/sciwhisper-core/src/balance.rs`](crates/sciwhisper-core/src/balance.rs)) может предложить коэффициенты из *Aν* = 0 как предупреждение. Продиктованная реакция остаётся как сказана.
- `choose_hypothesis` ([`crates/sciwhisper-core/src/nbest.rs`](crates/sciwhisper-core/src/nbest.rs)) читает не больше `MAX_HYPOTHESES` (16) записей, считая первую. Первая гипотеза сохраняется, если она уже является прочтением всего высказывания (`TopAlreadyParsed`); эта проверка выполняется раньше остальных. Более поздняя гипотеза заменяет её только когда первая не разобралась как целое высказывание и ровно одна близкая гипотеза (в пределах `MAX_HYPOTHESIS_EDITS` (2) символов) разобралась как целое высказывание и делит с первой научное слово (`NearHypothesisParsed`). Первая также сохраняется при `NearHypothesesDisagree`, `NoScientificAnchor`, `NothingNearParsed` и `Empty`. Строки длиннее `lattice::MAX_INPUT_BYTES` (16 КиБ) не сравниваются. Акустической оценки в этом выборе нет.

Химическая номенклатура — документированное подмножество в [docs/compiler/CHEMISTRY_NOMENCLATURE_RU.md](docs/compiler/CHEMISTRY_NOMENCLATURE_RU.md). Формулы реализованных алгоритмов — в [docs/compiler/MATHEMATICS_RU.md](docs/compiler/MATHEMATICS_RU.md).

## Что измерено и что нет

**Живых дикторов записано: 0.** В каждом из четырёх бенчмарк-отчётов `split_audit.speakers` равно 0.

**WER: не измерен.** В [`crates/sciwhisper-eval`](crates/sciwhisper-eval) нет метрики ошибок слов.

**Авторство.** Каждая запись бенчмарка — `handcrafted_text`, написанный автором парсера. Метки gold AST записаны вручную. Прохождение этих корпусов не измеряет живую речь, чужие формулировки, шум, акцент или задержку.

Все числа ниже — текст на входе, AST на выходе. Интервалы — поля `ci95_low` / `ci95_high` (`ci_method`: `wilson`), показанные до трёх знаков после запятой (нижняя граница округлена вниз, верхняя — вверх). При нулевом числителе отчёты также хранят `zero_count_upper95`, округлённый вверх тем же способом. Каждый из четырёх отчётов называет `program.git_commit` `f6b6d8104643d6d074eb7bb34c92f90a95bde8cc` и `program.git_dirty` `true`.

В каждом отчёте у `user_path.false_scientific_rewrite_rate` те же числитель и знаменатель, что у `metrics.false_scientific_rewrite_rate`.

| Корпус | Записей | Метрика | Значение | 95 % интервал Уилсона | Источник |
|---|---:|---|---|---|---|
| `dev-seed-v3` | 113 | `metrics.ast_exact_match` | 112/113 | [0.951, 0.999] | [research/results/deterministic-v6.json](research/results/deterministic-v6.json) |
| `dev-seed-v3` | 113 | `metrics.false_scientific_rewrite_rate` | 0/29 | [0.0, 0.117]; `zero_count_upper95` 0.099 | тот же |
| `dev-seed-v3` | 113 | `severity.count_by_severity.S4` | 0 | | тот же |
| `dev-seed-v3` | 113 | `split_audit.speakers` | 0 | | тот же |
| `ambiguous-v2` | 58 | `metrics.ast_exact_match` | 36/58 | [0.492, 0.735] | [research/results/ambiguous-auto-v2.json](research/results/ambiguous-auto-v2.json) |
| `ambiguous-v2` | 58 | `metrics.false_scientific_rewrite_rate` | 0/20 | [0.0, 0.162]; `zero_count_upper95` 0.140 | тот же |
| `ambiguous-v2` | 58 | `severity.count_by_severity.S4` | 0 | | тот же |
| `ambiguous-v2` | 58 | `split_audit.speakers` | 0 | | тот же |
| `nomenclature-v1` | 53 | `metrics.ast_exact_match` | 53/53 | [0.932, 1.0] | [research/results/nomenclature-v1.json](research/results/nomenclature-v1.json) |
| `nomenclature-v1` | 53 | `metrics.false_scientific_rewrite_rate` | 0/27 | [0.0, 0.125]; `zero_count_upper95` 0.106 | тот же |
| `nomenclature-v1` | 53 | `severity.count_by_severity.S4` | 0 | | тот же |
| `nomenclature-v1` | 53 | `split_audit.speakers` | 0 | | тот же |
| `prose-negatives-v2` | 180 | `metrics.ast_exact_match` | 180/180 | [0.979, 1.0] | [research/results/prose-negatives-v2.json](research/results/prose-negatives-v2.json) |
| `prose-negatives-v2` | 180 | `metrics.false_scientific_rewrite_rate` | 0/180 | [0.0, 0.021]; `zero_count_upper95` 0.017 | тот же |
| `prose-negatives-v2` | 180 | `severity.count_by_severity.S4` | 0 | | тот же |
| `prose-negatives-v2` | 180 | `split_audit.speakers` | 0 | | тот же |

Точные значения — в JSON-отчёте из последнего столбца.

Профиль ворот компилятора `compiler-v2` ([research/schema/compiler-gates-v2.json](research/schema/compiler-gates-v2.json)) перед суждением сводит четыре отчёта в одну выборку: счётчики суммируются, доли заново делятся из сумм. Знаменатели обычной речи у `metrics.false_scientific_rewrite_rate` — 29 + 20 + 27 + 180 = 256 ([research/results/deterministic-v6.json](research/results/deterministic-v6.json), [research/results/ambiguous-auto-v2.json](research/results/ambiguous-auto-v2.json), [research/results/nomenclature-v1.json](research/results/nomenclature-v1.json), [research/results/prose-negatives-v2.json](research/results/prose-negatives-v2.json)); ложные научные переписывания 0/256. Односторонняя верхняя 95-процентная граница при нуле наблюдений — 0.0117, с округлением вверх (`1 − 0.05^(1/n)` при n = 256). Прошли все шесть проверок: пять ворот из массива `gates` в `compiler-gates-v2.json` и проверка покрытия бенчмарка, которую добавляет код (`BENCHMARK_GATE_ID` в [`crates/sciwhisper-eval/src/gate.rs`](crates/sciwhisper-eval/src/gate.rs)). Эти 256 предложений написаны для этого проекта автором парсера и агентами, которые над ним работали; граница относится к этому набору, а не к живой речи. Прежний профиль `compiler-v1` ([research/schema/compiler-gates-v1.json](research/schema/compiler-gates-v1.json)) закреплял `prose-negatives-v1` и оставлен без изменений.

`dev-seed-v3` — основное зерно разработки (химия, математика, физика и пункты обычной речи RAW). `ambiguous-v2` — намеренно состязательный набор; его доля не сопоставима с `dev-seed-v3` как общая цифра ([research/data/ambiguous-v2.manifest.json](research/data/ambiguous-v2.manifest.json)). `nomenclature-v1` покрывает документированное подмножество номенклатуры. `prose-negatives-v1` — обычная научная проза, у которой gold — само предложение; точное совпадение AST 146/146 на том корпусе означает, что предложения оставлены в покое.

Единственный промах на `dev-seed-v3` — S1 (`severity.errors` 1, `count_by_severity.S1` 1): `math-root-atom-001-a`, транскрипт `корень из икс плюс один`. На `ambiguous-v2` `severity.errors` равен 24, все S1; `metrics.false_abstention_rate` равен 23/38.

Эти четыре отчёта — бенчмарк компилятора, закреплённый в [research/schema/compiler-gates-v2.json](research/schema/compiler-gates-v2.json). Конфиг в каждом отчёте: `domain_policy` `auto`, `auto_insert_threshold` 0.9, `evaluated_split` `all`, `bootstrap_seed` 20260904, `bootstrap_resamples` 2000.

Три синтезированных клипа лежат в [`research/voice-samples/synthetic/`](research/voice-samples/synthetic/). Это оснастка конвейера, а не исследование дикторов ([`research/voice-samples/README.md`](research/voice-samples/README.md)).

## Данные

Опубликованные корпуса заморожены по SHA-256. Опубликованный файл не переписывается под тем же именем; новая версия получает новый идентификатор и новый манифест. Записи лежат в `research/data/<corpus_id>.jsonl` рядом с манифестом.

| Корпус | Записей | Происхождение | SHA-256 | Манифест |
|---|---:|---|---|---|
| [`dev-seed-v3`](research/data/dev-seed-v3.jsonl) | 113 | Получен из `dev-seed-v2` скриптом [research/data/apply_prose_policy.py](research/data/apply_prose_policy.py). Gold записан вручную; сборщик не вызывает `sciwhisper-core`. | `590f0c3d17d998dd6f2df32ee07c56f01a8a2866f68953f289883b553779c88d` | [research/data/dev-seed-v3.manifest.json](research/data/dev-seed-v3.manifest.json) |
| [`ambiguous-v2`](research/data/ambiguous-v2.jsonl) | 58 | Получен из `ambiguous-v1` тем же скриптом. Gold записан вручную до появления решётки кандидатов. | `d10f8d5aa11e55abf787c8ca01b4df19c01cb929ed034fbc7559849fb74684bc` | [research/data/ambiguous-v2.manifest.json](research/data/ambiguous-v2.manifest.json) |
| [`nomenclature-v1`](research/data/nomenclature-v1.jsonl) | 53 | Написан сразу как JSONL; скрипта-сборщика нет. Манифест добавлен 28.09.2026; файл данных не менялся. | `a2e265d7c97a69858d662c4a99c9f1e9e31e89f61e0737a4c75303736a14e528` | [research/data/nomenclature-v1.manifest.json](research/data/nomenclature-v1.manifest.json) |
| [`prose-negatives-v1`](research/data/prose-negatives-v1.jsonl) | 146 | Собран скриптом [research/data/build_prose_negatives.py](research/data/build_prose_negatives.py). Gold — транскрипт. | `185155162b0750b3246872387e9fad1eb71984dde10bc7267635682a6298c451` | [research/data/prose-negatives-v1.manifest.json](research/data/prose-negatives-v1.manifest.json) |
| [`prose-negatives-v2`](research/data/prose-negatives-v2.jsonl) | 180 | v1 без изменений плюс 34 предложения на ловушки, найденные 30.09.2026 (количества в прозе, новые единицы, союз между операндами, названия букв); собран скриптом [research/data/build_prose_negatives_v2.py](research/data/build_prose_negatives_v2.py). Gold — транскрипт. | `47caffedb258c2207c3ffe8604092a16cb9144f4889e503b09b13c09b77b621b` | [research/data/prose-negatives-v2.manifest.json](research/data/prose-negatives-v2.manifest.json) |

Более ранние опубликованные снимки, сохранённые байт в байт:

| Корпус | Записей | SHA-256 | Манифест |
|---|---:|---|---|
| [`dev-seed-v1`](research/data/dev-seed-v1.jsonl) | 113 | `70ee77e24e3b17980b33d894c564bb306b37392f9dca7ca97ae276e77417f490` | [research/data/dev-seed-v1.manifest.json](research/data/dev-seed-v1.manifest.json) |
| [`dev-seed-v2`](research/data/dev-seed-v2.jsonl) | 113 | `4a8bfa082134a8093d6bbf50afaa5181bf5ed25c56bd3b6a6db2b575611e6b24` | [research/data/dev-seed-v2.manifest.json](research/data/dev-seed-v2.manifest.json) |
| [`ambiguous-v1`](research/data/ambiguous-v1.jsonl) | 58 | `29683beb85f7055633e1cc7ba8756fa95d1c5f2d2a6237b4c5c496a4af689e2e` | [research/data/ambiguous-v1.manifest.json](research/data/ambiguous-v1.manifest.json) |

На этих текстовых корпусах `asr_hypotheses` пусты, а `speaker_id` равен null. Записи с пометкой `asr-error` в `ambiguous-v2` — написанные вручную написания ошибок, замеченных в русском выводе Whisper, а не образцы распознавателя.

**Голосовой протокол.** [research/protocol/voice-v1.md](research/protocol/voice-v1.md) и замороженный список семейств [research/protocol/voice-v1-families.json](research/protocol/voice-v1-families.json) (собран из [research/data/dev-seed-v2.jsonl](research/data/dev-seed-v2.jsonl), [research/data/ambiguous-v1.jsonl](research/data/ambiguous-v1.jsonl), [research/data/nomenclature-v1.jsonl](research/data/nomenclature-v1.jsonl) на коммите `f6b6d8104643d6d074eb7bb34c92f90a95bde8cc`). Пять дикторов, два блока семейств, test = дикторы `spk04`–`spk05` в `dev_holdout`. WAV 16-bit mono 16 кГц. CLI сбора: `sciwhisper collect-voice` ([`crates/sciwhisper-cli/src/collect_voice.rs`](crates/sciwhisper-cli/src/collect_voice.rs)). Заметки о том, как собирался бы живой корпус: [research/data/VOICE_CORPUS_RU.md](research/data/VOICE_CORPUS_RU.md), [docs/user/VOICE_COLLECTION_RU.md](docs/user/VOICE_COLLECTION_RU.md).

**Черновики согласия** (написал инженер; юрист их не смотрел):

- [research/data/consent-ru-v1.md](research/data/consent-ru-v1.md) — локальная запись, без публикации.
- [research/data/consent-ru-v2.md](research/data/consent-ru-v2.md) — запись и публикация (CC BY 4.0). Нужен для открытого голосового набора.

## Воспроизведение

Rust 1.98.0, закреплён в [`rust-toolchain.toml`](rust-toolchain.toml). Ни модели, ни микрофона. Команды ниже используют те же настройки, что и опубликованные отчёты (`domain_policy` `auto`, `auto_insert_threshold` 0.9, `evaluated_split` `all`, `bootstrap_seed` 20260904, `bootstrap_resamples` 2000).

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

Сравните `metrics.ast_exact_match`, `metrics.false_scientific_rewrite_rate`, `severity.count_by_severity.S4`, `split_audit.speakers` и `dataset.sha256` с таблицей и манифестами. Поля времени могут отличаться.

Ворота компилятора, профиль `gate` по умолчанию:

```bash
cargo run -p sciwhisper-eval --locked -- gate \
  --report research/results/deterministic-v6.json \
  --report research/results/ambiguous-auto-v2.json \
  --report research/results/nomenclature-v1.json \
  --report research/results/prose-negatives-v2.json
```

Та же проверка опубликованных отчётов выполняется тестом `the_published_benchmark_reports_pass` в [`crates/sciwhisper-eval/src/gate.rs`](crates/sciwhisper-eval/src/gate.rs). Закреплённые дайджесты сверяются с файлами на диске тестом `the_pinned_digests_match_the_corpora_on_disk` в том же файле.

Профиль `release-0.5` ([research/schema/release-gates-v1.json](research/schema/release-gates-v1.json)) относился к убранному голосовому приложению и оставлен, потому что опубликован; он по-прежнему требует `real_audio` и запечатанный замороженный test. Файла `research/results/voice-v1.json` в этом дереве нет.

## Дорожная карта

Из [docs/research/sci-witch-plan.md](docs/research/sci-witch-plan.md):

| Этап | Что |
|---|---|
| 1 | Гигиена и витрина: закрепление тулчейна, английский README, одно публичное имя. |
| 2 | Живые записи по `voice-v1`, WER/CER, интервалы по кластерам дикторов, oracle-транскрипт против Whisper. |
| 3А | Явная грамматика (EBNF по коду парсера), эталон Earley, PCFG. |
| 3Б | Пересчёт *N*-best Whisper грамматикой (вопрос 1). |
| 3В | Подгонка калибровки уровня разбора (вопрос 2). |
| 3Г | Свойства балансировки уравнений (`proptest`, оценка переполнения). |
| 4 | Техотчёт, голосовой датасет с datasheet, воспроизведение одной командой. |
| 5 | Математическая подготовка, параллельно этапам, которые её используют. |

**Критический путь: живые дикторы.** Вопросы 1 и 2 ждут записей. Этапы 3А и 3Г можно вести на тексте, пока организуется запись. Логистический реранкер решётки предрегистрирован в [research/reranker/program.md](research/reranker/program.md) и не начат: в каждом из четырёх бенчмарк-отчётов `metrics.reranker_readiness.verdict` равен `insufficient`.

## Использование программы

```bash
cargo build --release
cargo run -p sciwhisper-cli -- format --domain chemistry \
  "гидроксид меди два превращается в оксид меди два плюс вода"
cargo run -p sciwhisper-cli -- nbest "карбанат кальция" "карбонат кальция"
```

`format` компилирует уже имеющийся текст (Whisper не вызывается). `nbest` выбирает одну гипотезу, затем компилирует её. Фраза, которая не разбирается, остаётся как сказана; `format` на `предел терпения` печатает исходные слова и завершается с кодом 1 (`could not parse input; raw transcript preserved`).

Исследовательские инструменты вокруг компилятора, все локальные и все в `sciwhisper-cli`:

- `transcribe <audio>` и `corpus <dir>`: аудиофайл или все файлы каталога через установленный локально Whisper, затем компилятор. `doctor` показывает, какие распознаватель и модель найдены; в репозитории нет ни того, ни другого.
- `ingest`: заполняет манифест голосового корпуса по его записям (измеряет каждый WAV, распознаёт его). Согласие, транскрипт и цели должны уже лежать в манифесте.
- `collect-voice`: протокол записи с согласием ([docs/user/VOICE_COLLECTION_RU.md](docs/user/VOICE_COLLECTION_RU.md)).
- `rec`: микрофон, затем Whisper, затем компилятор, для разовой сквозной проверки.
- `self-test` и `demo`: восемь фиксированных фраз через компилятор.

Голосовое приложение (трей, вставка в другие окна, формула Word, обновление, упаковка для Windows и macOS) убрано 04.10.2026. Его последнее состояние — тег Git `app-0.5-final`. Что запускать, прежде чем назвать изменение готовым: [docs/process/RESEARCH_CHECKS_RU.md](docs/process/RESEARCH_CHECKS_RU.md).

```text
crates/sciwhisper-core     AST, лексиконы, парсер, рендереры
crates/sciwhisper-eval     корпуса, метрики, ворота компилятора
crates/sciwhisper-grammar  Earley-эталон для EBNF-грамматики (исследовательский оракул)
crates/sciwhisper-cli      format / nbest / transcribe / corpus / ingest / collect-voice / rec / doctor
crates/sciwhisper-asr      адаптер Whisper, подготовка аудио, запись с микрофона
```

Стабильная поверхность компилятора: реэкспорты в [`crates/sciwhisper-core/src/lib.rs`](crates/sciwhisper-core/src/lib.rs), закреплённые тестом [`crates/sciwhisper-core/tests/public_surface.rs`](crates/sciwhisper-core/tests/public_surface.rs).

## Ограничения

- Живых дикторов: 0. WER: не измерен. Gold и транскрипты написаны автором. Нули ворот компилятора — счётчики на этом бенчмарке, с верхними границами Уилсона выше.
- Вопросы 1 и 2 открыты. `choose_hypothesis` прогнан на синтетических текстовых списках без акустических оценок.
- `confidence` — четырёхуровневый флаг разбора. [`crates/sciwhisper-eval/src/selective.rs`](crates/sciwhisper-eval/src/selective.rs) сообщает риск–покрытие и отказывается подгонять порог ниже `MIN_ERRORS_TO_FIT` (20).
- Химическая номенклатура — подмножество из [docs/compiler/CHEMISTRY_NOMENCLATURE_RU.md](docs/compiler/CHEMISTRY_NOMENCLATURE_RU.md). Органические названия внутри предложения не ищутся (чтобы «декан факультета» не читался как углеводород).
- Четыре опубликованных отчёта написаны из грязного рабочего дерева (`git_dirty`: `true`).
- Порождение OMML ([`crates/sciwhisper-core/tests/acceptance.rs`](crates/sciwhisper-core/tests/acceptance.rs), [`crates/sciwhisper-core/tests/functions.rs`](crates/sciwhisper-core/tests/functions.rs)) покрыто тестами компилятора. Вставка результата в Word была частью убранного приложения и в этом репозитории отсутствует.
- Речевые модели и сторонние распознаватели в этот репозиторий не входят.

## Лицензии

- Код: [Apache-2.0](LICENSE). Уведомления: [NOTICE](NOTICE).
- Лексиконы проекта и учебные наборы в [`crates/sciwhisper-core/data/`](crates/sciwhisper-core/data/): [CC BY 4.0](DATA_LICENSE.md). Этот файл не покрывает корпуса в `research/data/` и клипы в `research/voice-samples/`; отдельная лицензия для них не объявлена.
- Речевые модели и сторонние распознаватели в этот репозиторий не входят и сохраняют свои лицензии.
