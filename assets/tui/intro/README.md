# Petri TUI startup animation

`source.gif` is the supplied red-and-black Amoeba loading animation.
`manifest.json` and `frames.bin.zlib` retain the Amoeba web ASCII conversion: a
10-glyph density charset and four bytes per cell (`glyph index`, `red`, `green`,
`blue`). `terminal-frames.bin.zlib` is Petri's immutable playback asset. It crops the
known-safe source view, applies the final terminal glyph mapping and red-level
bit ahead of time, and losslessly compresses the byte-per-cell stream. Petri embeds only that compact
asset in normal builds, so the intro works offline without a GIF decoder or
per-frame RGB conversion. A bounded zlib decoder reconstructs the exact bytes once,
before terminal setup and the playback clock. The format is one complete zlib
stream with no trailing data; exact expanded length, glyph alphabet and SHA-256
are checked. Rendering, crop, timing, masks and input geometry are unchanged.

Regenerate the packed asset with Pillow installed:

```bash
python scripts/build-tui-intro-ascii.py path/to/ameba-loading-red-black-ascii-ghost-spin-morph-v6.gif assets/tui/intro
```

The web-parity conversion remains 240 by 83 cells, 187 frames, and 30 FPS. The
terminal asset is a 211 by 67 safe view containing all 207 source-GIF frames at
their exact 30 ms duration (33.3 FPS). The terminal stream stores **61,061 bytes**
and expands to the original **2,926,359 bytes**. The test-only RGB stream stores
**92,617 bytes** and expands to **14,900,160 bytes**. These are asset sizes, not
whole-executable measurements. Normal playback retains one 2.93 MB decoded heap
buffer; tests additionally decode the reference fixture. The authored motion keeps eight source columns and
four source rows of padding inside that view, while the responsive layout keeps
blank terminal rows between the animation, title, and Open button.

The storage migration compressed the existing bytes directly; it did not rerun
Pillow or resample the GIF. The generator now writes zlib level 9 and records the
decoded sizes and hashes in the manifest. Intentional future artwork changes
must update the independently pinned dimensions/hashes in `src/lab/intro.rs`
and the fixture assertions after review. Compression-library versions can change
compressed bytes without changing the authored stream; compare decoded hashes.

Playback advances in source order and wraps directly from the last frame to the
first. Petri caps the projected animation at 36 terminal rows on oversized
windows, then precomputes all 207 projected frames and their adjacent change
lists once per terminal size. Duplicate frame indexes do no work. Windows poll
timeouts are rounded to the API's whole-millisecond precision, preventing early
wakes from spinning through duplicate renders.

The first frame and terminal state changes use the normal Ratatui draw. In a
visible classic Windows console, steady ticks write only changed UTF-16
characters through the native console API, preserving the already-painted
true-color attributes and avoiding repeated ANSI parsing in `conhost`. ConPTY,
Windows Terminal, other platforms, and transient native-write failures use the
row-ordered buffered ANSI delta instead. Every path uses synchronized
presentation. This fixed-asset path avoids rebuilding or diffing the full
screen every 30 ms. Focus and destination transitions no longer pre-clear the
terminal; Ratatui diffs directly from the retained frame, avoiding the visible
black flash that a clear-then-redraw sequence can expose.

The intro overlays the same large Amoeba Farm title art used by Petri's normal
TUI header, rendered white on a black canvas. A one-time union of every authored
frame marks the animation's complete motion footprint; only that irregular area
is painted with the main TUI's dark navy, so it reads as a carved opening rather
than a rectangular panel. Visible animation cells use bold terminal weight and
a denser display mapping that promotes narrow middle-density glyphs to `#` and
`@`. The two authored red levels map to a brighter terminal pair over the navy
carve. The Open control is a solid white button with a black label.
