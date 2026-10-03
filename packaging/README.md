# packaging/

Каталог остался ради одного: инвентаря лицензий зависимостей.

| Файл | Зачем |
|---|---|
| `THIRD-PARTY-LICENSES.json` | Каждый сторонний крейт из `Cargo.lock` и заявленная им лицензия. Читает тест `crates/sciwhisper-eval/tests/licenses.rs`: падает, если в замке появился пакет без записи, если запись осталась без пакета или если лицензия не входит в разрешённый список. |
| `collect-licenses.py` | Пересобирает этот JSON из `cargo metadata --locked`: `python3 packaging/collect-licenses.py`. Запускать после смены набора зависимостей. |

Сборка и выпуск голосового приложения (Windows-архив, установщик, `.app`, whisper.cpp
и model pack, обновление) убраны 04.10.2026 вместе с приложением. Последнее состояние
с ними — тег `app-0.5-final`: `git show app-0.5-final:packaging/README.md`.
