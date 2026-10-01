# Карта поиска литературы

Для владельца проекта: что искать, где и зачем. Это не обзор литературы, а
маршрут к нему. Дополняет таблицу учебников в разделе «Этап 5» плана
[`docs/development/sci-witch-plan.md`](../docs/development/sci-witch-plan.md):
там — книги, чтобы понимать математику; здесь — статьи и области, чтобы
понять, где проект стоит среди чужих работ.

Все названные работы автор карты считает существующими, но **каждую
проверьте по первоисточнику**, прежде чем цитировать. Там, где уверенности
нет, дан поисковый запрос, а не ссылка.

## 1. Как называется то, что вы делаете

Главное, что даёт поиск: словарь. sci-witch собран из нескольких давно
существующих областей, и «проекта как этот» вы не найдёте — найдёте каждую
часть по отдельности.

| что делает sci-witch | как это называется в литературе |
|---|---|
| «двадцать градусов» → «20°», устная форма → письменная | **inverse text normalization (ITN)**; обратная задача — text normalization для синтеза речи |
| устная формула → дерево → LaTeX/OMML | **spoken mathematics recognition**, **speech-to-LaTeX**; более общее — **semantic parsing** |
| «гидроксид железа три» → Fe(OH)₃ | **chemical name to structure** (именование по IUPAC → структура) |
| выбрать лучшую из нескольких гипотез Whisper | **N-best rescoring**, **ASR error correction** |
| подсказать распознавателю научные слова | **contextual biasing** |
| грамматика + ошибки распознавания → самое правдоподобное дерево | **noisy channel model**; **error-correcting parsing** |
| не вставлять, если не уверен | **selective prediction**, **classification with reject option**, **abstention** |
| S0–S4: неверная вставка хуже отказа | **asymmetric loss / cost-sensitive decision**, Bayes decision rule with reject option |
| решётка кандидатов — набор, а не один ответ | **set-valued prediction**, **conformal prediction** |
| расстояние между деревьями | **tree edit distance** |

## 2. Математическое описание проекта в одной формуле

Это то, как проект описывается математически, — и заодно подсказка, какие
разделы искать.

- Вход — расшифровка $x$ (с ошибками). Выход — $y \in \mathcal{T} \cup
  \{\text{RAW}\}$, где $\mathcal{T}$ — деревья, порождаемые грамматикой $G$.
- **Шумный канал.** $\hat y = \arg\max_y P(y)\,P(x \mid y)$: $P(y)$ — априорная
  модель научной записи (PCFG по грамматике), $P(x \mid y)$ — модель ошибок
  распознавания. Это та же постановка, что у классического распознавания речи
  (Jelinek), и та же, что у error-correcting parsing, если канал — редакционное
  расстояние.
- **Решение с отказом.** Ответ выбирается не по максимуму вероятности, а по
  минимуму ожидаемой потери с асимметричной матрицей потерь (ваши классы
  S0–S4); RAW — отдельное действие со своей ценой. Порог вставки 0.9 — это
  порог Чоу (Chow, 1970), а кривая risk–coverage из `selective.rs` — стандартный
  инструмент selective classification.
- **Расстояние на выходах.** Взвешенное редакционное расстояние деревьев — метрика,
  если метрика стоимость операций (Tai, 1979); алгоритм — Zhang–Shasha (1989).
- **Неопределённость.** Калибровка уверенности (Platt; изотоническая регрессия) и
  **конформное предсказание**: оно даёт *набор* ответов с гарантией покрытия. Решётка
  кандидатов уже и есть набор ответов — связать её с конформным предсказанием —
  самостоятельная исследовательская линия.

Запросы: `noisy channel model speech recognition`, `Bayes decision rule reject option`,
`selective classification risk coverage`, `conformal prediction structured output`,
`set-valued prediction`.

## 3. Где искать

| место | для чего | заметка |
|---|---|---|
| **Google Scholar** | первый поиск, «Cited by» | лучший охват, слабые фильтры |
| **Semantic Scholar** | граф цитирований, краткие аннотации | удобно идти «вперёд» — кто цитировал |
| **arXiv** | свежие препринты | категории `cs.CL` (язык), `eess.AS` и `cs.SD` (речь, звук), `cs.FL` (формальные языки), `stat.ML` (калибровка, конформное) |
| **ACL Anthology** (aclanthology.org) | ACL, EMNLP, NAACL, Computational Linguistics | всё в открытом доступе |
| **ISCA Archive** (isca-archive.org) | Interspeech и другие речевые конференции ISCA | всё в открытом доступе; **главная площадка по речи** |
| **IEEE Xplore** | ICASSP, IEEE/ACM TASLP | чаще платно — через университет |
| **DBLP** (dblp.org) | полный список работ конкретного автора | когда нашли нужного человека |
| **Connected Papers**, **ResearchRabbit** | карта похожих работ вокруг одной статьи | с одной хорошей статьи находят десяток соседних |
| **eLibrary.ru**, **КиберЛенинка** | русскоязычные статьи | для русской речи и русской нормализации текста |
| **Архив «Диалога»** (dialog-21.ru) | главная российская конференция по компьютерной лингвистике | русский язык, в том числе речь |

