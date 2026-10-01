#!/usr/bin/env python3
"""Builds research/data/prose-negatives-v1.jsonl and its manifest.

Every corpus this project has is built around what the system should *write*.
This one is built around what it must **leave alone**.

The claim each record makes is the same, and it needs no annotation at all:

    Это обычная научная проза. Никакой записи здесь не продиктовано.
    Любое изменение — дефект.

That is why the file has no `target_ast`, no `expected_render` and no
`expected_mixed_output`: the gold *is* the transcript. A record is satisfied
when `interpret` keeps the words and `interpret_utterance` in MixedText hands
back the sentence byte for byte.

Why it exists: the conjunction trap — «натрий и калий стоят рядом» → `NaIK`,
«медь, о которой я говорил» → `CuO` — lived in the shipped path for the whole
life of the project and not one of the 337 records in the other four corpora
caught it. It was found by hand. That failure class (ordinary speech acquiring
notation nobody dictated) is the worst one in the severity taxonomy, S4, and
it was the least covered.

Provenance, honestly: these sentences are written by hand, by the same author
as the parser, and therefore carry that author's blind spots. A harvest of the
project's own Russian documentation was tried first and rejected — it yielded
21 sentences, all of them prose *about the parser* rather than science, which
is the wrong genre for this question. The real fix for the blind spots is
prose the author did not write; until then this corpus is a floor, not a
ceiling.

`dev-seed-v1`, `dev-seed-v2`, `nomenclature-v1` and `ambiguous-v1` are not
touched by this script.
"""

import hashlib
import json
import pathlib
from collections import Counter

OUT = pathlib.Path(__file__).resolve().parent
JSONL = OUT / "prose-negatives-v1.jsonl"
MANIFEST = OUT / "prose-negatives-v1.manifest.json"
SCHEMA_VERSION = 1
CORPUS_ID = "prose-negatives-v1"
CREATED = "2026-09-15"

RECORDS = []
_counters = Counter()

def prose(section, text, *tags, notes=None):
    """One sentence of ordinary scientific prose that must survive unchanged.

    The family is the *trap*, not the section: `prose-homonym` alone would be
    thirty-one records in one bucket, which both hides which trap failed and
    leaves the positional split with nothing for `dev_holdout`. Keyed by the
    first tag, one family is one thing that can go wrong.
    """
    family = "%s-%s" % (section, tags[0])
    _counters[family] += 1
    entry = {
        "dataset_schema_version": SCHEMA_VERSION,
        "id": "%s-%03d" % (family, _counters[family]),
        "family_id": family,
        "provenance": "handcrafted_text",
        "human_transcript": text,
        "asr_hypotheses": [],
        "target_domain": "plain",
        "target_action": "raw",
        "target_ast": None,
        "split": None,
        "tags": list(tags) + ["prose-negative"],
        "speaker_id": None,
    }
    if notes:
        entry["notes"] = notes
    RECORDS.append(entry)

# ------------------------------------------------------------------------
# 1. Одно­буквенные служебные слова рядом с названием элемента.
#    Здесь жил самый дорогой из найденных дефектов: «и» становилось йодом,
#    «с» — серой, «о» — кислородом, «у» — ураном.
# ------------------------------------------------------------------------
F = "prose-function-word"
prose(F, "Натрий и калий стоят в таблице рядом, поэтому их часто путают.", "conjunction")
prose(F, "Водород и кислород мы обсуждали на прошлой неделе.", "conjunction")
prose(F, "Азот и фосфор относятся к одной группе.", "conjunction")
prose(F, "Углерод и кремний ведут себя по-разному при одинаковых условиях.", "conjunction")
prose(F, "Медь и цинк лежат в соседних ящиках.", "conjunction")
prose(F, "Железо и никель притягиваются к магниту.", "conjunction")
prose(F, "Медь, о которой я говорил, лежит в дальнем шкафу.", "preposition-o")
prose(F, "Серебро, о котором шла речь, оказалось загрязнённым.", "preposition-o")
prose(F, "Мы поговорили о кальции и разошлись.", "preposition-o")
prose(F, "Водород у нас закончился ещё вчера.", "preposition-u")
prose(F, "Кислород у них хранится в отдельном помещении.", "preposition-u")
prose(F, "Хлор у поставщика бывает не всегда.", "preposition-u")
prose(F, "Натрий с хлором реагируют очень бурно, будьте осторожны.", "preposition-s")
prose(F, "Сера с водородом дают неприятный запах.", "preposition-s")
prose(F, "Магний с кислотой лучше не оставлять без присмотра.", "preposition-s")
prose(F, "Калий в лаборатории держат под слоем масла.", "preposition-v")
prose(F, "Углерод в этой пробе явно лишний.", "preposition-v")
prose(F, "Алюминий к утру покрылся плёнкой.", "preposition-k")
prose(F, "Цинк к тому времени уже растворился.", "preposition-k")
prose(F, "Фосфор а потом сера — вот такой был порядок.", "conjunction-a")
prose(F, "Бор я бы вообще не трогал руками.", "pronoun-ya")

