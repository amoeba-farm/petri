#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-1}"
NODE_BINARY="${NODE_BINARY:-node}"

cargo fmt -- --check
cargo check --locked --bin petri -j "${CARGO_BUILD_JOBS}"
cargo test --locked -j "${CARGO_BUILD_JOBS}" -- --test-threads=1
cargo clippy --locked --all-targets -j "${CARGO_BUILD_JOBS}"
# The dev build (scripts/install-dev.sh): the dev chain is its default profile
# (dev-chain), and it has the dev-only `petri faucet` (dev-faucet, DEV-ONLY:
# REMOVE FOR PROD), which compiles and parses.
cargo test --locked --features dev-chain,dev-faucet --bin petri -j "${CARGO_BUILD_JOBS}" -- --test-threads=1
cargo build --locked --bin petri -j "${CARGO_BUILD_JOBS}"
# A build without dev-chain, as a release is, defaults to production.
"${NODE_BINARY}" scripts/check-petri-defaults.mjs "${CARGO_TARGET_DIR:-target}/debug/petri" production
"${NODE_BINARY}" scripts/check-petri-research.mjs "${CARGO_TARGET_DIR:-target}/debug/petri"
