#!/bin/zsh

set -u
SCRIPT_DIR="${0:A:h}"
SCIWHISPER_BIN="$SCRIPT_DIR/sciwhisper"

if [[ ! -x "$SCIWHISPER_BIN" ]]; then
  SCIWHISPER_BIN="$SCRIPT_DIR/../../target/release/sciwhisper"
fi

if [[ ! -x "$SCIWHISPER_BIN" ]]; then
  echo "Не найден исполняемый файл sciwhisper."
  echo "Используйте официальный release-архив или поместите launcher рядом с бинарником."
  read "?Нажмите Enter для выхода." || true
  exit 1
fi

echo "=== SciWhisper: локальная проверка ==="
"$SCIWHISPER_BIN" self-test || {
  read "?Self-test завершился с ошибкой. Нажмите Enter." || true
  exit 1
}

echo
echo "=== Диагностика локального Whisper ==="
"$SCIWHISPER_BIN" doctor

echo
answer="n"
read "answer?Проверить микрофон в течение 6 секунд? [y/N] " || true
if [[ "$answer" == [yY] ]]; then
  echo "Произнесите: гидроксид меди два превращается в оксид меди два плюс вода"
  "$SCIWHISPER_BIN" rec --seconds 6 --domain chemistry --renderer all || {
    read "?Голосовой тест завершился с ошибкой. Нажмите Enter." || true
    exit 1
  }
fi

echo
read "?Проверка завершена. Нажмите Enter." || true
