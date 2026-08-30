# Release checklist

## Код

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
- [x] `cargo test --workspace`
- [x] `sciwhisper self-test`
- [x] `sciwhisper demo`
- [x] отсутствующая модель отклоняется без сетевого запроса
- [ ] временное аудио удаляется после успешной и ошибочной транскрипции

## Ручные проверки

- [ ] Windows 11 x64
- [ ] macOS arm64
- [ ] химия, математика и физика из `TESTING_RU.md`
- [ ] тишина не создаёт выражение
- [ ] `rec --seconds 3` завершается без Enter
- [ ] проверено поведение без `ffmpeg`

## Артефакты

- [x] бинарник и launcher находятся рядом
- [x] включены README, LICENSE, NOTICE, DATA_LICENSE, PRIVACY и KNOWN_LIMITATIONS
- [x] модель не попала в source archive
- [x] опубликованы SHA-256 checksums
- [ ] архив проверен на чистой пользовательской машине
- [ ] версия в Cargo, changelog и названии релиза совпадает

## Публикация

- [ ] репозиторий и issue tracker существуют
- [ ] Cargo metadata содержит настоящий repository URL
- [ ] security contact актуален
- [x] релиз честно помечен Technical Preview
- [ ] Windows installer не обещается до появления проверенного installer
