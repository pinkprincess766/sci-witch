#!/usr/bin/env python3
"""Builds research/data/dev-seed-v1.jsonl and its manifest.

The gold answers in this file are written **by hand from intent**. This script
never imports, calls or shells out to sciwhisper-core: a benchmark whose gold
is `interpret(transcript).ast` would only prove that the parser agrees with
itself.

Gold follows the AST conventions the project already documents (implicit
multiplication is `Juxt`, a quantity is `Juxt[Number, Unit]`, a species carries
its own coefficient, charge and state marker). Several records deliberately
encode an answer the current grammar cannot produce; those are findings, not
mistakes.
"""

import hashlib
import json
import pathlib
from collections import Counter

OUT = pathlib.Path(__file__).resolve().parent
JSONL = OUT / "dev-seed-v1.jsonl"
MANIFEST = OUT / "dev-seed-v1.manifest.json"
SCHEMA_VERSION = 1

# --------------------------------------------------------------- AST helpers

def atom(symbol, count=1):
    return {"Atom": {"symbol": symbol, "count": count}}

def group(parts, count):
    return {"Group": {"inner": {"parts": parts}, "count": count}}

def hydrate(count):
    return {"Hydrate": {"count": count}}

def formula(*parts):
    return {"parts": list(parts)}

def species(f, coefficient=1, charge=None, marker=None):
    return {"coefficient": coefficient, "formula": f, "charge": charge, "marker": marker}

def chem(s):
    return {"Chemical": {"Species": s}}

def reaction(left, right, arrow="Forward", condition=None):
    return {"Chemical": {"Equation": {"left": left, "arrow": arrow,
                                      "right": right, "condition": condition}}}

def math(node):
    return {"Math": node}

def num(text):
    return {"Number": text}

def sym(letter, case="Lower"):
    return {"Symbol": {"letter": letter, "alphabet": "Latin", "case": case}}

def greek(letter, case="Lower"):
    return {"Symbol": {"letter": letter, "alphabet": "Greek", "case": case}}

def binary(op, left, right):
    return {"Binary": {"op": op, "left": left, "right": right}}

def juxt(*items):
    return {"Juxt": list(items)}

def fraction(numerator, denominator):
    return {"Fraction": {"num": numerator, "den": denominator}}

def power(base, exponent):
    return {"Power": {"base": base, "exp": exponent}}

def subscript(base, sub):
    return {"Subscript": {"base": base, "sub": sub}}

def root(radicand, index=None):
    return {"Root": {"index": index, "radicand": radicand}}

def paren(inner):
    return {"Group": {"kind": "Paren", "inner": inner}}

def fn(kind, arg):
    return {"Function": {"kind": kind, "arg": arg}}

def summation(var=None, start=None, end=None, body=None):
    return {"Sum": {"var": var, "from": start, "to": end, "body": body}}

def integral(integrand=None, wrt=None, start=None, end=None):
    return {"Integral": {"from": start, "to": end, "integrand": integrand, "wrt": wrt}}

def derivative(kind, expr, variables):
    return {"Derivative": {"kind": kind, "expr": expr,
                           "variables": [{"variable": v, "order": o} for v, o in variables]}}

def limit(variable, target, direction, body):
    return {"Limit": {"variable": variable, "target": target,
                      "direction": direction, "body": body}}

def unit(*factors):
    return {"Unit": {"factors": [{"symbol": s, "power": p, "divide": d} for s, p, d in factors]}}

def quantity(value, *factors):
    return juxt(num(value), unit(*factors))

def neg(inner):
    return {"UnaryMinus": inner}

def delta(inner):
    return {"Delta": inner}

def vector(inner):
    return {"Vector": inner}

def absolute(inner):
    return {"Abs": inner}

def factorial(inner):
    return {"Factorial": inner}

INFINITY = "Infinity"

# ------------------------------------------------------------------ records

RECORDS = []

