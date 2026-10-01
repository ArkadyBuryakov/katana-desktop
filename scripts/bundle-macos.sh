#!/bin/sh
# Build "Katana Desktop.app" (run on macOS). Pass a target such as
# aarch64-apple-darwin or x86_64-apple-darwin, "universal" for both chips
# in one binary, or nothing for the host.
set -e
cd "$(dirname "$0")/.."
TARGET="$1"
if [ "$TARGET" = universal ]; then
  cargo build --release --target aarch64-apple-darwin
  cargo build --release --target x86_64-apple-darwin
  BIN="target/universal/katana-desktop"; mkdir -p target/universal
  lipo -create -output "$BIN" target/aarch64-apple-darwin/release/katana-desktop target/x86_64-apple-darwin/release/katana-desktop
elif [ -n "$TARGET" ]; then
  cargo build --release --target "$TARGET"; BIN="target/$TARGET/release/katana-desktop"
else
  cargo build --release; BIN="target/release/katana-desktop"
fi
APP="target/Katana Desktop.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/katana-desktop"

ICONSET="target/icon.iconset"
rm -rf "$ICONSET" && mkdir -p "$ICONSET"
for s in 16 32 128 256; do
  sips -z $s $s web/icon.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  d=$((s * 2)); sips -z $d $d web/icon.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/icon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Katana Desktop</string>
  <key>CFBundleDisplayName</key><string>Katana Desktop</string>
  <key>CFBundleIdentifier</key><string>pro.buryakov.katana-desktop</string>
  <key>CFBundleExecutable</key><string>katana-desktop</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.puzzle-games</string>
</dict></plist>
PLIST
# ad-hoc signature so Gatekeeper lets a locally built app run
codesign --force --deep -s - "$APP" 2>/dev/null || true
echo "Built $APP"
