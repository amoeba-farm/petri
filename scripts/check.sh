#!/usr/bin/env bash
# Builds and tests every component of the monorepo, or the ones named.
#
#   scripts/check.sh                    everything, in dependency order
#   scripts/check.sh futures api        just those components
#   scripts/check.sh --list             the component names
#
# Components: governance oracle futures api sdk cli web research docs.
# Each runs from its own folder, so its rust-toolchain.toml applies.
#
# Environment:
#   JOBS=N                       cargo / cargo-build-sbf parallelism; default half
#                                the cores (`nproc`), so two builds fit at once
#   FUTURES_TEST_DATABASE_URL    a scratch Postgres database for the API's
#                                history and candidate tests (they TRUNCATE
#                                its futures tables; never point it at a live
#                                database). Unset: those tests skip.
#   AGAVE_BIN                    the Agave 4.0.0 bin directory (cargo-build-sbf)
#   SKIP_SBF=1                   skip the SBF builds and the tests that need them
#   NPM_CI=1                     always reinstall node_modules (npm ci); by default
#                                only when missing or older than package-lock.json
#   WEB_PRODUCTION_BUILD=1       web: also the production build (next build), with
#                                its own npm ci; ship/ship.sh gate sets it
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
# An agent worktree (scripts/agent.sh new) carries its settings here: the shared
# Rust cache, no incremental data or debug info, its ports.
# shellcheck disable=SC1091
[ -f "$root/.agent.env" ] && . "$root/.agent.env"
JOBS="${JOBS:-$(( ($(nproc) + 1) / 2 ))}"
AGAVE_BIN="${AGAVE_BIN:-$HOME/.local/share/solana/install/releases/4.0.0/solana-release/bin}"
export PATH="$AGAVE_BIN:$PATH"
export CARGO_BUILD_JOBS="$JOBS"
ALL=(governance oracle futures api sdk cli web research docs ops-dashboard)
# Node tests in research/ that fail on main today (pre-existing; the research
# oracle reader test expects a catalog shape the API no longer serves).
# Skipped in the gating run, then run and reported; fix and remove.
RESEARCH_KNOWN_FAILURES='the oracle reader uses only the two public GET routes'
# The LiteSVM tests' oracle id (futures/integration/.cargo/config.toml): also
# used to compile-check the ops binaries without the dev chain's key.
TEST_ORACLE_ID=BwpnjMqU7f25DVq2wMvnWXmWCLnSZffPmcEs4eYM14Cj
# SBF stack errors. The SBF compiler reports a function whose stack frame
# passes 4 KB, or a call whose stack arguments overwrite the caller's frame, as
# "Error: ...", yet links the program and exits 0. At run time that program
# corrupts its own stack. sbf_build fails on them. Cargo replays the messages
# of cached crates, so a warm cache still shows them.
SBF_STACK_ERRORS='^Error: |overflows the maximum allowed frame|[Ee]xceeding the maximum stack offset|[Ss]tack offset.*exceeded|overwrites values in the frame'
# Stack errors in dead code: functions of governance_controller_v3's
# dependencies that the program never calls. The compiler reports them when it
# builds those crates, then the program's fat-LTO link drops them, and the
# whole-program build reports nothing. Checked 2026-10-15 (Agave 4.0.0,
# platform-tools v1.53): none of these symbols is in the linked, unstripped
# governance_controller_v3.so, and stripping that file gives the deployed
# artifact byte for byte. sbf_build checks again on every build and fails if
# one is ever linked in. The names are exact: a new toolchain or dependency
# version renames them, and then they fail until someone checks again.
SBF_DEAD_CODE=(
  # crypto-common 0.2.2 (digest 0.11, blake3, solana-program 2.3): hasher-state
  # SerializableState for the large state arrays; nothing in the program
  # serializes a hasher's state.
  '_ZN88_$LT$$u5b$u16$u3b$$u20$2048$u5d$$u20$as$u20$crypto_common..hazmat..SerializableState$GT$9serialize17h5f4a9ce89dbe6fafE'
  '_ZN88_$LT$$u5b$u16$u3b$$u20$2048$u5d$$u20$as$u20$crypto_common..hazmat..SerializableState$GT$11deserialize17h0bcdf6614552766bE'
  '_ZN88_$LT$$u5b$u16$u3b$$u20$4096$u5d$$u20$as$u20$crypto_common..hazmat..SerializableState$GT$9serialize17hae70901f395da310E'
  '_ZN88_$LT$$u5b$u16$u3b$$u20$4096$u5d$$u20$as$u20$crypto_common..hazmat..SerializableState$GT$11deserialize17h940a3d08bd7bc037E'
  '_ZN88_$LT$$u5b$u32$u3b$$u20$1024$u5d$$u20$as$u20$crypto_common..hazmat..SerializableState$GT$9serialize17h46c34a7942da08b9E'
  '_ZN88_$LT$$u5b$u32$u3b$$u20$1024$u5d$$u20$as$u20$crypto_common..hazmat..SerializableState$GT$11deserialize17ha56b1517cd51cd9fE'
  '_ZN88_$LT$$u5b$u32$u3b$$u20$2048$u5d$$u20$as$u20$crypto_common..hazmat..SerializableState$GT$9serialize17h0e923b5e8d189ad8E'
  '_ZN88_$LT$$u5b$u32$u3b$$u20$2048$u5d$$u20$as$u20$crypto_common..hazmat..SerializableState$GT$11deserialize17h055f2c8f16e56dd2E'
  '_ZN87_$LT$$u5b$u64$u3b$$u20$512$u5d$$u20$as$u20$crypto_common..hazmat..SerializableState$GT$9serialize17he2db5260add153dcE'
  '_ZN87_$LT$$u5b$u64$u3b$$u20$512$u5d$$u20$as$u20$crypto_common..hazmat..SerializableState$GT$11deserialize17h98ceb22dd2f08358E'
  '_ZN88_$LT$$u5b$u64$u3b$$u20$1024$u5d$$u20$as$u20$crypto_common..hazmat..SerializableState$GT$9serialize17he28932bed51f8acbE'
  '_ZN88_$LT$$u5b$u64$u3b$$u20$1024$u5d$$u20$as$u20$crypto_common..hazmat..SerializableState$GT$11deserialize17hfaae747f1ddb1912E'
  '_ZN88_$LT$$u5b$u128$u3b$$u20$256$u5d$$u20$as$u20$crypto_common..hazmat..SerializableState$GT$11deserialize17hf099676ccfa83406E'
  '_ZN88_$LT$$u5b$u128$u3b$$u20$512$u5d$$u20$as$u20$crypto_common..hazmat..SerializableState$GT$9serialize17hb9e97e00ebde5050E'
  '_ZN88_$LT$$u5b$u128$u3b$$u20$512$u5d$$u20$as$u20$crypto_common..hazmat..SerializableState$GT$11deserialize17hf072761d8d5f76b0E'
  # hybrid-array 0.4.14 (block-buffer, same path): Array::try_from_fn.
  '_ZN12hybrid_array7from_fn50_$LT$impl$u20$hybrid_array..Array$LT$T$C$U$GT$$GT$11try_from_fn17h2894aaeb78e35234E'
  # upgrade_controller, the earlier controller: its extend-target processor.
  # The V3 controller links upgrade_controller's account and Merkle helpers,
  # never this processor.
  '_ZN18upgrade_controller29release1_processor_v3_custody9execution24process_extend_target_v217haad254852cec96edE'
)
# Markers that only dev builds carry: governance-interface's localnet profile
# and the oracle's dev-authority and synthetic-controller profiles. A
# production artifact must carry none of them.
DEV_MARKERS=(
  AMEBA_LOCALNET_GOVERNANCE_DO_NOT_RELEASE
  AMEBA_ORACLE_LOCALNET_DEV_AUTHORITY
  AMEBA_ORACLE_SYNTHETIC_CONTROLLER_DO_NOT_RELEASE
)
# The test-USDC faucet is dev-only (ship/releases/2026-10-15-preflight.md,
# decision 3): web imports it only in localnet builds, the API and the CLI only
# with their dev-faucet feature. Its route is in every build that has it, and
# a production build or binary must not carry it.
FAUCET_MARKERS=(/v1/faucet)

