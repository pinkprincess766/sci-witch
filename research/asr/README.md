# research/asr: гипотезы Whisper с логарифмами токенов

Исследовательский инструмент для вопросов 1–2 плана. Python здесь не часть продукта. Скрипт `whisper_hypotheses.py` прогоняет одно окно звука (до 30 с) через `whisper.decode` из установленного пакета `openai-whisper` и пишет один JSON-файл с гипотезой, логарифмами её токенов и манифестом прогона. Что означают `avg_logprob`, `no_speech_prob`, откат температуры и seed в самом Whisper — в [`docs/research/ASR_SCORES_RU.md`](../../docs/research/ASR_SCORES_RU.md).

Запускать интерпретатором пакета Homebrew (в нём стоят `whisper` и `torch`), из любого каталога. Модель берётся из `~/.cache/whisper` (или `$XDG_CACHE_HOME/whisper`); скрипт ничего не скачивает.

## Команды

Жадный режим (температура 0, без отката, без поиска лучей, без меток времени):

```bash
PY=/opt/homebrew/opt/openai-whisper/libexec/bin/python
$PY research/asr/whisper_hypotheses.py greedy research/voice-samples/synthetic/quadratic.caf \
  --model base --out /tmp/greedy.json
```

Режим выборки (`best_of=N` при температуре больше нуля, seed обязателен):

```bash
$PY research/asr/whisper_hypotheses.py sample research/voice-samples/synthetic/quadratic.caf \
  --model base --temperature 0.6 --best-of 5 --seed 1 --out /tmp/sample.json
```

Общие флаги: `--model` (по умолчанию `base`), `--language` (по умолчанию `ru`), `--out`. Фикстура `crates/sciwhisper-asr/tests/fixtures/whisper-hypotheses-greedy-quadratic.json` получена первой командой (с `--model base`). Проверки пределов скрипта: `$PY research/asr/test_whisper_hypotheses.py`.

## Что в JSON

Верхний уровень: `schema_version` (1), `manifest`, `language` (язык, с которым шло декодирование), `hypotheses` (список; сейчас всегда один элемент).

`manifest`:

- `script.path`, `script.sha256` — скрипт относительно корня репозитория и его SHA-256;
- `python_version`, `torch_version`, `whisper_version`;
- `model.name`, `model.file`, `model.sha256` — имя модели, имя файла и SHA-256 файла весов;
- `mode` (`greedy` / `sample`), `temperature`, `best_of`, `beam_size` (всегда `null`), `seed` (`null` в жадном режиме);
- `device` (`cpu`), `fp16` (`false`), `language_requested`;
- `audio.path`, `audio.sha256` — файл звука (вне репозитория остаётся только имя файла) и его SHA-256;
- `sample_rate_hz` — частота, к которой ffmpeg приводит звук для Whisper (16000), не исходная; `audio_duration_s` — длительность после этого приведения.

Гипотеза:

- `text` — текст от Whisper;
- `tokens` — только текстовые токены, у каждого `id`, `text` (результат `tokenizer.decode([id])`; у токена, разрезающего символ UTF-8, это может быть символ-заменитель), `logprob`;
- `eot_logprob` — логарифм вероятности токена конца текста;
- `sum_logprob` — сумма по текстовым токенам и EOT;
- `mean_logprob`, `min_logprob` — по текстовым токенам, без EOT; `null`, если токенов нет;
- `window_avg_logprob` — `avg_logprob` самого Whisper (сумма, делённая на число токенов плюс один);
- `no_speech_prob` — вероятность `<|nospeech|>` в позиции SOT.

`logprob` посчитан нами: токены гипотезы подаются в модель принудительно (плюс EOT) с теми же фильтрами логитов, что у Whisper, температура 1. Это тот способ, который `ASR_SCORES_RU.md` сверил с `avg_logprob`. На фикстуре `sum_logprob` и `window_avg_logprob × (число токенов + 1)` расходятся на 5.6·10⁻⁷ относительной разности.

Читает файл `sciwhisper_asr::hypotheses::parse_hypotheses`. Он заново считает сумму, среднее и минимум по токенам и отказывается принимать файл, где они не сходятся.

## Чего скрипт не делает

- Не принимает звук длиннее одного окна (`MAX_AUDIO_S` = 30 с): отказывает с ошибкой, не режет и не склеивает.
- Отдаёт одну гипотезу. В режиме выборки `whisper.decode` сам выбирает лучшую из `best_of` выборок по `sum/len`; остальные кандидаты не возвращаются.
- Без меток времени: `sum_logprob` не включает токены-метки, в отличие от `avg_logprob` из командной строки `whisper`.
- Не делает откат температуры, поиск лучей, `condition_on_previous_text`, подсказок (`initial_prompt`).
- Воспроизводимость по seed проверена только на этой машине (CPU, torch 2.12.1, `base`, `quadratic.caf`): между машинами, версиями PyTorch, GPU и MPS не проверялась. `fp16`, модели кроме `base` (в том числе `large-v3-turbo`), языки кроме `ru` не проверялись.
- Если выборка упрётся в предел длины без EOT, `window_avg_logprob` не сойдётся с `sum_logprob` (см. «крайний случай» в `ASR_SCORES_RU.md`); скрипт этого не ловит.
- Не скачивает модели и не пишет в репозиторий ничего, кроме файла из `--out`.