# ------------------------------------------------------------------------
# 2. Омонимы: слова, которые в науке означают конструкцию, а в речи — нет.
# ------------------------------------------------------------------------
H = "prose-homonym"
prose(H, "Предел терпения у лаборанта был достигнут к четвергу.", "limit")
prose(H, "Всему есть предел, и этому эксперименту тоже.", "limit")
prose(H, "Мы подошли к пределу того, что можно сделать за смену.", "limit")
prose(H, "Производная статья расходов оказалась больше основной.", "derivative")
prose(H, "Производная от этой идеи работа была опубликована позже.", "derivative")
prose(H, "Порядок в лаборатории поддерживает дежурный.", "order")
prose(H, "Порядок величины здесь важнее точного значения.", "order")
prose(H, "Нарушение порядка работы с реактивами наказывается строго.", "order")
prose(H, "Степень доверия к этому источнику невысока.", "degree")
prose(H, "Он защитил степень в прошлом году.", "degree")
prose(H, "Степень износа оборудования никто не оценивал.", "degree")
prose(H, "Корень проблемы был совсем не в приборе.", "root")
prose(H, "Корень растения мы промыли и высушили.", "root")
prose(H, "Реакция коллег на эту новость была сдержанной.", "reaction")
prose(H, "Реакция зала оказалась неожиданно тёплой.", "reaction")
prose(H, "Его реакция была мгновенной, он успел отдёрнуть руку.", "reaction")
prose(H, "Интегральная оценка курса складывается из трёх частей.", "integral")
prose(H, "Интегральный подход к задаче требует времени.", "integral")
prose(H, "Сумма затрат превысила запланированную.", "sum")
prose(H, "В сумме получилось больше, чем мы рассчитывали.", "sum")
prose(H, "Разложение материала на складе шло медленно.", "decomposition")
prose(H, "Группа собралась в одиннадцать и работала до вечера.", "group")
prose(H, "Кольцо на пальце пришлось снять перед работой.", "ring")
prose(H, "Поле за институтом давно никто не косил.", "field")
prose(H, "Ядро коллектива не менялось много лет.", "nucleus")
prose(H, "Масса народу пришла на семинар.", "mass")
prose(H, "Сила его аргументов была не в цифрах.", "force")
prose(H, "Заряд бодрости после отпуска быстро кончился.", "charge")
prose(H, "Раствор проблемы нашёлся сам собой.", "solution",
      notes="«раствор» здесь в разговорном смысле; словарь его знать не обязан.")
prose(H, "Работа над ошибками заняла больше времени, чем сами опыты.", "work")
prose(H, "Мощность лаборатории по числу проб ограничена.", "power")

# ------------------------------------------------------------------------
# 3. Слова, которыми в этой программе открывается реакция, — в обычной речи.
# ------------------------------------------------------------------------
R = "prose-reaction-frame"
prose(R, "Он хорошо взаимодействует с коллегами и студентами.", "interacts")
prose(R, "Наш отдел взаимодействует с двумя другими институтами.", "interacts")
prose(R, "Эта методика плохо взаимодействует с существующим оборудованием.", "interacts")
prose(R, "Разговор между коллегами закончился ничем.", "between")
prose(R, "Между первым и вторым замером прошло около часа.", "between")
prose(R, "Между нами говоря, результат меня не убедил.", "between")
prose(R, "Команда разлагается на две независимые подгруппы.", "decompose")
prose(R, "Задача разлагается на несколько более простых.", "decompose")
prose(R, "В результате получается неплохо, если не торопиться.", "result")
prose(R, "В результате получается, что мы потеряли целый день.", "result")
prose(R, "Из этого получается вполне приличная статья.", "from-marker")
prose(R, "С образованием у него всё в порядке, он закончил университет.", "product-marker",
      notes="«с образованием» — то же слово, которым открывается сторона продуктов.")
