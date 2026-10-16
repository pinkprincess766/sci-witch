# Грамматика как объект: FIRST, FOLLOW и LL(1)

Этап 3А плана ([`sci-witch-plan.md`](../research/sci-witch-plan.md)), часть 1.
Здесь разобраны два файла: [`math.ebnf`](math.ebnf) и [`chem.ebnf`](chem.ebnf).
Они описывают то, что принимают рукописные парсеры
`crates/sciwhisper-core/src/parser/math.rs` и `chemistry.rs`, а не то, что
хотелось бы. Код продукта не менялся.

## Как проверяется этот документ

Тест `crates/sciwhisper-core/tests/grammar_first_follow.rs` читает оба EBNF-файла
собственным разборщиком и сверяет их с кодом:

- каждое имя, на которое есть ссылка, определено; ни одно правило не осталось
  без использования; у каждого правила есть ссылка на функцию и строку, и
  такая функция в `math.rs` или `chemistry.rs` существует (допуск по строке
  `LINE_DRIFT_TOLERANCE` = 40);
- терминалы математики — ровно варианты `enum Tok`; фразы терминалов — ровно
  списки `operators.yaml` и `aliases.yaml`, ни одной лишней и ни одной
  пропущенной; предикат `guard_starts_atom` — ровно `atom_token_is_supported`;
- FIRST, FOLLOW и список конфликтов **вычисляются** из EBNF; таблицы ниже,
  между маркерами `<!-- grammar-analysis:… -->`, тест сравнивает с
  вычисленными символ в символ. Изменили грамматику — тест падает, пока
  таблицы не перепечатаны;
- каждая строка таблицы, у которой первая ячейка `math`, `phys` или `chem`,
  — это цитата вывода компилятора (`interpret_utterance`, режим
  `MixedText`, рендер Unicode, то же, что печатает `sciwhisper format`).
  Тест прогоняет их заново и сравнивает с написанным;
- у каждой проверки есть тест, который пытается её пройти неправильно:
  грамматика с выдуманным именем, с фразой, которой нет в yaml, с
  терминалом, которого нет в `Tok`, с вычеркнутым конфликтом, с подправленной
  вручную таблицей, с неверной цитатой вывода.

Проза проверяется только человеком. Что в ней утверждается про код, взято из
чтения кода (со ссылками на строки) или из прогона (таблицы).

Обновить таблицы после правки грамматики:

```
CARGO_TARGET_DIR=/tmp/sw-target GRAMMAR_PRINT=1 cargo test -p sciwhisper-core \
  --test grammar_first_follow -- --nocapture --test-threads=1 the_embedded_tables
```

Тест печатает блоки целиком; их копируют между соответствующими маркерами.

## Сводка

<!-- grammar-analysis:summary:begin -->
| Грамматика | Нетерминалов | Порождённых (скобки, циклы, группы) | Терминалов | Конфликтов LL(1) | Размер FIRST (из всех терминалов) | Размер FOLLOW |
|---|---|---|---|---|---|---|
| math.ebnf как записана | 32 | 46 | 62 | 32 | `common_atom`: 21 из 62 | 57 |
| math.ebnf, все закрывающие слова обязательны | 32 | 37 | 62 | 19 | `common_atom`: 21 из 62 | 57 |
| chem.ebnf | 22 | 21 | 28 | 5 | `species`: 13 из 28 | 6 |
<!-- grammar-analysis:summary:end -->

«Нетерминалов» — правила, записанные в файле (без `guard_starts_atom`).
«Порождённых» — служебные нетерминалы, которые анализ вводит на каждую пару
скобок, цикл и группу: `~O<n>` — необязательная часть `[ … ]`, `~R<n>` — цикл
`{ … }`, `~G<n>` — группа `( … | … )`, номер по порядку внутри правила.
«Конфликтов» — число узлов, где выбор по одному токену невозможен (определение
ниже). Последние два столбца — размеры FIRST и FOLLOW для «начала операнда»
(`common_atom` в математике, `species` в химии).

Что видно сразу:

1. В математике **32** узла с конфликтом. **13** из них существуют только потому,
   что закрывающие слова («конец дроби», «закрыть скобку»…) необязательны;
   при обязательных остаётся **19**.
2. FOLLOW операнда — почти весь алфавит терминалов и в записанной
   грамматике, и в варианте с обязательными закрывающими словами (последний
   столбец): после операнда в этом языке может стоять почти что угодно —
   постфиксная операция, оператор, закрывающее слово любой объемлющей
   конструкции, начало следующего множителя. Размер FOLLOW сам по себе два
   варианта не различает; их различает число конфликтов.
3. Химия — **5** конфликтов, но это не показатель простоты: её парсер устроен
   иначе (раздел «Химия»), и большая часть неоднозначности лежит в
   пересечении словарей, которое FIRST/FOLLOW по классам слов не видит.

## Что считается конфликтом

Грамматика LL(1), если для каждого нетерминала с несколькими продукциями
множества, по которым выбирается продукция, не пересекаются. Для продукции
`A → α` это FIRST(α), а если α может быть пустой — ещё и FOLLOW(A).
Для необязательной части `[ X ]` это выбор между «взять X» и «пропустить», для
цикла `{ X }` — между «ещё раз» и «выйти». Конфликт — непустое пересечение.
Общий токен — тот, по которому парсер с одним токеном вперёд не может решить.

Рукописный парсер всё равно решает, и решает по правилу, которого нет в
грамматике: жадность («взять, если можно»), порядок попыток, взгляд на второй
токен, откат, повторный разбор. Для каждого конфликта ниже указано, что
именно делает код.

Идентификатор вида `common_atom~O4` — узел внутри правила `common_atom`
(четвёртая по счёту необязательная часть). Идентификаторы стабильны, пока не
менялся текст EBNF.

## Математика: FIRST и FOLLOW

