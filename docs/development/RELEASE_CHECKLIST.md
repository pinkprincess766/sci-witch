# Release checklist

Всё, что можно проверить программой, проверяется программой. Пункты со ссылкой на
команду — не для галочки: их ставит прогон, а не человек, который посмотрел и решил.

```bash
cargo run -p sciwhisper-eval -- evaluate --dataset research/data/dev-seed-v2.jsonl --output report.json
cargo run -p sciwhisper-eval -- gate --report report.json [--seal research/schema/frozen-test-seal.json]
```

`gate` возвращает ненулевой код, если хотя бы одни ворота не пройдены **или не измеримы**.
Не измеримо — это не «пропустить»: ворота о живой речи, посчитанные на письменном корпусе,
блокируют выпуск ровно так же, как провал. В release workflow они блокируют публикацию
тогда и только тогда, когда тег перестаёт быть предварительным (`v0.5.0`, а не `v0.5.0-rc1`).

## Код

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
- [x] `cargo test --workspace`
- [x] `sciwhisper self-test`
- [x] `sciwhisper demo`
- [x] `cargo test --workspace --locked`
- [x] `cargo clippy --workspace --all-targets --locked -- -D warnings`
- [x] отсутствующая модель отклоняется без сетевого запроса
- [x] временное аудио удаляется во всех исходах: успех, движок не запустился, ненулевой код, тайм-аут, нечитаемый результат, отсутствующая модель
- [x] в слое распознавания нет HTTP-клиента (проверяется тестом по исходникам)

## Ручные проверки

- [ ] **минимум 5 разных Windows-компьютеров** (требование 0.5): протокол заполнен на каждом
- [ ] несколько версий Word: раздел 6 протокола пройден на каждой доступной
- [ ] чистая установка без инструментов разработчика (без Python, без Rust, без Visual Studio)
- [ ] Windows 11 x64: заполнен и приложен протокол [`WINDOWS_ACCEPTANCE_RU.md`](../user/WINDOWS_ACCEPTANCE_RU.md)
      (установка, отрицательные проверки, трей, запись, Unicode, Word на каждой доступной
      версии, настройки, работа без ffmpeg, замер скорости)
- [ ] раздел 6 протокола пройден хотя бы на одной версии Word — иначе `0.2` не принята
- [ ] раздел 9 протокола заполнен: только после него выбор `small-q5_1` перестаёт быть предварительным
- [ ] macOS arm64
- [ ] химия, математика и физика из `../user/TESTING_RU.md`
- [ ] тишина не создаёт выражение
- [ ] `rec --seconds 3` завершается без Enter
- [x] проверено поведение без `ffmpeg`: WAV и микрофон работают без него, не-WAV даёт понятное сообщение (тесты `a_stereo_44k_wav_is_converted_without_ffmpeg`, `a_non_wav_file_without_ffmpeg_says_so_instead_of_failing_obscurely`)

## Автономный Windows-комплект

- [x] `packaging/windows/model-pack.json`: `whisper_cpp.commit` закреплён и сверяется до компиляции
- [x] `packaging/windows/model-pack.json`: SHA-256 и размер модели закреплены и сверяются
- [x] `source_url` модели указывает на неизменяемую ревизию, а не на ветку
- [x] обхода проверок (`ALLOW_UNPINNED_MODEL`) не существует
- [x] `whisper-cli.exe` собирается и запускается в обычном CI на каждый push и PR
- [x] тег whisper.cpp не является плавающим (`latest`/`main`/`master`) — проверяется workflow и тестом
- [x] release workflow собирает `whisper-cli.exe` из закреплённого исходника
- [x] release workflow запускает собранный `whisper-cli.exe --help`, в том числе из пути с пробелами и кириллицей
- [x] release workflow сверяет состав архива с `packaging/windows/BUNDLE_CONTENTS.json`
- [x] официальный комплект отказывается работать с model pack, помеченным как непроверенный
- [ ] размер основного архива и model pack записан в release notes
- [ ] **замер на Windows**: скорость `small-q5_1` на ноутбуке без дискретной GPU и качество на русской научной речи; до него рекомендация модели остаётся предварительной
- [ ] пользователь без Python распаковал оба архива и получил результат (ручная проверка)

## Артефакты

