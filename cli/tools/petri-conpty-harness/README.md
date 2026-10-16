# Petri native ConPTY harness

> **DEVELOPMENT AND RELEASE VALIDATION ONLY.** This crate is not production
> runtime code, is not shipped with Petri, and must never handle live trades,
> wallets, provider credentials, or production data.

This standalone Windows crate launches the real `petri.exe` inside a native
ConPTY session and checks the current terminal screen after real input events.
It exists to make TUI acceptance testing repeatable without automating Windows
Terminal or another visible terminal application.

## Production isolation

- The crate has `publish = false` and its own `Cargo.lock`.
- It is not a member or dependency of Petri's production Cargo package.
- Petri's release packaging and installers do not copy or invoke `tools/`.
- Every scenario runs in its own watchdog worker process.
- The Petri child receives a cleared, minimal environment.
- `PETRI_GUIDE_PROVIDER=off` is forced; Codex and Claude are never launched.
- Reduced motion is forced for deterministic non-animation scenarios; the
  startup scenario explicitly enables motion so its frame sampling is real.
- Startup runs with the real two-red true-color path enabled rather than
  `--no-color`, so cadence validation includes the ANSI styling users see.
- Backend and documentation requests go only to a harness-owned loopback 404
  server, so the suite never calls production services.
- The child uses a missing sandbox keypair, an isolated Solana config, DevNet,
  isolated config/cache paths, and no inherited wallet or provider secrets.
- Text captures are opt-in. Normal runs create no repository artifacts.

## What the smoke suite covers

- The startup intro's black canvas, white Amoeba Farm title, irregular dark-navy
  animation carve, two red glyph levels, and solid white Open button with black
  text are verified from the terminal's rendered cells before Home. The
  animation keeps blank terminal rows between the title and Open button, and
  common terminal sizes sustain motion before Enter opens the
  existing Home or Terms destination, and a native
  resize from 120x32 to 320x90 sustains at least ten distinct screen frames
  across twelve 60 ms samples at the GIF's native 30 ms cadence. After draining
  the resize's one-time full redraw, the harness counts synchronized frame
  presentations and requires at least 80 percent of the authored cadence with
  a bound on long presentation stalls. The expanded steady-state sample is also
  capped at 200,000 terminal bytes per second, which rejects a regression to
  full ANSI cell repainting. It then shrinks to 80x24 and rejects any fully
  blank completed presentation during either resize or the Open-to-Home
  handoff, waiting for each synchronized frame boundary rather than sampling a
  resize while its VT stream is still in flight.
  Startup text artifacts capture that validated intro frame rather than the
  destination shown after Enter.
- Petri passes its interactive-terminal gate at 60x20, 80x24, and 120x32.
- Short Home and Help screens keep the `lines hidden` notice and scroll hint in
  the visible footer, including after Help scrolls; collapsing Help boxes keep
  their own in-box notice across native terminal resizes.
- Space pins and Esc dismisses the selected Help preview.
- A native Windows no-button mouse-move record reaches Crossterm, opens the
  Help hover preview, and moving away dismisses it.
- The Guide opens and stays responsive with its provider explicitly off.
- Native ConPTY resize events redraw Help at 60x20, 80x24, and 120x32.
- `q` exits successfully and the harness drains shutdown output, verifies the
  cursor, pointer, and focus cleanup after the final render, and confirms the
  current screen is the blank primary buffer rather than stale TUI content.
- Output and scenario duration are hard-bounded.

The harness parses the VT stream into current screen cells with `vt100`. It
does not assert against stale raw ANSI history. On older Windows builds it also
answers ConPTY's initial cursor-position query before launching Petri and keeps
output draining during shutdown.

## Run it

From `ameba_cli` on Windows:

```powershell
cargo test --manifest-path tools/petri-conpty-harness/Cargo.toml --locked -j1 -- --test-threads=1
cargo run --manifest-path tools/petri-conpty-harness/Cargo.toml --locked -- smoke --binary target/release/petri.exe
```

Or use the wrapper:

```powershell
tools/petri-conpty-harness/run.ps1
tools/petri-conpty-harness/run.ps1 -Scenario help-mouse-move -ShowScreen
```

Available scenarios:

```text
startup
help-keyboard-preview
help-mouse-move
guide-off
help-resize
```

Use `--artifacts <outside-or-ignored-directory>` only when a normalized text
capture is useful for review. The default local `artifacts/` directory is
ignored.

## Scope

This is a TUI release check, not proof of backend, wallet, transaction, oracle,
or on-chain correctness. Live-provider and live-trading tests intentionally do
not belong in this deterministic suite.