Площадки, куда такая работа могла бы пойти: **Interspeech** и **ICASSP** (речь),
**ACL/EMNLP/NAACL** и их Findings (язык), **SPECOM** и **Диалог** (русская речь и язык),
воркшопы по доступности математики и по научным документам.

## 4. По направлениям

Каждое направление привязано к этапу плана.

### A. Устная форма → письменная (весь проект целиком)

Самая близкая к sci-witch область. Отсюда же — готовые приёмы для грамматик.

- Sproat R., Jaitly N. *RNN Approaches to Text Normalization: A Challenge.* arXiv, 2016. Вместе с ним — соревнование Google по нормализации текста на Kaggle (2017), в том числе **для русского языка**.
- Zhang H., Sproat R. и соавт. *Neural Models of Text Normalization for Speech Applications.* Computational Linguistics, 2019.
- Gorman K., Sproat R. *Finite-State Text Processing.* 2021 (книга; WFST-грамматики, библиотека Pynini).
- Запрос: `NeMo inverse text normalization` — работа группы NVIDIA о переходе ITN из грамматик в продакшн (проверьте авторов и год).
- Запросы: `inverse text normalization`, `spoken form written form`, `WFST text normalization`, `обратная нормализация текста`, `нормализация текста для синтеза речи`.

### B. Устная математика и химия (предметная область)

Здесь работ мало, и это хорошая новость: ниша не занята.

- Запросы: `speech to LaTeX`, `spoken mathematics recognition`, `mathematical speech recognition`, `voice input mathematics accessibility`. За 2024–2025 годы должны найтись работы и датасеты по speech-to-LaTeX — начните с них и идите по ссылкам.
- Обратное направление — **математика → речь** для экранных дикторов: правила MathSpeak и ClearSpeak. Их словарь «как читают формулы вслух» — готовый материал для вашей грамматики.
- Lowe D. M., Corbett P. T., Murray-Rust P., Glen R. C. *Chemical Name to Structure: OPSIN, an Open Source Solution.* J. Chem. Inf. Model., 2011. OPSIN — эталонный разборщик IUPAC-названий (письменных, английских); с ним честно сравнивать номенклатурную часть.
- Запросы: `chemical named entity recognition`, `IUPAC name to structure`, `chemical name parsing`.

### C. Ошибки распознавания и выбор гипотезы (этап 3Б, «оптимизация обработки голоса»)

- Radford A. и соавт. *Robust Speech Recognition via Large-Scale Weak Supervision.* 2022 (статья о Whisper). Прочитать обязательно: как устроен декодер, откуда берутся ошибки.
- Salazar J. и соавт. *Masked Language Model Scoring.* ACL 2020 — переранжирование N-best гипотез языковой моделью.
- Pundak G. и соавт. *Deep Context: End-to-End Contextual Speech Recognition.* SLT 2018 — contextual biasing, подсказка списка слов распознавателю.
- Запросы: `N-best rescoring`, `ASR error correction`, `contextual biasing speech recognition`, `shallow fusion`, `domain-specific ASR`, `named entity ASR errors`.

### D. Грамматики (этап 3А)

- Earley J. *An Efficient Context-Free Parsing Algorithm.* CACM, 1970.
- Stolcke A. *An Efficient Probabilistic Context-Free Parsing Algorithm that Computes Prefix Probabilities.* Computational Linguistics, 1995 — вероятностный Earley, прямо то, что нужно для PCFG в 3А.
- Aho A. V., Peterson T. G. *A Minimum Distance Error-Correcting Parser for Context-Free Languages.* SIAM J. Computing, 1972 — восстановление после ошибок как задача разбора.
- Geng S. и соавт. *Grammar-Constrained Decoding for Structured NLP Tasks without Finetuning.* EMNLP 2023 — грамматика как ограничение на генерацию; мост между вашей грамматикой и нейросетевыми моделями.
- Запросы: `error-correcting parsing`, `PCFG`, `grammar-constrained decoding`, `ambiguity detection context-free grammar`.