def ast_record(family, suffix, transcript, domain, target, tags, render=None):
    entry = {
        "dataset_schema_version": SCHEMA_VERSION,
        "id": f"{family}-{suffix}",
        "family_id": family,
        "provenance": "handcrafted_text",
        "human_transcript": transcript,
        "asr_hypotheses": [],
        "target_domain": domain,
        "target_action": "ast",
        "target_ast": target,
        "split": None,
        "tags": tags,
        "speaker_id": None,
    }
    if render:
        entry["expected_render"] = {"unicode": render}
    RECORDS.append(entry)

def raw_record(family, suffix, transcript, tags):
    RECORDS.append({
        "dataset_schema_version": SCHEMA_VERSION,
        "id": f"{family}-{suffix}",
        "family_id": family,
        "provenance": "handcrafted_text",
        "human_transcript": transcript,
        "asr_hypotheses": [],
        "target_domain": "plain",
        "target_action": "raw",
        "target_ast": None,
        "split": None,
        "tags": tags,
        "speaker_id": None,
    })

# ------------------------------------------------------------------ chemistry

C = "chemistry"
water = formula(atom("H", 2), atom("O"))
ast_record("chem-water-001", "a", "вода", C, chem(species(water)), ["formula", "substance"], "H₂O")
ast_record("chem-water-001", "b", "аш два о", C, chem(species(water)), ["formula", "spelled"])

sulfuric = formula(atom("H", 2), atom("S"), atom("O", 4))
ast_record("chem-sulfuric-001", "a", "серная кислота", C, chem(species(sulfuric)), ["formula", "acid"], "H₂SO₄")
ast_record("chem-sulfuric-001", "b", "аш два эс о четыре", C, chem(species(sulfuric)), ["formula", "spelled"])

ast_record("chem-hydrochloric-001", "a", "соляная кислота", C,
           chem(species(formula(atom("H"), atom("Cl")))), ["formula", "acid"])
ast_record("chem-nitric-001", "a", "азотная кислота", C,
           chem(species(formula(atom("H"), atom("N"), atom("O", 3)))), ["formula", "acid"])
ast_record("chem-ammonia-001", "a", "аммиак", C,
           chem(species(formula(atom("N"), atom("H", 3)))), ["formula", "substance"])
ast_record("chem-methane-001", "a", "метан", C,
           chem(species(formula(atom("C"), atom("H", 4)))), ["formula", "organic"])

co2 = formula(atom("C"), atom("O", 2))
ast_record("chem-co2-001", "a", "углекислый газ", C, chem(species(co2)), ["formula", "trivial-name"])
ast_record("chem-co2-001", "b", "диоксид углерода", C, chem(species(co2)), ["formula", "systematic-name"])

nacl = formula(atom("Na"), atom("Cl"))
ast_record("chem-nacl-001", "a", "поваренная соль", C, chem(species(nacl)), ["formula", "trivial-name"])
ast_record("chem-nacl-001", "b", "хлорид натрия", C, chem(species(nacl)), ["formula", "systematic-name"])

naoh = formula(atom("Na"), atom("O"), atom("H"))
ast_record("chem-naoh-001", "a", "гидроксид натрия", C, chem(species(naoh)), ["formula", "hydroxide"])
ast_record("chem-naoh-001", "b", "едкий натр", C, chem(species(naoh)), ["formula", "trivial-name"])

ast_record("chem-caoh2-001", "a", "гидроксид кальция", C,
           chem(species(formula(atom("Ca"), group([atom("O"), atom("H")], 2)))),
           ["formula", "hydroxide", "group"], "Ca(OH)₂")
ast_record("chem-feoh3-001", "a", "гидроксид железа три", C,
           chem(species(formula(atom("Fe"), group([atom("O"), atom("H")], 3)))),
           ["formula", "hydroxide", "oxidation-state"])
ast_record("chem-fe2o3-001", "a", "оксид железа три", C,
           chem(species(formula(atom("Fe", 2), atom("O", 3)))),
           ["formula", "oxide", "oxidation-state"])
ast_record("chem-so3-001", "a", "оксид серы шесть", C,
           chem(species(formula(atom("S"), atom("O", 3)))), ["formula", "oxide"])