<!-- grammar-analysis:math:first-follow:begin -->
| Нетерминал | ε | FIRST | FOLLOW |
|---|---|---|---|
| `math_input` | нет | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FUNCTION` `FUNC_FILLER` `INF` `INTEGRAL` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NUM` `ORDINAL` `PARTIAL` `PLUS` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` | `$` |
| `eq_expr` | нет | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FUNCTION` `FUNC_FILLER` `INF` `INTEGRAL` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NUM` `ORDINAL` `PARTIAL` `PLUS` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `rel_op` | нет | `EQ` `GE` `GT` `LE` `LT` `NE` | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FUNCTION` `FUNC_FILLER` `INF` `INTEGRAL` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NUM` `ORDINAL` `PARTIAL` `PLUS` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` |
| `add_expr` | нет | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FUNCTION` `FUNC_FILLER` `INF` `INTEGRAL` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NUM` `ORDINAL` `PARTIAL` `PLUS` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `add_op` | нет | `MINUS` `PLUS` `PLUS_MINUS` | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FUNCTION` `FUNC_FILLER` `INF` `INTEGRAL` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NUM` `ORDINAL` `PARTIAL` `PLUS` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` |
| `mul_expr` | нет | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FUNCTION` `FUNC_FILLER` `INF` `INTEGRAL` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NUM` `ORDINAL` `PARTIAL` `PLUS` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `mul_op` | нет | `DIV` `TIMES` | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FUNCTION` `FUNC_FILLER` `INF` `INTEGRAL` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NUM` `ORDINAL` `PARTIAL` `PLUS` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` |
| `unary_expr` | нет | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FUNCTION` `FUNC_FILLER` `INF` `INTEGRAL` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NUM` `ORDINAL` `PARTIAL` `PLUS` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `juxt` | нет | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FUNCTION` `FUNC_FILLER` `INF` `INTEGRAL` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `L_BRACE` `L_BRACK` `L_PAREN` `NUM` `ORDINAL` `PARTIAL` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `juxt_item` | нет | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FUNCTION` `INF` `INTEGRAL` `LIMIT` `L_BRACE` `L_BRACK` `L_PAREN` `NUM` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `postfix` | нет | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FUNCTION` `FUNC_FILLER` `INF` `INTEGRAL` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `L_BRACE` `L_BRACK` `L_PAREN` `NUM` `ORDINAL` `PARTIAL` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `postfix_op` | нет | `CUBED` `DEGREE` `FACT` `POW_START` `SQUARED` `SUB_KW` `UNIT` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `atom` | нет | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FUNCTION` `FUNC_FILLER` `INF` `INTEGRAL` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `L_BRACE` `L_BRACK` `L_PAREN` `NUM` `ORDINAL` `PARTIAL` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `common_atom` | нет | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FUNCTION` `INF` `INTEGRAL` `LIMIT` `L_BRACE` `L_BRACK` `L_PAREN` `NUM` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `prefix_atom` | нет | `FUNC_FILLER` `LIMIT_LEFT` `LIMIT_RIGHT` `ORDINAL` `PARTIAL` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `symbol_atom` | нет | `SYM` `WEAK_SYM` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `application` | нет | `FROM` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `argument_list` | нет | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FUNCTION` `FUNC_FILLER` `INF` `INTEGRAL` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NUM` `ORDINAL` `PARTIAL` `PLUS` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `unit_expr` | нет | `UNIT` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `unit_factor` | нет | `UNIT` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `fraction_tail` | нет | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FUNCTION` `FUNC_FILLER` `INF` `INTEGRAL` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NUM` `NUMER` `ORDINAL` `PARTIAL` `PLUS` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `root_body` | нет | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FUNCTION` `FUNC_FILLER` `INF` `INTEGRAL` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NUM` `ORDINAL` `PARTIAL` `PLUS` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `nary_tail` | да | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FROM` `FUNCTION` `INF` `INTEGRAL` `LIMIT` `L_BRACE` `L_BRACK` `L_PAREN` `NUM` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `TO` `VECTOR_KW` `WEAK_SYM` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `integral_tail` | да | `ABS_KW` `BY` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FROM` `FUNCTION` `INF` `INTEGRAL` `INT_END` `LIMIT` `L_BRACE` `L_BRACK` `L_PAREN` `NUM` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `differential` | нет | `BY` `SYM` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `derivative_tail` | нет | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FROM` `FUNCTION` `FUNC_FILLER` `INF` `INTEGRAL` `LIMIT` `L_BRACE` `L_BRACK` `L_PAREN` `NUM` `ORDINAL` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `deriv_var` | нет | `AND_BY` `BY` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `limit_tail` | нет | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FROM` `FUNCTION` `FUNC_FILLER` `INF` `INTEGRAL` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `L_BRACE` `L_BRACK` `L_PAREN` `NUM` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `approach` | нет | `LIMIT_VAR` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `limit_target` | нет | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FUNCTION` `INF` `INTEGRAL` `LIMIT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NUM` `PLUS` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `direction` | нет | `LIMIT_LEFT` `LIMIT_RIGHT` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FROM` `FUNCTION` `FUNC_FILLER` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
| `mul_body` | нет | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FUNCTION` `INF` `INTEGRAL` `LIMIT` `L_BRACE` `L_BRACK` `L_PAREN` `NUM` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` | `$` `ABS_KW` `AND_BY` `BY` `COMMA` `CUBED` `DEGREE` `DELTA` `DENOM` `DERIVATIVE` `DIV` `ELLIPSIS` `EQ` `FACT` `FRAC_END` `FRAC_START` `FUNCTION` `GE` `GT` `INF` `INTEGRAL` `INT_END` `LE` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `LIMIT_VAR` `LT` `L_BRACE` `L_BRACK` `L_PAREN` `MINUS` `NE` `NUM` `PLUS` `PLUS_MINUS` `POW_END` `POW_START` `PRODUCT` `PROD_END` `ROOT` `ROOT_END` `ROOT_START` `R_BRACE` `R_BRACK` `R_PAREN` `SQUARED` `SUB_KW` `SUM` `SUM_END` `SYM` `TENDS` `TIMES` `TO` `UNIT` `VECTOR_KW` `WEAK_SYM` |
<!-- grammar-analysis:math:first-follow:end -->

Читать: `$` — конец входа. Столбец ε — может ли нетерминал быть пустым; в
математике пустыми бывают только `nary_tail` и `integral_tail` («сумма» и
«интеграл» без границ и без тела). Все уровни приоритета
(`eq_expr … unary_expr`) имеют один и тот же FIRST: токены, с которых может
начинаться выражение. Операторные правила (`rel_op`, `add_op`, `mul_op`)
имеют FIRST из своих токенов, а FOLLOW — токены начала операнда: после
оператора идёт операнд.

## Математика: конфликты LL(1)

<!-- grammar-analysis:math:conflicts:begin -->
| Идентификатор | Что | Конструкция | Общие токены |
|---|---|---|---|
| `add_expr~R1` | loop | `{ add_op mul_expr }` | `MINUS` `PLUS` `PLUS_MINUS` |
| `argument_list~R1` | loop | `{ COMMA mul_expr }` | `COMMA` |
| `common_atom~O1` | optional | `[ unit_expr ]` | `UNIT` |
| `common_atom~O2` | optional | `[ juxt_item ]` | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FUNCTION` `INF` `INTEGRAL` `LIMIT` `L_BRACE` `L_BRACK` `L_PAREN` `NUM` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` |
| `common_atom~O3` | optional | `[ R_PAREN ]` | `R_PAREN` |
| `common_atom~O4` | optional | `[ R_BRACK ]` | `R_BRACK` |
| `common_atom~O5` | optional | `[ R_BRACE ]` | `R_BRACE` |
| `common_atom~O6` | optional | `[ ROOT_END ]` | `ROOT_END` |
| `common_atom~O7` | optional | `[ SUM_END ]` | `SUM_END` |
| `common_atom~O8` | optional | `[ PROD_END ]` | `PROD_END` |
| `derivative_tail~R1` | loop | `{ deriv_var }` | `AND_BY` `BY` |
| `eq_expr~R1` | loop | `{ rel_op add_expr }` | `EQ` `GE` `GT` `LE` `LT` `NE` |
| `fraction_tail~O2` | optional | `[ FRAC_END ]` | `FRAC_END` |
| `integral_tail~G1` | group | `( FROM postfix TO postfix ¦ [ FROM ] )` | `FROM` |
| `integral_tail~O2` | optional | `[ mul_body ]` | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FUNCTION` `INF` `INTEGRAL` `LIMIT` `L_BRACE` `L_BRACK` `L_PAREN` `NUM` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` |
| `integral_tail~O3` | optional | `[ differential ]` | `BY` `SYM` |
| `integral_tail~O4` | optional | `[ INT_END ]` | `INT_END` |
| `juxt_item~R1` | loop | `{ postfix_op }` | `CUBED` `DEGREE` `FACT` `POW_START` `SQUARED` `SUB_KW` `UNIT` |
| `juxt~R1` | loop | `{ juxt_item }` | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FUNCTION` `INF` `INTEGRAL` `LIMIT` `L_BRACE` `L_BRACK` `L_PAREN` `NUM` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` |
| `limit_tail~O4` | optional | `[ direction ]` | `LIMIT_LEFT` `LIMIT_RIGHT` |
| `mul_body~R1` | loop | `{ juxt_item }` | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FUNCTION` `INF` `INTEGRAL` `LIMIT` `L_BRACE` `L_BRACK` `L_PAREN` `NUM` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` |
| `mul_body~R2` | loop | `{ mul_op unary_expr }` | `DIV` `TIMES` |
| `mul_expr~R1` | loop | `{ mul_op unary_expr }` | `DIV` `TIMES` |
| `nary_tail~O2` | optional | `[ TO postfix ]` | `TO` |
| `nary_tail~O3` | optional | `[ mul_body ]` | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FUNCTION` `INF` `INTEGRAL` `LIMIT` `L_BRACE` `L_BRACK` `L_PAREN` `NUM` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` |
| `postfix_op~O1` | optional | `[ POW_END ]` | `POW_END` |
| `postfix~R1` | loop | `{ postfix_op }` | `CUBED` `DEGREE` `FACT` `POW_START` `SQUARED` `SUB_KW` `UNIT` |
| `root_body` | alternatives | `2 alternatives` | `ABS_KW` `DELTA` `DERIVATIVE` `ELLIPSIS` `FACT` `FRAC_START` `FUNCTION` `FUNC_FILLER` `INF` `INTEGRAL` `LIMIT` `LIMIT_LEFT` `LIMIT_RIGHT` `L_BRACE` `L_BRACK` `L_PAREN` `NUM` `ORDINAL` `PARTIAL` `PRODUCT` `ROOT` `ROOT_START` `SUM` `SYM` `VECTOR_KW` `WEAK_SYM` |
| `symbol_atom~O1` | optional | `[ application ¦ NUM ]` | `NUM` |
| `unit_expr~O1` | optional | `[ DIV ]` | `DIV` |
| `unit_expr~R1` | loop | `{ DIV unit_factor }` | `DIV` |
| `unit_factor~O1` | optional | `[ SQUARED ¦ CUBED ]` | `CUBED` `SQUARED` |
<!-- grammar-analysis:math:conflicts:end -->

Все 32 узла сгруппированы по причинам. Заголовок группы перечисляет
идентификаторы; тест требует, чтобы каждый вычисленный идентификатор стоял в
заголовке `####`.

#### К1. Необязательные закрывающие слова: `common_atom~O3` `common_atom~O4` `common_atom~O5` `common_atom~O6` `common_atom~O7` `common_atom~O8` `fraction_tail~O2` `postfix_op~O1` `integral_tail~O4` `eq_expr~R1` `integral_tail~O3` `nary_tail~O2` `nary_tail~O3`

Пара правил: «взять закрывающее слово» против «пропустить»:
`[ R_PAREN ]`, `[ R_BRACK ]`, `[ R_BRACE ]`, `[ ROOT_END ]`, `[ SUM_END ]`,
`[ PROD_END ]` (все в `common_atom`), `[ FRAC_END ]` (в `fraction_tail`),
`[ POW_END ]` (в `postfix_op`), `[ INT_END ]` (в `integral_tail`). Общий токен
— само закрывающее слово: он в FIRST необязательной части и одновременно в
FOLLOW всей конструкции, потому что конструкции вкладываются друг в друга.
Остальные четыре узла (`eq_expr~R1`, `integral_tail~O3`, `nary_tail~O2`,
`nary_tail~O3`) — следствие: если закрывающее слово можно опустить, за
вложенной конструкцией может идти всё, что идёт за внешней, и FOLLOW
расползается; пересечения возникают на токенах отношений, `BY`, `TO` и на
начале операнда.

Что делает код: закрывающее слово достаётся **самой внутренней** открытой
конструкции, внешняя остаётся незакрытой (`parse_fraction` L715,
`parse_atom` L592-L610, `parse_postfix` L471). Одно слово меняет дерево:

| Домен | Фраза | Вывод компилятора |
|---|---|---|
| math | начало дроби а знаменатель начало дроби бэ знаменатель цэ конец дроби конец дроби плюс один | (a)/((b)/(c)) + 1 |
| math | начало дроби а знаменатель начало дроби бэ знаменатель цэ конец дроби плюс один | (a)/((b)/(c) + 1) |
| math | открыть скобку икс плюс открыть скобку игрек закрыть скобку закрыть скобку плюс один | (x + (y)) + 1 |
| math | открыть скобку икс плюс открыть скобку игрек закрыть скобку плюс один | (x + (y) + 1) |
| math | икс начало степени два плюс три конец степени плюс один | x^{2 + 3} + 1 |
| math | икс начало степени два плюс три плюс один | x^{2 + 3 + 1} |

Предупреждение при этом есть не у всех: скобка `()` и степень предупреждают
(confidence 0.95 → 0.7), дробь, `[]`, `{}`, сумма, интеграл — молча
(`math.ebnf`, пункт С10).

Остальные узлы группы, по одной фразе на узел (в строке видно, какое прочтение
выбрал парсер):

- `common_atom~O4` (`R_BRACK`), `common_atom~O5` (`R_BRACE`), `common_atom~O6`
  (`ROOT_END`): первые три строки; закрывающее слово взяла внутренняя
  скобка или корень, внешние остались открытыми, «плюс один» попало внутрь.
- `common_atom~O7` (`SUM_END`), `common_atom~O8` (`PROD_END`),
  `integral_tail~O4` (`INT_END`): четвёртая, пятая и шестая строки — одно
  закрывающее слово на две вложенные конструкции. Предупреждения нет.
- `integral_tail~O3` (`BY`, `SYM`): седьмая и восьмая строки. Внутренний
  интеграл без дифференциала берёт «по икс» себе, и у объемлющей производной не
  остаётся переменной; в восьмой строке «дэ икс» принадлежит интегралу, и «по
  игрек» достаётся производной.
- `nary_tail~O2` (`TO`): девятая строка. «до эн» взяла внутренняя сумма, у
  внешней верхней границы нет (`∑_{k=∑^{n}}`).
- `nary_tail~O3` (начало операнда): десятая и одиннадцатая строки. Тело
  внутренней суммы жадно берёт «эм ка»; во второй фразе «конец суммы» стоит
  после «эм», и «ка» достаётся телу внешней суммы, то есть дерево другое
  ((∑ m)·k вместо ∑ mk), но печать **одинаковая**: вывод не показывает
  различия (по дампу AST одноразового прогона `parse_math`).

