#!/usr/bin/env python3
"""Convert the fixed Petri startup GIF into web and terminal frame assets."""

from __future__ import annotations

import argparse
import bisect
import hashlib
import json
import math
import shutil
import zlib
from pathlib import Path

from PIL import Image, ImageSequence


ASCII_CHARSET = " .:-=+*#%@"
TERMINAL_CHARSET = " .=+*#@"
TERMINAL_SYMBOL_MAP = (0, 1, 2, 3, 4, 5, 6, 6, 6, 6)
TERMINAL_BRIGHT_RED_THRESHOLD = 208
TERMINAL_BRIGHT_MASK = 0x80
TERMINAL_VIEW_LEFT = 15
TERMINAL_VIEW_RIGHT = 226
TERMINAL_VIEW_TOP = 8
TERMINAL_VIEW_BOTTOM = 75


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path, help="Source GIF")
    parser.add_argument("output", type=Path, help="Output asset directory")
    parser.add_argument("--cols", type=int, default=240)
    parser.add_argument("--fps", type=int, default=30)
    parser.add_argument("--cell-aspect", type=float, default=0.55)
    return parser.parse_args()


def composite_rgb(frame: Image.Image) -> Image.Image:
    rgba = frame.convert("RGBA")
    background = Image.new("RGBA", rgba.size, (0, 0, 0, 255))
    background.alpha_composite(rgba)
    return background.convert("RGB")


def load_frames(source: Path) -> tuple[list[Image.Image], list[int]]:
    with Image.open(source) as image:
        frames: list[Image.Image] = []
        durations: list[int] = []
        for frame in ImageSequence.Iterator(image):
            frames.append(composite_rgb(frame))
            duration = int(frame.info.get("duration") or image.info.get("duration") or 0)
            durations.append(duration if duration > 0 else 100)
    if not frames:
        raise ValueError("source GIF contains no frames")
    return frames, durations


def frame_indices_for_fps(durations: list[int], fps: int) -> list[int]:
    cumulative: list[int] = []
    total_ms = 0
    for duration in durations:
        total_ms += duration
        cumulative.append(total_ms)
    output_count = max(1, math.ceil(total_ms * fps / 1000))
    return [
        min(len(durations) - 1, bisect.bisect_right(cumulative, index * 1000 / fps))
        for index in range(output_count)
    ]


def pack_frame(frame: Image.Image, cols: int, rows: int) -> bytes:
    resized = frame.resize((cols, rows), Image.Resampling.BOX)
    packed = bytearray(cols * rows * 4)
    offset = 0
    for red, green, blue in resized.getdata():
        # The source is deliberately monochrome red. Channel intensity keeps its
        # full glyph range while RGB remains available for terminal color.
        density = max(red, green, blue)
        char_index = round(density / 255 * (len(ASCII_CHARSET) - 1))
        packed[offset : offset + 4] = bytes((char_index, red, green, blue))
        offset += 4
    return bytes(packed)


def pack_terminal_frame(packed_frame: bytes, cols: int, rows: int) -> bytes:
    if TERMINAL_VIEW_RIGHT > cols or TERMINAL_VIEW_BOTTOM > rows:
        raise ValueError("terminal source view exceeds the packed ASCII frame")

    terminal = bytearray(
        (TERMINAL_VIEW_RIGHT - TERMINAL_VIEW_LEFT)
        * (TERMINAL_VIEW_BOTTOM - TERMINAL_VIEW_TOP)
    )
    target_offset = 0
    for row in range(TERMINAL_VIEW_TOP, TERMINAL_VIEW_BOTTOM):
        for column in range(TERMINAL_VIEW_LEFT, TERMINAL_VIEW_RIGHT):
            source_offset = (row * cols + column) * 4
            symbol = TERMINAL_SYMBOL_MAP[packed_frame[source_offset]]
            code = symbol
            if symbol and packed_frame[source_offset + 1] >= TERMINAL_BRIGHT_RED_THRESHOLD:
                code |= TERMINAL_BRIGHT_MASK
            terminal[target_offset] = code
            target_offset += 1
    return bytes(terminal)


def main() -> None:
    args = parse_args()
    if args.cols <= 0 or args.fps <= 0 or args.cell_aspect <= 0:
        raise ValueError("cols, fps, and cell-aspect must be positive")

    source = args.source.resolve(strict=True)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    frames, durations = load_frames(source)
    width, height = frames[0].size
    rows = max(1, round(height / width * args.cols * args.cell_aspect))
    frame_indices = frame_indices_for_fps(durations, args.fps)

    packed_source_frames = [pack_frame(frame, args.cols, rows) for frame in frames]
    packed = bytearray()
    for frame_index in frame_indices:
        packed.extend(packed_source_frames[frame_index])

    unique_frame_durations = set(durations)
    if len(unique_frame_durations) != 1:
        raise ValueError("the fixed terminal animation requires one exact frame duration")
    terminal_frame_duration_ms = unique_frame_durations.pop()
    terminal_frames = bytearray()
    for packed_frame in packed_source_frames:
        terminal_frames.extend(pack_terminal_frame(packed_frame, args.cols, rows))

    frames_path = output / "frames.bin.zlib"
    temporary_frames_path = output / "frames.bin.zlib.tmp"
    temporary_frames_path.write_bytes(zlib.compress(packed, level=9))
    temporary_frames_path.replace(frames_path)
    terminal_frames_path = output / "terminal-frames.bin.zlib"
    temporary_terminal_frames_path = output / "terminal-frames.bin.zlib.tmp"
    temporary_terminal_frames_path.write_bytes(zlib.compress(terminal_frames, level=9))
    temporary_terminal_frames_path.replace(terminal_frames_path)
    manifest = {
        "version": 1,
        "fps": args.fps,
        "cols": args.cols,
        "rows": rows,
        "frameCount": len(frame_indices),
        "charset": ASCII_CHARSET,
        "encoding": "u8-index-rgb-raw",
        "data": "frames.bin.zlib",
        "storage": {"codec": "zlib", "bytes": len(packed),
                    "sha256": hashlib.sha256(packed).hexdigest()},
        "terminal": {
            "frameCount": len(packed_source_frames),
            "frameDurationMs": terminal_frame_duration_ms,
            "cols": TERMINAL_VIEW_RIGHT - TERMINAL_VIEW_LEFT,
            "rows": TERMINAL_VIEW_BOTTOM - TERMINAL_VIEW_TOP,
            "charset": TERMINAL_CHARSET,
            "encoding": "u8-display-index-bright-bit7",
            "brightMask": TERMINAL_BRIGHT_MASK,
            "data": "terminal-frames.bin.zlib",
            "storage": {"codec": "zlib", "bytes": len(terminal_frames),
                        "sha256": hashlib.sha256(terminal_frames).hexdigest()},
            "sourceView": {
                "left": TERMINAL_VIEW_LEFT,
                "right": TERMINAL_VIEW_RIGHT,
                "top": TERMINAL_VIEW_TOP,
                "bottom": TERMINAL_VIEW_BOTTOM,
            },
        },
    }
    (output / "manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )
    source_copy = output / "source.gif"
    if source != source_copy.resolve():
        shutil.copyfile(source, source_copy)
    print(
        f"wrote {len(frame_indices)} frames at {args.cols}x{rows}, "
        f"{args.fps} fps ({len(packed)} web bytes); "
        f"{len(packed_source_frames)} terminal frames at "
        f"{terminal_frame_duration_ms} ms ({len(terminal_frames)} bytes)"
    )


if __name__ == "__main__":
    main()