ast_record("chem-al2o3-001", "a", "оксид алюминия", C,
           chem(species(formula(atom("Al", 2), atom("O", 3)))), ["formula", "oxide"])
ast_record("chem-k2so4-001", "a", "сульфат калия", C,
           chem(species(formula(atom("K", 2), atom("S"), atom("O", 4)))), ["formula", "salt"])
ast_record("chem-alcl3-001", "a", "хлорид алюминия", C,
           chem(species(formula(atom("Al"), atom("Cl", 3)))), ["formula", "salt"])

ast_record("chem-cu-ion-001", "a", "ион меди два плюс", C,
           chem(species(formula(atom("Cu")), charge=2)), ["ion", "charge"], "Cu²⁺")
ast_record("chem-sulfate-ion-001", "a", "сульфат ион два минус", C,
           chem(species(formula(atom("S"), atom("O", 4)), charge=-2)), ["ion", "charge"], "SO₄²⁻")
ast_record("chem-coefficient-001", "a", "два аш два о", C,
           chem(species(water, coefficient=2)), ["coefficient"], "2H₂O")
ast_record("chem-precipitate-001", "a", "гидроксид меди два осадок", C,
           chem(species(formula(atom("Cu"), group([atom("O"), atom("H")], 2)), marker="Precipitate")),
           ["state-marker"], "Cu(OH)₂↓")
ast_record("chem-hydrate-001", "a", "медный купорос", C,
           chem(species(formula(atom("Cu"), atom("S"), atom("O", 4), hydrate(5)))),
           ["hydrate", "trivial-name"], "CuSO₄·5H₂O")

ast_record("chem-reaction-cu-001", "a",
           "гидроксид меди два превращается в оксид меди два плюс вода", C,
           reaction([species(formula(atom("Cu"), group([atom("O"), atom("H")], 2)))],
                    [species(formula(atom("Cu"), atom("O"))), species(water)]),
           ["reaction", "balanced"], "Cu(OH)₂ → CuO + H₂O")
# Dictated exactly as spoken, and deliberately not conserving atoms: the
# validator may warn, but the corpus records what the speaker said.
ast_record("chem-reaction-unbalanced-001", "a",
           "уксусная кислота окисляется до аш два о", C,
           reaction([species(formula(atom("C"), atom("H", 3), atom("C"), atom("O"), atom("O"), atom("H")))],
                    [species(water)]),
           ["reaction", "unbalanced"])
ast_record("chem-zinc-ferrite-001", "a", "феррит цинка", C,
           chem(species(formula(atom("Zn"), atom("Fe", 2), atom("O", 4)))), ["formula", "ferrite"])
# A documented ontology gap: barium ferrite has no entry, so the right answer
# cannot be built by the current grammar at all.
ast_record("chem-ferrite-ba-001", "a", "феррит бария", C,
           chem(species(formula(atom("Ba"), atom("Fe", 12), atom("O", 19)))),
           ["formula", "ferrite", "known-gap"])

# --------------------------------------------------------------- mathematics

M = "mathematics"
quadratic = binary("Eq",
                   binary("Sub",
                          binary("Add", power(sym("x"), num("2")), juxt(num("2"), sym("x"))),
                          num("3")),
                   num("0"))
ast_record("math-quadratic-001", "a", "икс в квадрате плюс два икс минус три равно нулю", M,
           math(quadratic), ["equation", "precedence"])
ast_record("math-quadratic-001", "b", "икс в квадрате плюс два икс минус три равняется нулю", M,
           math(quadratic), ["equation", "paraphrase"])

ast_record("math-fraction-word-001", "a", "икс делённое на игрек", M,
           math(binary("Div", sym("x"), sym("y"))), ["fraction", "natural-speech"])
ast_record("math-fraction-boundary-001", "a",
           "начало дроби числитель икс знаменатель игрек конец дроби", M,
           math(fraction(sym("x"), sym("y"))), ["fraction", "boundary-command"])