prose(R, "Курс читают с образованием небольших групп по интересам.", "product-marker")
prose(R, "Если нагреть обстановку, ничего хорошего не выйдет.", "condition")
prose(R, "При нагревании спора обычно затихает сама.", "condition")
prose(R, "Смесь мнений в комитете превращается в бесконечное обсуждение.", "forward")
prose(R, "Его энтузиазм переходит в упрямство слишком быстро.", "forward")
prose(R, "Осадок от этого разговора остался у всех.", "precipitate")
prose(R, "Газ в машине закончился на полпути к институту.", "gas-marker")
prose(R, "Ион я впервые услышал это слово в школе.", "ion-marker",
      notes="Маркер иона в начале фразы, за которым нет вещества.")

# ------------------------------------------------------------------------
# 4. Маркеры самокоррекции, использованные как обычные слова.
# ------------------------------------------------------------------------
C = "prose-correction-marker"
prose(C, "Нет, так мы до вечера не закончим.", "no")
prose(C, "Нет никакой уверенности, что прибор исправен.", "no")
prose(C, "Точнее сказать трудно, данных мало.", "more-precisely")
prose(C, "Вернее всего будет повторить измерение завтра.", "rather")
prose(C, "Не всё так просто, а разбираться придётся долго.", "not-but")
prose(C, "Не в приборе дело, а в самой методике.", "not-but")
prose(C, "Не сегодня, а в следующий понедельник.", "not-but")
prose(C, "Поправка вышла в следующем номере журнала.", "amendment")
prose(C, "То есть мы возвращаемся к тому, с чего начали.", "that-is")
prose(C, "Я имел в виду совсем другое исследование.", "i-meant")

# ------------------------------------------------------------------------
# 5. Числа и единицы в обычной речи.
# ------------------------------------------------------------------------
N = "prose-number"
prose(N, "Девяносто девять процентов работы делает подготовка.", "percent")
prose(N, "Три раза переделывали, и всё равно вышло криво.", "times")
prose(N, "Во втором случае результат оказался устойчивее.", "ordinal")
prose(N, "Он пришёл в два и ушёл в четыре.", "clock")
prose(N, "Двадцать лет назад это считалось невозможным.", "years")
prose(N, "Мы прошли метров сто вперёд и вернулись.", "unit-loose")
prose(N, "Секунду подождите, я найду нужную запись.", "unit-loose")
prose(N, "Он на голову выше всех остальных в группе.", "idiom")
prose(N, "Тысяча мелочей отвлекает от главного.", "scale-word",
      notes="«тысяча» — множитель, который лексикон чисел применить не умеет.")
prose(N, "Миллион раз уже это обсуждали.", "scale-word")
prose(N, "Половина группы не явилась на занятие.", "fraction-word")

# ------------------------------------------------------------------------
# 6. Названия веществ, упомянутые мимоходом.
#    Самый спорный раздел: см. `substitution_policy` в манифесте.
# ------------------------------------------------------------------------
S = "prose-substance-mention"
prose(S, "Вода в кране опять холодная.", "water")
prose(S, "Воду для опыта надо брать дистиллированную.", "water")
prose(S, "Аммиаком пахло по всему коридору.", "ammonia")
prose(S, "Кислота пролилась на стол, но никто не пострадал.", "acid")
prose(S, "Соль закончилась, принесите из соседней лаборатории.", "salt")
prose(S, "Пробу оксида я взял ещё в понедельник.", "oxide")
prose(S, "Гидроксид хранится на верхней полке слева.", "hydroxide")
prose(S, "Метан в этой смеси нас не интересует.", "methane")
prose(S, "Углекислый газ в помещении надо контролировать.", "co2")
prose(S, "Спирт выдают под запись и только по заявке.", "ethanol")
prose(S, "Ферритовая антенна лежала в коробке с проводами.", "ferrite")
prose(S, "Медный купорос мы заказали, но он ещё не пришёл.", "vitriol")

