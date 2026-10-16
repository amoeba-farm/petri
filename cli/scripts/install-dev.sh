#!/usr/bin/env bash
# Builds the dev chain's petri and installs it as /opt/ameba-dev/bin/petri.
# This is the dev build: `dev-chain` makes the dev chain its default profile
# (the loopback futures API and validator; a release build defaults to
# production), and `dev-faucet` adds the dev-only `petri faucet` (DEV-ONLY:
# REMOVE FOR PROD). A release build (the packaging scripts) leaves both off.
# Run it from the main checkout, after a merge that changes cli/.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
bin_dir="${AMEBA_DEV_BIN_DIR:-/opt/ameba-dev/bin}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}"
cd "$root"  # so rust-toolchain.toml applies
cargo build --release --locked -j "${CARGO_JOBS:-3}" --bin petri --features dev-chain,dev-faucet
built="$CARGO_TARGET_DIR/release/petri"
# With no flags, environment or saved settings it must talk to the dev chain.
node scripts/check-petri-defaults.mjs "$built" dev
install -d -m 755 "$bin_dir"
install -m 755 "$built" "$bin_dir/petri.new"
mv -f "$bin_dir/petri.new" "$bin_dir/petri"
"$bin_dir/petri" --version
