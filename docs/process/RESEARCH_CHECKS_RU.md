# Проверки исследовательского репозитория

С 04.10.2026 проект исследовательский: компилятор текста в научную структуру и
статья. Голосовое приложение и его выпуск (Windows-архив, установщик,
автообновление, чек-лист выпуска) убраны; последнее состояние с ними — тег
`app-0.5-final` (`git show app-0.5-final:docs/process/RELEASE_CHECKLIST.md`).
Здесь остались проверки, которые переживают это решение.

## Что запускать

Пять команд из [`AGENTS.md`](../../AGENTS.md) — во временном `CARGO_TARGET_DIR`
вне проекта. Если менялось ядро, разбор или корпуса — ещё ворота компилятора:

```bash
cargo run -p sciwhisper-eval -- gate \
  --report research/results/deterministic-v6.json \
  --report research/results/ambiguous-auto-v2.json \
  --report research/results/nomenclature-v1.json \
  --report research/results/prose-negatives-v2.json
```

## Ворота компилятора

Контракт — [`COMPILER_CONTRACT_RU.md`](../compiler/COMPILER_CONTRACT_RU.md),
ворота — [`research/schema/compiler-gates-v2.json`](../../research/schema/compiler-gates-v2.json).
`gate` читает этот файл по умолчанию и судит закреплённый набор корпусов
(`dev-seed-v3`, `ambiguous-v2`, `nomenclature-v1`, `prose-negatives-v2`) как
одну выборку. Корпуса закреплены по SHA-256 в самом файле ворот, одиночный
отчёт и корпус не из списка профиль не принимает.

`gate` возвращает ненулевой код, если хотя бы одни ворота не пройдены **или не
измеримы**. «Не измеримо» — не «пропустить»: ворота, которым нужна живая речь,
посчитанные на письменном корпусе, блокируют так же, как провал.

## Ворота голосовых записей

[`research/schema/release-gates-v1.json`](../../research/schema/release-gates-v1.json)
— профиль голосового приложения `release-0.5`. Схема опубликована и остаётся:
на неё ссылаются тесты в `crates/sciwhisper-eval/src/gate.rs` и опубликованные
документы. Приложение снято, профиль никакой выпуск больше не решает. Профиль
всегда называется явно (`--gates research/schema/release-gates-v1.json`): по умолчанию
`gate` читает профиль компилятора, и голосовой отчёт, проверенный без `--gates`,
был бы проверен не теми воротами.

Состояние на 04.10.2026: записей `provenance=real_audio` нет, дикторов нет,
frozen test не запечатан; ворота о живой речи не измеримы. Протокол сбора —
[`research/protocol/voice-v1.md`](../../research/protocol/voice-v1.md),
[`VOICE_CORPUS_RU.md`](../../research/data/VOICE_CORPUS_RU.md),
[`VOICE_COLLECTION_RU.md`](../user/VOICE_COLLECTION_RU.md).

## Frozen test

- корпус записей собран и проходит `sciwhisper-eval validate-dataset`;
- корпус запечатан: `sciwhisper-eval seal --dataset ... --out research/schema/frozen-test-seal.json --release 0.5 --date <ISO>`;
- печать сделана **до** первого прогона ворот на этом корпусе.

`seal` отказывается перезаписать существующую печать. Тест, который можно
запечатать заново после того, как результат стал известен, — это тест, который
подгоняют.

## Лицензии зависимостей

- инвентарь [`packaging/THIRD-PARTY-LICENSES.json`](../../packaging/THIRD-PARTY-LICENSES.json)
  совпадает с `Cargo.lock` (тест `the_inventory_describes_exactly_what_the_lockfile_locks`);
- каждая лицензия из инвентаря входит в разрешённый список (тест `every_dependency_ships_under_a_licence_this_project_accepts`);
- лицензии, требующие уведомления при распространении (MPL-2.0,
  CDLA-Permissive-2.0, BSL-1.0), названы в [`NOTICE`](../../NOTICE) поимённо
  (тест `licences_that_require_a_notice_are_named_in_notice`);
- при смене набора зависимостей инвентарь пересобирают: `python3 packaging/collect-licenses.py`.

## Что проверяет слой распознавания

Тесты `crates/sciwhisper-asr/tests/voice_pack.rs` — на поддельном движке, без
микрофона, модели и сети:

- временное аудио удаляется во всех исходах: успех, движок не запустился,
  ненулевой код, тайм-аут, нечитаемый результат, отсутствующая модель;
- отсутствующая модель отклоняется без сетевого запроса; в слое распознавания
  нет HTTP-клиента (проверяется тестом по исходникам);
- WAV и микрофон работают без `ffmpeg`, не-WAV даёт понятное сообщение
  (`a_stereo_44k_wav_is_converted_without_ffmpeg`,
  `a_non_wav_file_without_ffmpeg_says_so_instead_of_failing_obscurely`).
