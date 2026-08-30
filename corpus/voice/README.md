# Голосовой корпус SciWhisper

ТЗ §17.2 требует минимум 10 носителей русского, разные микрофоны и шум.
Этого набора **ещё нет**: живые дикторы должны быть записаны отдельно, с согласия.

## Раскладка

```
corpus/voice/
  README.md              ← этот файл
  manifest.yaml          ← метаданные клипов
  synthetic/             ← TTS-семена для отладки конвейера, НЕ замена носителей
  speakers/              ← реальные записи: speaker-id / clip.wav
```

Каждый клип:

- `wav` 16 kHz mono предпочтительно (конвейер сам ресемплит)
- поле `spoken` — ожидаемая нормализованная фраза
- поле `expect_unicode` — ожидаемый рендер
- `speaker`, `mic`, `noise` — для отчёта ASR vs parser

Прогон:

```bash
cargo run -p sciwhisper-cli -- corpus corpus/voice/synthetic --domain chemistry
```

Реальные записи кладите в `speakers/<id>/` и дописывайте `manifest.yaml`.
Аудио в git не обязательно: храните локально или в отдельном pack.
