# Документация sci-witch

Продукт — компилятор текста в научную структуру; голосовое приложение —
референсный фронтенд. Документы разложены по темам.

## Для пользователей и тестировщиков (`user/`)

- [Как проверить программу без знания программирования](user/TESTING_RU.md)
- [Приватность](user/PRIVACY_RU.md)
- [Известные ограничения](user/KNOWN_LIMITATIONS_RU.md)
- [Записать голосовой набор для проверки](user/VOICE_COLLECTION_RU.md)
- [Приёмка на Windows 11](user/WINDOWS_ACCEPTANCE_RU.md)

## Компилятор (`compiler/`)

- [Контракт компилятора: что обещано, ворота допуска](compiler/COMPILER_CONTRACT_RU.md)
- [Техническое задание](compiler/SPECIFICATION_RU.md)
- [Архитектура научного декодера](compiler/ARCHITECTURE_RU.md)
- [Голосовая грамматика](compiler/GRAMMAR_RU.md)
- [Естественная диктовка](compiler/NATURAL_DICTATION_RU.md)
- [Математика под капотом](compiler/MATHEMATICS_RU.md)
- [Доказательства корректности вычислительного ядра](compiler/FORMAL_GUARANTEES_RU.md)
- [Балансировка уравнений: когда положительное решение одно](compiler/BALANCE_KERNEL_RU.md)
- [Решётка кандидатов (Candidate Lattice v1)](compiler/CANDIDATE_LATTICE_RU.md)
- [Композиционная химическая номенклатура](compiler/CHEMISTRY_NOMENCLATURE_RU.md)
- [Источники химического лексикона](compiler/IUPAC_SOURCES.md)

## Грамматика как объект (`grammar/`)

- [FIRST, FOLLOW и LL(1)-конфликты](grammar/ANALYSIS_RU.md)
- [`math.ebnf`](grammar/math.ebnf) и [`chem.ebnf`](grammar/chem.ebnf): EBNF, записанная по коду

## Исследования (`research/`)

- [План: от приложения к исследовательскому проекту](research/sci-witch-plan.md)
- [Протокол ML-исследования v1.0](research/ML_RESEARCH_RU.md)
- [Лаборатория измерений](research/ML_LAB_RU.md)

Корпуса, схемы и результаты измерений лежат не здесь, а в [`../research/`](../research/README_RU.md).

## Процесс (`process/`)

- [Проверка перед выпуском версии](process/RELEASE_CHECKLIST.md)
- [Автообновление](process/AUTO_UPDATE_RU.md)
- [Работа в паре: Claude и Grok](process/PAIR_WORKFLOW_RU.md)

## Решения (`decisions/`)

- [ADR-0001: язык, поставка и границы Этапа 1](decisions/0001-stack.md)
- [ADR-0002: SciWhisper как надстройка на Whisper](decisions/0002-whisper-overlay.md)
- [ADR-0003: геометрия научных ошибок](decisions/0003-geometry-of-scientific-errors.md)

## Прочее

- Изображения для README и инструкций — в `images/`.
- Старые пути `docs/development/…` — в [карте переездов](development/README.md).