step() { printf '\n==> %s: %s\n' "$component" "$*"; }
run() { step "$*"; "$@"; }
sbf() { [ "${SKIP_SBF:-}" != 1 ]; }
# node_known <pattern> <files...>: the gating run skips the tests whose names
# match <pattern>; they then run on their own and are only reported.
node_known() {
  local pattern="$1"; shift
  run node --test --test-skip-pattern="$pattern" "$@"
  step "known failures matching \"$pattern\" (reported only)"
  if node --test --test-name-pattern="$pattern" "$@" >/dev/null 2>&1; then
    echo "  they pass now; remove the pattern from check.sh"
  else
    echo "  still failing"
  fi
}
npm_install() {
  # An agent worktree links node_modules to the main checkout's while the
  # lockfile matches; a changed lockfile gets a local install, never one
  # through the link (that would change the live site's packages).
  if [ -L node_modules ]; then
    if cmp -s package-lock.json "$(readlink -f node_modules)/../package-lock.json"; then return 0; fi
    step "package-lock.json differs from the main checkout's: installing locally"
    rm node_modules
  fi
  if [ "${NPM_CI:-}" = 1 ] || [ ! -f node_modules/.package-lock.json ] \
    || [ package-lock.json -nt node_modules/.package-lock.json ]; then
    run npm ci "$@"
  fi
}
# sbf_build <target-dir>:<program>... -- <command...>: run an SBF build and
# fail on its stack errors (SBF_STACK_ERRORS). Each <program> is one the build
# links, in cargo's <target-dir>; an SBF_DEAD_CODE error is allowed only while
# its function is in none of them.
sbf_build() {
  local specs=() spec so found log line sym errors=0 dead=() linked=()
  while [ "$1" != -- ]; do specs+=("$1"); shift; done
  shift
  log="$(mktemp)"
  step "$*"
  if ! "$@" 2>&1 | tee "$log"; then rm -f "$log"; return 1; fi
  step "SBF stack errors"
  while IFS= read -r line; do
    sym="$(sed -nE 's/^Error: (Function|A function call in method) ([^ ]+) .*/\2/p' <<<"$line")"
    if [ -n "$sym" ] && in_list "$sym" "${SBF_DEAD_CODE[@]}"; then
      in_list "$sym" "${dead[@]}" || dead+=("$sym")
    else
      echo "  $line"; errors=$((errors + 1))
    fi
  done < <(grep -aE "$SBF_STACK_ERRORS" "$log" || true)
  rm -f "$log"
  # The linked (unstripped) programs: <target-dir>/sbpf<arch>-solana-solana/release.
  for spec in "${specs[@]}"; do
    mapfile -t found < <(compgen -G "${spec%:*}/sbpf*-solana-solana/release/${spec##*:}.so" || true)
    [ ${#found[@]} -gt 0 ] || found=("${spec%:*}/sbpf-solana-solana/release/${spec##*:}.so")
    linked+=("${found[@]}")
  done
  for so in "${linked[@]}"; do
    if [ ! -f "$so" ]; then
      echo "  missing linked program $so: its dead code can't be checked"; errors=$((errors + 1)); continue
    fi
    for sym in "${dead[@]}"; do
      if grep -qaF -- "$sym" "$so"; then
        echo "  $sym is in SBF_DEAD_CODE but linked into $so"; errors=$((errors + 1))
      fi
    done
  done
  if [ "$errors" -gt 0 ]; then
    echo "  SBF stack check failed ($errors)" >&2
    return 1
  fi
  echo "  none in ${#linked[@]} linked program(s)$([ ${#dead[@]} -eq 0 ] \
    || echo "; ${#dead[@]} function(s) of SBF_DEAD_CODE reported, linked into none")"
}
in_list() { local x="$1" y; shift; for y in "$@"; do [ "$x" = "$y" ] && return 0; done; return 1; }
# release_markers <artifact>...: production artifacts carry no DEV_MARKERS.
release_markers() {
  local so m bad=0
  step "dev-build markers in $*"
  for so in "$@"; do
    [ -f "$so" ] || { echo "  missing $so"; bad=1; continue; }
    for m in "${DEV_MARKERS[@]}"; do
      if grep -qaF -- "$m" "$so"; then echo "  $so carries $m: a dev build, never release it"; bad=1; fi
    done
  done
  [ "$bad" = 0 ] || return 1
  echo "  none"
}
# SBPF v1 (ship/targets.md, "First mainnet launch"): the production builds of
# the oracle (here) and the futures program (futures/scripts/build-sbf.sh) pass
# these to cargo-build-sbf, and sbpf_v1 checks the result. The ELF e_flags
# (4 bytes LE at offset 48) hold the SBPF version: 0 = v0, 1 = v1.
SBPF_ARCH=(--arch v1 --tools-version v1.53)
sbpf_v1() {
  local so flags bad=0
  step "SBPF v1 in $*"
  for so in "$@"; do
    [ -f "$so" ] || { echo "  missing $so"; bad=1; continue; }
    flags="$(od -An -t u4 -j 48 -N 4 "$so" | tr -d ' ')"
    if [ "$flags" = 1 ]; then echo "  $so: v1"; else echo "  $so: e_flags $flags, not v1"; bad=1; fi
  done
  [ "$bad" = 0 ] || { echo "  a production artifact that isn't SBPF v1: never release it" >&2; return 1; }
}
# no_faucet <path>...: production artifacts (files, or folders searched
# recursively) carry no FAUCET_MARKERS.
no_faucet() {
  local m hits bad=0
  step "dev faucet in $*"
  for m in "${FAUCET_MARKERS[@]}"; do
    hits="$(grep -rlaF --exclude-dir=cache -- "$m" "$@" || true)"
    if [ -n "$hits" ]; then echo "  \"$m\" in:"; head -20 <<<"$hits" | sed 's/^/    /'; bad=1; fi
  done
  if [ "$bad" != 0 ]; then echo "  a production build with the dev faucet: never release it" >&2; return 1; fi
  echo "  none"
}

check_governance() {
  cd "$root/governance"
  run cargo fmt --all -- --check
  run cargo test --locked -p ameba-governance-interface -p governance_controller_v3 -j "$JOBS"
  run cargo test --locked -p ameba-governance-interface -p governance_controller_v3 --features localnet-governance -j "$JOBS"
  sbf || return 0
  # The futures integration suite loads this same controller artifact. Share
  # its SBF intermediates, while native workspaces keep separate targets.
  local controller_cache="${AMEBA_CARGO_CACHE:+$AMEBA_CARGO_CACHE/futures/sbf-governance}"
  controller_cache="${controller_cache:-${CARGO_TARGET_DIR:-$PWD/target}/sbf}"
  sbf_build "$controller_cache:governance_controller_v3" -- env CARGO_TARGET_DIR="$controller_cache" cargo-build-sbf \
    --manifest-path programs/governance_controller_v3/Cargo.toml --sbf-out-dir target/deploy -- --locked -j "$JOBS"
  release_markers target/deploy/governance_controller_v3.so
}

check_oracle() {
  cd "$root/oracle"
  run cargo fmt --all -- --check
  run cargo test --workspace --locked -j "$JOBS"
  run cargo test -p ameba_oracle --lib --locked -j "$JOBS" --features synthetic-governance-controller
  AMEBA_ORACLE_PROGRAM_ID=9MBnqUCLEgkCxZHDPPrJGdwEJAiGU8K5tD3R5SD5MUkm \
    run cargo test -p ameba_oracle --lib --locked -j "$JOBS" --features localnet-governance
  AMEBA_ORACLE_PROGRAM_ID=9MBnqUCLEgkCxZHDPPrJGdwEJAiGU8K5tD3R5SD5MUkm \
    run cargo test -p ameba_oracle --locked -j "$JOBS" --features localnet-dev-authority
  sbf || return 0
  local sbf_target="${CARGO_TARGET_DIR:-$PWD/target}"
  # Production artifact (no features, SBPF v1): target/deploy.
  sbf_build "$sbf_target:ameba_oracle" -- cargo-build-sbf --manifest-path programs/ameba_oracle/Cargo.toml \
    "${SBPF_ARCH[@]}" --sbf-out-dir target/deploy -- --locked -j "$JOBS"
  release_markers target/deploy/ameba_oracle.so
  sbpf_v1 target/deploy/ameba_oracle.so
  ls -l target/deploy/ameba_oracle.so
  # Test-only artifact on the shortened test calendar, on the production arch
  # so the SBF tests run what ships; never deployed.
  sbf_build "$sbf_target:ameba_oracle" -- cargo-build-sbf --manifest-path programs/ameba_oracle/Cargo.toml \
    "${SBPF_ARCH[@]}" --features test-sbf --sbf-out-dir target/deploy-test -- --locked -j "$JOBS"
  SBF_OUT_DIR="$PWD/target/deploy-test" run cargo test -p ameba_oracle --locked -j "$JOBS" --features test-sbf
}

check_futures() {
  cd "$root/futures"
  run cargo fmt --all -- --check
  run cargo clippy --workspace --all-targets --locked -j "$JOBS" -- -D warnings
  run cargo test --workspace --locked -j "$JOBS"
  # The ops binaries and unit tests, built for the tests' oracle id (the dev
  # chain's build is scripts/build-ops.sh, for the .localnet key).
  (cd ops && AMEBA_ORACLE_PROGRAM_ID="$TEST_ORACLE_ID" \
    run cargo test --locked -j "$JOBS" --target-dir "${CARGO_TARGET_DIR:-$root/futures/target}/ops-check")
  # The mainnet build (scripts/build-ops.sh --mainnet): the production oracle
  # profile, no dev tools. Its own target: the oracle crate's features differ.
  (cd ops && unset AMEBA_ORACLE_PROGRAM_ID && \
    run cargo test --locked -j "$JOBS" --no-default-features --target-dir "${CARGO_TARGET_DIR:-$root/futures/target}/ops-mainnet-check")
  sbf || return 0
  # build-sbf.sh's cargo target directories (its intermediates, and those of
  # build-oracle.sh --test).
  local build="${AMEBA_CARGO_CACHE:+$AMEBA_CARGO_CACHE/futures}"; build="${build:-$PWD/target}"
  sbf_build "$build/sbf-futures:ameba_futures" "$build/sbf-governance:governance_controller_v3" \
    "$build/sbf-oracle-test:ameba_oracle" -- scripts/build-sbf.sh
  # The production programs (the test oracle in target/deploy-oracle-test is a
  # dev build, never deployed).
  release_markers target/deploy/{ameba_futures,governance_controller_v3}.so
  sbpf_v1 target/deploy/ameba_futures.so
  # This is a separate Cargo workspace with a different feature graph. Program
  # crates emit un-hashed .rlib files (cdylib + lib), so sharing its target with
  # the main workspace can replace artifacts that Cargo still considers fresh.
  (cd integration && run cargo test --locked -j "$JOBS" \
    --target-dir "${CARGO_TARGET_DIR:-$root/futures/target}/integration-check")
}

check_api() {
  cd "$root/api"
  # The API's own verification is `cargo test --release` (api/AGENTS.md); its
  # code is not rustfmt- or clippy-clean, so neither gates here yet. The tests
  # run on the dev build (with its faucet, as api/ops/install.sh builds it).
  if [ -z "${FUTURES_TEST_DATABASE_URL:-}" ]; then
    step "FUTURES_TEST_DATABASE_URL is unset: the Postgres-backed tests skip"
  fi
  run cargo test --release --locked -j "$JOBS" --features dev-faucet
  # The production binary: no features, so no dev faucet.
  run cargo build --release --locked -j "$JOBS" -p ameba-futures-api
  no_faucet "${CARGO_TARGET_DIR:-$PWD/target}/release/ameba-futures-api"
}

check_sdk() {
  cd "$root/sdk"
  npm_install --ignore-scripts
  run npm run typecheck
  run npm test
  run cargo fmt -- --check
  run cargo test --locked -j "$JOBS"
}

check_cli() {
  cd "$root/cli"
  # The MCP server check runs target/debug/petri unless told otherwise; with a
  # shared CARGO_TARGET_DIR (agent worktrees) the fresh build is there.
  run env CARGO_BUILD_JOBS="$JOBS" ${CARGO_TARGET_DIR:+PETRI_MCP_PETRI_BIN="$CARGO_TARGET_DIR/debug/petri"} scripts/devloop.sh
  # devloop's petri has a release build's features: no dev faucet.
  no_faucet "${CARGO_TARGET_DIR:-$PWD/target}/debug/petri"
  # The license corpus the release packages ship must match Cargo.lock (the
  # public mirror's CI checks it too; a stale one failed it on 2026-10-17).
  run cargo fetch --locked
  run node scripts/generate-third-party-licenses.mjs --check
}

check_web() {
  cd "$root/web"
  # The production build gets its own install, as a release build does
  # (ship/targets.md), never one through the link to the live site's.
  if [ "${WEB_PRODUCTION_BUILD:-}" = 1 ] && [ -L node_modules ]; then
    step "production build: a local install instead of the node_modules link"
    rm node_modules
  fi
  npm_install
  run npx tsc --noEmit -p .
  run npm run lint
  run node --test tests/*.test.mjs test/*.test.mjs
  # The production build catches what dev never runs: NODE_ENV=production
  # code, prerendering, page data. It takes about 5 minutes and up to about
  # 3 GB of memory on the shared dev VM, too much for every check (agent.sh
  # merge runs this on every web or api change), so only the ship gate asks
  # for it. Its prerender reads the futures API at FUTURES_API_URL (default
  # :4500). It refuses to run where a `next dev` is up
  # (scripts/run-next-with-lock.mjs), so gate web in a worktree, not the live
  # main checkout.
  if [ "${WEB_PRODUCTION_BUILD:-}" = 1 ]; then
    run npm run build
    # What the server runs and the browser loads (next build leaves only its
    # cache from earlier builds).
    no_faucet .next/server .next/static
  fi
}

check_research() {
  cd "$root/research"
  node_known "$RESEARCH_KNOWN_FAILURES" test/*.test.mjs
  run cargo fmt -- --check
  run cargo test --locked -j "$JOBS"
}

check_ops-dashboard() {
  cd "$root/ops-dashboard"
  for file in server.mjs lib/*.mjs public/*.mjs test/*.mjs; do run node --check "$file"; done
  run bash -n deploy/install.sh
  run node --test test/*.test.mjs
}

check_docs() {
  cd "$root/docs"
  npm_install
  # The editor's Validate (local-editor/jobs.mjs): content, build, built site.
  run npm run check:content
  run npm run build
  run npm run check:built
  run node --test local-editor/editor.test.mjs
}

case "${1:-}" in
  --list) printf '%s\n' "${ALL[@]}"; exit 0 ;;
  -h|--help) sed -n '2,23p' "$0"; exit 0 ;;
esac
selected=("$@")
[ ${#selected[@]} -gt 0 ] || selected=("${ALL[@]}")
for component in "${selected[@]}"; do
  case " ${ALL[*]} " in
    *" $component "*) ;;
    *) echo "unknown component: $component (one of: ${ALL[*]})" >&2; exit 2 ;;
  esac
done
failed=()
for component in "${selected[@]}"; do
  set +e
  # In an agent worktree, Rust builds use the component's shared cache.
  ( set -e; [ -z "${AMEBA_CARGO_CACHE:-}" ] || export CARGO_TARGET_DIR="$AMEBA_CARGO_CACHE/$component"
    "check_$component" )
  rc=$?
  set -e
  if [ "$rc" -eq 0 ]; then
    printf '\n==> %s: ok\n' "$component"
  else
    printf '\n==> %s: FAILED (exit %s)\n' "$component" "$rc"
    failed+=("$component")
  fi
done
if [ ${#failed[@]} -gt 0 ]; then
  printf '\nfailed: %s\n' "${failed[*]}"
  exit 1
fi
printf '\nall ok: %s\n' "${selected[*]}"
