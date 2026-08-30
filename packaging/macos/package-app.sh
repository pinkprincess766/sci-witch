#!/bin/zsh

set -euo pipefail

ROOT_DIR="${0:A:h:h:h}"
APP_DIR="${1:-$ROOT_DIR/dist/SciWhisper.app}"
BIN_PATH="${SCIWHISPER_BIN:-$ROOT_DIR/target/release/sciwhisper}"
APP_ICON_PATH="$ROOT_DIR/assets/branding/si-witch-app-icon-wink-broom-sand-v1.png"

if [[ ! -x "$BIN_PATH" ]]; then
  echo "Не найден собранный бинарник: $BIN_PATH" >&2
  exit 1
fi

rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"
cp "$ROOT_DIR/packaging/macos/Info.plist" "$APP_DIR/Contents/Info.plist"
cp "$BIN_PATH" "$APP_DIR/Contents/MacOS/sciwhisper"
chmod +x "$APP_DIR/Contents/MacOS/sciwhisper"

sips -z 1024 1024 "$APP_ICON_PATH" \
  --out "$APP_DIR/Contents/Resources/SciWhisper.png" >/dev/null

# Ad-hoc signing keeps the bundle identity stable enough for local privacy permissions.
codesign --force --deep --sign - "$APP_DIR"
echo "$APP_DIR"
