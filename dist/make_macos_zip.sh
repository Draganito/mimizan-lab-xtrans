#!/usr/bin/env bash
# Builds the Apple silicon archive.
#
# Result (under target/macos/, not committed):
#   mimizan-lab-xtrans-<version>-aarch64-macos.zip
#     Mimizan Lab X-Trans.app
#       Contents/MacOS/xtrans2dng-gui   what a double click starts
#       Contents/MacOS/xtrans2dng       the command
#       Contents/Resources/AppIcon.icns
#       Contents/Resources/third-party
#     README.md, LICENSE
#
# The two programs link only what comes with macOS. The bundle is not
# signed with a developer's name and not sandboxed.
#
# Usage:  dist/make_macos_zip.sh [--skip-build]

set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$PROJECT_DIR/target}"
CLI="$TARGET_DIR/dist/xtrans2dng"
GUI="$TARGET_DIR/dist/xtrans2dng-gui"
APP_NAME="Mimizan Lab X-Trans"

if [[ "$(uname -m)" != "arm64" ]]; then
    echo "ERROR: this archive is the Apple silicon build (uname -m is $(uname -m))"
    exit 1
fi

BUILD=1
for arg in "$@"; do
    case "$arg" in
        --skip-build) BUILD=0 ;;
        *) echo "ERROR: unknown argument $arg"; exit 2 ;;
    esac
done

if [[ "$BUILD" == 1 ]]; then
    echo "==> cargo build --profile dist --workspace"
    (cd "$PROJECT_DIR" && cargo build --profile dist --workspace)
fi

# Homebrew, libomp and libstdc++ are the ones that would make a program
# depend on a library the machine does not already have.
for bin in "$CLI" "$GUI"; do
    [[ -x "$bin" ]] || { echo "ERROR: $bin missing"; exit 1; }
    echo "==> binary: $bin ($(du -h "$bin" | cut -f1))"
    otool -L "$bin"
    # The first line is the binary's own name.
    while read -r lib _; do
        case "$lib" in
            /usr/lib/*|/System/*) ;;
            *) echo "ERROR: $bin links $lib"; exit 1 ;;
        esac
    done < <(otool -L "$bin" | tail -n +2)
done

VERSION=$("$CLI" --version | awk '{print $2}')
[[ -n "$VERSION" ]] || { echo "ERROR: could not read version"; exit 1; }
echo "==> version $VERSION"
[[ "$("$GUI" --version)" == "xtrans2dng-gui $VERSION" ]] || { echo "ERROR: the window names another version"; exit 1; }

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
NAME="mimizan-lab-xtrans-$VERSION-aarch64-macos"
APP="$STAGE/$NAME/$APP_NAME.app"
MACOS="$APP/Contents/MacOS"
RESOURCES="$APP/Contents/Resources"
mkdir -p "$MACOS" "$RESOURCES"
cp "$CLI" "$GUI" "$MACOS/"
chmod 755 "$MACOS/xtrans2dng" "$MACOS/xtrans2dng-gui"
cp -R "$PROJECT_DIR/dist/third-party" "$RESOURCES/"

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleName</key>
	<string>${APP_NAME}</string>
	<key>CFBundleDisplayName</key>
	<string>${APP_NAME}</string>
	<key>CFBundleIdentifier</key>
	<string>ch.bojovic.mimizanlabxtrans</string>
	<key>CFBundleVersion</key>
	<string>${VERSION}</string>
	<key>CFBundleShortVersionString</key>
	<string>${VERSION}</string>
	<key>CFBundleExecutable</key>
	<string>xtrans2dng-gui</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleIconFile</key>
	<string>AppIcon</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>LSMinimumSystemVersion</key>
	<string>11.0</string>
	<key>LSApplicationCategoryType</key>
	<string>public.app-category.photography</string>
	<key>NSHighResolutionCapable</key>
	<true/>
</dict>
</plist>
EOF
plutil -lint "$APP/Contents/Info.plist" >/dev/null

# iconutil wants one PNG per size, including the @2x names.
ICONS="$PROJECT_DIR/gui/assets"
ICONSET="$STAGE/AppIcon.iconset"
mkdir -p "$ICONSET"
cp "$ICONS/icon-16.png" "$ICONSET/icon_16x16.png"
cp "$ICONS/icon-32.png" "$ICONSET/icon_16x16@2x.png"
cp "$ICONS/icon-32.png" "$ICONSET/icon_32x32.png"
cp "$ICONS/icon-64.png" "$ICONSET/icon_32x32@2x.png"
cp "$ICONS/icon-128.png" "$ICONSET/icon_128x128.png"
cp "$ICONS/icon-256.png" "$ICONSET/icon_128x128@2x.png"
cp "$ICONS/icon-256.png" "$ICONSET/icon_256x256.png"
cp "$ICONS/icon-512.png" "$ICONSET/icon_256x256@2x.png"
cp "$ICONS/icon-512.png" "$ICONSET/icon_512x512.png"
cp "$ICONS/icon-1024.png" "$ICONSET/icon_512x512@2x.png"
iconutil -c icns "$ICONSET" -o "$RESOURCES/AppIcon.icns"
rm -rf "$ICONSET"

test -s "$RESOURCES/AppIcon.icns"
test -s "$RESOURCES/third-party/THIRD-PARTY.txt"
test -s "$RESOURCES/third-party/LGPL-2.1.txt"
test "$(plutil -extract CFBundleExecutable raw "$APP/Contents/Info.plist")" = "xtrans2dng-gui"
# The programs as they lie in the bundle.
test "$("$MACOS/xtrans2dng" --version)" = "xtrans2dng $VERSION"
test "$("$MACOS/xtrans2dng-gui" --version)" = "xtrans2dng-gui $VERSION"

cp "$PROJECT_DIR/README.md" "$PROJECT_DIR/LICENSE" "$STAGE/$NAME/"

OUT="$TARGET_DIR/macos"
mkdir -p "$OUT"
rm -f "$OUT"/mimizan-lab-xtrans-*-aarch64-macos.zip
ZIP="$OUT/$NAME.zip"
ditto -c -k --keepParent "$STAGE/$NAME" "$ZIP"
echo "==> $ZIP ($(du -h "$ZIP" | cut -f1))"
