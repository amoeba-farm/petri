#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_NAME="Petri"
COMMAND_NAME="petri"

if [[ $# -ne 0 ]]; then
  echo "Usage: $0" >&2
  exit 2
fi

if [[ "$(basename "$SCRIPT_DIR")" == "Resources" && "$(basename "$(dirname "$SCRIPT_DIR")")" == "Contents" ]]; then
  SOURCE_APP="$(cd "$SCRIPT_DIR/../.." && pwd)"
else
  SOURCE_APP="$SCRIPT_DIR/$APP_NAME.app"
fi
if [[ ! -d "$SOURCE_APP" ]]; then
  echo "Missing packaged app: $SOURCE_APP" >&2
  echo "Run scripts/package-petri-macos.sh first." >&2
  exit 1
fi
/usr/bin/codesign --verify --deep --strict "$SOURCE_APP"
/usr/bin/xcrun stapler validate "$SOURCE_APP"
/usr/sbin/spctl --assess --type execute --verbose=2 "$SOURCE_APP"

mkdir -p "$HOME/Applications" "$HOME/.local/bin"
INSTALL_APP="$HOME/Applications/Petri.app"
STAGED_APP="$HOME/Applications/.Petri.app.install.$$"
cleanup_staged_app() {
  rm -rf "$STAGED_APP"
}
trap cleanup_staged_app EXIT
rm -rf "$STAGED_APP"
/usr/bin/ditto "$SOURCE_APP" "$STAGED_APP"
/usr/bin/codesign --verify --deep --strict "$STAGED_APP"
/usr/bin/xcrun stapler validate "$STAGED_APP"
/usr/sbin/spctl --assess --type execute --verbose=2 "$STAGED_APP"
rm -rf "$INSTALL_APP"
mv "$STAGED_APP" "$INSTALL_APP"
/usr/bin/codesign --verify --deep --strict "$INSTALL_APP"
/usr/bin/xcrun stapler validate "$INSTALL_APP"
/usr/sbin/spctl --assess --type execute --verbose=2 "$INSTALL_APP"
trap - EXIT
chmod 755 \
  "$INSTALL_APP/Contents/MacOS/$APP_NAME" \
  "$INSTALL_APP/Contents/Resources/petri" \
  "$INSTALL_APP/Contents/Resources/petri.command"
ln -sfn "$INSTALL_APP/Contents/Resources/petri" "$HOME/.local/bin/$COMMAND_NAME"

echo "Installed $APP_NAME:"
echo "  App:     $INSTALL_APP"
echo "  Command: $HOME/.local/bin/$COMMAND_NAME"
echo ""
echo "Open it from Finder or run $COMMAND_NAME."