ast_record("math-root-atom-001", "a", "корень из икс плюс один", M,
           math(binary("Add", root(sym("x")), num("1"))), ["root", "binding"])
ast_record("math-root-group-001", "a", "начало корня икс плюс один конец корня", M,
           math(root(binary("Add", sym("x"), num("1")))), ["root", "boundary-command"])
ast_record("math-power-001", "a", "два в степени десять", M,
           math(power(num("2"), num("10"))), ["power"])
ast_record("math-cube-001", "a", "икс в кубе", M,
           math(power(sym("x"), num("3"))), ["power"])
ast_record("math-factorial-001", "a", "факториал четырёх ка", M,
           math(factorial(juxt(num("4"), sym("k")))), ["factorial", "implicit-product"])
ast_record("math-abs-001", "a", "модуль икс", M, math(absolute(sym("x"))), ["absolute-value"])
ast_record("math-sin-001", "a", "синус от икс", M, math(fn("Sin", sym("x"))), ["function"])
ast_record("math-sin-001", "b", "синус икс", M, math(fn("Sin", sym("x"))), ["function", "paraphrase"])
ast_record("math-ln-001", "a", "натуральный логарифм от икс", M, math(fn("Ln", sym("x"))), ["function"])
ast_record("math-exp-001", "a", "экспонента от икс", M, math(fn("Exp", sym("x"))), ["function"])
ast_record("math-sum-001", "a", "сумма от ка равно нулю до бесконечности", M,
           math(summation(var=sym("k"), start=num("0"), end=INFINITY)), ["sum", "bounds"])
ast_record("math-integral-def-001", "a",
           "интеграл от нуля до единицы икс в квадрате по икс", M,
           math(integral(integrand=power(sym("x"), num("2")), wrt=sym("x"),
                         start=num("0"), end=num("1"))),
           ["integral", "bounds"], "∫₀¹ x² dx")
indefinite = math(integral(integrand=sym("f"), wrt=sym("x")))
ast_record("math-integral-indef-001", "a", "интеграл эф дэ икс", M, indefinite, ["integral", "differential"])
ast_record("math-integral-indef-001", "b", "интеграл эф по икс", M, indefinite, ["integral", "paraphrase"])

first_derivative = math(derivative("Ordinary", sym("f"), [(sym("x"), 1)]))
ast_record("math-deriv-first-001", "a", "производная эф по икс", M, first_derivative,
           ["derivative", "counterfactual"])
ast_record("math-deriv-first-001", "b", "первая производная эф по икс", M, first_derivative,
           ["derivative", "paraphrase"])
ast_record("math-deriv-second-001", "a", "вторая производная игрек по икс", M,
           math(derivative("Ordinary", sym("y"), [(sym("x"), 2)])),
           ["derivative", "higher-order", "counterfactual"], "d²y/dx²")
ast_record("math-deriv-third-001", "a", "производная третьего порядка игрек по тэ", M,
           math(derivative("Ordinary", sym("y"), [(sym("t"), 3)])),
           ["derivative", "higher-order"])
ast_record("math-partial-001", "a", "частная производная тэ большое по икс", M,
           math(derivative("Partial", sym("T", "Upper"), [(sym("x"), 1)])),
           ["derivative", "partial", "counterfactual"])
ast_record("math-mixed-partial-001", "a",
           "частная производная второго порядка тэ большое по икс и по игрек", M,
           math(derivative("Partial", sym("T", "Upper"), [(sym("x"), 1), (sym("y"), 1)])),
           ["derivative", "partial", "mixed"], "∂²T/(∂x∂y)")

ast_record("math-limit-zero-001", "a",
           "предел при икс стремящемся к нулю синуса икс делённого на икс", M,
           math(limit(sym("x"), num("0"), "TwoSided",
                      binary("Div", fn("Sin", sym("x")), sym("x")))),
           ["limit", "counterfactual"])
ast_record("math-limit-inf-001", "a",
           "предел функции эф при тэ стремящемся к бесконечности", M,
           math(limit(sym("t"), INFINITY, "TwoSided", sym("f"))),
           ["limit", "infinity", "counterfactual"])