| Домен | Фраза | Вывод компилятора |
|---|---|---|
| math | открыть квадратную скобку икс плюс открыть квадратную скобку игрек закрыть квадратную скобку плюс один | [x + [y] + 1] |
| math | открыть фигурную скобку икс плюс открыть фигурную скобку игрек закрыть фигурную скобку плюс один | {x + {y} + 1} |
| math | начало корня икс плюс начало корня игрек конец корня плюс один | √(x + √y + 1) |
| math | сумма от ка равно один до эн сумма от эм равно один до эн ка эм конец суммы | ∑_{k=1}^{n} ∑_{m=1}^{n} km |
| math | произведение от ка равно один до эн произведение от эм равно один до эн ка эм конец произведения | ∏_{k=1}^{n} ∏_{m=1}^{n} km |
| math | интеграл интеграл эф дэ икс конец интеграла | ∫ ∫ f dx |
| math | производная интеграл эф по икс | производная ∫ f dx |
| math | производная интеграл эф дэ икс по игрек | d(∫ f dx)/dy |
| math | сумма от ка равно сумма до эн | ∑_{k=∑^{n}} |
| math | сумма от ка равно один до эн сумма эм ка | ∑_{k=1}^{n} ∑ mk |
| math | сумма от ка равно один до эн сумма эм конец суммы ка | ∑_{k=1}^{n} ∑ mk |

#### К2. Жадная склейка и тело конструкции: `juxt~R1` `mul_body~R1` `common_atom~O2` `integral_tail~O2`

- `juxt~R1` — цикл `{ juxt_item }` (неявное умножение) против того, что идёт
  следом. Следом может идти тело суммы или произведения: нижняя граница
  `FROM eq_expr` кончается операндом, а тело `mul_body` начинается тоже
  операндом. Общие токены — все начала операнда. Код жадный: что можно
  склеить, склеивает. Сумма без «до» проглатывает тело в нижнюю границу;
  «до» отделяет её.
- `mul_body~R1` — тот же цикл внутри тела: тело берёт все соседние множители.
- `common_atom~O2` — `DELTA [ juxt_item ]`: «дельта» либо стоит сама, либо
  берёт следующий операнд. Код: если дальше число, аргумент не берётся,
  остаётся δ, а число идёт следующим множителем, с предупреждением
  (L558-L564); иначе берёт.
- `integral_tail~O2` — `[ mul_body ]` против `[ differential ]`, общий токен
  `SYM`: буква «д» в «эф дэ икс» — и множитель, и начало дифференциала. Код
  решает по **второму** токену (`is_differential_here`, L386) и флагу
  `stop_at_differential` (С4).

| Домен | Фраза | Вывод компилятора |
|---|---|---|
| math | икс ка | xk |
| math | икс запятая игрек | икс запятая игрек |
| math | икс и игрек | икс и игрек |
| math | сумма от ка равно один до эн ка | ∑_{k=1}^{n} k |
| math | сумма от ка равно один ка | ∑_{k=1k} |
| math | интеграл эф дэ икс | ∫ f dx |
| math | интеграл дэ | ∫ d |
| math | интеграл интеграл эф дэ икс дэ игрек | ∫ ∫ f dxdy |
| math | дельта икс | ΔX |
| math | дельта два | δ2 |

#### К3. Постфиксная операция против соседнего операнда: `postfix~R1` `juxt_item~R1`

Цикл `{ postfix_op }` против того, что может стоять после операнда: общие
токены — `FACT`, `SQUARED`, `CUBED`, `DEGREE`, `POW_START`, `SUB_KW`, `UNIT`.
Для `FACT` это буквальная неоднозначность: слово «факториал» бывает
постфиксом (`x!`) и приставкой (`FACT juxt`). Код выбирает постфикс, если
операнд уже есть. Для остальных токенов причина другая: аргумент функции,
модуля и вектора разбирается как `postfix`, и степень сразу за аргументом
уходит внутрь. Квадрат функции без скобок сказать нельзя: «синус в квадрате
икс» остаётся текстом (последние две строки).

| Домен | Фраза | Вывод компилятора |
|---|---|---|
| math | икс факториал игрек | x!y |
| math | факториал икс игрек | (xy)! |
| math | синус икс плюс один | sin(x) + 1 |
| math | синус икс в квадрате | sin(x²) |
| math | модуль икс в квадрате | \|x²\| |
| math | вектор эф в квадрате | f²⃗ |
| math | открыть скобку синус икс закрыть скобку в квадрате | (sin(x))² |
| math | синус в квадрате икс | синус в квадрате икс |

#### К4. Единицы измерения: `common_atom~O1` `unit_factor~O1` `unit_expr~R1` `unit_expr~O1`

- `common_atom~O1` — `NUM [ unit_expr ]` против `postfix_op = unit_expr`;
  токен `UNIT`. Одно и то же дерево получается двумя путями — безвредная
  неоднозначность записи, не языка.
- `unit_factor~O1` — степень единицы `[ SQUARED | CUBED ]` против
  `postfix_op = SQUARED`. Код: степень принадлежит единице («метров в квадрате»
  = м², а не (5 м)²).
- `unit_expr~R1` и `unit_expr~O1` — `{ DIV unit_factor }` и `[ DIV ]` против
  `mul_op = DIV`; общий токен `DIV`. Нужно смотреть на второй токен: за «на»
  идёт единица или нет. Код (`parse_unit_expr`, L1047) съедает «на» всегда, а
  единицу берёт, только если она есть.

| Домен | Фраза | Вывод компилятора |
|---|---|---|
| phys | десять метров в квадрате | 10 м² |
| phys | пять метров на секунду | 5 м/с |
| phys | пять метров на два | 5 м 2 |

Последняя строка — не просто конфликт, а потеря слова: «на» исчезло из
вывода (см. «Найденное в коде»).

#### К5. Корень без «конец корня»: `root_body` `add_expr~R1` `mul_expr~R1` `mul_body~R2`

- `root_body = postfix | add_expr` — две альтернативы с общим FIRST (все
  начала операнда): «корень из икс плюс один» — это √x + 1 или √(x + 1).
  Правило записано как объединение двух проходов парсера (`RootBinding`,
  L198): в каждом проходе одна альтернатива, выбор делает параметр, а не
  вход.
- `add_expr~R1`, `mul_expr~R1`, `mul_body~R2` — наведённые конфликты: если
  подкоренное — `add_expr`, то за ним может идти `PLUS`, `MINUS`, `TIMES`,
  `DIV` внешнего выражения.

Что делает код: первый проход — узкое чтение (`NextAtom`), второй
(`reparse_with`, L154) — широкое, и оно запускается **только если сразу после
подкоренного стоит `PLUS` или `MINUS`** (L635-L636). Для «корень из два икс»
и «корень из икс на два» широкое чтение отличается, но не предлагается.

| Домен | Фраза | Вывод компилятора |
|---|---|---|
| math | корень из икс плюс один | √x + 1 |
| math | начало корня икс плюс один конец корня | √(x + 1) |
| math | корень из два икс | √2x |
| math | начало корня два икс конец корня | √(2x) |
| math | корень из икс на два | √x/2 |

#### К6. «Висячие» разделители во вложенных конструкциях: `argument_list~R1` `derivative_tail~R1` `limit_tail~O4`

- `argument_list~R1` — `{ COMMA mul_expr }` против той же запятой у объемлющей
  функции. Запятую получает внутренняя.
- `derivative_tail~R1` — `{ deriv_var }` против `BY` у объемлющей
  производной. Все «по» получает внутренняя; у внешней переменной не остаётся,
  и фраза целиком остаётся текстом.
- `limit_tail~O4` — `[ direction ]` («слева», «справа») против такого же слова
  у объемлющего предела. Получает внутренний.

| Домен | Фраза | Вывод компилятора |
|---|---|---|
| math | эф от икс запятая игрек | f(x, y) |
| math | эф от гэ от икс запятая игрек | f(g(x, y)) |
| math | производная эф по икс по игрек | d²f/(dxdy) |
| math | производная производная эф по икс | производная df/dx |
| math | предел эф при икс стремящемся к пределу гэ при игрек стремящемся к нулю слева | lim_{x→lim_{y→0⁻} g} f |

#### К7. Нижняя граница интеграла: `integral_tail~G1`

`( FROM postfix TO postfix | [ FROM ] )`: обе альтернативы начинаются с
`FROM`, а различаются тем, встретится ли «до» после выражения произвольной
длины. Это не LL(k) ни при каком k. Код (`parse_integral`, L787-L799)
разбирает выражение, ищет «до» и, не найдя, **откатывает позицию** — единственное
место с откатом во всём `math.rs`.

| Домен | Фраза | Вывод компилятора |
|---|---|---|
| math | интеграл от нуля до единицы икс дэ икс | ∫₀¹ x dx |
| math | интеграл от эф дэ икс | ∫ f dx |

#### К8. Целое число после буквы: `symbol_atom~O1`

Правило «буква + целое → индекс» (`parse_symbol_atom`, L511, вызывается из
`parse_atom`, L553) добавило в грамматику один конфликт. `symbol_atom = SYM [
application | NUM ] | WEAK_SYM [ application ]`: токен `NUM` после буквы может
быть индексом (`[ NUM ]`) и началом следующего множителя (`juxt_item`), и общий
токен — `NUM`. Код берёт индекс жадно, и только после обычной буквы: слабая
буква (`WEAK_SYM`) индекс не берёт, школьная десятичная (`2,5`) — не индекс.
Правило стоит в `parse_atom`, а не в `postfix_op`: после `parse_atom` слабая и
обычная буква — один и тот же `Math::Symbol`, и `postfix_op` не мог бы их
различить. Тот же индекс бывает и явным («икс индекс два», `SUB_KW` в
`postfix_op`); дерево одно. Ограничения — `math.ebnf`, пункт С15.

| Домен | Фраза | Вывод компилятора |
|---|---|---|
| math | икс два | x₂ |
| math | икс индекс два | x₂ |
| math | икс два игрек | x₂y |
| math | икс два три | x₂3 |
| math | икс два целых пять | x2,5 |
| math | два икс в квадрате | 2x² |
| math | а два | а два |
| math | и два | и два |

«икс два три» — индекс «два» и множитель «три»; «икс два игрек» — индекс и
множитель; «икс два целых пять» — токен `2,5`, а не индекс, поэтому произведение;
«а два» и «и два» не индекс и не произведение, остаются текстом. Числа в
индексе до правки давали произведение (`x2`), теперь — `x₂`.

### Остальное, что не конфликт, но влияет на разбор

| Домен | Фраза | Вывод компилятора |
|---|---|---|
| math | икс равно нулю плюс один | x = 0 + 1 |
| math | открыть скобку икс равно игрек равно зет | (x = y = z) |
| math | предел эф при икс стремящемся к нулю слева | lim_{x→0⁻} f |
| math | предел при икс стремящемся к нулю слева эф | lim_{x→0⁻} f |

«икс равно нулю плюс один»: «равно нулю» токенизатор раскладывает на `EQ` и
`NUM("0")`, поэтому остальное разбирается как продолжение выражения (L1341).

## Гипотеза плана

> Граничные команды («начало дроби … конец дроби») делают вложенные
> конструкции LL(1), а естественные (без границ) дают неоднозначность;
> `RootBinding` и `reparse_with` в `math.rs` — прямой её симптом.

