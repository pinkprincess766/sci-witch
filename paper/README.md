# Статья

Черновик статьи о компиляторе sci-witch: [`main.tex`](main.tex), литература —
[`refs.bib`](refs.bib).

- **Числа не пишутся руками.** [`numbers.tex`](numbers.tex) генерирует тест
  `crates/sciwhisper-eval/tests/paper_numbers.rs` из отчётов
  `research/results/*.json` и падает, если статья разошлась с отчётами. После
  намеренного изменения отчёта:
  `PAPER_NUMBERS_WRITE=1 cargo test -p sciwhisper-eval --test paper_numbers`.
  Второй тест того же файла не даёт вписать число в `main.tex` литералом вместо
  макроса.
- **Обзор литературы** пишет владелец проекта; он подключается из
  `review.tex`, когда файл появится в этой папке. Пока его нет, на месте раздела
  стоит заглушка.
- **Литература** — только работы, которые цитирует статья, с выходными данными,
  сверенными по первоисточникам (карта поиска:
  [`research/LITERATURE_MAP_RU.md`](../research/LITERATURE_MAP_RU.md)).

Сборка: `latexmk -pdf main.tex` (нужен TeX Live или MiKTeX с пакетами `babel`
для русского, `booktabs`, `hyperref`).