ast_record("math-limit-left-001", "a",
           "предел слева при икс стремящемся к нулю эф", M,
           math(limit(sym("x"), num("0"), "FromLeft", sym("f"))),
           ["limit", "one-sided", "counterfactual"], "lim_{x→0⁻} f")
ast_record("math-limit-right-001", "a",
           "предел справа при икс стремящемся к двум эф", M,
           math(limit(sym("x"), num("2"), "FromRight", sym("f"))),
           ["limit", "one-sided", "counterfactual"])

ast_record("math-precedence-001", "a", "два плюс три умножить на четыре", M,
           math(binary("Add", num("2"), binary("Mul", num("3"), num("4")))), ["precedence"])
ast_record("math-paren-001", "a",
           "открыть скобку два плюс три закрыть скобку умножить на четыре", M,
           math(binary("Mul", paren(binary("Add", num("2"), num("3"))), num("4"))),
           ["precedence", "boundary-command"])
ast_record("math-subscript-001", "a", "икс индекс один", M,
           math(subscript(sym("x"), num("1"))), ["subscript"])
ast_record("math-greek-001", "a", "пи", M, math(greek("π")), ["greek"])
ast_record("math-inequality-001", "a", "икс больше или равно нулю", M,
           math(binary("Ge", sym("x"), num("0"))), ["relation"])

# ------------------------------------------------------------------- physics

P = "physics"
ast_record("phys-length-001", "a", "три метра", P, math(quantity("3", ("м", 1, False))), ["unit"])
ast_record("phys-length-cm-001", "a", "четыре сантиметра", P,
           math(quantity("4", ("см", 1, False))), ["unit", "prefix"])
ast_record("phys-lambda-001", "a", "лямбда равно шестьсот тридцать два нанометра", P,
           math(binary("Eq", greek("λ"), quantity("632", ("нм", 1, False)))),
           ["unit", "greek", "equation"])
ast_record("phys-g-001", "a", "девять целых восемьдесят одна метра на секунду в квадрате", P,
           math(quantity("9,81", ("м", 1, False), ("с", 2, True))),
           ["unit", "compound", "decimal"], "9,81 м/с²")
ast_record("phys-delta-g-001", "a", "дельта же равно минус эн эф е", P,
           math(binary("Eq", delta(sym("G", "Upper")),
                       neg(juxt(sym("n"), sym("F", "Upper"), sym("E", "Upper"))))),
           ["delta", "equation"])
ast_record("phys-vector-f-001", "a", "вектор эф равен эм умножить на вектор а", P,
           math(binary("Eq", vector(sym("F", "Upper")),
                       binary("Mul", sym("m"), vector(sym("a"))))),
           ["vector", "equation", "unknown-dimension"])
ast_record("phys-newton-001", "a", "пять ньютонов", P, math(quantity("5", ("Н", 1, False))), ["unit", "derived"])
ast_record("phys-joule-001", "a", "десять джоулей", P, math(quantity("10", ("Дж", 1, False))), ["unit", "derived"])
ast_record("phys-watt-001", "a", "сто ватт", P, math(quantity("100", ("Вт", 1, False))), ["unit", "derived"])
ast_record("phys-volt-001", "a", "пять вольт", P, math(quantity("5", ("В", 1, False))), ["unit", "derived"])
ast_record("phys-ampere-001", "a", "три ампера", P, math(quantity("3", ("А", 1, False))), ["unit", "base"])
ast_record("phys-mole-001", "a", "два моля", P, math(quantity("2", ("моль", 1, False))), ["unit", "base"])
# Natural Russian plurals that the unit lexicon does not list yet.
ast_record("phys-kelvin-001", "a", "триста кельвинов", P,
           math(quantity("300", ("К", 1, False))), ["unit", "base", "known-gap"])
ast_record("phys-pascal-001", "a", "сто паскалей", P,
           math(quantity("100", ("Па", 1, False))), ["unit", "derived", "known-gap"])