**Вердикт: подтверждена частично.** Измерение: тест
`mandatory_closers_remove_the_nesting_conflicts_and_only_those` берёт ту же
грамматику и заменяет каждое необязательное закрывающее слово (`R_PAREN`,
`R_BRACK`, `R_BRACE`, `FRAC_END`, `POW_END`, `ROOT_END`, `SUM_END`,
`PROD_END`, `INT_END`) обязательным. Строка «все закрывающие слова
обязательны» в сводке.

1. **Подтверждено для границ, которые обязательны.** Из 32 узлов 13 (группа
   К1) исчезают целиком. Ни в одном из оставшихся нет закрывающего токена.
   Если бы «конец дроби» был обязательным, вложенные дроби, скобки, степени
   и корни с границами не давали бы конфликтов: пара «начало … конец» — это
   как раз то, что делает вложенность однозначной за один токен.
2. **Опровергнуто для кода как он есть.** Граничные команды в `math.rs`
   **необязательны** (кроме «знаменатель»). Парсер принимает дробь без
   «конец дроби», скобку без «закрыть скобку», степень без «конец степени»;
   для дроби, `[]`, `{}`, суммы, произведения и интеграла даже без
   предупреждения (С10). Поэтому вложенные конструкции с границами в коде не
   LL(1): закрывающее слово достаётся самой внутренней, и добавление одного
   слова меняет дерево (строки К1). Это не «естественная речь без границ», а
   конструкция с границами, у которых конец можно не говорить.
3. **Конфликтов, не связанных с границами, 19, и они остаются при любых
   границах.** Они лежат там, где у конструкции границы нет вообще:
   неявное умножение (К2), постфиксные операции и приставки (К3), единицы
   (К4), корень (К5), разделители «по», «запятая», «слева» во вложенных
   конструкциях (К6), нижняя граница интеграла (К7), число после буквы (К8). То есть неоднозначность
   есть и у естественной речи (как предполагала гипотеза), и у записанных
   границ с необязательным концом (чего гипотеза не предполагала).
4. **`RootBinding` и `reparse_with` — симптом, но не единственный и не
   полный.** Верно, что это прямое следствие конфликта `root_body`: две
   альтернативы с общим FIRST, и код разрешает их не грамматикой, а вторым
   проходом (С6). Но (а) это единственный из 32 узлов, где неоднозначность
   вынесена наружу как `alternatives`; остальные 31 решается молча и жадно
   (К1-К8); (б) второй проход запускается только при `PLUS`/`MINUS` после
   подкоренного, хотя конфликты `mul_expr~R1` и `mul_body~R2` показывают, что
   то же самое есть для `TIMES` и `DIV`, а строки К5 — что для неявного
   умножения («корень из два икс»).

Чего это не доказывает: что язык, а не запись, неоднозначен. Конфликт LL(1)
говорит о месте, где одного токена мало; неоднозначность языка — это два
разных дерева у одной фразы. Такие фразы тут показаны (К1, К3, К5), но
перебор по длине и руками доказанная однозначность подграммы с границами —
отдельная часть плана и **здесь не сделаны**.

## Химия

Парсер химии устроен иначе, чем математический. Он не рекурсивный спуск по
токенам, а работа над словами и словарями (`chem.ebnf`, заголовок):
фраза режется по связкам, каждый кусок читается как вещество **по порядку
попыток**, и побеждает первая подошедшая. Это упорядоченный выбор (как в
PEG), а не выбор КС-грамматики. FIRST/FOLLOW по классам слов показывает, где
порядок решает; он не видит слов, которые принадлежат двум классам сразу
(таблица пересечений ниже).

### FIRST и FOLLOW

<!-- grammar-analysis:chem:first-follow:begin -->
| Нетерминал | ε | FIRST | FOLLOW |
|---|---|---|---|
| `chem_input` | нет | `ANIONIC_COMPLEX_WORD` `ANION_CLASS` `CLASS_ADJECTIVE` `CLASS_WORD` `ELECTRON_NAME` `ELEMENT_WORD` `FUNCTION_LETTER` `HYDROCARBON_WORD` `INTEGER` `ION_LEAD_IN` `ION_MARKER` `LETTER_NAME` `REACTION_OPEN` `SUBSTANCE_NAME` | `$` |
| `reaction` | нет | `ANIONIC_COMPLEX_WORD` `ANION_CLASS` `CLASS_ADJECTIVE` `CLASS_WORD` `ELECTRON_NAME` `ELEMENT_WORD` `FUNCTION_LETTER` `HYDROCARBON_WORD` `INTEGER` `ION_LEAD_IN` `ION_MARKER` `LETTER_NAME` `REACTION_OPEN` `SUBSTANCE_NAME` | `$` |
| `side` | нет | `ANIONIC_COMPLEX_WORD` `ANION_CLASS` `CLASS_ADJECTIVE` `CLASS_WORD` `ELECTRON_NAME` `ELEMENT_WORD` `FUNCTION_LETTER` `HYDROCARBON_WORD` `INTEGER` `ION_LEAD_IN` `ION_MARKER` `LETTER_NAME` `SUBSTANCE_NAME` | `$` `ARROW_JOINT` `REACTION_NOUN` |
| `side_joint` | нет | `CONJUNCTION` `JOIN_REAGENT` `PLUS` | `ANIONIC_COMPLEX_WORD` `ANION_CLASS` `CLASS_ADJECTIVE` `CLASS_WORD` `ELECTRON_NAME` `ELEMENT_WORD` `FUNCTION_LETTER` `HYDROCARBON_WORD` `INTEGER` `ION_LEAD_IN` `ION_MARKER` `LETTER_NAME` `SUBSTANCE_NAME` |
| `species` | нет | `ANIONIC_COMPLEX_WORD` `ANION_CLASS` `CLASS_ADJECTIVE` `CLASS_WORD` `ELECTRON_NAME` `ELEMENT_WORD` `FUNCTION_LETTER` `HYDROCARBON_WORD` `INTEGER` `ION_LEAD_IN` `ION_MARKER` `LETTER_NAME` `SUBSTANCE_NAME` | `$` `ARROW_JOINT` `CONJUNCTION` `JOIN_REAGENT` `PLUS` `REACTION_NOUN` |
| `species_head` | нет | `ANIONIC_COMPLEX_WORD` `ANION_CLASS` `CLASS_ADJECTIVE` `CLASS_WORD` `ELECTRON_NAME` `ELEMENT_WORD` `FUNCTION_LETTER` `HYDROCARBON_WORD` `INTEGER` `ION_LEAD_IN` `ION_MARKER` `LETTER_NAME` `SUBSTANCE_NAME` | `$` `ARROW_JOINT` `CONJUNCTION` `HYDRATE_WORD` `JOIN_REAGENT` `PLUS` `REACTION_NOUN` |
| `species_core` | нет | `ANIONIC_COMPLEX_WORD` `ANION_CLASS` `CLASS_ADJECTIVE` `CLASS_WORD` `ELECTRON_NAME` `ELEMENT_WORD` `FUNCTION_LETTER` `HYDROCARBON_WORD` `ION_LEAD_IN` `ION_MARKER` `LETTER_NAME` `SUBSTANCE_NAME` | `$` `ARROW_JOINT` `CONJUNCTION` `HYDRATE_WORD` `JOIN_REAGENT` `PLUS` `REACTION_NOUN` |
| `cationic_complex` | нет | `ION_LEAD_IN` | `$` `ARROW_JOINT` `CONJUNCTION` `HYDRATE_WORD` `JOIN_REAGENT` `PLUS` `REACTION_NOUN` |
| `anionic_complex` | нет | `ANIONIC_COMPLEX_WORD` | `$` `ARROW_JOINT` `CONJUNCTION` `HYDRATE_WORD` `JOIN_REAGENT` `PLUS` `REACTION_NOUN` |
| `material_class` | нет | `CLASS_ADJECTIVE` `CLASS_WORD` | `$` `ARROW_JOINT` `CONJUNCTION` `HYDRATE_WORD` `JOIN_REAGENT` `PLUS` `REACTION_NOUN` |
| `oxidation` | нет | `INTEGER` `OXIDATION_MARKER` `ROMAN` | `$` `ARROW_JOINT` `CONJUNCTION` `ELEMENT_BY_WORD` `HYDRATE_WORD` `JOIN_REAGENT` `PLUS` `REACTION_NOUN` `STATE_MARKER` |
| `marked_species` | нет | `ANION_CLASS` `ELEMENT_WORD` `FUNCTION_LETTER` `ION_MARKER` `LETTER_NAME` | `$` `ARROW_JOINT` `CONJUNCTION` `HYDRATE_WORD` `JOIN_REAGENT` `PLUS` `REACTION_NOUN` |
| `ion_species` | нет | `ANION_CLASS` `ELEMENT_WORD` `ION_MARKER` | `$` `ARROW_JOINT` `CONJUNCTION` `HYDRATE_WORD` `JOIN_REAGENT` `PLUS` `REACTION_NOUN` `STATE_MARKER` |
| `ion_body` | нет | `ANION_CLASS` `ELEMENT_WORD` | `$` `ARROW_JOINT` `CHARGE_MINUS` `CONJUNCTION` `HYDRATE_WORD` `INTEGER` `ION_MARKER` `JOIN_REAGENT` `PLUS` `REACTION_NOUN` `STATE_MARKER` |
| `ion_sign` | нет | `CHARGE_MINUS` `PLUS` | `$` `ARROW_JOINT` `CONJUNCTION` `HYDRATE_WORD` `JOIN_REAGENT` `PLUS` `REACTION_NOUN` `STATE_MARKER` |
| `systematic_salt` | нет | `ANION_CLASS` | `$` `ARROW_JOINT` `CONJUNCTION` `HYDRATE_WORD` `JOIN_REAGENT` `PLUS` `REACTION_NOUN` `STATE_MARKER` |
| `spelled_formula` | нет | `ELEMENT_WORD` `FUNCTION_LETTER` `LETTER_NAME` | `$` `ARROW_JOINT` `CONJUNCTION` `HYDRATE_WORD` `JOIN_REAGENT` `PLUS` `REACTION_NOUN` `STATE_MARKER` |
| `spelled_item` | нет | `ELEMENT_WORD` `FUNCTION_LETTER` `GROUPING` `LETTER_NAME` | `$` `ARROW_JOINT` `CONJUNCTION` `ELEMENT_WORD` `FUNCTION_LETTER` `GROUPING` `HYDRATE_WORD` `JOIN_REAGENT` `LETTER_NAME` `PLUS` `REACTION_NOUN` `STATE_MARKER` |
| `element_item` | нет | `ELEMENT_WORD` `FUNCTION_LETTER` `LETTER_NAME` | `$` `ARROW_JOINT` `CONJUNCTION` `ELEMENT_WORD` `FUNCTION_LETTER` `GROUPING` `HYDRATE_WORD` `JOIN_REAGENT` `LETTER_NAME` `PLUS` `REACTION_NOUN` `STATE_MARKER` |
| `spelled_element` | нет | `ELEMENT_WORD` `FUNCTION_LETTER` `LETTER_NAME` | `$` `ARROW_JOINT` `CONJUNCTION` `ELEMENT_WORD` `FUNCTION_LETTER` `GROUPING` `HYDRATE_WORD` `INTEGER` `JOIN_REAGENT` `LETTER_NAME` `PLUS` `REACTION_NOUN` `STATE_MARKER` |
| `element_pair` | нет | `FUNCTION_LETTER` `LETTER_NAME` | `$` `ARROW_JOINT` `CONJUNCTION` `ELEMENT_WORD` `FUNCTION_LETTER` `GROUPING` `HYDRATE_WORD` `INTEGER` `JOIN_REAGENT` `LETTER_NAME` `PLUS` `REACTION_NOUN` `STATE_MARKER` |
| `pair_word` | нет | `FUNCTION_LETTER` `LETTER_NAME` | `$` `ARROW_JOINT` `CONJUNCTION` `ELEMENT_WORD` `FUNCTION_LETTER` `GROUPING` `HYDRATE_WORD` `INTEGER` `JOIN_REAGENT` `LETTER_NAME` `PLUS` `REACTION_NOUN` `STATE_MARKER` |
<!-- grammar-analysis:chem:first-follow:end -->