# ------------------------------------------------------------------------
# 6b. Греческие буквы, которые в русском — обычные слова.
#     Найдено этим корпусом: «Дельта между планом и фактом» давало δ.
#     Остальные держатся только защитой «одно обычное слово без другой науки
#     вокруг», то есть случайно, а не по правилу, — поэтому записаны все.
# ------------------------------------------------------------------------
G = "prose-greek-word"
prose(G, "Дельта между планом и фактом оказалась заметной.", "delta")
prose(G, "Дельта реки за лето сильно обмелела.", "delta")
prose(G, "Дельта была небольшой, но её заметили.", "delta")
prose(G, "Эта работа ещё не закончена.", "eta")
prose(G, "Эта проба оказалась загрязнённой.", "eta")
prose(G, "Пи в этом контексте ничего не значит.", "pi")
prose(G, "Альфа и омега всей методики — чистота посуды.", "alpha-omega")
prose(G, "Бета версия программы вышла в марте.", "beta")
prose(G, "Гамма цветов на графике выбрана крайне неудачно.", "gamma")
prose(G, "Ро выше, чем мы ожидали от этого сплава.", "rho")
prose(G, "Сигма в отчёте посчитана по неправильной формуле.", "sigma")
prose(G, "Тау в этой модели вообще не определена.", "tau")
prose(G, "Каппа коэффициент взят из старой статьи.", "kappa")
prose(G, "Мю в знаменателе я бы перепроверил.", "mu")
prose(G, "Ню это его любимая приставка в разговоре.", "nu")
prose(G, "Омега три полезны для сосудов, так пишут.", "omega")

# ------------------------------------------------------------------------
# 7. Длинная связная проза: абзац, а не отдельная фраза.
#    Здесь проверяется, что поиск участков не срабатывает где-то в середине.
# ------------------------------------------------------------------------
L = "prose-paragraph"
prose(L, "Сначала мы обсудили порядок работы, потом распределили обязанности, "
         "и только после этого приступили к подготовке образцов.", "long")
prose(L, "Предел, до которого можно довести чистоту в наших условиях, "
         "определяется не методикой, а состоянием вытяжки.", "long")
prose(L, "Реакция коллег была предсказуемой: сначала недоверие, потом интерес, "
         "и в результате получается, что идею приняли.", "long")
prose(L, "Натрий и калий мы держим отдельно, медь и цинк можно рядом, "
         "а кислоты вообще в другом шкафу.", "long")
prose(L, "Степень готовности отчёта я бы оценил как среднюю, "
         "потому что раздел про производные ещё не написан.", "long")
prose(L, "Если нагреть обсуждение до такой температуры, никакого решения "
         "не выйдет, и в результате получается только испорченный вечер.", "long")
prose(L, "Он взаимодействует с людьми примерно так же, как с приборами: "
         "аккуратно, медленно и с недоверием к первому результату.", "long")
prose(L, "Вода в системе жёсткая, осадок на стенках появляется за неделю, "
         "и никакая фильтрация тут уже не помогает.", "long")

# ------------------------------------------------------------------------
# 8. Сложные случаи: научная лексика плотно, но ничего не продиктовано.
# ------------------------------------------------------------------------
D = "prose-dense"
prose(D, "Порядок производной в этой статье нигде явно не указан.", "dense")
prose(D, "Степень окисления там не обсуждается вообще.", "dense")
prose(D, "Предел последовательности он объяснял два занятия подряд.", "dense")
prose(D, "Коэффициенты в уравнении подбирали вручную и долго.", "dense")
prose(D, "Индексы у него всегда выходят мелкими и нечитаемыми.", "dense")
prose(D, "Заряд иона в этой таблице почему-то не проставлен.", "dense")
prose(D, "Стрелка на доске была нарисована не в ту сторону.", "dense")
prose(D, "Интеграл он берёт быстрее всех на потоке.", "dense")
prose(D, "Синус и косинус путают чаще, чем кажется.", "dense",
      notes="«и» между двумя названиями функций.")
prose(D, "Логарифм в основании десять здесь удобнее натурального.", "dense")
prose(D, "Дробь на доске стёрли раньше, чем я успел переписать.", "dense")
prose(D, "Скобки он расставляет по настроению.", "dense")
prose(D, "Числитель и знаменатель у него всё время меняются местами.", "dense")
prose(D, "Вектор приложения усилий выбран неудачно.", "dense")
prose(D, "Матрица ответственности в проекте так и не появилась.", "dense")
prose(D, "Функция этого отдела мне до сих пор непонятна.", "dense")
prose(D, "Аргумент у него всегда один и тот же.", "dense")

# --------------------------------------------------------------- splits

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

# ---------------------------------------------------------------- write

def canonical_line(record):
    return json.dumps(record, ensure_ascii=False, sort_keys=True, separators=(",", ":"))