- [x] бинарник и launcher находятся рядом
- [x] включены README, LICENSE, NOTICE, DATA_LICENSE, PRIVACY и KNOWN_LIMITATIONS
- [x] модель не попала в source archive и не хранится в Git
- [x] собранный `whisper-cli.exe` не хранится в Git
- [x] `whisper/BUILD_MANIFEST.json` внутри архива называет tag, commit, флаги сборки и SHA-256 движка
- [x] `whisper/MODEL_MANIFEST.json` внутри model pack называет имя, размер, SHA-256, источник и лицензию
- [ ] архив проверен на чистой пользовательской машине
- [ ] версия в Cargo, changelog и названии релиза совпадает

## Ворота допуска 0.5 (`sciwhisper-eval gate`)

Определены в [`research/schema/release-gates-v1.json`](../../research/schema/release-gates-v1.json).
Каждые судятся по границе доверительного интервала, а не по точечной оценке.

- [ ] `end-to-end-accuracy` — нижняя граница 95% CI ≥ 0.90
- [ ] `shipped-end-to-end-accuracy` — нижняя граница 95% CI пользовательского `MixedText` ≥ 0.90
- [ ] `auto-insert-precision` — нижняя граница 95% CI ≥ 0.97
- [ ] `no-dangerous-rewrites-shipped` — **верхняя** граница 95% CI в пользовательском пути ≤ 0.02
- [ ] `no-dangerous-rewrites-parser` — **верхняя** граница 95% CI в узком парсере ≤ 0.02
- [ ] `no-s4-errors` — ровно 0
- [ ] `structural-validity` — нижняя граница 95% CI ≥ 0.99
- [x] `split-hygiene` — ни семейство, ни диктор не пересекают split

Текущее состояние: **семь из восьми не измеримы**. Корпус `dev-seed-v2` содержит 0 записей
`provenance=real_audio` и 0 дикторов, а frozen test не запечатан. Никакой прогон на этом
корпусе не может подтвердить требования 0.5 — их закрывают записи, а не код. Единственные
измеримые ворота, `split-hygiene`, проходят. Протокол сбора:
[VOICE_CORPUS_RU.md](../../research/data/VOICE_CORPUS_RU.md).

## Frozen test

- [ ] голосовой корпус собран и проходит `validate-dataset`
- [ ] корпус запечатан: `sciwhisper-eval seal --dataset ... --out research/schema/frozen-test-seal.json --release 0.5 --date <ISO>`
- [ ] печать сделана **до** первого прогона ворот на этом корпусе

`seal` отказывается перезаписать существующую печать. Тест, который можно запечатать
заново после того, как результат стал известен, — это тест, который подгоняют.

## Лицензии

- [x] инвентарь [`packaging/THIRD-PARTY-LICENSES.json`](../../packaging/THIRD-PARTY-LICENSES.json)
      совпадает с `Cargo.lock` (тест `the_inventory_describes_exactly_what_the_lockfile_locks`)
- [x] каждая лицензия из инвентаря входит в разрешённый список (тест `every_dependency_ships_under_a_licence_this_project_accepts`)
- [x] лицензии, требующие уведомления при распространении (MPL-2.0, CDLA-Permissive-2.0,
      BSL-1.0), названы в NOTICE поимённо (тест `licences_that_require_a_notice_are_named_in_notice`)
- [x] лицензии whisper.cpp и весов модели названы в NOTICE с версией и происхождением
- [ ] при добавлении зависимости инвентарь перегенерирован: `python3 packaging/collect-licenses.py`

## Подпись и SmartScreen

- [ ] цифровая подпись Windows — **нет**, и до неё выпуск обязан честно об этом говорить
- [x] `README-WINDOWS.txt` объясняет, что предупреждение SmartScreen верное, а не ложное
- [x] `README.md` не обещает подписи
- [x] release workflow публикует `SHA256SUMS.txt` рядом с артефактами
- [x] везде, где упоминается контрольная сумма, сказано, что это **целостность, а не
      подлинность**: файл сумм едет в том же выпуске, что и архивы

## Публикация

- [x] репозиторий и раздел Issues существуют
- [ ] Cargo metadata содержит настоящий repository URL
- [ ] security contact актуален
- [ ] релиз опубликован как предварительная версия
- [ ] installer собран и приложен к выпуску (`SciWhisper-<версия>-Windows-x64-Setup.exe`)
- [ ] installer проверен вручную: разделы 10–14 протокола [`WINDOWS_ACCEPTANCE_RU.md`](../user/WINDOWS_ACCEPTANCE_RU.md)
- [ ] до заполнения этих разделов installer в release notes описывается как непроверенный