Формула по буквам (`spelled_formula`): FIRST — слово-элемент, название
латинской буквы или служебное слово-буква; слово «дважды» первым быть не
может, потому что группировать ему нечего (`element_item` обязателен первым).

### Конфликты LL(1)

<!-- grammar-analysis:chem:conflicts:begin -->
| Идентификатор | Что | Конструкция | Общие токены |
|---|---|---|---|
| `chem_input` | alternatives | `2 alternatives` | `ANIONIC_COMPLEX_WORD` `ANION_CLASS` `CLASS_ADJECTIVE` `CLASS_WORD` `ELECTRON_NAME` `ELEMENT_WORD` `FUNCTION_LETTER` `HYDROCARBON_WORD` `INTEGER` `ION_LEAD_IN` `ION_MARKER` `LETTER_NAME` `SUBSTANCE_NAME` |
| `ion_body~O1` | optional | `[ INTEGER ]` | `INTEGER` |
| `ion_species~O4` | optional | `[ ion_sign ]` | `PLUS` |
| `marked_species~G1` | group | `( ion_species ¦ systematic_salt ¦ spelled_formula ¦ ELEMENT_WORD )` | `ANION_CLASS` `ELEMENT_WORD` |
| `spelled_element` | alternatives | `4 alternatives` | `FUNCTION_LETTER` `LETTER_NAME` |
<!-- grammar-analysis:chem:conflicts:end -->

#### Х1. Реакция или одно вещество: `chem_input`

`reaction | species` — обе начинаются с вещества, FIRST совпадают почти
целиком. Выбор требует просмотреть **всю** фразу до связки-стрелки
(`parse_reaction`, L72-L110 режет весь список слов до решения), то есть
неограниченный взгляд вперёд. Код: сначала `parse_reaction`; нет стрелки —
читает как вещество.

| Домен | Фраза | Вывод компилятора |
|---|---|---|
| chem | гидроксид меди два | Cu(OH)₂ |
| chem | гидроксид меди два превращается в оксид меди два плюс вода | Cu(OH)₂ → CuO + H₂O |
| chem | реакция идёт быстрее | реакция идёт быстрее |

#### Х2. «плюс» — заряд или связка, число — заряд или степень окисления: `ion_species~O4` `ion_body~O1`

`ion_species~O4` — `[ ion_sign ]`, общий токен `PLUS`: «плюс» в конце иона
— знак заряда, но то же слово — связка между реагентами (`side_joint`).
Решает `plus_is_a_charge` (L82-L88): заряд, только если дальше не начинается
новое вещество (`starts_a_species`, L242) — взгляд вперёд на неограниченное
число слов. `ion_body~O1` — `[ INTEGER ]` после элемента против `[ INTEGER ]`
после тела иона: «ион меди два» — степень окисления или величина заряда;
два числа подряд код отвергает (L512).

| Домен | Фраза | Вывод компилятора |
|---|---|---|
| chem | ион меди два плюс | Cu²⁺ |
| chem | ион серебра плюс ион хлора превращается в хлорид серебра осадок | Ag⁺ + Cl⁻ → AgCl↓ |
| chem | ион меди два три | Cu²⁺ три |
| chem | ион серебра плюс ион хлора | Ag⁺ плюс Cl⁻ |
| chem | ион натрия плюс ион хлора | Na⁺ плюс Cl⁻ |
| chem | ион хлора плюс ион серебра | Cl⁻ плюс Ag⁺ |
| chem | ион серебра и ион хлора | Ag⁺ и Cl⁻ |

Последние четыре строки — фразы без стрелки: их читают промежутки
`utterance.rs`, а не `parse_reaction`; см. Д3 (дефект исправлен).

#### Х3. Какой словарь: `marked_species~G1`

`( ion_species | systematic_salt | spelled_formula | ELEMENT_WORD )`,
общие токены `ANION_CLASS` и `ELEMENT_WORD`. Название аниона начинает и ион, и
систематическую соль; слово-элемент — ион, формулу по буквам и одиночный
элемент. Порядок попыток (L388-L403): ион, соль, формула по буквам,
элемент. Самое неприятное следствие — вещество из двух словарей читается
по-разному в зависимости от пути (строки «водород»; для названия
двухатомного элемента это исправлено, см. Д2).

| Домен | Фраза | Вывод компилятора |
|---|---|---|
| chem | гидроксид меди два осадок | Cu(OH)₂↓ |
| chem | водород | H₂ |
| chem | водород газ | H₂↑ |
| chem | кислород газ | O₂↑ |
| chem | углекислый газ | CO₂ |
| chem | вода газ | вода газ |
| chem | медный купорос | CuSO₄·5H₂O |

#### Х4. Пара букв или две буквы: `spelled_element`

`element_pair | ELEMENT_WORD | LETTER_NAME | FUNCTION_LETTER`: общие токены
`LETTER_NAME`, `FUNCTION_LETTER`. «эн а» — это Na (таблица пар проверяется
раньше, `chemistry_element_at`, L803), а не N и «а».

| Домен | Фраза | Вывод компилятора |
|---|---|---|
| chem | эн а о аш | NaOH |
| chem | аш два эс о четыре | H₂SO₄ |
| chem | марганец о два | MnO₂ |
| chem | натрий и калий стоят рядом | натрий и калий стоят рядом |
| chem | два и три | два и три |
| chem | о | O₂ |
| chem | купрум о аш дважды | Cu(OH)₂ |
| chem | дважды | дважды |

### Пересечения словарей

Слова, принадлежащие двум классам. FIRST и FOLLOW считают классы
непересекающимися терминалами; здесь — то, чего они не видят. Список
вычисляется из загруженного лексикона.

<!-- grammar-analysis:overlaps:begin -->
| Область | Пересечение |
|---|---|
| математика | `в знаменателе` (`delimiters.denominator`) начинается со слова, которое также: латинская буква, кириллическая буква |
| математика | `в квадрат` (`postfix_power.squared`) начинается со слова, которое также: латинская буква, кириллическая буква |
| математика | `в квадрате` (`postfix_power.squared`) начинается со слова, которое также: латинская буква, кириллическая буква |
| математика | `в куб` (`postfix_power.cubed`) начинается со слова, которое также: латинская буква, кириллическая буква |
| математика | `в кубе` (`postfix_power.cubed`) начинается со слова, которое также: латинская буква, кириллическая буква |
| математика | `в минус первой` (`postfix_power.inverse`) начинается со слова, которое также: латинская буква, кириллическая буква |
| математика | `в минус первую` (`postfix_power.inverse`) начинается со слова, которое также: латинская буква, кириллическая буква |
| математика | `в степени` (`postfix_power.degree`) начинается со слова, которое также: латинская буква, кириллическая буква |
| математика | `в числителе` (`delimiters.numerator`) начинается со слова, которое также: латинская буква, кириллическая буква |
| математика | `дельта` (`delimiters.delta`) начинается со слова, которое также: греческая буква |
| математика | `и по` (`delimiters.and_by`) начинается со слова, которое также: латинская буква, кириллическая буква |
| химия | имя вещества и название элемента: `азот` `водород` `кислород` `хлор` |
| химия | название элемента и название латинской буквы: `аш` `ка` `о` `пэ` `це` `цэ` `эн` `эс` `эф` |
| химия | имя вещества и название аниона или катиона: — |
| химия | название элемента и название аниона или катиона: — |
| химия и математика | служебное слово-буква `а`: буква |
| химия и математика | служебное слово-буква `б`: буква |
| химия и математика | служебное слово-буква `в`: буква |
| химия и математика | служебное слово-буква `ж`: буква |
| химия и математика | служебное слово-буква `же`: буква |
| химия и математика | служебное слово-буква `и`: буква |
| химия и математика | служебное слово-буква `к`: только служебное слово |
| химия и математика | служебное слово-буква `о`: элемент и буква |
| химия и математика | служебное слово-буква `с`: буква |
| химия и математика | служебное слово-буква `у`: буква |
| химия и математика | служебное слово-буква `я`: только служебное слово |
<!-- grammar-analysis:overlaps:end -->

Что из этого следует для разбора:

- Слово «в» (латинская `v`) и фразы «в квадрате», «в степени», «в знаменателе»:
  токенизатор берёт самую длинную ключевую фразу раньше буквы
  (`tokenize`, L1131 перед L1164), и «икс в квадрате» — степень, а не
  произведение «икс v квадрат». Это разрешение порядком, не грамматикой.
- «дельта» — ключевое слово раньше греческой буквы δ; сама буква получается
  только на пути без аргумента (L565-L567).
- Служебные слова-буквы (`FUNCTION_LETTER`) — союзы и предлоги. В математике
  слабая буква не склеивается, если дальше операнд, в химии допускается
  только в контексте (`spelled_context`, L747). «и» ещё и связка реакции
  (`CONJUNCTION`), и этот конфликт FIRST/FOLLOW тоже не видит: у классов
  разные имена. Он проверяется запусками: «натрий и калий стоят рядом» и
  «два и три» остаются текстом, «марганец о два» читается как MnO₂.
  Слова `к` и `я` стоят в списке, хотя ни в одном словаре букв и элементов их
  нет: сегодня запись в списке ничего не меняет.
- Вещества и элементы: `азот`, `водород`, `кислород`, `хлор` есть в обоих
  словарях. Результат одинаковый (`H₂`, `O₂`…) только на пути через словарь
  веществ; через путь «формула по буквам» получался атом («водород газ» →
  `H↑`). Исправлено (Д2): название двухатомного элемента даёт молекулу и с
  маркером; буквы («аш газ») по-прежнему дают атом `H↑`, как продиктовано.

## Найденное в коде (вне задачи, ничего не исправлено)

Всё ниже найдено по ходу анализа запусками; код продукта не менялся.

**Д1. Слово «на» после единицы пропадает.** `пять метров на два` печатается
как `5 м 2` (строка в К4; `math.rs` L1047: `DIV` съеден, единица не добавлена).
Слово ушло без следа и без предупреждения.

