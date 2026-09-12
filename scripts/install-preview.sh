#!/usr/bin/env bash
set -euo pipefail
package="$(cd "$(dirname "$0")" && pwd)"
system="$(uname -s)"; arch="$(uname -m)"
case "$system:$arch" in
  Darwin:arm64|Darwin:x86_64) stem="Petri-macos-$arch"; extension=zip ;;
  Linux:x86_64) stem=Petri-linux-x86_64; extension=tar.gz ;;
  *) echo 'Unsupported platform or architecture' >&2; exit 1 ;;
esac
if [[ ! -f "$package/SHA256SUMS" ]]; then
  stage="$(mktemp -d -t petri-download.XXXXXXXX)"
  base="https://github.com/amoeba-farm/petri/releases/latest/download/$stem.$extension"
  curl --fail --location --proto '=https' --tlsv1.2 "$base" -o "$stage/$stem.$extension"
  curl --fail --location --proto '=https' --tlsv1.2 "$base.sha256" -o "$stage/checksum"
  (cd "$stage"; if [[ "$system" == Darwin ]]; then shasum -a 256 -c checksum; else sha256sum -c checksum; fi)
  if [[ "$extension" == zip ]]; then unzip -q "$stage/$stem.$extension" -d "$stage"; else tar -xzf "$stage/$stem.$extension" -C "$stage"; fi
  package="$stage/$stem"
fi
(cd "$package"; if [[ "$system" == Darwin ]]; then shasum -a 256 -c SHA256SUMS; else sha256sum -c SHA256SUMS; fi)
mkdir -p "$HOME/.local/bin"
if [[ "$system" == Darwin ]]; then
  app="$HOME/Applications/Petri Preview.app"
  mkdir -p "$HOME/Applications"
  /usr/bin/ditto "$package/Petri.app" "$app"
  binary="$app/Contents/Resources/petri"
else
  mkdir -p "$HOME/.local/share/petri-preview"
  cp "$package/petri" "$HOME/.local/share/petri-preview/petri"
  binary="$HOME/.local/share/petri-preview/petri"
fi
ln -sfn "$binary" "$HOME/.local/bin/petri"
"$HOME/.local/bin/petri" --version
echo 'Petri installed. Run ~/.local/bin/petri, or add ~/.local/bin to PATH.'
echo 'This Devnet preview is not publisher-signed or notarized. macOS users may need Privacy & Security > Open Anyway for Petri Preview.app.'