### E. Отказ, калибровка, конформное предсказание (этап 3В и безопасность)

- Chow C. K. *On Optimum Recognition Error and Reject Tradeoff.* IEEE Trans. Information Theory, 1970 — отказ как оптимальное решение.
- El-Yaniv R., Wiener Y. *On the Foundations of Noise-free Selective Classification.* JMLR, 2010.
- Geifman Y., El-Yaniv R. *Selective Classification for Deep Neural Networks.* NeurIPS 2017.
- Guo C. и соавт. *On Calibration of Modern Neural Networks.* ICML 2017 (уже в плане).
- Platt J. (1999) и Zadrozny B., Elkan C. (2002) — калибровка по Платту и изотоническая.
- Angelopoulos A., Bates S. *A Gentle Introduction to Conformal Prediction and Distribution-Free Uncertainty Quantification.* arXiv, 2021 — лучший вход в конформное предсказание.
- Vovk V., Gammerman A., Shafer G. *Algorithmic Learning in a Random World.* 2005 (книга, первоисточник).

### F. Как измерять (этап 2)

- Morris A. C., Maier V., Green P. *From WER and RIL to MER and WIL.* Interspeech 2004 — чем плох WER и что вместо него.
- Запрос: `semantic error rate ASR`, `ASR evaluation beyond WER` — ваша метрика «точное совпадение дерева» ближе к ним, чем к WER.
- Zhang K., Shasha D. (1989); Tai K.-C. (1979); Bille P. *A Survey on Tree Edit Distance and Related Problems.* Theoretical Computer Science, 2005.
- Cameron A. C., Gelbach J. B., Miller D. L. *Bootstrap-Based Improvements for Inference with Clustered Errors.* Review of Economics and Statistics, 2008 — кластерный бутстрэп по дикторам.
- Dietterich T. G. *Approximate Statistical Tests for Comparing Supervised Classification Learning Algorithms.* Neural Computation, 1998 — парные сравнения, тест Макнемара.
- Gebru T. и соавт. *Datasheets for Datasets* (уже в плане).

### G. Оптимизация распознавания на обычном компьютере

«Оптимизация обработки голоса» в практическом смысле: быстрее и дешевле на CPU
без потери качества на научных словах.

- Gandhi S., von Platen P., Rush A. M. *Distil-Whisper.* arXiv, 2023 — дистилляция Whisper.
- Запросы: `ASR model quantization`, `on-device speech recognition`, `whisper distillation`, `speculative decoding speech`, `streaming ASR latency`.
- Вопрос, который стоит задать литературе: теряют ли сжатые модели именно редкие научные слова сильнее, чем общую речь. Если ответа нет — это ваш вопрос.

### H. Русская речь: данные и сообщество

- Karpov N. и соавт. *Golos: Russian Dataset for Speech Research.* Interspeech 2021.
- Ardila R. и соавт. *Common Voice: A Massively-Multilingual Speech Corpus.* LREC 2020 (русская часть под CC0).
- Запросы: `Russian speech recognition`, `Open STT Russian`, `распознавание русской речи`.
- Журнал «Информатика и автоматизация» (бывшие «Труды СПИИРАН») — русская речевая группа Санкт-Петербурга; конференции SPECOM, «Диалог», AINL.

## 5. Как искать, чтобы не утонуть

1. **Начинайте с обзора, не со статьи.** Запрос `survey` или `tutorial` плюс тема. Один хороший обзор экономит двадцать статей.
2. **Идите по ссылкам в обе стороны.** Назад — список литературы нужной статьи. Вперёд — «Cited by» в Scholar или Semantic Scholar: кто её развивал.
3. **Читайте в три прохода.** Аннотация и рисунки → введение и выводы → метод, только если статья нужна. Большинство статей дальше первого прохода не идут.
4. **Ведите таблицу**, а не закладки: статья | вопрос | метод | данные | результат | как связано с sci-witch (этап плана). Через месяц таблица станет разделом «Related work» техотчёта.
5. **Отделяйте «чем это похоже» от «чем мы отличаемся».** Второе — и есть ваш вклад. Сейчас он выглядит так: русский язык, химия и математика вместе, отказ как первоклассное решение с асимметричной ценой ошибок, честная оценка на авторском тексте.

## 6. На что не тратить время сейчас

- На обучение своей модели распознавания: для неё нужны тысячи часов данных, которых нет.
- На обзоры больших языковых моделей «в общем»: сюда относится только ограниченная генерация по грамматике (раздел D).
- На сравнение с коммерческими системами диктовки: их нельзя воспроизвести, и их метрики не опубликованы.
