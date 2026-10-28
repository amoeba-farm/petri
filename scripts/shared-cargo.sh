#!/usr/bin/env bash
# Cargo on the agent worktrees' shared Rust cache, without one worktree's
# build standing in for another's.
#
#   scripts/shared-cargo.sh <component> <cargo args...>   cargo in <component>/, on the shared cache
#   scripts/shared-cargo.sh <component> clippy <args...> [-- <lints...>]
#   . scripts/shared-cargo.sh                             the functions (scripts/check.sh, scripts/sbf-cache.sh)
#
# Agent worktrees (scripts/agent.sh new) share one Cargo build directory per
# component under $AMEBA_CARGO_CACHE, so crates.io crates build once for all
# of them. Left to itself, Cargo also shares the monorepo's own crates there:
#   - crates built as cdylib + lib (the programs: ameba_futures, ameba_oracle,
#     governance_controller_v3, upgrade_controller) get unhashed file names
#     (deps/lib<name>.rlib), so another worktree's build, or another rustc's,
#     replaces the file while this worktree's fingerprint still calls it fresh
#     (2026-10-20/21: "expected &[Arc], found &Vec<Arc>" pointing into another
#     worktree, E0514);
#   - a workspace's members hash by their path inside the workspace, the same
#     in every worktree, and Cargo's freshness is file times, so a worktree
#     takes another worktree's newer build of them as its own (2026-10-14: a
#     test binary with a deleted worktree's CARGO_MANIFEST_DIR);
#   - binaries are copied to one <target>/debug/<name> for every worktree
#     (debug/petri: whichever agent built last).
# shared_cargo_env keeps those apart with Cargo's own hashing:
#   - __CARGO_DEFAULT_LIB_METADATA (Cargo's switch for the standard library's
#     builds) gives cdylib + lib crates hashed names. It changes every
#     crate's hash, once, to the same value in every worktree.
#   - RUSTC_WORKSPACE_WRAPPER is a pass-through to rustc at this worktree's
#     path (shared-cargo/rustc). Cargo hashes the wrapper's path into the
#     workspace members' hashes (that's how it keeps clippy's results apart
#     from check's), so each worktree gets its own; crates outside the
#     workspace already hash by their absolute path, and crates.io crates by
#     name and version, so those stay shared. `cargo clippy` puts its own
#     wrapper there, so shared_clippy runs clippy through shared-cargo/clippy-driver.
#   - With Cargo 1.91+ (build.build-dir), only intermediates go to the cache:
#     binaries land in the worktree's <component>/target.
# Never for SBF builds: the hashes are in the programs' symbols (check.sh's
# SBF_DEAD_CODE names them exactly) and production artifacts must not change.
# scripts/sbf-cache.sh runs SBF builds without these settings.
shared_cargo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"

# cache_of <component>: its cache under $AMEBA_CARGO_CACHE. The CLI's Rust 1.95
# builds (since 2026-10-20) got their own while two toolchains could replace
# each other's program rlibs (E0514); hashed names keep them apart now, and the
# name stays so the cache does too.
cache_of() { case "$1" in cli) echo cli-1.95 ;; *) echo "$1" ;; esac; }

# shared_cargo_env <cache> <dir>: export the settings for builds of the Cargo
# workspace in <dir> on $AMEBA_CARGO_CACHE/<cache>.
shared_cargo_env() {
  local cache="$AMEBA_CARGO_CACHE/$1" dir="$2" minor
  export __CARGO_DEFAULT_LIB_METADATA=ameba-shared-cache
  export RUSTC_WORKSPACE_WRAPPER="$shared_cargo_root/scripts/shared-cargo/rustc"
  minor="$(cd "$dir" && cargo --version | sed -nE 's/^cargo 1\.([0-9]+)\..*/\1/p')"
  if [ "${minor:-0}" -ge 91 ]; then
    export CARGO_BUILD_BUILD_DIR="$cache" CARGO_TARGET_DIR="$dir/target"
  else
    unset CARGO_BUILD_BUILD_DIR
    export CARGO_TARGET_DIR="$cache"
  fi
}