**Д2. «газ» после названия элемента меняет формулу. Исправлено.** `водород газ`
печатался как `H↑`, `кислород газ` как `O↑`, а `водород` — `H₂`; теперь `H₂↑`
и `O₂↑` (строки в Х3). Исправление — `try_named_diatomic` перед `try_spelled`;
то же теперь для азота, фтора, хлора, брома и йода, в том числе без маркера
(`бром` → `Br₂`, раньше `Br`). Дефект, как он был найден: причина —
порядок попыток в `parse_species_inner`: словарь веществ проверяется до
отрывания маркера (L344), после отрывания слово читает `try_spelled` (L398),
который возвращает атом, а правило двухатомности для одиночного элемента
стоит позже (L403) и до него дело не доходит. Опасное переписывание: ничто в
выводе не говорит, что водород стал атомарным.

**Д3. Ион хлора превращается в молекулу хлора. Исправлено.** `ион натрия плюс
ион хлора` печатался как `Na⁺ Cl₂`, `ион хлора плюс ион серебра` — как `Cl⁺ Ag`
(строки в Х2; слова «плюс» и второй «ион» исчезли); теперь `Na⁺ плюс Cl⁻` и
`Cl⁻ плюс Ag⁺`. Причина: `try_ion` убирал слова-маркеры иона с любого места,
и участок «ион натрия плюс ион» читался как Na⁺ со знаком «плюс»; маркер после
знака заряда теперь отказывает разбор. Кроме того, `stops_before_an_oxidation_state`
больше не считает степенью окисления число, за которым стоит «иона»
(«ион кальция плюс два иона хлора»). Со словом «и» вместо «плюс» (`ион серебра и ион хлора`) результат был верным
и остался: `Ag⁺ и Cl⁻`.

**Д4. Вложенный интеграл теряет флаг дифференциала.** `parse_integral`
выключает `stop_at_differential` по выходу (L803, L806), не восстанавливая
прежнее значение, в отличие от `stop_at_comma` (L841-L843). В
`интеграл интеграл эф дэ икс дэ игрек` внутренний интеграл съедает «дэ икс»,
после чего внешний уже не останавливается на «дэ игрек» и берёт его множителем.
Разбор одноразовым прогоном `parse_math`: внешний `Integral { integrand:
Some(Juxt([Integral {…}, Symbol(d), Symbol(y)])), wrt: None }`. Печать
выглядит правильно (`∫ ∫ f dxdy`, строка в К2), поэтому дефект не виден по
выводу; дерево неверное. Не проверяется тестом.

**Д5. Предупреждения при откате интеграла дублируются.** `интеграл от икс
начало степени два дэ икс`: один и тот же «unclosed power; inserting anyway»
выдан дважды, потому что пробный разбор после «от» (L789) добавил
предупреждение, откат (L797) вернул позицию, а список предупреждений не
откатил. Прогон `parse_math`, не проверяется тестом.

**Д6. Рекурсия разбора ограничена `MAX_PARSE_DEPTH`.** Раньше ни одна
функция `parse_*` не считала вложенность, и публичный `interpret` на 1000
«открыть скобку» в потоке со стеком 8 МиБ (отладочная сборка) убивал
процесс. Теперь каждое вложенное подвыражение входит через `Parser::nested`
(`math.rs`, проверка на L241); выше 64 — обычная ошибка разбора, фраза
остаётся словами. 64 меньше `MAX_MATH_DEPTH` (128) в `validate.rs`.
`interpret_utterance` по-прежнему ограничен `MAX_UTTERANCE_WORDS = 400` и
`MAX_SPAN_WORDS = 64` (`utterance.rs` L33-L35).

**Д7. Широкое чтение корня предлагается не всегда** (К5): только после `PLUS`
и `MINUS`.

**Д8. «дельта икс» печатается как `ΔX` с заглавной буквой** (строка в К2):
`parse_atom` L570-L574 поднимает любую одиночную латинскую букву до заглавной
(задумано для «дельта же» = ΔG). Для математики «дельта икс» — Δx; возможно,
это нужно ограничить физикой, но решать владельцу.

## Эталон Earley и сравнение с парсером

Грамматику `math.ebnf` разбирает независимый распознаватель Earley
(крейт `crates/sciwhisper-grammar`, вне пути продукта; пустые правила —
по Aycock–Horspool, пределы `MAX_TOKENS` и `MAX_ITEMS`). Тест
`crates/sciwhisper-grammar/tests/differential.rs` прогоняет классы токенов
каждой фразы четырёх корпусов (`research::math_token_classes`, режимы
математики и физики) через распознаватель и через рукописный парсер и
сверяет результат с [`grammar-differential-v1.json`](../../research/results/grammar-differential-v1.json).

На 02.10.2026, оба режима вместе: из 808 прогонов (404 фразы × 2 режима) 670 не доходят до
сравнения — токенизатор математики отказывает на обычных словах (вся
`prose-negatives-v2`, большая часть остальных). Из 138 сравнённых: оба
принимают 114, оба отвергают 14, **только парсер — 0**, только грамматика —
10. Все 10 — буквенная химия в режиме математики («аш два о», «цэ о два»,
«аш два эс о четыре»): грамматика принимает надмножество, потому что
запрет на «слабую» букву после числа или индекса (`may_join_juxtaposition`,
семантические ограничения в `math.ebnf`) в КС-правилах не выражен. Это
ожидаемое расхождение, а не дефект: в автоматическом режиме эти фразы
читает химия. Расхождений, где код принимает то, чего нет в грамматике, не
найдено.

Число деревьев вывода принятой цепочки считается по той же таблице Earley,
без упакованного леса: динамика по завершённым правилам (`count_parses` в
`crates/sciwhisper-grammar/src/earley.rs`). Предел `MAX_PARSES` равен 1000.
Ровно 1000 — точное число. Больше тысячи и цикл через пустое слово
(`A → A`, повтор `{ [ A ] }`) дают «≥ 1000», и счёт возвращает управление.
Отчёт — [`grammar-differential-v2.json`](../../research/results/grammar-differential-v2.json).
Клетки «принимает / не принимает» в нём те же, что в v1. Знаменатель —
фразы, которые Earley принял (оба принимают + только грамматика): 124 из
808 прогонов. Это число деревьев КС-грамматики. Семантические ограничения
по-прежнему записаны прозой, а рукописный парсер по-прежнему строит одно
дерево упорядоченным выбором.

| корпус | режим | принято | 1 | 2 | 3–10 | > 10 | из них ≥ 1000 |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ambiguous-v2 | математика | 9 | 5 | 2 | 2 | 0 | 0 |
| ambiguous-v2 | физика | 9 | 5 | 2 | 2 | 0 | 0 |
| dev-seed-v3 | математика | 39 | 31 | 6 | 2 | 0 | 0 |
| dev-seed-v3 | физика | 59 | 31 | 19 | 9 | 0 | 0 |
| nomenclature-v1 | математика | 4 | 4 | 0 | 0 | 0 | 0 |
| nomenclature-v1 | физика | 4 | 4 | 0 | 0 | 0 | 0 |
| prose-negatives-v2 | математика | 0 | 0 | 0 | 0 | 0 | 0 |
| prose-negatives-v2 | физика | 0 | 0 | 0 | 0 | 0 | 0 |
| вместе | | 124 | 80 | 29 | 15 | 0 | 0 |

Больше одного дерева — у 44 прогонов из 124. Выше десяти — ни одного.
Самое большое число — 8: физика, «девять целых восемьдесят одна метра на
секунду в квадрате». По 6: «интеграл пять ньютонов по два метра»,
«интеграл эф дэ икс» (оба режима), «производная десять метров по пять
секунд». Тридцать фраз с наибольшими числами лежат в отчёте; при равном
числе порядок — текст, корпус, режим.

Дальше по плану 3А: восстановление после ошибок распознавания. PCFG —
следующий раздел, упакованный лес — раздел «Упакованный лес разборов».

## PCFG: вероятности правил

Каждой продукции `math.ebnf` после перевода в BNF (`productions` в
`crates/sciwhisper-grammar/src/earley.rs`; `[A]`, `{A}` и группы — свои
нетерминалы с суффиксами `~O`, `~R`, `~G`) приписана вероятность. Отчёт —
[`pcfg-v1.json`](../../research/results/pcfg-v1.json), тест
`crates/sciwhisper-grammar/tests/pcfg_report.rs`.

Как считается:

- Обучение — фразы split `train` четырёх корпусов, оценка — `validation`.
  Записи `dev_holdout` пропускаются до токенизации.
- Все деревья принятой фразы перечисляет `derivations`, их не больше
  `MAX_PARSES`. Фраза с N деревьями добавляет 1/N к каждому использованию
  продукции в каждом дереве (`expected_counts` в `pcfg.rs`). Это упрощение:
  план (`sci-witch-plan.md`, пункт PCFG) для неоднозначных фраз называл
  inside-outside, то есть EM. В v1 его нет; EM по перечисленным деревьям
  добавлен в v2 (подраздел ниже).
- Вероятность продукции — частота среди продукций той же левой части со
  сглаживанием add-one (`PSEUDOCOUNT` = 1.0). Без сглаживания продукция,
  которой не было в train, давала бы фразе validation log P = −∞.
- `log P(фраза)` — логарифм суммы вероятностей всех её деревьев
  (`sentence_log_prob` в `likelihood.rs`, через log-sum-exp). Перплексия —
  на токен: exp(−Σ log P / число классов токенов), натуральный логарифм.
- Базовая линия — равномерная PCFG: все альтернативы одной левой части
  равновероятны.
- Собственность (сумма вероятностей всех конечных деревьев равна 1)
  проверяет `check_consistency` в `consistency.rs`. Критерий Бута–Томпсона:
  спектральный радиус ρ матрицы средних чисел потомков меньше 1. Проверка
  односторонняя: если ‖Mᵏ‖∞ < 1 для какой-то степени k ≤ 2^20, то ρ < 1.
  Критическая грамматика (ρ = 1) остаётся «не доказано».

| режим | train: фраз в модели / всего | validation: фраз в модели / всего | токенов validation | перплексия (обученная) | перплексия (равномерная) | validation с продукциями, которых нет в train | собственность обученной | собственность равномерной |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- |
| математика | 25 / 217 | 19 / 123 | 82 | 17.73 | 265.76 | 12 из 19 | да, ‖M⁶⁴‖∞ = 0.53 | не доказана |
| физика | 35 / 217 | 25 / 123 | 104 | 14.82 | 228.23 | 11 из 25 | да, ‖M⁶⁴‖∞ = 0.16 | не доказана |

Остальные фразы до модели не дошли: в математике токенизатор отказал на
189 фразах train и 101 фразе validation, Earley отверг по 3. Больше
`MAX_PARSES` деревьев не оказалось ни у одной фразы.

Что из этого следует и чего не следует:

