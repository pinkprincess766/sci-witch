#!/usr/bin/env bash

set -u
script_dir="$(cd -- "$(dirname -- "$0")" && pwd)"
sciwhisper_bin="$script_dir/sciwhisper"

if [[ ! -x "$sciwhisper_bin" ]]; then
  sciwhisper_bin="$script_dir/../../target/release/sciwhisper"
fi

if [[ ! -x "$sciwhisper_bin" ]]; then
  echo "Не найден исполняемый файл sciwhisper."
  echo "Используйте официальный release-архив или поместите launcher рядом с бинарником."
  read -r -p "Нажмите Enter для выхода." || true
  exit 1
fi

echo "=== SciWhisper: локальная проверка ==="
"$sciwhisper_bin" self-test || {
  read -r -p "Self-test завершился с ошибкой. Нажмите Enter." || true
  exit 1
}

echo
echo "=== Активные настройки ==="
"$sciwhisper_bin" settings show

echo
echo "Для изменения настроек выполните:"
echo "  $sciwhisper_bin settings configure"

read -r -p "Проверка завершена. Нажмите Enter." || true