# shared_clippy <cargo check args...> [-- <lints...>]: `cargo clippy`, through
# this worktree's wrapper (cargo-clippy's protocol: the lints in CLIPPY_ARGS).
shared_clippy() {
  local args=() lints="" lint
  while [ $# -gt 0 ] && [ "$1" != -- ]; do args+=("$1"); shift; done
  [ $# -eq 0 ] || shift
  for lint in "$@"; do lints+="${lint}__CLIPPY_HACKERY__"; done
  CLIPPY_ARGS="$lints" RUSTC_WORKSPACE_WRAPPER="$shared_cargo_root/scripts/shared-cargo/clippy-driver" \
    cargo check "${args[@]}"
}

# sbf_env <command...>: run <command> without shared_cargo_env's settings.
sbf_env() { env -u __CARGO_DEFAULT_LIB_METADATA -u RUSTC_WORKSPACE_WRAPPER -u CARGO_BUILD_BUILD_DIR "$@"; }

# sbf_claim <target-dir>...: in a worktree, take the SBF lock for the rest of
# this shell (fd 9) and make each <target-dir> this worktree's. See
# scripts/sbf-cache.sh.
sbf_claim() {
  [ -n "${AMEBA_CARGO_CACHE:-}" ] || return 0
  if [ -z "${AMEBA_SBF_LOCKED:-}" ]; then
    mkdir -p "$AMEBA_CARGO_CACHE"
    exec 9>"$AMEBA_CARGO_CACHE/sbf.lock"
    flock -n 9 || { echo "sbf-cache: waiting for another worktree's SBF build" >&2; flock 9; }
    export AMEBA_SBF_LOCKED=1
  fi
  local dir fp entry pkg dropped
  local -A ours=()
  for pkg in $(sbf_packages); do ours[$pkg]=1; done
  for dir in "$@"; do
    mkdir -p "$dir"
    [ "$(cat "$dir/.ameba-sbf-owner" 2>/dev/null)" != "$shared_cargo_root" ] || continue
    dropped=0
    # The host's (build scripts) and each SBF target's release fingerprints.
    for fp in "$dir"/release/.fingerprint "$dir"/*/release/.fingerprint; do
      [ -d "$fp" ] || continue
      local stale=()
      for entry in "$fp"/*; do
        [ -d "$entry" ] || continue
        pkg="${entry##*/}"; pkg="${pkg%-*}"
        [ -z "${ours[$pkg]:-}" ] || stale+=("$entry")
      done
      [ ${#stale[@]} -eq 0 ] && continue
      # Under Cargo's own lock on that directory.
      flock "${fp%/.fingerprint}/.cargo-lock" rm -rf -- "${stale[@]}"
      dropped=$((dropped + ${#stale[@]}))
    done
    printf '%s\n' "$shared_cargo_root" > "$dir/.ameba-sbf-owner"
    [ "$dropped" -eq 0 ] || echo "sbf-cache: ${dir##*/cargo/} was another worktree's build: rebuilding this worktree's crates" >&2
  done
}

# sbf_packages: the names of the monorepo's own packages (path crates) in this tree.
sbf_packages() {
  find "$shared_cargo_root" \( -name target -o -name node_modules -o -name .git -o -name .next \) -prune \
    -o -name Cargo.toml -type f -print0 \
    | xargs -0 awk '/^\[/ { p = ($0 == "[package]") } p && /^name *=/ { gsub(/^name *= *"|".*$/, ""); print; p = 0 }' \
    | sort -u
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  set -euo pipefail
  [ $# -ge 2 ] || { sed -n '5,7p' "$0" >&2; exit 2; }
  component="$1"; shift
  [ -f "$shared_cargo_root/$component/Cargo.toml" ] || { echo "no Cargo workspace $component/" >&2; exit 2; }
  # shellcheck disable=SC1091
  [ -f "$shared_cargo_root/.agent.env" ] && . "$shared_cargo_root/.agent.env"
  cd "$shared_cargo_root/$component"
  if [ -z "${AMEBA_CARGO_CACHE:-}" ]; then exec cargo "$@"; fi
  shared_cargo_env "$(cache_of "$component")" "$PWD"
  if [ "$1" = clippy ]; then shift; shared_clippy "$@"; else exec cargo "$@"; fi
fi