- **Это демонстрация метода, а не модель языка.** В модели 25–35 фраз
  train и 114–143 токена. Больше половины фраз validation требуют
  продукцию, которой в train не было, и их вероятность держится на
  сглаживании. Перплексия обученной модели на самом train — 8.83 и 9.02,
  на validation — 17.73 и 14.82.
- **Равномерная PCFG не собственная** (доказано в v2, подраздел ниже). Часть
  вероятности уходит на бесконечные деревья, и её перплексия — оценка по
  дефектной модели, а не базовая линия. Отношение «17.7 против 265.8»
  поэтому не мера качества грамматики.
- Обученная модель собственная в обоих режимах. Это ожидаемо: оценки по
  частотам на деревьях дают собственную PCFG (Chi, Geman 1998), а
  добавление единицы здесь этого не испортило — это проверено, а не
  предположено.

### EM и нижняя граница ρ (pcfg-v2)

Отчёт [`pcfg-v2.json`](../../research/results/pcfg-v2.json), тест
`crates/sciwhisper-grammar/tests/pcfg_report_v2.rs`. Данные и разбиение те же,
что в v1; числа модели «деление поровну» совпадают с v1, это проверяет тест.

- **EM.** `em_estimate` в `pcfg.rs`: дерево фразы получает вес P(дерево) /
  Σ P(деревьев этой фразы) при текущей модели, затем вероятности пересчитываются
  со сглаживанием add-one. Со сглаживанием это MAP-оценка при априорном
  распределении Дирихле; растёт целевая функция с априорным слагаемым, а не
  само правдоподобие. Старт — деление поровну, `EM_ITERATIONS` = 100. Деревья
  уже перечислены, поэтому таблица inside-outside не нужна.
- **Нижняя граница ρ.** `rho_lower_bound` в `consistency.rs`: для
  неотрицательной матрицы tr(Mᵏ) ≤ n·ρᵏ, значит ρ ≥ (tr(Mᵏ)/n)^(1/k). Если
  граница больше 1 и все нетерминалы, достижимые из начального, продуктивны
  (`unproductive_nonterminals` пуст), PCFG не собственная (Бут–Томпсон).

| режим | модель | перплексия train | перплексия validation | ‖Mᵏ‖∞ < 1 | ρ ≥ |
| --- | --- | ---: | ---: | --- | ---: |
| математика | поровну (v1) | 8.83 | 17.73 | да, k = 64 | 0.964 |
| математика | EM | 8.85 | 18.01 | да, k = 64 | 0.965 |
| математика | равномерная | 229.21 | 265.76 | нет | 1.245 |
| физика | поровну (v1) | 9.02 | 14.82 | да, k = 64 | 0.945 |
| физика | EM | 8.73 | 14.51 | да, k = 64 | 0.944 |
| физика | равномерная | 222.66 | 228.23 | нет | 1.245 |

- **Равномерная PCFG доказанно не собственная:** ρ ≥ 1.245, непродуктивных
  нетерминалов нет. Граница посчитана в f64 без интервальной арифметики, но
  она далеко от 1 по сравнению с ошибкой округления.
- **У обученных моделей ρ зажат с двух сторон:** не меньше 0.94–0.96 и меньше
  1. Они собственные, но близки к критической точке: средний вывод длинный.
- **EM почти ничего не меняет.** В физике перплексия validation падает с
  14.82 до 14.51, в математике растёт с 17.73 до 18.01. Неоднозначных фраз
  мало, а фраз validation — 19 и 25; значима ли разница, не проверялось, и
  на таком объёме вывода из неё делать нельзя. Целевая функция EM
  растёт в обоих режимах; в математике правдоподобие train при этом слегка
  падает, потому что EM здесь максимизирует MAP, а не правдоподобие.

## Неоднозначность: перебор коротких цепочек

Пункт плана «Неоднозначность», способ (2): перебрать все цепочки классов
токенов ограниченной длины, которые выводит `math.ebnf`, и для каждой
сосчитать деревья. Цепочки перечисляет `sentences_up_to` в
`crates/sciwhisper-grammar/src/generate.rs` (до `MAX_LEN` = 4: на длине 5
цепочек уже миллионы), деревья считает `count_parses`. Отчёт —
[`grammar-ambiguity-v1.json`](../../research/results/grammar-ambiguity-v1.json),
тест `crates/sciwhisper-grammar/tests/ambiguity_report.rs`. Обычный прогон
теста сверяет длины 1–3; длину 4 (167 368 цепочек, около трёх минут в
отладочной сборке) — только с `GRAMMAR_AMBIGUITY_FULL=1`.

| длина | цепочек | неоднозначных | 2 дерева | 3–10 | больше 10 | наибольшее число |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 9 | 0 | 0 | 0 | 0 | — |
| 2 | 229 | 47 | 47 | 0 | 0 | 2 |
| 3 | 6182 | 2367 | 1546 | 821 | 0 | 6 |
| 4 | 167368 | 89381 | 43705 | 44831 | 845 | 17 |

Каждая сгенерированная цепочка принята распознавателем, ни одна не дошла до
`MAX_PARSES`. Фразы корпусов из `grammar-differential-v2.json`, чья цепочка
не длиннее 4, найдены перебором с тем же числом деревьев (тест сверяет 14
таких фраз).

Что видно:

- Самые короткие неоднозначные цепочки — длины 2, все 47 по два дерева, и
  все объясняются уже найденными конфликтами LL(1):
  - 36 начинаются с `DELTA`, `SUM`, `PRODUCT` или `INTEGRAL`: конструкция с
    необязательным телом (`DELTA [ juxt_item ]`, `SUM nary_tail`), за которой
    стоит ещё один множитель. Множитель — либо тело, либо следующий
    сомножитель неявного произведения. Конфликт К2. Пример: `DELTA SYM`.
  - 9 начинаются с `ROOT`: корень без «конец корня», конфликт К5.
  - `SYM NUM` — К8: число после буквы — индекс или множитель.
  - `NUM UNIT` — К4: единица измерения.
- На длине 4 неоднозначна больше половины цепочек (89 381 из 167 368). Доля
  растёт с длиной: 0, 21 % и 38 % на длинах 1–3.

Чего это не значит. Рукописный парсер строит одно дерево упорядоченным
выбором и для этих конструкций берёт тело жадно, так что неоднозначность
грамматики — не ошибка вывода. Семантические ограничения из `math.ebnf`
(прозой) здесь не применяются, поэтому часть цепочек парсер вовсе
отвергает. Цепочки — классы токенов, не слова: одна цепочка соответствует
многим фразам, и не всякая из них встречается в речи. Перебор доказывает
неоднозначность найденных цепочек, но ничего не говорит о длинах больше 4.

## Упакованный лес разборов

`parse_forest` (`crates/sciwhisper-grammar/src/earley/forest.rs`) строит лес
по той же таблице Earley, что `count_parses`: один узел на пару «нетерминал,
отрезок», у узла — список способов его вывести (продукция и узлы её
нетерминалов). Деревья леса — ровно те, что считает `count_parses`; тесты
сверяют это и с `derivations` на случайных грамматиках. Это не
бинаризованный SPPF Скотта: способ вывода перечисляет всех детей продукции
сразу. Для коротких правых частей `math.ebnf` этого хватает.

На неоднозначной сумме `s = s "+" s | "x"` с 12 слагаемыми 58 786 деревьев
укладываются в 78 узлов. На фразах корпусов наоборот: лес больше списка
деревьев. Отчёт —
[`grammar-forest-v1.json`](../../research/results/grammar-forest-v1.json),
тест `crates/sciwhisper-grammar/tests/forest_report.rs`, все разбиения
(ничего не обучается).

| корпус | режим | фраз | наибольшее число деревьев | наибольшее число узлов |
| --- | --- | ---: | ---: | ---: |
| dev-seed-v3 | математика | 39 | 6 | 82 |
| dev-seed-v3 | физика | 59 | 8 | 82 |
| ambiguous-v2 | оба | 9 | 3 | 59 |
| nomenclature-v1 | оба | 4 | 1 | 39 |
| prose-negatives-v2 | оба | 0 | — | — |

Фразы короткие (до 9 классов токенов), а грамматика проходит через длинные
цепочки нетерминалов с одним ребёнком (`add_expr` → `mul_expr` →
`unary_expr` → `juxt` → `postfix`, плюс служебные `~O`, `~R`, `~G`), поэтому узлов много, а деревьев
мало: «интеграл эф дэ икс» — 6 деревьев и 82 узла. Экономия от леса на этих
данных нулевая; он нужен там, где число деревьев растёт быстрее числа
отрезков.

## Химия: Earley и сравнение с парсером

То же сравнение для `chem.ebnf`: тест
`crates/sciwhisper-grammar/tests/differential_chem.rs`, отчёт
[`grammar-differential-chem-v1.json`](../../research/results/grammar-differential-chem-v1.json).
Терминалы здесь — классы слов, их выдаёт `research::chem_token_classes`
(слово, которому не подошёл ни один класс, — отказ токенизатора). Начало —
первое синтаксическое правило, `chem_input`. Парсер «принял», если
`interpret` в `Domain::Chemistry` вернул `Node::Chemical` с уверенностью выше
нуля; отказ приходит как `Node::Text` с уверенностью 0. Те же четыре корпуса.
Записать заново: `GRAMMAR_CHEM_WRITE=1`.

На 09.10.2026, 404 фразы:

| корпус | фраз | оба принимают | оба отвергают | только грамматика | только парсер | токенизатор отказал |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| dev-seed-v3 | 113 | 29 | 0 | 2 | 0 | 82 |
| ambiguous-v2 | 58 | 10 | 1 | 0 | 0 | 47 |
| nomenclature-v1 | 53 | 22 | 3 | 7 | 0 | 21 |
| prose-negatives-v2 | 180 | 0 | 0 | 0 | 0 | 180 |
| вместе | 404 | 61 | 4 | 9 | 0 | 330 |

Три вывода.

1. **Только парсер — 0.** Ни одной фразы, которую код читает как химию, а
   цепочка классов не выводится. Но это про 74 сравнённые фразы, а не про
   404: 330 фраз до грамматики не дошли. Отказавшие фразы парсер тоже не
   принимает (список `tokenize_fail_parser_accepts` в отчёте пуст), так что
   слепого пятна «парсер принял, а грамматику спросить нельзя» нет.
2. **Проза в этом сравнении не участвует.** Все 180 фраз `prose-negatives-v2`
   — отказ токенизатора («поправка», «журнал» — не слова ни одного класса).
   Свойство «обычная речь не становится формулой» это сравнение не проверяет;
   его проверяют ворота компилятора на этом корпусе.
3. **Только грамматика — 9 прогонов, 8 разных фраз.** Это ожидаемое
   надмножество (семантическая часть `chem.ebnf` не проверяется), но каждую
   фразу объяснили по коду, см. ниже.

Только парсер (0 фраз): объяснять нечего.

Только грамматика, все 9 прогонов (в скобках корпус):

