#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
OUTPUT_ROOT="$REPO_ROOT/dist"
APP_NAME="Petri"
SKIP_BUILD=0
PACKAGE_VERSION="$(awk '
  /^\[package\]$/ { in_package = 1; next }
  /^\[/ { if (in_package) exit }
  in_package && /^version = "/ {
    value = $0
    sub(/^version = "/, "", value)
    sub(/".*$/, "", value)
    print value
    exit
  }
' "$REPO_ROOT/Cargo.toml")"
BUNDLE_VERSION="${PACKAGE_VERSION%%[-+]*}"

if [[ -z "$PACKAGE_VERSION" || ! "$BUNDLE_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Cargo.toml does not contain a macOS-compatible package version." >&2
  exit 1
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-build)
      SKIP_BUILD=1
      shift
      ;;
    --output)
      OUTPUT_ROOT="$2"
      shift 2
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "package-petri-macos.sh must run on macOS." >&2
  exit 1
fi

if [[ "$SKIP_BUILD" -eq 0 ]]; then
  node "$SCRIPT_DIR/build-sdk-runtime.mjs"
  export PETRI_REQUIRE_SDK_RUNTIME=1
  export CARGO_PROFILE_RELEASE_LTO=false
  RUSTFLAG_SEPARATOR=$'\x1f'
  for remap_flag in "--remap-path-prefix=$REPO_ROOT=." "--remap-path-prefix=$HOME=<home>"; do
    if [[ -n "${CARGO_ENCODED_RUSTFLAGS:-}" ]]; then
      CARGO_ENCODED_RUSTFLAGS+="$RUSTFLAG_SEPARATOR$remap_flag"
    else
      CARGO_ENCODED_RUSTFLAGS="$remap_flag"
    fi
  done
  export CARGO_ENCODED_RUSTFLAGS
  cargo build --release --locked --bin petri
fi

BINARY="$REPO_ROOT/target/release/petri"
node "$SCRIPT_DIR/build-sdk-runtime.mjs" --verify
ICON="$REPO_ROOT/assets/Petri.icns"
LICENSE_PATH="$REPO_ROOT/LICENSE"
THIRD_PARTY_NOTICES_PATH="$REPO_ROOT/THIRD_PARTY_NOTICES.md"
THIRD_PARTY_LICENSES_PATH="$REPO_ROOT/THIRD_PARTY_LICENSES.md"
if [[ ! -x "$BINARY" ]]; then
  echo "Missing executable Petri release binary: $BINARY" >&2
  exit 1
fi
if [[ ! -f "$ICON" ]]; then
  echo "Missing Petri application icon: $ICON" >&2
  exit 1
fi
if [[ ! -f "$LICENSE_PATH" ]]; then
  echo "Missing Apache-2.0 license: $LICENSE_PATH" >&2
  exit 1
fi
if [[ ! -f "$THIRD_PARTY_NOTICES_PATH" ]]; then
  echo "Missing third-party notices: $THIRD_PARTY_NOTICES_PATH" >&2
  exit 1
fi
if [[ ! -f "$THIRD_PARTY_LICENSES_PATH" ]]; then
  echo "Missing third-party license corpus: $THIRD_PARTY_LICENSES_PATH" >&2
  exit 1
fi

ARCH="$(uname -m)"
PACKAGE_STEM="${APP_NAME// /-}-macos-$ARCH"
PACKAGE_ROOT="$OUTPUT_ROOT/$PACKAGE_STEM"
APP_ROOT="$PACKAGE_ROOT/$APP_NAME.app"
MACOS_DIR="$APP_ROOT/Contents/MacOS"
RESOURCES_DIR="$APP_ROOT/Contents/Resources"

rm -rf "$PACKAGE_ROOT"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"
cp "$BINARY" "$RESOURCES_DIR/petri"
cp "$ICON" "$RESOURCES_DIR/Petri.icns"
cp "$LICENSE_PATH" "$RESOURCES_DIR/LICENSE"
cp "$THIRD_PARTY_NOTICES_PATH" "$RESOURCES_DIR/THIRD_PARTY_NOTICES.md"
cp "$THIRD_PARTY_LICENSES_PATH" "$RESOURCES_DIR/THIRD_PARTY_LICENSES.md"
cp "$SCRIPT_DIR/install-petri-macos.sh" "$RESOURCES_DIR/install-petri-macos.sh"
chmod 755 "$RESOURCES_DIR/petri"
chmod 755 "$RESOURCES_DIR/install-petri-macos.sh"

cat > "$RESOURCES_DIR/petri.command" <<'COMMAND'
#!/usr/bin/env bash
set -euo pipefail
RESOURCES_DIR="$(cd "$(dirname "$0")" && pwd)"
exec "$RESOURCES_DIR/petri" tui
COMMAND
chmod 755 "$RESOURCES_DIR/petri.command"

cat > "$MACOS_DIR/$APP_NAME" <<'LAUNCHER'
#!/usr/bin/env bash
set -euo pipefail
CONTENTS_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PETRI_COMMAND="$CONTENTS_DIR/Resources/petri.command"
/usr/bin/open -a Terminal "$PETRI_COMMAND"
LAUNCHER
chmod 755 "$MACOS_DIR/$APP_NAME"

cat > "$APP_ROOT/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleDisplayName</key><string>$APP_NAME</string>
  <key>CFBundleExecutable</key><string>$APP_NAME</string>
  <key>CFBundleIdentifier</key><string>farm.amoeba.petri</string>
  <key>CFBundleIconFile</key><string>Petri.icns</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>$APP_NAME</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$BUNDLE_VERSION</string>
  <key>CFBundleVersion</key><string>$BUNDLE_VERSION</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
</dict>
</plist>
PLIST

: "${PETRI_MACOS_SIGNING_IDENTITY:?Set PETRI_MACOS_SIGNING_IDENTITY for release packaging}"
: "${PETRI_MACOS_NOTARY_PROFILE:?Set PETRI_MACOS_NOTARY_PROFILE for release packaging}"
/usr/bin/codesign --force --deep --options runtime --timestamp \
  --sign "$PETRI_MACOS_SIGNING_IDENTITY" "$APP_ROOT"
/usr/bin/codesign --verify --deep --strict --verbose=2 "$APP_ROOT"

cat > "$PACKAGE_ROOT/README.txt" <<README
Petri for macOS
===============

Double-click $APP_NAME.app to open Petri in Terminal, or install it for this
user by running:

  ./$APP_NAME.app/Contents/Resources/install-petri-macos.sh

The installer puts the app in ~/Applications and creates the petri
command in ~/.local/bin. The installer and all project and third-party license
materials are covered by the signed $APP_NAME.app bundle.
README

ZIP_PATH="$OUTPUT_ROOT/$PACKAGE_STEM.zip"
NOTARY_ZIP_PATH="$OUTPUT_ROOT/$PACKAGE_STEM-notary.zip"
cleanup_notary_zip() {
  rm -f "$NOTARY_ZIP_PATH"
}
trap cleanup_notary_zip EXIT
rm -f "$ZIP_PATH" "$NOTARY_ZIP_PATH"
(
  cd "$PACKAGE_ROOT"
  /usr/bin/ditto -c -k --keepParent "$APP_NAME.app" "$NOTARY_ZIP_PATH"
)

/usr/bin/xcrun notarytool submit "$NOTARY_ZIP_PATH" \
  --keychain-profile "$PETRI_MACOS_NOTARY_PROFILE" --wait
/usr/bin/xcrun stapler staple "$APP_ROOT"
/usr/bin/xcrun stapler validate "$APP_ROOT"
/usr/bin/codesign --verify --deep --strict --verbose=2 "$APP_ROOT"
/usr/sbin/spctl --assess --type execute --verbose=2 "$APP_ROOT"
rm -f "$NOTARY_ZIP_PATH"
(
  cd "$OUTPUT_ROOT"
  /usr/bin/ditto -c -k --keepParent "$PACKAGE_STEM" "$ZIP_PATH"
)
trap - EXIT

(
  cd "$OUTPUT_ROOT"
  /usr/bin/shasum -a 256 "$PACKAGE_STEM.zip" > "$PACKAGE_STEM.zip.sha256"
)

echo "Created Petri macOS app package:"
echo "  App: $APP_ROOT"
echo "  Zip: $ZIP_PATH"
echo "  SHA256: $ZIP_PATH.sha256"
