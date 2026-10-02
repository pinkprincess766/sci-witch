# `research/` — исследовательский контур sci-witch

Здесь живут данные, схемы и результаты измерений. Код приложения сюда не
смешивается: лаборатория использует публичный API `sciwhisper-core` и измеряет
ровно тот детерминированный конвейер, который есть в репозитории.

```text
research/
├── data/
│   ├── build_dev_seed.py          # строит dev-seed-v2
│   ├── build_ambiguous_seed.py    # строит ambiguous-v1
│   ├── build_prose_negatives.py   # строит prose-negatives-v1
│   ├── build_prose_negatives_v2.py # выводит prose-negatives-v2 из v1
│   ├── apply_prose_policy.py      # выводит dev-seed-v3 и ambiguous-v2 из v2 и v1
│   ├── ast_helpers.py             # общие конструкторы AST для сборщиков
│   ├── dev-seed-v1.jsonl          # замороженный исторический корпус 0.2/0.3
│   ├── dev-seed-v2.jsonl          # замороженный корпус до политики прозы
│   ├── dev-seed-v3.jsonl          # текущий текстовый корпус разработки
│   ├── ambiguous-v1.jsonl         # неоднозначные фразы до политики прозы
│   ├── ambiguous-v2.jsonl         # текущий корпус неоднозначных фраз
│   ├── nomenclature-v1.jsonl      # отдельная приёмка научной номенклатуры
│   ├── prose-negatives-v1.jsonl   # обычные фразы, которые обязаны остаться словами
│   ├── prose-negatives-v2.jsonl   # v1 + количества, единицы, союзы (30.09.2026)
│   ├── *.manifest.json            # SHA-256, состав и границы каждой версии
│   ├── VOICE_CORPUS_RU.md         # как собрать настоящий голосовой корпус
│   ├── consent-ru-v1.md           # согласие без публикации
│   └── consent-ru-v2.md           # черновик согласия с публикацией (CC BY 4.0)
├── protocol/
│   ├── voice-v1.md                # предрегистрация первого голосового теста
│   └── voice-v1-families.json     # замороженный список семейств voice-v1
├── schema/
│   ├── CANONICAL_AST_V1_RU.md
│   ├── severity-v1.json
│   ├── release-gates-v1.json      # голосовое приложение: 150–200 real_audio
│   ├── compiler-gates-v1.json     # компилятор, прежний профиль (prose-negatives-v1)
│   ├── compiler-gates-v2.json     # компилятор: текст, без микрофона (текущий)
│   └── ast-distance-v1.json
├── reranker/
│   ├── README_RU.md
│   └── program.md
├── LITERATURE_MAP_RU.md           # что и где искать в литературе
└── results/
    ├── deterministic-v1.json      # dev-seed-v1, исторический baseline 0.2
    ├── deterministic-v2.json      # dev-seed-v1, исторический прогон 0.3
    ├── deterministic-v4.json      # dev-seed-v2, report schema 4
    ├── deterministic-v5.json      # dev-seed-v2, report schema 5
    ├── deterministic-v6.json      # dev-seed-v3, текущий отчёт
    ├── ambiguous-auto-v1.json     # ambiguous-v1
    ├── ambiguous-auto-v2.json     # ambiguous-v2, текущий отчёт
    ├── nomenclature-v1.json       # nomenclature-v1
    ├── prose-negatives-v1.json    # prose-negatives-v1
    ├── prose-negatives-v2.json    # prose-negatives-v2
    ├── lattice-v1.json            # решётка кандидатов на ambiguous-v1
    ├── lattice-v2.json            # решётка кандидатов на ambiguous-v2
    ├── dev-seed-v2-lattice-v1.json
    └── ambiguous-v*-lattice.jsonl # кандидаты решётки по записям
```

Практическое руководство —
[`docs/research/ML_LAB_RU.md`](../docs/research/ML_LAB_RU.md).
Карта литературы: названия смежных областей, математическая постановка и где
искать работы — [`LITERATURE_MAP_RU.md`](LITERATURE_MAP_RU.md).
Формальная грамматика устной математической и химической речи (EBNF, FIRST и
FOLLOW, LL(1)-конфликты) лежит не здесь, а в [`docs/grammar/`](../docs/grammar/):
тест `crates/sciwhisper-core/tests/grammar_first_follow.rs` сверяет её с кодом.
Это вход для этапа 3А плана; данные и результаты измерений остаются здесь.

