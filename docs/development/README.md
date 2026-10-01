# Карта переездов: `docs/development/` → новые места

Каталог `docs/development/` разобран по темам. Эта карта нужна потому, что на
старые пути ссылаются опубликованные манифесты корпусов, отчёты измерений и
схемы ворот (`research/data/`, `research/results/`, `research/schema/`), а
менять их нельзя: SHA-256 записан в манифестах. Встретив в них путь вида
`docs/development/…`, ищите документ по таблице.

| Старый путь | Новый путь |
| --- | --- |
| `docs/development/COMPILER_CONTRACT_RU.md` | [`docs/compiler/COMPILER_CONTRACT_RU.md`](../compiler/COMPILER_CONTRACT_RU.md) |
| `docs/development/ARCHITECTURE_RU.md` | [`docs/compiler/ARCHITECTURE_RU.md`](../compiler/ARCHITECTURE_RU.md) |
| `docs/development/SPECIFICATION_RU.md` | [`docs/compiler/SPECIFICATION_RU.md`](../compiler/SPECIFICATION_RU.md) |
| `docs/development/GRAMMAR_RU.md` | [`docs/compiler/GRAMMAR_RU.md`](../compiler/GRAMMAR_RU.md) |
| `docs/development/MATHEMATICS_RU.md` | [`docs/compiler/MATHEMATICS_RU.md`](../compiler/MATHEMATICS_RU.md) |
| `docs/development/CHEMISTRY_NOMENCLATURE_RU.md` | [`docs/compiler/CHEMISTRY_NOMENCLATURE_RU.md`](../compiler/CHEMISTRY_NOMENCLATURE_RU.md) |
| `docs/development/IUPAC_SOURCES.md` | [`docs/compiler/IUPAC_SOURCES.md`](../compiler/IUPAC_SOURCES.md) |
| `docs/development/NATURAL_DICTATION_RU.md` | [`docs/compiler/NATURAL_DICTATION_RU.md`](../compiler/NATURAL_DICTATION_RU.md) |
| `docs/development/CANDIDATE_LATTICE_RU.md` | [`docs/compiler/CANDIDATE_LATTICE_RU.md`](../compiler/CANDIDATE_LATTICE_RU.md) |
| `docs/development/BALANCE_KERNEL_RU.md` | [`docs/compiler/BALANCE_KERNEL_RU.md`](../compiler/BALANCE_KERNEL_RU.md) |
| `docs/development/FORMAL_GUARANTEES_RU.md` | [`docs/compiler/FORMAL_GUARANTEES_RU.md`](../compiler/FORMAL_GUARANTEES_RU.md) |
| `docs/development/sci-witch-plan.md` | [`docs/research/sci-witch-plan.md`](../research/sci-witch-plan.md) |
| `docs/development/ML_LAB_RU.md` | [`docs/research/ML_LAB_RU.md`](../research/ML_LAB_RU.md) |
| `docs/development/ML_RESEARCH_RU.md` | [`docs/research/ML_RESEARCH_RU.md`](../research/ML_RESEARCH_RU.md) |
| `docs/development/RELEASE_CHECKLIST.md` | [`docs/process/RELEASE_CHECKLIST.md`](../process/RELEASE_CHECKLIST.md) |
| `docs/development/PAIR_WORKFLOW_RU.md` | [`docs/process/PAIR_WORKFLOW_RU.md`](../process/PAIR_WORKFLOW_RU.md) |
| `docs/development/AUTO_UPDATE_RU.md` | [`docs/process/AUTO_UPDATE_RU.md`](../process/AUTO_UPDATE_RU.md) |
| `docs/development/decisions/` | [`docs/decisions/`](../decisions/) |

Другие переезды той же перестройки:

| Старый путь | Новый путь |
| --- | --- |
| `CONTRIBUTING.md`, `SECURITY.md` | `.github/CONTRIBUTING.md`, `.github/SECURITY.md` |
| `CLAUDE.md` | `.claude/CLAUDE.md` |
| `assets/branding/` | `packaging/branding/` |
| `corpus/voice/` | `research/voice-samples/` |

Внутри релизного архива раскладка прежняя: файлы `SECURITY.md` и
`CONTRIBUTING.md` лежат в корне архива, документы — в `docs/user/` и
`docs/development/`.
