# Источники химического лексикона

SciWhisper использует рекомендации IUPAC как нормативную основу для канонических
названий и статуса retained names. Русские формы, падежи и ошибки распознавания
Whisper являются локальными алиасами проекта, а не официальным переводом IUPAC.

## Первичные источники

- IUPAC Blue Book, P-12 — preferred, preselected and retained names:
  <https://iupac.qmul.ac.uk/BlueBook/P1.html>
- IUPAC Blue Book, P-34 — functional parent compounds:
  <https://iupac.qmul.ac.uk/BlueBook/P3.html>
- IUPAC Blue Book, P-61 и P-65 — retained haloforms and carboxylic acids:
  <https://iupac.qmul.ac.uk/BlueBook/P6.html>
- IUPAC Blue Book, P-107 — glycerol in general nomenclature:
  <https://iupac.qmul.ac.uk/BlueBook/P10.html>
- IUPAC Red Book 2005 — inorganic nomenclature:
  <https://publications.iupac.org/books/rbook/Red_Book_2005.pdf>
- Краткие руководства IUPAC по органической и неорганической номенклатуре:
  <https://iupac.org/what-we-do/nomenclature/brief-guides/>

## Проверяемые базы материалов

Для распространённых названий материалов, которые не являются retained names
IUPAC, допускаются отдельные авторитетные химические базы с явным статусом
записи. Например, `феррит цинка` / `ZnFe₂O₄` подтверждён записью
NIH PubChem: <https://pubchem.ncbi.nlm.nih.gov/compound/Iron-zinc-oxide-_Fe2ZnO4>.
Такие записи не помечаются как канонические названия IUPAC.

## Как устроены данные

Каждая новая подтверждённая запись может содержать:

- `canonical_name` — английское имя из источника IUPAC;
- `nomenclature_status` — статус имени, например `retained_pin` или
  `retained_general`;
- `source` — ключ первичного источника из секции `sources`;
- `names` — русские формы для диктовки;
- `asr_aliases` — наблюдавшиеся ошибки Whisper, которые нельзя выдавать за
  нормативные химические названия.

Словарь не присваивает одну формулу неоднозначным смесям и бытовым словам без
уточнения. Например, «царская водка» является смесью, а слово «сода» может
обозначать разные вещества. Такие выражения должны сохраняться как текст либо
требовать уточняющего названия.

Проект хранит только краткие фактические соответствия «название — формула —
статус — ссылка». Текст и таблицы публикаций IUPAC в репозиторий не копируются.