ast_record("phys-mismatch-length-time-001", "a", "три метра плюс четыре секунды", P,
           math(binary("Add", quantity("3", ("м", 1, False)), quantity("4", ("с", 1, False)))),
           ["unit", "dimension-mismatch"])
ast_record("phys-mismatch-volt-ampere-001", "a", "пять вольт плюс три ампера", P,
           math(binary("Add", quantity("5", ("В", 1, False)), quantity("3", ("А", 1, False)))),
           ["unit", "dimension-mismatch"])
ast_record("phys-compatible-lengths-001", "a", "три метра плюс четыре сантиметра", P,
           math(binary("Add", quantity("3", ("м", 1, False)), quantity("4", ("см", 1, False)))),
           ["unit", "dimension-compatible"])
ast_record("phys-inverse-metre-001", "a", "один метр в минус первой", P,
           math(power(quantity("1", ("м", 1, False)), num("-1"))), ["unit", "power"])
ast_record("phys-sine-of-length-001", "a", "синус трёх метров", P,
           math(fn("Sin", quantity("3", ("м", 1, False)))), ["function", "dimensioned-argument"])
ast_record("phys-dimensioned-exponent-001", "a", "два в степени три метра", P,
           math(power(num("2"), quantity("3", ("м", 1, False)))), ["power", "dimensioned-exponent"])
ast_record("phys-deriv-velocity-001", "a", "производная десять метров по пять секунд", P,
           math(derivative("Ordinary", quantity("10", ("м", 1, False)),
                           [(quantity("5", ("с", 1, False)), 1)])),
           ["derivative", "unit", "provable-dimension"])
ast_record("phys-integral-work-001", "a", "интеграл пять ньютонов по два метра", P,
           math(integral(integrand=quantity("5", ("Н", 1, False)),
                         wrt=quantity("2", ("м", 1, False)))),
           ["integral", "unit", "provable-dimension"])

# ------------------------------------------------------------------ raw / OOD

raw_record("raw-limit-patience-001", "a", "предел терпения", ["raw", "homonym"])
raw_record("raw-limit-patience-001", "b", "предел терпения был исчерпан", ["raw", "homonym", "sentence"])
raw_record("raw-derivative-published-001", "a", "производная была опубликована", ["raw", "homonym"])
raw_record("raw-order-of-magnitude-001", "a", "порядок величины", ["raw", "homonym"])
raw_record("raw-water-boiled-001", "a", "вода закипела в чайнике", ["raw", "substance-mentioned"])
raw_record("raw-ammonia-smell-001", "a", "аммиак имеет резкий запах", ["raw", "substance-mentioned"])
raw_record("raw-acid-storage-001", "a", "серная кислота хранится в лаборатории", ["raw", "substance-mentioned"])
raw_record("raw-integral-course-001", "a", "он изучает интеграл в университете", ["raw", "term-mentioned"])
raw_record("raw-sum-in-words-001", "a", "сумма прописью", ["raw", "term-mentioned"])
raw_record("raw-root-of-problem-001", "a", "корень проблемы лежит глубже", ["raw", "homonym"])
raw_record("raw-degree-of-trust-001", "a", "степень доверия", ["raw", "homonym"])
raw_record("raw-body-function-001", "a", "функция организма", ["raw", "homonym"])
raw_record("raw-memory-module-001", "a", "модуль памяти", ["raw", "homonym"])
raw_record("raw-ferrite-antenna-001", "a", "феррит используется в антеннах", ["raw", "substance-mentioned"])
raw_record("raw-battery-charge-001", "a", "заряд аккумулятора", ["raw", "homonym"])
raw_record("raw-violent-reaction-001", "a", "реакция была бурной", ["raw", "homonym"])
raw_record("raw-oxide-sample-001", "a", "оксид попал в пробу", ["raw", "term-mentioned"])
raw_record("raw-two-words-001", "a", "он сказал два слова", ["raw", "number-mentioned"])
raw_record("raw-incomplete-derivative-by-001", "a", "производная по", ["raw", "incomplete"])
raw_record("raw-incomplete-derivative-expr-001", "a", "производная эф", ["raw", "incomplete"])
raw_record("raw-incomplete-second-derivative-001", "a", "вторая производная", ["raw", "incomplete"])
raw_record("raw-incomplete-limit-var-001", "a", "предел при икс", ["raw", "incomplete"])
raw_record("raw-incomplete-limit-body-001", "a",
           "предел функции при тэ стремящемся к бесконечности", ["raw", "incomplete"])