- «феррит бария» (dev-seed-v3, nomenclature-v1) и «феррит меди»
  (nomenclature-v1). Объяснено поведением парсера: `try_material_class`
  отказывает, потому что шаблон шпинельного феррита подтверждён не для всех
  металлов (`material-classes.yaml`); грамматика `CLASS_WORD ELEMENT_BY_WORD`
  принимает любой элемент. В Х1–Х11 этого ограничения нет.
- «гидроксид» (dev-seed-v3). Объяснено, но это неточность `chem.ebnf`:
  `try_ion` требует хотя бы одно слово-маркер «ион» (`is_ion_marker`), а
  `ion_species` в грамматике оба маркера делает необязательными, хотя
  комментарий над правилом говорит «хотя бы один обязателен». Парсер прав,
  грамматика шире своего комментария.
- «гексацианоуглерод калия» (nomenclature-v1). Объяснено: центрального атома
  «углерод» нет в таблице анионных центров (`coordination.yaml`), парсер
  отказывает. Токенизатор при этом даёт `ANIONIC_COMPLEX_WORD`: по замыслу
  `chem_token_classes` относит к классу и слово, которое парсер узнаёт как
  комплекс, но отвергает (`anionic_claims`).
- «гексацианокупрат натрия» (nomenclature-v1). Объяснено: у меди степени
  окисления 1 и 2, а в названии она не указана; парсер отказывает.
  В грамматике `[ oxidation ]` необязательна. В Х1–Х11 нет.
- «метен» и «метин» (nomenclature-v1). Объяснено: `organic.rs` отказывает
  алкену и алкину с одним атомом углерода (нужно хотя бы два). Комментарий над
  `HYDROCARBON_WORD` это знает («метин — отказ»), Х9 перечисляет только
  `MAX_CARBONS`.
- «эф икс» (nomenclature-v1). Объяснено:
  «эф» — `ELEMENT_WORD`, «икс» — `LETTER_NAME`, а `chemistry_element` берёт
  букву только если она символ элемента (`elements_by_symbol`); X такой
  буквой не является. Класс `LETTER_NAME` в грамматике — любая латинская
  буква, то есть шире кода. Неточность описания, парсер прав.

Дефектов парсера среди расхождений нет: семь прогонов из девяти — осознанные
отказы парсера («такого катиона, центра, длины цепи нет в таблице»), два
(«гидроксид», «эф икс») — неточность `chem.ebnf`, шире кода. `chem.ebnf` не
менялся. Неверных классов в этих девяти прогонах `chem_token_classes` не
дал; надо лишь помнить его правило про отказанные, но узнанные комплексы и
углеводороды (см. выше).

Число деревьев вывода (70 принятых цепочек): 1 дерево — 64, 2 — одна, 3–10 —
пять, больше десяти — ни одной.

| корпус | принято | 1 | 2 | 3–10 | > 10 |
| --- | ---: | ---: | ---: | ---: | ---: |
| dev-seed-v3 | 31 | 30 | 1 | 0 | 0 |
| ambiguous-v2 | 10 | 7 | 0 | 3 | 0 |
| nomenclature-v1 | 29 | 27 | 0 | 2 | 0 |
| prose-negatives-v2 | 0 | 0 | 0 | 0 | 0 |

Двойное дерево — «ион меди два плюс»: число после элемента читается и как
степень окисления (`ion_body`), и как заряд (`ion_species`), это Х2. Три
дерева — реакция, где сторона начинается одним словом-элементом («натрий
взаимодействует с хлором…»): слово выводится тремя путями, как `ion_species`,
`spelled_formula` и `ELEMENT_WORD` (Х3, а «без маркера» тут добавляет
неточность `ion_species`). Четыре дерева — две ионные реакции из
nomenclature-v1, 2 × 2 по Х2. Это чтение из таблицы, а не отдельная проверка.

## Исправление ошибок распознавания на уровне слов (A4)

Шесть фраз `ambiguous-v2` с тегом `asr-error` — химия с ошибкой внутри слова,
и до грамматики `math.ebnf` они не доходят. Поэтому A4 сделан на уровне слов,
а не токенов: `nearest_lexicon_words` в `crates/sciwhisper-core/src/research.rs`
ищет слова словаря химии (названия элементов, анионов, веществ и отдельные
слова многословных названий) на наименьшем расстоянии Левенштейна, не больше
`MAX_WORD_EDITS` = 2, для слов не короче `MIN_CORRECTED_WORD_CHARS` = 5.
Ничьи возвращаются все, выбор не делается. В решётку и интерпретатор функция
не подключена; ворота компилятора не изменились.

Отчёт [`word-repair-v1.json`](../../research/results/word-repair-v1.json),
тест `crates/sciwhisper-core/tests/word_repair_report.rs`. Слово заменяется,
только если ответ единственный; `d1` — на расстоянии 1, `d2` — до 2.
«Верно» — то же правило, что у детерминированного прогона `sciwhisper-eval`
(первый кандидат при уровне не ниже 0.9, иначе текст). `dev_holdout` не
читается.

| корпус | фраз | верно без правки | `d1`: стало верно / стало неверно | `d2`: стало верно / стало неверно | `d2`: ложных научных переписываний |
| --- | ---: | ---: | --- | --- | ---: |
| dev-seed-v3 | 92 | 92 | 0 / 0 | 0 / 9 | 0 |
| ambiguous-v2 | 49 | 29 | 3 / 0 | 3 / 0 | 0 |
| nomenclature-v1 | 53 | 53 | 0 / 0 | 0 / 7 | 4 |
| prose-negatives-v2 | 146 | 146 | 0 / 0 | 0 / 0 | 0 |

- **`d1` чинит три фразы и больше ничего не меняет:** «карбанат кальция»,
  «хларид натрия», «сернея кислота». Две из трёх находит и `VowelRepair`,
  но детерминированный прогон его кандидата не берёт;
  «сернея» → «серная» (е → я) `VowelRepair` не умеет.
- **«гидраксид железа три» и «кидроксидж лезо три» верны и без правки:** их
  уже исправляет нормализация (`normalize::words`). «кидроксидж» к тому же
  даёт ничью «гидроксид» / «гидроксида», и правило его не тронуло бы.
- **«гырдксд жлз тр»** остаётся текстом, как и должно: ни одно слово
  словаря не ближе двух правок («гидроксид» — в четырёх).
- **`d2` вредит научным фразам, а не прозе.** «метан» становится «титан»,
  «метра» — «сера», «феррит» — «феррум», «октан» — «титан»: 16 верных фраз
  становятся неверными, из них 4 — ложные научные переписывания в
  `nomenclature-v1`. В `prose-negatives-v2` заменено 26 слов («вчера» →
  «сера», «налей» → «калий»), но парсер ни одну из этих фраз научной не
  прочитал.

Вывод на этих данных: расстояние 1 с единственным ответом безопасно и
полезно, расстояние 2 — нет. Данных мало: три починенные фразы — это все
фразы с такой ошибкой в корпусах. Ошибки живого ASR (voice-v1) могут быть
другими.

## Ограничения анализа

Что не покрыто и где EBNF приближённо описывает код.

1. **Лексический слой не анализируется.** Терминалы математики — токены
   `Tok`; как слова превращаются в токены (самая длинная фраза, потом число,
   потом буква, потом единица — `tokenize`, L1111-L1191), в FIRST/FOLLOW не
   входит. Пересечения классов слов разрешены этим порядком и перечислены
   выше, а не вычислены как конфликты.
2. **Запятая стёрта.** В синтаксических правилах `COMMA` есть только в списке
   аргументов; в семи других местах парсер её пропускает, а между двумя
   операндами **не** пропускает (она рвёт неявное произведение, «икс запятая
   игрек» — текст). Стирание неточно именно в этом (`math.ebnf`, С1).
3. **Предикаты и условия на значения.** `may_join_juxtaposition` (С2),
   `is_differential_here` (С4), правило числа после «дельты» (С5), порядок
   производной (С8), направление предела (С9), число после буквы (С15) — условия на значения, а не на
   токены; грамматика принимает больше, чем код. Явно: она допускает «два а
   икс», которое код отвергает.
4. **`starts_atom` и FIRST(atom) различны** (С3); тела конструкций записаны
   через `juxt_item`, чтобы это учесть, но `UNIT` и `POW_START` в защитном
   предикате остаются, и конструкция с ними в начале тела отвергается внутри
   `parse_atom`, а не защитой.
5. **`root_body` — объединение двух проходов.** Конфликт на нём есть по
   построению: в одном проходе парсера альтернатива одна.
6. **«равно нулю», «в степени N», «в минус первой»** раскладываются
   токенизатором на несколько токенов и в синтаксисе отдельных терминалов не
   имеют; они записаны в лексической части (`EQ_ZERO`, `DEGREE_ORDINAL`,
   `DEGREE_INVERSE`, пометка `emits`).
7. **Описана допустимость, не дерево.** EBNF говорит, какие цепочки токенов
   принимаются; какое дерево строится, видно только из кода и из таблиц с
   выводом (приоритет, левая ассоциативность, `Juxt`, `Group`).
8. **Предупреждения и откаты.** Что разбор принял с предупреждением (`unclosed
   parenthesis`, `unclosed power`), в EBNF отдельно не отмечено; они только в С10.
9. **Число конфликтов — свойство записи, а не языка.** Тот же язык можно
   записать иначе и получить другое число. Конфликт — место, где нужна
   дополнительная информация, но не доказательство неоднозначности фразы;
   различие показано в разделе «Гипотеза плана».
10. **Химия — не КС-выбор.** Её порядок попыток, глобальное разбиение по
    связкам и неограниченный взгляд вперёд в `plus_is_a_charge` в грамматике
    выражены лишь комментариями и разделом «семантические ограничения» в
    конце `chem.ebnf`. Координационные названия («гексацианоферрат») —
    морфология внутри одного слова; в грамматике они один терминал.
11. **Выбор домена** (`interpret.rs`: `route`) в грамматику не входит: он
    пробует химию, математику и физику и сравнивает результаты и ключевые
    слова.
12. **Деление на промежутки** (`utterance.rs`) в грамматику не входит; поэтому
    на уровне вывода отказ разбора может выглядеть как «часть фразы стала
    формулой» (Д3).
13. **Ещё впереди** (части плана 3А): доказательство однозначности подграммы
    с границами руками; восстановление после ошибок (Ахо–Петерсон).
    Упакованный лес есть (раздел «Упакованный лес разборов»). PCFG есть: v1 —
    деление поровну, v2 — EM (раздел «PCFG: вероятности правил»). Перебор
    неоднозначных цепочек до длины 4 есть (раздел «Неоднозначность: перебор
    коротких цепочек»). Сравнение
    с корпусом есть: клетки в `grammar-differential-v1.json`, распределение
    числа разборов в `grammar-differential-v2.json`.
14. **Номера строк** в ссылках — для этой копии `math.rs` и `chemistry.rs`;
    тест допускает смещение до 40 строк и требует, чтобы функция существовала.
    После слияния чужих правок их надо пересверить.