## Версии и воспроизводимость

`dataset_schema_version` описывает формат одной записи: версия 1 — текст,
версия 2 дополнительно умеет хранить аудио и согласие. Название корпуса
(`dev-seed-v1`, `dev-seed-v2`, `dev-seed-v3`) — версия самих данных. Это разные оси версий.

Опубликованный корпус не переписывается под тем же именем. `dev-seed-v1`
сохранён побайтово вместе со своим SHA-256, поэтому отчёты 0.2/0.3 остаются
воспроизводимыми. Исправленная разметка и новые проверки получили имя
`dev-seed-v2` и отдельный манифест. Политика «название вещества в прозе не
подменяется» изменила ожидаемый ответ у пяти записей; их вывел
[`apply_prose_policy.py`](data/apply_prose_policy.py) новыми версиями
`dev-seed-v3` (1 запись) и `ambiguous-v2` (4 записи), а `dev-seed-v2` и
`ambiguous-v1` остались побайтово прежними.

## Текущий корпус разработки

`dev-seed-v3` содержит 113 рукописных текстовых записей в 103 семействах.
От `dev-seed-v2` он отличается одной записью (`raw-acid-storage-001-a`: у неё
убран `expected_mixed_output`), состав и разбиение те же:

| Домен | Записей |
|---|---:|
| chemistry | 29 |
| mathematics | 33 |
| physics | 22 |
| plain (RAW/OOD) | 29 |

| Split | Записей | Семейств |
|---|---:|---:|
| train | 59 | 53 |
| validation | 33 | 30 |
| dev_holdout | 21 | 20 |

Разделение выполнено по семействам: парафразы одной конструкции не попадают
в разные split. Все записи имеют `provenance: handcrafted_text`, пустые
`asr_hypotheses` и `speaker_id: null`. Поэтому корпус не измеряет Whisper,
акцент, шум, микрофон или задержку живой диктовки.

`dev_holdout` также не является frozen test: он уже открывался при разработке.
Настоящий frozen test создаётся только из записей живых людей, с согласием и
разделением по дикторам.

## Откуда взялся gold

Каждый `target_ast` написан руками из смысла фразы. Генератор корпуса не
импортирует `sciwhisper-core` и не получает правильный ответ из текущего
парсера. Иначе benchmark просто подтверждал бы собственный код.

`expected_mixed_output` используется только там, где пользовательский режим
должен сохранить предложение, но заменить доказанный научный фрагмент. Во
всех остальных AST-записях ожидаемый Unicode выводится из ручного `target_ast`,
а RAW-запись по умолчанию обязана остаться исходным текстом.

## Текущие измерения

На `dev-seed-v3` ([`deterministic-v6.json`](results/deterministic-v6.json))
детерминированное ядро даёт 112/113 exact match; единственная ошибка —
безопасный отказ на неоднозначной границе корня (`math-root-atom-001-a`).
Пользовательский `MixedText` путь совпадает с ожидаемым документом в 113/113
случаях.

Это результат на маленьком текстовом dev-корпусе, не обещание качества живой
речи. Например, 0 опасных переписываний из 29 RAW-записей всё ещё совместимы
примерно с 9,8% истинной частотой по односторонней 95% границе. Ворота 0.5
поэтому требуют реальный, запечатанный голосовой корпус и не проходят на этих
данных независимо от красивой точечной оценки.

Ворота компилятора ([`compiler-gates-v2.json`](schema/compiler-gates-v2.json))
судят не один отчёт, а четыре корпуса вместе, закреплённые по SHA-256:
`dev-seed-v3`, `ambiguous-v2`, `nomenclature-v1`, `prose-negatives-v2`. Обычных
предложений в них 29 + 20 + 27 + 180 = 256, ложных научных переписываний 0;
односторонняя 95% граница при нуле — 1,16%. Все эти фразы написаны для
проекта — автором парсера и агентами, — так что граница относится к ним, а не к
живой речи.