raw_record("raw-incomplete-integral-001", "a", "интеграл", ["raw", "incomplete"])
raw_record("raw-incomplete-exp-001", "a", "экспонента", ["raw", "incomplete"])
raw_record("raw-incomplete-division-001", "a", "икс деленное на", ["raw", "incomplete"])
raw_record("raw-bare-hydroxide-001", "a", "гидроксид", ["raw", "incomplete"])
raw_record("raw-bare-sine-001", "a", "синус", ["raw", "incomplete"])

# ------------------------------------------------------------------- splits
# Assignment is per family and purely positional, so paraphrases of one
# construction can never land on both sides of a split.

families = []
for record in RECORDS:
    if record["family_id"] not in families:
        families.append(record["family_id"])
families.sort()
split_of = {}
for index, family in enumerate(families):
    bucket = index % 10
    split_of[family] = "train" if bucket < 5 else ("validation" if bucket < 8 else "dev_holdout")
for record in RECORDS:
    record["split"] = split_of[record["family_id"]]

# --------------------------------------------------------------------- write

def canonical_line(record):
    return json.dumps(record, ensure_ascii=False, sort_keys=True, separators=(",", ":"))

RECORDS.sort(key=lambda record: record["id"])
ids = [record["id"] for record in RECORDS]
assert len(ids) == len(set(ids)), "duplicate id in the generated corpus"
text = "\n".join(canonical_line(record) for record in RECORDS) + "\n"
JSONL.write_text(text, encoding="utf-8")

domain_counts = Counter(record["target_domain"] for record in RECORDS)
manifest = {
    "manifest_schema_version": 1,
    "corpus_id": "dev-seed-v1",
    "created": "2026-09-04",
    "file": "dev-seed-v1.jsonl",
    "sha256": hashlib.sha256(text.encode("utf-8")).hexdigest(),
    "dataset_schema_version": SCHEMA_VERSION,
    "records": len(RECORDS),
    "families": len(families),
    "counts_by_domain": dict(sorted(domain_counts.items())),
    "counts_by_split": dict(sorted(Counter(r["split"] for r in RECORDS).items())),
    "counts_by_action": dict(sorted(Counter(r["target_action"] for r in RECORDS).items())),
    "counts_by_provenance": dict(sorted(Counter(r["provenance"] for r in RECORDS).items())),
    "counts_by_tag": dict(sorted(Counter(t for r in RECORDS for t in r["tags"]).items())),
    "families_by_split": {
        split: sorted(f for f in families if split_of[f] == split)
        for split in ("train", "validation", "dev_holdout")
    },
    "gold_provenance": (
        "Every target_ast was written by hand from the intended meaning, following the AST "
        "conventions documented in docs/development/ARCHITECTURE_RU.md and MATHEMATICS_RU.md. "
        "This builder never calls sciwhisper-core, so no gold answer is a copy of the parser's output."
    ),
    "audio": "none",
    "asr": "none: this is a text-level corpus, so ASR accuracy is not measured and no record carries a speaker_id",
    "limitations": [
        "This is a development seed, not a frozen test set and not a real voice corpus.",
        "It is small: rare-event rates carry wide intervals and must be read with their bounds.",
        "Records tagged known-gap encode answers the current grammar cannot build; they are findings, not corpus errors.",
    ],
}
MANIFEST.write_text(json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
                    encoding="utf-8")

print(f"{len(RECORDS)} records, {len(families)} families")
print("domains:", dict(sorted(domain_counts.items())))
print("splits :", dict(sorted(Counter(r['split'] for r in RECORDS).items())))
print("sha256 :", manifest["sha256"])
