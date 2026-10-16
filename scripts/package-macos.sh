#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:?Informe a versão como primeiro argumento.}"
BINARY_PATH="${2:-target/release/localcodepilot-desktop}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPOSITORY_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DIST_ROOT="$REPOSITORY_ROOT/dist"
PACKAGE_NAME="LocalCodePilot-$VERSION-macos-universal"
WORK_ROOT="$DIST_ROOT/.macos-package-$VERSION"
APP_PATH="$WORK_ROOT/LocalCodePilot.app"
CONTENTS_PATH="$APP_PATH/Contents"
MACOS_PATH="$CONTENTS_PATH/MacOS"
RESOURCES_PATH="$CONTENTS_PATH/Resources"
DMG_SOURCE="$WORK_ROOT/dmg"
DMG_PATH="$DIST_ROOT/$PACKAGE_NAME.dmg"
CHECKSUM_PATH="$DIST_ROOT/SHA256SUMS-macos.txt"
SOURCE_ICON="$REPOSITORY_ROOT/assets/branding/icon-512.png"

case "$WORK_ROOT" in
  "$DIST_ROOT"/*) ;;
  *) echo "Pasta temporária fora de dist: $WORK_ROOT" >&2; exit 1 ;;
esac

if [[ "$BINARY_PATH" != /* ]]; then
  BINARY_PATH="$REPOSITORY_ROOT/$BINARY_PATH"
fi
if [[ ! -f "$BINARY_PATH" ]]; then
  echo "Executável não encontrado: $BINARY_PATH" >&2
  exit 1
fi
if [[ ! -f "$SOURCE_ICON" ]]; then
  echo "Ícone não encontrado: $SOURCE_ICON" >&2
  exit 1
fi
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
  echo "Versão inválida: $VERSION" >&2
  exit 1
fi

BASE_VERSION="${VERSION%%-*}"
rm -rf "$WORK_ROOT"
rm -f "$DMG_PATH" "$CHECKSUM_PATH"
mkdir -p "$MACOS_PATH" "$RESOURCES_PATH" "$DMG_SOURCE"

cp "$BINARY_PATH" "$MACOS_PATH/LocalCodePilot"
chmod 755 "$MACOS_PATH/LocalCodePilot"
cp "$REPOSITORY_ROOT/LICENSE" "$RESOURCES_PATH/LICENSE"
cp "$REPOSITORY_ROOT/docs/README-MACOS.txt" "$RESOURCES_PATH/README-MACOS.txt"

ICONSET_PATH="$WORK_ROOT/LocalCodePilot.iconset"
mkdir -p "$ICONSET_PATH"
sips -z 16 16 "$SOURCE_ICON" --out "$ICONSET_PATH/icon_16x16.png" >/dev/null
sips -z 32 32 "$SOURCE_ICON" --out "$ICONSET_PATH/icon_16x16@2x.png" >/dev/null
sips -z 32 32 "$SOURCE_ICON" --out "$ICONSET_PATH/icon_32x32.png" >/dev/null
sips -z 64 64 "$SOURCE_ICON" --out "$ICONSET_PATH/icon_32x32@2x.png" >/dev/null
sips -z 128 128 "$SOURCE_ICON" --out "$ICONSET_PATH/icon_128x128.png" >/dev/null
sips -z 256 256 "$SOURCE_ICON" --out "$ICONSET_PATH/icon_128x128@2x.png" >/dev/null
sips -z 256 256 "$SOURCE_ICON" --out "$ICONSET_PATH/icon_256x256.png" >/dev/null
sips -z 512 512 "$SOURCE_ICON" --out "$ICONSET_PATH/icon_256x256@2x.png" >/dev/null
sips -z 512 512 "$SOURCE_ICON" --out "$ICONSET_PATH/icon_512x512.png" >/dev/null
sips -z 1024 1024 "$SOURCE_ICON" --out "$ICONSET_PATH/icon_512x512@2x.png" >/dev/null
iconutil -c icns "$ICONSET_PATH" -o "$RESOURCES_PATH/LocalCodePilot.icns"

cat > "$CONTENTS_PATH/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDisplayName</key>
  <string>LocalCodePilot</string>
  <key>CFBundleExecutable</key>
  <string>LocalCodePilot</string>
  <key>CFBundleIconFile</key>
  <string>LocalCodePilot</string>
  <key>CFBundleIdentifier</key>
  <string>br.com.localcodepilot.desktop</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>LocalCodePilot</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>$BASE_VERSION</string>
  <key>CFBundleVersion</key>
  <string>$BASE_VERSION</string>
  <key>LSMinimumSystemVersion</key>
  <string>12.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
EOF

plutil -lint "$CONTENTS_PATH/Info.plist"
SIGNING_IDENTITY="${MACOS_SIGNING_IDENTITY:--}"
if [[ "$SIGNING_IDENTITY" == "-" ]]; then
  codesign --force --deep --sign - "$APP_PATH"
else
  codesign --force --deep --options runtime --timestamp --sign "$SIGNING_IDENTITY" "$APP_PATH"
fi
codesign --verify --deep --strict --verbose=2 "$APP_PATH"

cp -R "$APP_PATH" "$DMG_SOURCE/LocalCodePilot.app"
ln -s /Applications "$DMG_SOURCE/Applications"
hdiutil create -volname "LocalCodePilot" -srcfolder "$DMG_SOURCE" -ov -format UDZO "$DMG_PATH" >/dev/null
hdiutil verify "$DMG_PATH" >/dev/null
shasum -a 256 "$DMG_PATH" | sed "s#  .*/#  #" > "$CHECKSUM_PATH"

rm -rf "$WORK_ROOT"
echo "Pacote criado: $DMG_PATH"
cat "$CHECKSUM_PATH"