RECORDS.sort(key=lambda record: record["id"])
ids = [record["id"] for record in RECORDS]
assert len(ids) == len(set(ids)), "duplicate id in the generated corpus"
texts = [record["human_transcript"] for record in RECORDS]
assert len(texts) == len(set(texts)), "duplicate sentence in the generated corpus"
for record in RECORDS:
    assert record["target_action"] == "raw", record["id"]
    assert record["target_ast"] is None, record["id"]
    assert "expected_mixed_output" not in record, record["id"]

text = "\n".join(canonical_line(record) for record in RECORDS) + "\n"
JSONL.write_text(text, encoding="utf-8")

manifest = {
    "manifest_schema_version": 1,
    "corpus_id": CORPUS_ID,
    "created": CREATED,
    "file": JSONL.name,
    "sha256": hashlib.sha256(text.encode("utf-8")).hexdigest(),
    "dataset_schema_version": SCHEMA_VERSION,
    "records": len(RECORDS),
    "families": len(families),
    "counts_by_domain": dict(sorted(Counter(r["target_domain"] for r in RECORDS).items())),
    "counts_by_split": dict(sorted(Counter(r["split"] for r in RECORDS).items())),
    "counts_by_action": dict(sorted(Counter(r["target_action"] for r in RECORDS).items())),
    "counts_by_provenance": dict(sorted(Counter(r["provenance"] for r in RECORDS).items())),
    "counts_by_tag": dict(sorted(Counter(t for r in RECORDS for t in r["tags"]).items())),
    "families_by_split": {
        split: sorted(f for f in families if split_of[f] == split)
        for split in ("train", "validation", "dev_holdout")
    },
    "claim": (
        "Ни одно предложение в этом файле не содержит продиктованной научной записи. "
        "Правильный ответ для каждого — само предложение, слово в слово. Любое изменение "
        "является дефектом класса S4 по research/schema/severity-v1.json."
    ),
    "why": (
        "The conjunction trap («натрий и калий стоят рядом» -> NaIK, «медь, о которой я "
        "говорил» -> CuO) survived the entire life of the project and none of the 337 records "
        "in the other four corpora caught it; it was found by hand. S4 is the worst class in "
        "the taxonomy and it was the least covered: dev-seed-v2 carried 29 negatives and "
        "ambiguous-v1 carried 19."
    ),
    "gold_provenance": (
        "There is no annotation in this corpus and nothing to get wrong: the gold is the "
        "transcript itself. This builder never calls sciwhisper-core, and no record was "
        "written or removed after seeing what the system does with it."
    ),
    "substitution_policy": (
        "Decided: a substance name inside ordinary prose is not substituted. It is mentioned, "
        "not dictated, so the sentence comes back as it was said. Running this corpus is what "
        "showed the previous behaviour was not a decision at all but a side effect of word "
        "count — in the same sentence frame «Вода в помещении надо контролировать» was left "
        "alone while «Углекислый газ в помещении надо контролировать» became «CO2 в "
        "помещении…», because the strength rule counted words. "
        "Dictation is unaffected: a name said on its own is still a formula, and so is a "
        "dictated list, which is separated from prose by coverage — the names have to be a "
        "majority of their sentence. See is_bare_substance and MIN_ENUMERATION_SPANS in "
        "crates/sciwhisper-core/src/utterance.rs. "
        "The four records that declared the old behaviour live on in ambiguous-v2 and "
        "dev-seed-v3, derived by research/data/apply_prose_policy.py; ambiguous-v1 and "
        "dev-seed-v2 keep their published SHA-256 and their old expectations."
    ),
    "audio": "none",
    "asr": "none: this is a text-level corpus and no record carries a speaker_id.",
    "limitations": [
        "Handwritten by the same author as the parser, so it carries that author's blind "
        "spots. It is a floor on S4 coverage, not a ceiling.",
        "A harvest of the project's own Russian documentation was tried first and rejected: "
        "it yielded 21 sentences, all of them prose about the parser rather than about "
        "science, which is the wrong genre for this question.",
        "Prose dictated by a real person through a real recognizer will differ from this in "
        "ways nobody can predict from a keyboard.",
        "Passing this corpus proves that these traps are closed, not that the class is.",
    ],
}
MANIFEST.write_text(json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
                    encoding="utf-8")

print("%d records, %d families" % (len(RECORDS), len(families)))
print("splits :", dict(sorted(Counter(r["split"] for r in RECORDS).items())))
print("sha256 :", manifest["sha256"])
