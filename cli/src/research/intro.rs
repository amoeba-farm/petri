//! The introduction: a light page where amoebas crawl under a frosted glow,
//! with the wordmark and "click to start" in the middle. It follows the
//! user's reference (a page of ASCII-tile sharks around a title): the same
//! canvas, grid, plankton, tiles and glow, with amoebas for the sharks and no
//! chat bar.
//!
//! `run` shows it on every interactive start while the first reads run in
//! the background, so the main view opens with its data and nothing on the
//! screen says it is loading. A left click, Enter or Space turns the program
//! on; q, Esc and Ctrl-C quit. A frame is a pure function of the seed, the
//! screen's size and the time: `petri tui --render 100x30 --view intro --at 3`
//! prints one.

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::{buffer::Buffer, layout::Rect, style::Color};
use std::{
    f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU},
    time::Duration,
};

/// About 24 frames a second.
pub const FRAME: Duration = Duration::from_micros(41_667);
/// The seed of rendered frames (`--render ... --view intro`) and the tests.
pub const RENDER_SEED: u64 = 0x0a0e_ba5e_ed00_2026;
/// The one line under the wordmark.
pub const HINT: &str = "click to start";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Turn the program on: the normal TUI from here on.
    Start,
    Quit,
}

/// A left click anywhere, Enter or Space starts; q, Esc and Ctrl-C quit, as
/// they do in the TUI. Everything else (pointer moves, scrolls, other keys)
/// leaves the introduction up.
pub fn handle(event: &Event) -> Option<Outcome> {
    match event {
        Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
            Some(Outcome::Start)
        }
        Event::Key(key) if key.kind != KeyEventKind::Release => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                return (key.code == KeyCode::Char('c')).then_some(Outcome::Quit);
            }
            match key.code {
                KeyCode::Enter | KeyCode::Char(' ') => Some(Outcome::Start),
                KeyCode::Char('q') | KeyCode::Esc => Some(Outcome::Quit),
                _ => None,
            }
        }
        _ => None,
    }
}

/// A different arrangement on every start.
pub fn seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(RENDER_SEED, |d| d.as_nanos() as u64)
}

type Rgb = [f32; 3];
/// The page, and the frosted glow behind the title (the reference's colours).
const CANVAS: Rgb = [250.0, 250.0, 248.0];
const GLOW: Rgb = [236.0, 235.0, 231.0];
/// The wordmark, and the line under it.
const INK: Rgb = [48.0, 47.0, 44.0];
const QUIET: Rgb = [146.0, 145.0, 141.0];
/// What floats on the page: bubbles, the specks at a body's edge, plankton.
const BUBBLE: Rgb = [112.0, 114.0, 118.0];
const SPECK: Rgb = [150.0, 150.0, 148.0];
const PLANKTON: Rgb = [178.0, 178.0, 176.0];
/// The few tinted tiles and plankton: teal, sage, rose, lavender.
const TINTS: [Rgb; 4] = [
    [112.0, 168.0, 152.0],
    [144.0, 172.0, 118.0],
    [196.0, 140.0, 154.0],
    [150.0, 140.0, 196.0],
];
/// A warm granule.
const WARM: Rgb = [206.0, 132.0, 66.0];
/// Tile glyphs from the edge to the core.
const RAMP: [char; 10] = ['.', ':', '-', 't', 'S', '%', 'X', '8', '@', '#'];
/// A body's outline is sampled at this many angles each frame.
const SAMPLES: usize = 96;
/// How often a shimmering tile's glyph and shade change (per second);
/// about half the tiles hold still.
const SHIMMER: f32 = 1.2;
/// How unevenly the body flows within a pseudopod cycle (0 is steady): it
/// surges after the pseudopod has reached out, and never quite stops.
const PULSE: f32 = 0.75;
/// The lead pseudopod's head start (in steps): the next one has grown this
/// far by the time the body turns to follow it.
const HEAD_START: f32 = 0.35;
/// How far apart amoebas keep: this many radii each, and a cell more.
const KEEP_CLEAR: f32 = 1.2;
/// The core's share of the radius; pseudopods reach past it.
const CORE: f32 = 0.85;

fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}
/// A fixed pseudo-random number in [0, 1) for these coordinates.
fn unit(seed: u64, a: u64, b: u64, c: u64) -> f32 {
    let h = mix64(
        seed ^ mix64(
            a.wrapping_add(0x9e37_79b9_7f4a_7c15)
                ^ mix64(b.wrapping_mul(0x632b_e59b_d9b4_e019) ^ mix64(c.wrapping_add(17))),
        ),
    );
    (h >> 40) as f32 / (1u64 << 24) as f32
}
fn smoothstep(from: f32, to: f32, x: f32) -> f32 {
    let t = ((x - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
fn lerp(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}
fn tone(level: f32, tint: Rgb) -> Rgb {
    [level * tint[0], level * tint[1], level * tint[2]]
}
fn color(c: Rgb) -> Color {
    let channel = |v: f32| v.round().clamp(0.0, 255.0) as u8;
    Color::Rgb(channel(c[0]), channel(c[1]), channel(c[2]))
}
/// Tile shades come in steps of 12, so neighbouring tiles share colours
/// and a moving body recolours only the cells whose step changes.
fn shade(level: f32) -> f32 {
    (level / 12.0).round() * 12.0
}
/// atan2 to within about 0.004 radians, much cheaper than the library's.
fn angle(y: f32, x: f32) -> f32 {
    let (ax, ay) = (x.abs(), y.abs());
    let (small, big) = if ax >= ay { (ay, ax) } else { (ax, ay) };
    let a = small / big.max(1e-12);
    let mut r = a * (FRAC_PI_4 + 0.273 * (1.0 - a));
    if ay > ax {
        r = FRAC_PI_2 - r;
    }
    if x < 0.0 {
        r = PI - r;
    }
    if y < 0.0 { -r } else { r }
}
/// A table of a closed outline sampled at N angles, read at `theta`.
fn lookup<const N: usize>(table: &[f32; N], theta: f32) -> f32 {
    let f = theta.rem_euclid(TAU) * (N as f32 / TAU);
    let i = (f as usize).min(N - 1);
    let w = f - i as f32;
    table[i] * (1.0 - w) + table[(i + 1) % N] * w
}
/// The angle between two headings, in (-π, π].
fn turn(a: f32, b: f32) -> f32 {
    (a - b + PI).rem_euclid(TAU) - PI
}

/// The wordmark "petri" in thin rounded box lines: an ascender row, the
/// x-height rows and a descender row.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Mark {
    /// Width of p and e; t's, r's.
    bowl: u16,
    stem: u16,
    gap: u16,
    x_height: u16,
}
impl Mark {
    /// The largest that fits about two fifths of the width.
    fn pick(cols: u16, rows: u16) -> Option<Self> {
        let x_height = if rows >= 34 && cols >= 90 { 4 } else { 3 };
        let limit =
            (f32::from(cols) * 0.42).max(f32::from(cols.saturating_sub(6)).min(22.0)) as u16;
        (3..=2 * x_height + 1)
            .rev()
            .map(|bowl| Self {
                bowl,
                stem: (bowl * 3).div_ceil(5),
                gap: if bowl >= 7 { 3 } else { 2 },
                x_height,
            })
            .find(|m| m.width() <= limit && rows >= m.rows() + 4)
    }
    fn widths(&self) -> [u16; 5] {
        [self.bowl, self.bowl, self.stem, self.stem.max(3) - 1, 1]
    }
    fn width(&self) -> u16 {
        self.widths().iter().sum::<u16>() + 4 * self.gap
    }
    fn rows(&self) -> u16 {
        self.x_height + 2
    }
    /// The glyph at a column and row of the wordmark.
    fn glyph(&self, col: u16, row: u16) -> char {
        let mut left = 0;
        for (letter, width) in self.widths().into_iter().enumerate() {
            if col < left + width {
                return self.letter(letter, width, col - left, row);
            }
            left += width + self.gap;
            if col < left {
                return ' ';
            }
        }
        ' '
    }
    fn letter(&self, letter: usize, width: u16, c: u16, r: u16) -> char {
        let (h, last) = (self.x_height, width - 1);
        let rule = |first: char, end: char| match c {
            0 => first,
            c if c == last => end,
            _ => '─',
        };
        match letter {
            // p: a bowl on a stem that runs down to the descender.
            0 => match r {
                1 => rule('╭', '╮'),
                r if r == h => rule('├', '╯'),
                r if r > 1 && r < h && (c == 0 || c == last) => '│',
                r if r == h + 1 && c == 0 => '│',
                _ => ' ',
            },
            // e: a small bowl closed by the bar, open below.
            1 => {
                let bar = (h + 2) / 2;
                match r {
                    1 => rule('╭', '╮'),
                    r if r == bar => rule('├', '╯'),
                    r if r > 1 && r < bar && (c == 0 || c == last) => '│',
                    r if r == h => rule('╰', '─'),
                    r if r > bar && r < h && c == 0 => '│',
                    _ => ' ',
                }
            }
            // t: a stem through a crossbar, turning out at the foot.
            2 => {
                let stem = width / 3;
                match r {
                    0 if c == stem => '│',
                    1 if c == stem => '┼',
                    1 => '─',
                    r if r > 1 && r < h && c == stem => '│',
                    r if r == h && c == stem => '╰',
                    r if r == h && c > stem => '─',
                    _ => ' ',
                }
            }
            // r: a stem with a rounded arm.
            3 => match r {
                1 => rule('╭', '─'),
                r if r > 1 && r <= h && c == 0 => '│',
                _ => ' ',
            },
            // i: a dot over a stem.
            _ => match r {
                0 => '•',
                r if r <= h => '│',
                _ => ' ',
            },
        }
    }
}

/// Where everything sits on a screen of this size.
#[derive(Debug, Clone, Copy)]
struct Layout {
    cols: u16,
    rows: u16,
    mark: Option<Mark>,
    /// The wordmark's top-left cell (or the plain name's), and the hint's.
    mark_at: (u16, u16),
    hint_at: (u16, u16),
    /// The cells the title takes, which nothing covers: x0, y0, x1, y1
    /// (exclusive ends).
    clear: [f32; 4],
    /// The glow's centre and semi-axes, in cells.
    glow: [f32; 4],
}
impl Layout {
    fn new(cols: u16, rows: u16) -> Self {
        let mark = Mark::pick(cols, rows);
        let (width, height) = mark.map_or((5, 1), |m| (m.width(), m.rows()));
        // Centred, a little above the middle; the hint where the reference
        // has its chat bar.
        let top = (f32::from(rows) * 0.42 - f32::from(height) / 2.0)
            .round()
            .max(0.0) as u16;
        let gap = if rows >= 24 { 3 } else { 2 };
        let hint_y = (top + height - 1 + gap).min(rows.saturating_sub(1));
        let hint_width = HINT.chars().count() as u16;
        let mark_at = (cols.saturating_sub(width) / 2, top);
        let hint_at = (cols.saturating_sub(hint_width) / 2, hint_y);
        let clear = [
            f32::from(mark_at.0.min(hint_at.0)),
            f32::from(top),
            f32::from((mark_at.0 + width).max(hint_at.0 + hint_width)),
            f32::from(hint_y + 1),
        ];
        let (w, h) = (f32::from(cols), f32::from(rows));
        let ax = (w * 0.3).max(f32::from(width) / 2.0 + 6.0);
        let ay = (ax / 2.2)
            .max((clear[3] - clear[1]) / 2.0 + 3.0)
            .min(h * 0.46);
        Self {
            cols,
            rows,
            mark,
            mark_at,
            hint_at,
            clear,
            glow: [w / 2.0, h * 0.5, ax, ay],
        }
    }
    /// The glow's elliptical distance: 0 at its centre, 1 at its rim.
    fn ellipse(&self, x: u16, y: u16) -> f32 {
        let dx = (f32::from(x) + 0.5 - self.glow[0]) / self.glow[2];
        let dy = (f32::from(y) + 0.5 - self.glow[1]) / self.glow[3];
        (dx * dx + dy * dy).sqrt()
    }
    /// The page under a cell: off-white, deepening into the frosted glow.
    fn page(&self, x: u16, y: u16) -> Rgb {
        lerp(
            CANVAS,
            GLOW,
            1.0 - smoothstep(0.45, 1.15, self.ellipse(x, y)),
        )
    }
    /// How much of what floats over a cell shows through the glow: faint
    /// near its centre and nothing on or around the title, which sits in a
    /// rounded clearing (a superellipse just around its cells).
    fn shows(&self, x: u16, y: u16) -> f32 {
        let (cx, cy) = (f32::from(x) + 0.5, f32::from(y) + 0.5);
        let [x0, y0, x1, y1] = self.clear;
        let (a, b) = ((x1 - x0) / 2.0 + 3.0, (y1 - y0) / 2.0 + 1.5);
        let dx = (cx - (x0 + x1) / 2.0) / a;
        let dy = (cy - (y0 + y1) / 2.0) / b;
        let round = (dx.powi(4) + dy.powi(4)).sqrt().sqrt();
        let near_title = smoothstep(1.0, 1.7, round);
        let glow = 0.22 + 0.78 * smoothstep(0.3, 1.05, self.ellipse(x, y));
        glow * near_title
    }
    fn in_title(&self, x: u16, y: u16) -> bool {
        let (cx, cy) = (f32::from(x) + 0.5, f32::from(y) + 0.5);
        let [x0, y0, x1, y1] = self.clear;
        cx >= x0 && cx <= x1 && cy >= y0 && cy <= y1
    }
}

/// One amoeba's fixed traits.
#[derive(Debug, Clone)]
struct Body {
    /// The core's diameter in columns: a share of the width, within bounds.
    /// Pseudopods reach past it.
    share: f32,
    min: f32,
    max: f32,
    /// Seconds per pseudopod cycle, and how far into its first cycle it is
    /// at the start.
    period: f32,
    offset: f32,
    /// Outline harmonics k = 2..5: amplitude, phase, drift (radians/s).
    harmonics: [[f32; 3]; 4],
    /// The way cycle n wanders before steering: heading + bend·n +
    /// Σ amplitude·sin(frequency·n + phase), and a little noise.
    heading: f32,
    bend: f32,
    bends: [[f32; 3]; 2],
    /// How far a cycle carries it, in radii.
    reach: f32,
    tint: Rgb,
    /// The nucleus's orbit: phase, speed (radians/s), its radius in radii.
    nucleus: [f32; 3],
    /// Vacuoles and granules: distance (radii), angle, drift (radians/s).
    vacuoles: [[f32; 3]; 3],
}
impl Body {
    fn new(seed: u64, index: usize) -> Self {
        let u = |k: u64| unit(seed, 0xb0d1 + index as u64, k, 0);
        let sign = |k: u64| if u(k) < 0.5 { -1.0 } else { 1.0 };
        // One large, two medium, two small (the reference's mix).
        let (share, min, max, period) = match index {
            0 => (0.24, 14.0, 38.0, 3.6),
            1 => (0.13, 10.0, 17.0, 3.0),
            2 => (0.115, 9.0, 15.0, 2.9),
            3 => (0.0, 9.0, 9.0, 2.4),
            _ => (0.0, 10.0, 10.0, 2.5),
        };
        let amplitudes = [0.09, 0.065, 0.045, 0.03];
        let mut harmonics = [[0.0; 3]; 4];
        for (k, h) in harmonics.iter_mut().enumerate() {
            let k = k as u64;
            *h = [
                amplitudes[k as usize] * (0.7 + 0.6 * u(10 + k)),
                TAU * u(20 + k),
                (0.12 + 0.3 * u(30 + k)) * sign(40 + k),
            ];
        }
        let mut vacuoles = [[0.0; 3]; 3];
        for (k, v) in vacuoles.iter_mut().enumerate() {
            let k = k as u64;
            *v = [
                0.35 + 0.25 * u(50 + k),
                TAU * u(60 + k),
                (0.1 + 0.2 * u(70 + k)) * sign(80 + k),
            ];
        }
        let tint = match (index as u64 + seed) % 3 {
            0 => [0.97, 0.985, 1.035],
            1 => [1.035, 1.0, 0.93],
            _ => [1.0, 1.0, 1.0],
        };
        Self {
            share,
            min,
            max,
            period: period * (0.9 + 0.2 * u(3)),
            offset: 0.45 * u(4),
            harmonics,
            heading: TAU * u(5),
            bend: (u(6) - 0.5) * 0.12,
            bends: [
                [0.7, 0.45 + 0.2 * u(7), TAU * u(8)],
                [0.45, 0.2 + 0.1 * u(9), TAU * u(91)],
            ],
            reach: if index >= 3 { 1.15 } else { 1.0 + 0.15 * u(92) },
            tint,
            nucleus: [
                TAU * u(93),
                (0.1 + 0.15 * u(94)) * sign(95),
                0.26 + 0.05 * u(96),
            ],
            vacuoles,
        }
    }
    fn diameter(&self, cols: f32, rows: f32) -> f32 {
        (self.share * cols)
            .clamp(self.min, self.max)
            .min(rows * 1.2)
    }
    /// The core's radius in square units (a row, or two columns).
    fn radius(&self, cols: f32, rows: f32) -> f32 {
        self.diameter(cols, rows) / 4.0
    }
    fn heading_at(&self, seed: u64, index: usize, n: i64) -> f32 {
        let m = n as f32;
        self.heading
            + self.bend * m
            + self
                .bends
                .iter()
                .map(|[a, f, p]| a * (f * m + p).sin())
                .sum::<f32>()
            + 0.6 * (unit(seed, index as u64, n as u64, 7) - 0.5)
    }
}

/// The amoebas on a screen this wide: one large, then medium and small.
fn cast(cols: u16) -> &'static [usize] {
    match cols {
        90.. => &[0, 1, 2, 3, 4],
        60.. => &[0, 1, 3, 4],
        _ => &[0, 1, 3],
    }
}

/// How far the body has flowed through its cycle (0..1, in steps).
fn flow(u: f32) -> f32 {
    u - PULSE * (TAU * u).sin() / TAU
}
/// How far the lead pseudopod's front has reached from where its cycle
/// started, in steps: its head start, then out over the first half.
fn reach_out(u: f32) -> f32 {
    let grown = if u < 0.55 {
        1.0 - (1.0 - u / 0.55).powi(2)
    } else {
        1.0
    };
    HEAD_START + (1.0 - HEAD_START) * grown
}
/// A smooth union of two distances (k wide), so lobes flare into the body.
fn smin(a: f32, b: f32, k: f32) -> f32 {
    if k <= 0.0 {
        return a.min(b);
    }
    let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
    b + (a - b) * h - k * h * (1.0 - h)
}
fn ahead(at: [f32; 2], step: f32, heading: f32) -> [f32; 2] {
    [at[0] + step * heading.cos(), at[1] + step * heading.sin()]
}
fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
/// How close a step from `a` to `b` passes to `p`.
fn passing(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let along =
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (dx * dx + dy * dy).max(1e-6)).clamp(0.0, 1.0);
    distance([a[0] + dx * along, a[1] + dy * along], p)
}

/// Wanders where it likes in the middle; from `from` of the way out (1 at
/// an edge) it turns back toward the middle, fully by `to`, so bodies go
/// partly off-screen and return (the bigger the body, the sooner it turns).
fn steer(wander: f32, at: [f32; 2], size: [f32; 3], from: f32, to: f32) -> f32 {
    let [width, height, radius] = size;
    let (cx, cy) = (width / 2.0, height / 2.0);
    let (hx, hy) = ((cx - 0.5 * radius).max(1.0), (cy - 0.5 * radius).max(1.0));
    let (ex, ey) = ((at[0] - cx) / hx, (at[1] - cy) / hy);
    let away = smoothstep(from, to, (ex * ex + ey * ey).sqrt());
    let home = (cy - at[1]).atan2(cx - at[0]);
    wander + away * turn(home, wander)
}

/// One amoeba's crawl: its cycle, where the cycle started, its heading and
/// step, the heading before, and the next cycle's once chosen (halfway
/// through this one, when the next pseudopod starts to grow).
#[derive(Debug, Clone, Copy)]
struct Crawl {
    cycle: i64,
    at: [f32; 2],
    heading: f32,
    step: f32,
    last: f32,
    next: Option<(f32, f32)>,
}

/// The amoebas crawl together, so each can keep clear of the others: the
/// events (choosing a heading, starting a cycle) run in time order up to
/// the frame's time. It starts over when time goes back or the screen
/// changes, so a frame doesn't depend on the frames before it.
#[derive(Debug, Clone)]
struct Crowd {
    cols: u16,
    rows: u16,
    time: f32,
    radius: [f32; 5],
    crawls: [Crawl; 5],
}

impl Crowd {
    fn new(seed: u64, bodies: &[Body], cols: u16, rows: u16) -> Self {
        let (w, h) = (f32::from(cols), f32::from(rows));
        let (width, height) = (w / 2.0, h);
        let still = Crawl {
            cycle: 0,
            at: [0.0; 2],
            heading: 0.0,
            step: 0.0,
            last: 0.0,
            next: None,
        };
        let mut crowd = Self {
            cols,
            rows,
            time: 0.0,
            radius: [0.0; 5],
            crawls: [still; 5],
        };
        // Spread round the glow, so none start on top of another.
        let cast = cast(cols);
        let first = TAU * unit(seed, 0xc0, 0, 0);
        for (slot, &i) in cast.iter().enumerate() {
            let jitter = 0.3 * unit(seed, 0xc1, slot as u64, 0);
            let angle = first + TAU * (slot as f32 + jitter) / cast.len() as f32;
            crowd.radius[i] = bodies[i].radius(w, h);
            crowd.crawls[i].at = [
                width / 2.0 * (1.0 + 0.72 * angle.cos()),
                height / 2.0 * (1.0 + 0.68 * angle.sin()),
            ];
        }
        for &i in cast {
            let at = crowd.crawls[i].at;
            let (heading, step) = crowd.choose(seed, bodies, i, 0.0, at, 0);
            crowd.crawls[i] = Crawl {
                heading,
                step,
                last: heading,
                ..crowd.crawls[i]
            };
        }
        crowd
    }

    fn phase(body: &Body, cycle: i64, t: f32) -> f32 {
        (t / body.period + body.offset - cycle as f32).clamp(0.0, 1.0)
    }

    fn centre(&self, bodies: &[Body], i: usize, t: f32) -> [f32; 2] {
        let c = &self.crawls[i];
        let u = Self::phase(&bodies[i], c.cycle, t);
        ahead(c.at, c.step * flow(u), c.heading)
    }

    fn run(&mut self, seed: u64, bodies: &[Body], t: f32) {
        loop {
            // The earliest event due by `t`.
            let mut due: Option<(f32, usize)> = None;
            for &i in cast(self.cols) {
                let (c, b) = (&self.crawls[i], &bodies[i]);
                let at = if c.next.is_none() { 0.5 } else { 1.0 };
                let when = (c.cycle as f32 + at - b.offset) * b.period;
                if when <= t && due.is_none_or(|(w, _)| when < w) {
                    due = Some((when, i));
                }
            }
            let Some((when, i)) = due else { break };
            let c = self.crawls[i];
            self.crawls[i] = match c.next {
                None => {
                    let from = ahead(c.at, c.step, c.heading);
                    let next = self.choose(seed, bodies, i, when, from, c.cycle + 1);
                    Crawl {
                        next: Some(next),
                        ..c
                    }
                }
                Some((heading, step)) => Crawl {
                    cycle: c.cycle + 1,
                    at: ahead(c.at, c.step, c.heading),
                    heading,
                    step,
                    last: c.heading,
                    next: None,
                },
            };
        }
        self.time = t;
    }

    /// The heading and step of a cycle starting `from`, chosen at `t`:
    /// wandering, turned back from the edges, steered round any amoeba its
    /// step would come near (where that one is or is going), and shorter
    /// when it still can't keep clear.
    fn choose(
        &self,
        seed: u64,
        bodies: &[Body],
        i: usize,
        t: f32,
        from: [f32; 2],
        cycle: i64,
    ) -> (f32, f32) {
        let (width, height) = (f32::from(self.cols) / 2.0, f32::from(self.rows));
        let full = bodies[i].reach * self.radius[i];
        let size = [width, height, self.radius[i]];
        let mut heading = steer(bodies[i].heading_at(seed, i, cycle), from, size, 0.55, 1.0);
        let others = cast(self.cols).iter().filter(|&&j| j != i).map(|&j| {
            let o = &self.crawls[j];
            let end = ahead(o.at, o.step, o.heading);
            let then = o.next.map_or(end, |(h, s)| ahead(end, s, h));
            let clear = KEEP_CLEAR * (self.radius[i] + self.radius[j]) + 1.0;
            ([self.centre(bodies, j, t), end, then], clear)
        });
        for (spots, clear) in others.clone() {
            let to = ahead(from, full, heading);
            let Some((near, spot)) = spots
                .iter()
                .map(|&s| (passing(from, to, s), s))
                .min_by(|a, b| a.0.total_cmp(&b.0))
            else {
                continue;
            };
            let push = smoothstep(1.5 * clear, clear, near);
            // Pass it on the side already favoured, rather than turn back.
            let bearing = (spot[1] - from[1]).atan2(spot[0] - from[0]);
            let off = turn(heading, bearing);
            if push > 0.0 && off.abs() < FRAC_PI_2 + 0.4 {
                let around = bearing + off.signum() * (FRAC_PI_2 + 0.4);
                heading += push * turn(around, heading);
            }
        }
        // Never pushed off the screen: past an edge, home comes first.
        heading = steer(heading, from, size, 0.95, 1.25);
        let to = ahead(from, full, heading);
        let room = others
            .map(|(spots, clear)| {
                spots
                    .iter()
                    .map(|&s| passing(ahead(from, 0.3 * full, heading), to, s))
                    .fold(f32::MAX, f32::min)
                    / clear
            })
            .fold(f32::MAX, f32::min);
        (heading, full * smoothstep(0.9, 1.1, room).max(0.25))
    }
}

/// A pseudopod: a tapering lobe from the body's centre out past its edge.
#[derive(Debug, Clone, Copy, Default)]
struct Pod {
    /// How far it reaches past the core (square units).
    beyond: f32,
    /// Its tip's centre, from the body's centre; its half-widths at the
    /// base and the tip; how widely it flares into the body.
    tip: [f32; 2],
    base: f32,
    end: f32,
    flare: f32,
    /// Held back while young, so it comes out of the body smoothly.
    held: f32,
}
impl Pod {
    fn distance(&self, px: f32, py: f32) -> f32 {
        let [tx, ty] = self.tip;
        let along = ((px * tx + py * ty) / (tx * tx + ty * ty).max(1e-6)).clamp(0.0, 1.0);
        let width = self.base + (self.end - self.base) * along;
        let (dx, dy) = (px - tx * along, py - ty * along);
        (dx * dx + dy * dy).sqrt() - width + self.held
    }
}

/// One amoeba at one moment. Lengths are in square units: a row, or two
/// columns (cells are about twice as tall as wide).
#[derive(Debug, Clone)]
struct State {
    centre: [f32; 2],
    radius: f32,
    /// The core's outline at SAMPLES angles.
    outline: [f32; SAMPLES],
    heading: f32,
    /// The place in the pseudopod cycle (0..1).
    phase: f32,
    /// The lead pseudopod, the next one, and two to the side.
    pods: [Pod; 4],
    nucleus: [f32; 2],
    /// The nucleus's wobbling outline at 32 angles.
    nucleus_outline: [f32; 32],
}
impl State {
    fn outline_at(&self, theta: f32) -> f32 {
        lookup(&self.outline, theta)
    }
    /// Signed distance to the edge from a point (relative to the centre):
    /// negative inside the core or a pseudopod.
    fn edge(&self, px: f32, py: f32) -> f32 {
        // ρ/r scaled to the radius: the distance near the edge, and smooth
        // through the centre, where the angle means nothing.
        let rho = (px * px + py * py).sqrt();
        let mut d = (rho / self.outline_at(angle(py, px)) - 1.0) * CORE * self.radius;
        for pod in &self.pods {
            if pod.flare > 0.0 {
                d = smin(d, pod.distance(px, py), pod.flare);
            }
        }
        d
    }
    /// The furthest the body reaches from its centre.
    fn extent(&self) -> f32 {
        let core = self.outline.iter().fold(0.0f32, |a, &b| a.max(b));
        self.pods.iter().fold(core, |a, p| {
            a.max(p.tip[0].hypot(p.tip[1]) + p.end + p.flare)
        })
    }
    fn pod(&self, heading: f32, beyond: f32) -> Pod {
        if beyond <= 0.0 {
            return Pod::default();
        }
        let r = self.radius;
        let grown = smoothstep(0.0, 0.3 * r, beyond);
        let (base, end) = ((0.17 * r + 0.15) * grown, (0.1 * r).max(0.45) * grown);
        let length = self.outline_at(heading) + beyond - end;
        Pod {
            beyond,
            tip: [length * heading.cos(), length * heading.sin()],
            base,
            end,
            flare: (0.12 * r + 0.15) * grown,
            held: 0.2 * r * (1.0 - grown),
        }
    }
}

pub struct Intro {
    seed: u64,
    bodies: Vec<Body>,
    crowd: Option<Crowd>,
    /// What doesn't move, for the current screen size.
    still: Option<Still>,
    /// What each cell holds this frame: 0 page, 1 a mark, 2 a tile.
    cover: Vec<u8>,
}

/// A cell's fixed random numbers.
#[derive(Debug, Clone, Copy)]
struct Grain {
    /// Its shimmer phase, or none (negative): it holds still at `calm`.
    shimmer: f32,
    calm: f32,
    speck: f32,
    tint: f32,
    colour: u8,
}

/// What doesn't move, for one screen size: the page with its grid, each
/// cell's page colour, how much shows through the glow there, and its
/// grain. Every frame starts from this copy, so cells nothing moved over
/// are the same as last frame and the terminal is sent nothing for them.
struct Still {
    area: Rect,
    lay: Layout,
    canvas: Buffer,
    page: Vec<Rgb>,
    shows: Vec<f32>,
    grain: Vec<Grain>,
}
impl Still {
    fn new(seed: u64, area: Rect) -> Self {
        let lay = Layout::new(area.width, area.height);
        let mut canvas = Buffer::empty(area);
        let cells = usize::from(area.width) * usize::from(area.height);
        let mut page = Vec::with_capacity(cells);
        let mut shows = Vec::with_capacity(cells);
        let mut grain = Vec::with_capacity(cells);
        for y in 0..area.height {
            for x in 0..area.width {
                let colour = lay.page(x, y);
                // The faint square grid.
                let grid = x % 4 == 1 && y % 2 == 1 && !lay.in_title(x, y);
                canvas[(area.x + x, area.y + y)]
                    .set_char(if grid { '·' } else { ' ' })
                    .set_fg(color(lerp(colour, [150.0, 150.0, 148.0], 0.15)))
                    .set_bg(color(colour));
                page.push(colour);
                shows.push(lay.shows(x, y));
                let u = |k: u64| unit(seed, u64::from(x), u64::from(y), k);
                grain.push(Grain {
                    shimmer: if u(1) < 0.5 { 7.0 * u(2) } else { -1.0 },
                    calm: u(3),
                    speck: 5.0 * u(4),
                    tint: 4.0 * u(5),
                    colour: (u(6) * TINTS.len() as f32) as u8 % TINTS.len() as u8,
                });
            }
        }
        Self {
            area,
            lay,
            canvas,
            page,
            shows,
            grain,
        }
    }
}

impl Intro {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            bodies: (0..5).map(|i| Body::new(seed, i)).collect(),
            crowd: None,
            still: None,
            cover: Vec::new(),
        }
    }

    /// The crowd at `t` on a screen this size.
    fn advance(&mut self, cols: u16, rows: u16, t: f32) {
        let stale = self
            .crowd
            .as_ref()
            .is_none_or(|c| c.cols != cols || c.rows != rows || t < c.time);
        if stale {
            self.crowd = Some(Crowd::new(self.seed, &self.bodies, cols, rows));
        }
        if let Some(crowd) = self.crowd.as_mut() {
            crowd.run(self.seed, &self.bodies, t);
        }
    }

    /// An amoeba crawls in cycles: a pseudopod reaches out ahead, the body
    /// flows into it while it is absorbed, and the next one, already
    /// growing another way, takes over. Its front holds still while the
    /// rest catches up. A smaller pseudopod to one side swells and is
    /// withdrawn around each turn.
    fn state(&mut self, index: usize, cols: u16, rows: u16, t: f32) -> State {
        self.advance(cols, rows, t);
        let seed = self.seed;
        let body = &self.bodies[index];
        let Some(crowd) = self.crowd.as_ref() else {
            unreachable!("advance sets the crowd")
        };
        let c = crowd.crawls[index];
        let radius = crowd.radius[index];
        let phase = Crowd::phase(body, c.cycle, t);
        let centre = ahead(c.at, c.step * flow(phase), c.heading);
        // The body turns its long axis to the new way early in the cycle.
        let axis = c.last + turn(c.heading, c.last) * smoothstep(0.0, 0.4, phase);
        let mut outline = [0.0; SAMPLES];
        for (j, r) in outline.iter_mut().enumerate() {
            let theta = j as f32 * TAU / SAMPLES as f32;
            let wobble = body
                .harmonics
                .iter()
                .enumerate()
                .map(|(k, [a, p, w])| a * ((k as f32 + 2.0) * theta + p + w * t).sin())
                .sum::<f32>();
            let stretch = 0.08 * (2.0 * turn(theta, axis)).cos();
            *r = radius * (CORE + wobble + stretch);
        }
        let [orbit, speed, size] = body.nucleus;
        let orbit = orbit + speed * t;
        let nucleus_radius = (size * radius).max(0.8);
        let mut nucleus_outline = [0.0; 32];
        for (j, r) in nucleus_outline.iter_mut().enumerate() {
            let a = j as f32 * TAU / 32.0;
            *r = nucleus_radius
                * (1.0 + 0.12 * (2.0 * a + 0.7 * t).sin() + 0.08 * (3.0 * a - 0.5 * t).sin());
        }
        let mut state = State {
            centre,
            radius,
            outline,
            heading: c.heading,
            phase,
            pods: [Pod::default(); 4],
            nucleus: [
                centre[0] + 0.3 * radius * orbit.cos(),
                centre[1] + 0.3 * radius * orbit.sin(),
            ],
            nucleus_outline,
        };
        // The lead: out ahead, then absorbed as the body flows in. Held to
        // a short step, it still reaches out mid-cycle and draws back.
        let full = body.reach * radius;
        let lead = c.step * (reach_out(phase) - flow(phase)).max(0.0)
            + (full - c.step).max(0.0) * 0.6 * (PI * phase).sin().powi(2);
        // The next: chosen halfway, it grows to the lead's head start.
        let (next_heading, next) = match c.next {
            Some((heading, step)) => (heading, step * HEAD_START * smoothstep(0.5, 1.0, phase)),
            None => (c.heading, 0.0),
        };
        // To the sides: one swells around each turn, another way each time,
        // and one mid-cycle, off the lead's flank.
        let side = t / body.period + body.offset + 0.5;
        let turn_n = side.floor() as i64 as u64;
        let swell = (PI * side.fract()).sin().powi(2);
        let side_heading = TAU * unit(seed, index as u64, turn_n, 14);
        let side_reach = radius * (0.4 + 0.35 * unit(seed, index as u64, turn_n, 13)) * swell;
        let (flank_heading, flank) = if (0.05..0.75).contains(&phase) {
            let roll = unit(seed, index as u64, c.cycle as u64, 15);
            let flank = if roll < 0.5 { -1.0 } else { 1.0 };
            let off = flank * (0.9 + 1.3 * (roll * 2.0).fract());
            let swell = (PI * (phase - 0.05) / 0.7).sin().powi(2);
            let length = 0.35 + 0.25 * unit(seed, index as u64, c.cycle as u64, 16);
            (c.heading + off, radius * length * swell)
        } else {
            (c.heading, 0.0)
        };
        // A pseudopod draws back from another amoeba in its way.
        let others = cast(cols)
            .iter()
            .filter(|&&j| j != index)
            .map(|&j| (crowd.centre(&self.bodies, j, t), crowd.radius[j]));
        let room = |heading: f32, beyond: f32| {
            let tip = ahead(centre, state.outline_at(heading) + beyond, heading);
            let keep = others
                .clone()
                .map(|(c, r)| smoothstep(1.2 * r + 0.5, 1.2 * r + 3.0, distance(tip, c)))
                .fold(1.0, f32::min);
            beyond * keep
        };
        let pods = [
            state.pod(c.heading, room(c.heading, lead)),
            state.pod(next_heading, room(next_heading, next)),
            state.pod(side_heading, room(side_heading, side_reach)),
            state.pod(flank_heading, room(flank_heading, flank)),
        ];
        state.pods = pods;
        state
    }

    /// Draws the frame for time `t` (seconds since the introduction opened).
    pub fn draw(&mut self, buf: &mut Buffer, area: Rect, t: f32) {
        let (cols, rows) = (area.width, area.height);
        if cols == 0 || rows == 0 {
            return;
        }
        if self.still.as_ref().is_none_or(|s| s.area != area) {
            self.still = Some(Still::new(self.seed, area));
        }
        let Some(still) = self.still.take() else {
            return;
        };
        if buf.area == area {
            buf.content.clone_from_slice(&still.canvas.content);
        } else {
            for y in area.top()..area.bottom() {
                for x in area.left()..area.right() {
                    buf[(x, y)] = still.canvas[(x, y)].clone();
                }
            }
        }
        let lay = still.lay;
        let mut cover = std::mem::take(&mut self.cover);
        cover.clear();
        cover.resize(usize::from(cols) * usize::from(rows), 0);
        let mut canvas = Canvas {
            buf,
            area,
            lay,
            still: &still,
            cover: &mut cover,
        };
        plankton(&mut canvas, self.seed, t);
        for &index in cast(cols) {
            let state = self.state(index, cols, rows, t);
            amoeba(
                &mut canvas,
                self.seed,
                index,
                &self.bodies[index],
                &state,
                t,
            );
        }
        // The title on the glow.
        let Canvas { buf, .. } = canvas;
        let mut put = |x: u16, y: u16, ch: char, ink: Rgb| {
            if x < cols && y < rows {
                let page = still.page[usize::from(y) * usize::from(cols) + usize::from(x)];
                buf[(area.x + x, area.y + y)]
                    .set_char(ch)
                    .set_fg(color(ink))
                    .set_bg(color(page));
            }
        };
        match lay.mark {
            Some(mark) => {
                for row in 0..mark.rows() {
                    for col in 0..mark.width() {
                        let ch = mark.glyph(col, row);
                        if ch != ' ' {
                            put(lay.mark_at.0 + col, lay.mark_at.1 + row, ch, INK);
                        }
                    }
                }
            }
            None => {
                for (i, ch) in "petri".chars().enumerate() {
                    put(lay.mark_at.0 + i as u16, lay.mark_at.1, ch, INK);
                }
            }
        }
        for (i, ch) in HINT.chars().enumerate() {
            put(lay.hint_at.0 + i as u16, lay.hint_at.1, ch, QUIET);
        }
        self.cover = cover;
        self.still = Some(still);
    }
}

/// Writes cells through the glow: what floats over the page fades toward
/// it near the centre and vanishes around the title.
struct Canvas<'a> {
    buf: &'a mut Buffer,
    area: Rect,
    lay: Layout,
    still: &'a Still,
    cover: &'a mut Vec<u8>,
}
impl Canvas<'_> {
    fn set(&mut self, x: u16, y: u16, ch: char, fg: Rgb, bg: Rgb) {
        self.buf[(self.area.x + x, self.area.y + y)]
            .set_char(ch)
            .set_fg(color(fg))
            .set_bg(color(bg));
    }
    fn index(&self, x: i32, y: i32) -> Option<usize> {
        let (cols, rows) = (i32::from(self.lay.cols), i32::from(self.lay.rows));
        (x >= 0 && y >= 0 && x < cols && y < rows).then(|| (y * cols + x) as usize)
    }
    /// A mark on the page (plankton, a speck, a bubble), where no tile is.
    fn mark(&mut self, x: i32, y: i32, ch: char, ink: Rgb) {
        let Some(i) = self.index(x, y) else {
            return;
        };
        let shows = self.still.shows[i];
        if self.cover[i] >= 2 || shows < 0.3 {
            return;
        }
        let page = self.still.page[i];
        self.set(x as u16, y as u16, ch, lerp(page, ink, shows), page);
        self.cover[i] = 1;
    }
    /// A body tile; under the glow its glyph goes first, then the tile.
    fn tile(&mut self, x: i32, y: i32, ch: char, ink: Rgb, fill: Rgb) {
        let Some(i) = self.index(x, y) else {
            return;
        };
        let (shows, page) = (self.still.shows[i], self.still.page[i]);
        let bg = lerp(page, fill, shows);
        let glyph = smoothstep(0.45, 0.8, shows);
        let ch = if glyph > 0.0 { ch } else { ' ' };
        self.set(x as u16, y as u16, ch, lerp(bg, ink, glyph), bg);
        self.cover[i] = 2;
    }
}

/// Dashes and dots drifting over the page: single ones everywhere, and
/// loose patches near the sides, a few of them tinted.
fn plankton(canvas: &mut Canvas, seed: u64, t: f32) {
    let (w, h) = (f32::from(canvas.lay.cols), f32::from(canvas.lay.rows));
    let singles = ((w * h) / 170.0).clamp(6.0, 160.0) as u64;
    for i in 0..singles {
        let u = |k: u64| unit(seed, 0x9a00 + i, k, 1);
        let speed = (0.25 + 0.9 * u(2)) * if u(3) < 0.5 { -1.0 } else { 1.0 };
        let sway = 1.5 * (t * 0.3 * (0.5 + u(5)) + TAU * u(6)).sin();
        let x = (u(0) * w + speed * t + sway).rem_euclid(w);
        let y = (u(1) * h + (u(4) - 0.5) * 0.35 * t + 0.4 * (t * 0.21 + TAU * u(7)).sin())
            .rem_euclid(h);
        let ch = match u(8) {
            v if v < 0.6 => '-',
            v if v < 0.85 => '.',
            _ => '·',
        };
        let ink = lerp(PLANKTON, CANVAS, u(9) * 0.3);
        canvas.mark(x as i32, y as i32, ch, ink);
    }
    let patches = (4.0 + (w * h) / 1200.0).min(14.0) as u64;
    for p in 0..patches {
        let u = |k: u64| unit(seed, 0x9b00 + p, k, 2);
        let side = u(0) * 0.15;
        let (rx, ry) = (4.0 + 6.0 * u(1), 2.0 + 2.5 * u(2));
        let cx =
            if p % 2 == 0 { side } else { 1.0 - side } * w + 2.0 * (t * 0.2 + TAU * u(3)).sin();
        let fall = (0.15 + 0.3 * u(4)) * if u(5) < 0.5 { -1.0 } else { 1.0 };
        let cy = (u(6) * h + fall * t).rem_euclid(h + 2.0 * ry) - ry;
        let (cx, cy) = (cx.round() as i32, cy.round() as i32);
        let (rxi, ryi) = (rx as i32, ry as i32);
        for gy in -ryi..=ryi {
            for gx in (-rxi..=rxi).step_by(2) {
                let d = (gx as f32 / rx).powi(2) + (gy as f32 / ry).powi(2);
                let id = (p << 32) ^ ((gx + 64) as u64) << 8 ^ (gy + 64) as u64;
                if d > 1.0 || unit(seed, id, 0, 3) > 0.7 * (1.0 - 0.6 * d) {
                    continue;
                }
                // Members come and go slowly.
                let blink = (t * 0.4 + 3.0 * unit(seed, id, 1, 3)).floor() as u64;
                if unit(seed, id, blink, 4) > 0.8 {
                    continue;
                }
                let roll = unit(seed, id, 2, 3);
                let ch = match roll {
                    v if v < 0.55 => '-',
                    v if v < 0.8 => '·',
                    _ => '.',
                };
                let tint = unit(seed, id, 3, 3);
                let ink = if tint < 0.18 {
                    TINTS[(tint * 22.0) as usize % TINTS.len()]
                } else {
                    lerp(PLANKTON, CANVAS, 0.3 * tint)
                };
                canvas.mark(cx + gx, cy + gy, ch, ink);
            }
        }
    }
}

/// One amoeba as tiles: lighter at the membrane, darker toward the core, a
/// darker nucleus, a few vacuoles and granules, specks at the edge and
/// bubbles shed by a reaching pseudopod.
fn amoeba(canvas: &mut Canvas, seed: u64, index: usize, body: &Body, state: &State, t: f32) {
    // Pseudopods are drawn like the rest of the body: the tiles follow the
    // distance to the edge.
    let (cols, rows) = (i32::from(canvas.lay.cols), i32::from(canvas.lay.rows));
    let extent = state.extent() + 1.0;
    let [cx, cy] = state.centre;
    let x0 = (((cx - extent) * 2.0).floor() as i32).max(0);
    let x1 = (((cx + extent) * 2.0).ceil() as i32).min(cols - 1);
    let y0 = ((cy - extent).floor() as i32).max(0);
    let y1 = ((cy + extent).ceil() as i32).min(rows - 1);
    let vacuoles = match index {
        0 => 3,
        1 | 2 => 2,
        _ => 1,
    };
    let vacuole_at = |k: usize| {
        let [distance, angle, drift] = body.vacuoles[k];
        let a = angle + drift * t;
        let d = distance * state.radius;
        [cx + d * a.cos(), cy + d * a.sin()]
    };
    let mut spots = [[f32::MAX; 2]; 3];
    for (k, spot) in spots.iter_mut().enumerate().take(vacuoles) {
        *spot = vacuole_at(k);
    }
    for y in y0..=y1 {
        for x in x0..=x1 {
            let px = (x as f32 + 0.5) / 2.0 - cx;
            let py = y as f32 + 0.5 - cy;
            let edge = state.edge(px, py);
            let (xu, yu) = (x as u64, y as u64);
            let grain = canvas.still.grain[(y * cols + x) as usize];
            if edge >= 0.0 {
                // Specks just outside the edge.
                if edge < 0.9 {
                    let roll = unit(seed, xu, yu, (t * 0.8 + grain.speck) as u64);
                    if roll < 0.22 {
                        let ch = ['\'', '.', ',', '·'][(roll * 18.0) as usize % 4];
                        canvas.mark(x, y, ch, SPECK);
                    }
                }
                continue;
            }
            let depth = (-edge / (CORE * state.radius)).min(1.0);
            // Glyphs and shades shimmer about once a second, on half the
            // tiles; shades come in steps (`shade`).
            let shimmer = if grain.shimmer >= 0.0 {
                unit(seed, xu, yu, (t * SHIMMER + grain.shimmer) as u64)
            } else {
                grain.calm
            };
            let membrane = edge > -0.45;
            let (mut ch, mut fill, mut ink);
            if membrane {
                let level = shade(214.0 + (shimmer - 0.5) * 12.0);
                fill = tone(level, body.tint);
                ink = tone(level - 74.0, body.tint);
                ch = if shimmer < 0.5 { ':' } else { '.' };
            } else {
                let level =
                    shade(198.0 - 112.0 * smoothstep(-0.2, 0.7, depth) + (shimmer - 0.5) * 26.0);
                fill = tone(level, body.tint);
                ink = if level > 178.0 {
                    tone(level - 84.0, body.tint)
                } else {
                    tone((level + 108.0).min(242.0), body.tint)
                };
                let i = ((depth * 1.15 + (shimmer - 0.5) * 0.3).clamp(0.0, 0.999) * 10.0) as usize;
                ch = RAMP[i];
            }
            // The nucleus, wobbling as it drifts.
            let (nx, ny) = (px + cx - state.nucleus[0], py + cy - state.nucleus[1]);
            let dn = (nx * nx + ny * ny).sqrt() / lookup(&state.nucleus_outline, angle(ny, nx));
            let nucleus = dn < 1.0 && !membrane;
            if nucleus {
                let level = shade(112.0 - 46.0 * (1.0 - dn) + (shimmer - 0.5) * 14.0);
                fill = tone(level, [0.97, 0.9, 1.1]);
                ink = [222.0, 214.0, 236.0];
                ch = if dn < 0.45 {
                    '@'
                } else if shimmer < 0.5 {
                    '8'
                } else {
                    '#'
                };
            }
            // Vacuoles (light, 'o') and one warm granule.
            for (k, spot) in spots.iter().enumerate().take(vacuoles) {
                let (dx, dy) = (px + cx - spot[0], py + cy - spot[1]);
                if !membrane && !nucleus && dx * dx + dy * dy < 0.36 {
                    if k == 2 {
                        ink = WARM;
                        ch = 'o';
                    } else {
                        fill = lerp(fill, [234.0, 236.0, 234.0], 0.7);
                        ink = [104.0, 106.0, 110.0];
                        ch = 'o';
                    }
                }
            }
            // A few tinted tiles and glyphs, changing slowly.
            let tinted = unit(seed, xu, yu, (t * 0.35 + grain.tint) as u64 + 77);
            if !membrane && !nucleus {
                let colour = TINTS[usize::from(grain.colour)];
                if tinted < 0.05 {
                    fill = lerp(fill, colour, 0.5);
                } else if tinted < 0.08 {
                    ink = colour;
                }
            }
            canvas.tile(x, y, ch, ink, fill);
        }
    }
    // Bubbles shed beside the reaching pseudopod, drifting off.
    let lead = &state.pods[0];
    if (0.05..0.9).contains(&state.phase) && lead.beyond > 0.0 {
        let (hx, hy) = (state.heading.cos(), state.heading.sin());
        let tip = lead.tip[0].hypot(lead.tip[1]) + lead.end;
        for b in 0..if index == 0 { 2 } else { 1 } {
            let begin = 0.05 + 0.14 * b as f32;
            if state.phase < begin {
                continue;
            }
            let q = (state.phase - begin) / (0.9 - begin);
            let side = if b == 0 { 1.0 } else { -1.0 };
            let along = tip - 0.3 * state.radius + 0.8 * q;
            let across = side * (0.35 * state.radius + 0.6 + 1.2 * q);
            let bx = cx + along * hx - across * hy;
            let by = cy + along * hy + across * hx;
            let ch = if q < 0.75 { 'o' } else { '.' };
            canvas.mark((bx * 2.0).floor() as i32, by.floor() as i32, ch, BUBBLE);
        }
    }
}

/// The frame at `t` seconds, as the terminal would show it.
pub fn frame(seed: u64, width: u16, height: u16, t: f32) -> Buffer {
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    Intro::new(seed).draw(&mut buf, area, t);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, MouseEvent};

    fn text(buf: &Buffer) -> Vec<String> {
        let area = buf.area;
        (0..area.height)
            .map(|y| (0..area.width).map(|x| buf[(x, y)].symbol()).collect())
            .collect()
    }
    fn tile_cells(buf: &Buffer) -> Vec<(u16, u16)> {
        let lay = Layout::new(buf.area.width, buf.area.height);
        let mut cells = Vec::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                if buf[(x, y)].bg != color(lay.page(x, y)) {
                    cells.push((x, y));
                }
            }
        }
        cells
    }

    #[test]
    fn title_is_centred_and_clear_of_everything() {
        for (width, height) in [(100, 30), (80, 24), (160, 48), (45, 16)] {
            let lay = Layout::new(width, height);
            let mark = lay.mark.expect("A wordmark fits");
            let left = lay.mark_at.0;
            let right = width - left - mark.width();
            assert!(left.abs_diff(right) <= 1, "{width}x{height}");
            // A little above the middle.
            let middle = f32::from(lay.mark_at.1) + f32::from(mark.rows()) / 2.0;
            assert!(middle < f32::from(height) / 2.0, "{width}x{height}");
            for step in 0..120 {
                let t = step as f32 * 0.5;
                let buf = frame(RENDER_SEED, width, height, t);
                for row in 0..mark.rows() {
                    for col in 0..mark.width() {
                        let (x, y) = (left + col, lay.mark_at.1 + row);
                        let cell = &buf[(x, y)];
                        let want = mark.glyph(col, row);
                        if want != ' ' {
                            assert_eq!(cell.symbol(), want.to_string());
                            assert_eq!(cell.fg, color(INK));
                        }
                        // The glow keeps the title's cells clear: no tile,
                        // no plankton.
                        assert_eq!(cell.bg, color(lay.page(x, y)), "{width}x{height} at {t}");
                        if want == ' ' {
                            assert!(
                                matches!(cell.symbol(), " "),
                                "{width}x{height} at {t}: {:?} in the title",
                                cell.symbol()
                            );
                        }
                    }
                }
                let hint = text(&buf)[usize::from(lay.hint_at.1)].clone();
                let at = usize::from(lay.hint_at.0);
                let around: String = hint.chars().skip(at - 1).take(HINT.len() + 2).collect();
                assert_eq!(around, format!(" {HINT} "), "{width}x{height} at {t}");
            }
        }
        // No other words: tiles only hold single glyphs from the ramp ('t',
        // 'o' and capitals among them).
        for t in [0.0, 3.0, 11.0] {
            let screen = text(&frame(RENDER_SEED, 100, 30, t)).join("\n");
            let words = screen
                .split(|c: char| !c.is_ascii_lowercase())
                .filter(|w| w.len() > 1 && !w.chars().all(|c| c == 't' || c == 'o'))
                .collect::<Vec<_>>();
            assert_eq!(words, ["click", "start"], "{screen}");
        }
    }

    #[test]
    fn amoebas_are_tiles_that_move() {
        let a = frame(RENDER_SEED, 100, 30, 2.0);
        let b = frame(RENDER_SEED, 100, 30, 5.0);
        let (ta, tb) = (tile_cells(&a), tile_cells(&b));
        assert!(ta.len() > 150, "{} tiles", ta.len());
        assert!(tb.len() > 150, "{} tiles", tb.len());
        assert_ne!(ta, tb);
        // Glyphs from the ramp inside the tiles.
        let glyphs = ta
            .iter()
            .filter(|&&(x, y)| {
                let s = a[(x, y)].symbol();
                RAMP.iter().any(|c| s == c.to_string()) || s == "o"
            })
            .count();
        assert!(glyphs > 80, "{glyphs} glyphs");
        // The same seed and time draw the same frame; another seed doesn't.
        assert_eq!(a, frame(RENDER_SEED, 100, 30, 2.0));
        assert_ne!(a, frame(RENDER_SEED + 1, 100, 30, 2.0));
    }

    #[test]
    fn a_frame_does_not_depend_on_the_frames_before() {
        let area = Rect::new(0, 0, 100, 30);
        let mut intro = Intro::new(RENDER_SEED);
        let mut buf = Buffer::empty(area);
        for step in 0..200 {
            intro.draw(&mut buf, area, step as f32 / 24.0);
        }
        intro.draw(&mut buf, area, 300.0);
        assert_eq!(buf, frame(RENDER_SEED, 100, 30, 300.0));
        intro.draw(&mut buf, area, 1.0);
        assert_eq!(buf, frame(RENDER_SEED, 100, 30, 1.0));
    }

    /// Where a body's mass is: the mean of its points, 0.2 units apart.
    fn centroid(state: &State) -> [f32; 2] {
        let reach = state.extent();
        let n = (reach / 0.2).ceil() as i32;
        let (mut x, mut y, mut count) = (0.0, 0.0, 0.0);
        for i in -n..=n {
            for j in -n..=n {
                let (px, py) = (i as f32 * 0.2, j as f32 * 0.2);
                if state.edge(px, py) < 0.0 {
                    x += px;
                    y += py;
                    count += 1.0;
                }
            }
        }
        [state.centre[0] + x / count, state.centre[1] + y / count]
    }

    /// Columns from a body's leftmost cell to its rightmost.
    fn span(state: &State) -> f32 {
        let reach = state.extent();
        let (mut left, mut right) = (f32::MAX, f32::MIN);
        let n = (reach / 0.25).ceil() as i32;
        for i in -n..=n {
            for j in -n..=n {
                let (px, py) = (i as f32 * 0.25, j as f32 * 0.25);
                if state.edge(px, py) < 0.0 {
                    left = left.min(px);
                    right = right.max(px);
                }
            }
        }
        (right - left) * 2.0
    }

    #[test]
    fn a_pseudopod_cycle_moves_the_centroid() {
        let mut intro = Intro::new(RENDER_SEED);
        // Roomy, so most steps are full.
        for &index in cast(160) {
            let body = intro.bodies[index].clone();
            let mut checked = 0;
            for cycle in 1..24 {
                let start = (cycle as f32 - body.offset + 0.002) * body.period;
                let a = intro.state(index, 160, 45, start);
                let crawl = intro.crowd.as_ref().unwrap().crawls[index];
                assert_eq!(crawl.cycle, cycle);
                // A full step: not held short by another amoeba.
                if crawl.step < 0.99 * body.reach * a.radius {
                    continue;
                }
                checked += 1;
                let mid = intro.state(index, 160, 45, start + 0.33 * body.period);
                let b = intro.state(index, 160, 45, start + 0.996 * body.period);
                let ahead = [a.heading.cos(), a.heading.sin()];
                let (ca, cm, cb) = (centroid(&a), centroid(&mid), centroid(&b));
                let along = |p: [f32; 2]| (p[0] - ca[0]) * ahead[0] + (p[1] - ca[1]) * ahead[1];
                // The pseudopod reaches out ahead first, 40-80% of the
                // radius past the body...
                let lead = mid.pods[0].beyond / mid.radius;
                assert!(mid.pods[0].beyond > a.pods[0].beyond, "{index}/{cycle}");
                assert!(along(cm) > 0.0, "{index}/{cycle}");
                assert!((0.4..=0.8).contains(&lead), "{index}/{cycle}: {lead}");
                // ...and the body flows after it, a step a cycle.
                assert!(
                    along(cb) > 0.7 * crawl.step,
                    "{index}/{cycle}: {} of {}",
                    along(cb),
                    crawl.step
                );
            }
            assert!(checked >= 2, "{index}: {checked}");
        }
    }

    #[test]
    fn one_to_three_pseudopods_show_at_a_time() {
        let mut intro = Intro::new(RENDER_SEED);
        let (mut bare, mut samples) = (0, 0);
        for step in 0..(90 * 10) {
            let t = step as f32 / 10.0;
            for &index in cast(100) {
                let state = intro.state(index, 100, 30, t);
                let shown = state
                    .pods
                    .iter()
                    .filter(|p| p.beyond > 0.15 * state.radius)
                    .count();
                assert!(shown <= 3);
                samples += 1;
                if shown == 0 {
                    bare += 1;
                }
            }
        }
        // Only an amoeba hemmed in by others (short steps) is ever without
        // one.
        assert!(bare * 50 < samples, "{bare} of {samples}");
    }

    #[test]
    fn motion_is_continuous() {
        // From one frame to the next (1/24 s) nothing jumps: not the
        // centre, not the outline or a pseudopod, not at a cycle's turn.
        let mut intro = Intro::new(RENDER_SEED);
        let mut before: Vec<State> = cast(100)
            .iter()
            .map(|&i| intro.state(i, 100, 30, 0.0))
            .collect();
        for step in 1..(60 * 24) {
            let t = step as f32 / 24.0;
            for (k, &index) in cast(100).iter().enumerate() {
                let now = intro.state(index, 100, 30, t);
                let was = &before[k];
                let moved = distance(now.centre, was.centre);
                assert!(moved < 0.5, "{index} at {t}: moved {moved}");
                let reach = now.extent().max(was.extent());
                let n = (reach / 0.5).ceil() as i32;
                for i in -n..=n {
                    for j in -n..=n {
                        let (x, y) = (
                            now.centre[0] + i as f32 * 0.5,
                            now.centre[1] + j as f32 * 0.5,
                        );
                        let a = now.edge(x - now.centre[0], y - now.centre[1]);
                        let b = was.edge(x - was.centre[0], y - was.centre[1]);
                        if a.min(b) < 1.0 {
                            assert!((a - b).abs() < 0.6, "{index} at {t}: {a} vs {b}");
                        }
                    }
                }
                before[k] = now;
            }
        }
    }

    #[test]
    fn amoebas_keep_apart() {
        for seed in [RENDER_SEED, 7, 42] {
            for (cols, rows) in [(100, 30), (160, 45), (72, 24)] {
                let mut intro = Intro::new(seed);
                for step in 0..(120 * 4) {
                    let t = step as f32 / 4.0;
                    let states = cast(cols)
                        .iter()
                        .map(|&i| intro.state(i, cols, rows, t))
                        .collect::<Vec<_>>();
                    for (k, a) in states.iter().enumerate() {
                        for b in &states[k + 1..] {
                            if distance(a.centre, b.centre) > a.extent() + b.extent() {
                                continue;
                            }
                            // No cell inside both.
                            let reach = a.extent();
                            let [cx, cy] = a.centre;
                            let (x0, x1) = (
                                ((cx - reach) * 2.0) as i32 - 1,
                                ((cx + reach) * 2.0) as i32 + 1,
                            );
                            let (y0, y1) = ((cy - reach) as i32 - 1, (cy + reach) as i32 + 1);
                            for y in y0..=y1 {
                                for x in x0..=x1 {
                                    let (px, py) = ((x as f32 + 0.5) / 2.0, y as f32 + 0.5);
                                    let inside = |s: &State| {
                                        s.edge(px - s.centre[0], py - s.centre[1]) < 0.0
                                    };
                                    assert!(
                                        !(inside(a) && inside(b)),
                                        "seed {seed} {cols}x{rows} at {t}: cell {x},{y}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn sizes_follow_the_reference() {
        // The largest spans about a quarter of the width, the small ones
        // 8-10 columns; two medium between.
        for (cols, rows) in [(100, 30), (160, 45)] {
            let mut intro = Intro::new(RENDER_SEED);
            let mut spans = [0.0f32; 5];
            let samples = 60;
            for step in 0..samples {
                for &index in cast(cols) {
                    let state = intro.state(index, cols, rows, step as f32 * 0.5);
                    spans[index] += span(&state) / samples as f32;
                }
            }
            let width = f32::from(cols);
            assert!(
                (0.2..=0.28).contains(&(spans[0] / width)),
                "{cols}: {spans:?}"
            );
            for small in [3, 4] {
                assert!((7.5..=12.0).contains(&spans[small]), "{cols}: {spans:?}");
            }
            for medium in [1, 2] {
                assert!(
                    spans[medium] > spans[4] && spans[medium] < spans[0],
                    "{cols}: {spans:?}"
                );
            }
        }
        assert_eq!(cast(100).len(), 5);
        assert_eq!(cast(72).len(), 4);
        assert_eq!(cast(40).len(), 3);
    }

    /// Milliseconds and bytes per frame through a real crossterm backend,
    /// as a terminal would receive them. Run it under the profile the
    /// users run (petri-live's `live`, scripts/petri-live.sh) or release:
    /// `cargo test --profile live <PROFILE_CONFIG> bench_frames -- --ignored
    /// --nocapture`.
    #[test]
    #[ignore = "a benchmark"]
    fn bench_frames() {
        use ratatui::{Terminal, TerminalOptions, Viewport, backend::CrosstermBackend};
        use std::{cell::Cell, io, rc::Rc, time::Instant};
        #[derive(Clone, Default)]
        struct Counter(Rc<Cell<usize>>);
        impl io::Write for Counter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0.set(self.0.get() + bytes.len());
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        for (cols, rows) in [(100, 30), (200, 55), (250, 70), (320, 90)] {
            let counter = Counter::default();
            let viewport = Viewport::Fixed(Rect::new(0, 0, cols, rows));
            let mut terminal = Terminal::with_options(
                CrosstermBackend::new(counter.clone()),
                TerminalOptions { viewport },
            )
            .unwrap();
            let mut intro = Intro::new(RENDER_SEED);
            let mut draw = |t: f32| {
                terminal
                    .draw(|frame| {
                        let area = frame.area();
                        intro.draw(frame.buffer_mut(), area, t)
                    })
                    .unwrap();
            };
            draw(0.0);
            counter.0.set(0);
            let frames = 24 * 20;
            let started = Instant::now();
            for k in 1..=frames {
                draw(k as f32 / 24.0);
            }
            let ms = started.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);
            let bytes = counter.0.get() / frames as usize;
            println!("{cols}x{rows}: {ms:.2} ms/frame, {bytes} bytes/frame");
        }
    }

    #[test]
    fn click_enter_or_space_start_and_q_quits() {
        let key = |code| Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
        let mouse = |kind| {
            Event::Mouse(MouseEvent {
                kind,
                column: 3,
                row: 7,
                modifiers: KeyModifiers::NONE,
            })
        };
        assert_eq!(
            handle(&mouse(MouseEventKind::Down(MouseButton::Left))),
            Some(Outcome::Start)
        );
        assert_eq!(handle(&key(KeyCode::Enter)), Some(Outcome::Start));
        assert_eq!(handle(&key(KeyCode::Char(' '))), Some(Outcome::Start));
        assert_eq!(handle(&key(KeyCode::Char('q'))), Some(Outcome::Quit));
        assert_eq!(handle(&key(KeyCode::Esc)), Some(Outcome::Quit));
        assert_eq!(
            handle(&Event::Key(KeyEvent::new(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL
            ))),
            Some(Outcome::Quit)
        );
        assert_eq!(handle(&mouse(MouseEventKind::Moved)), None);
        assert_eq!(handle(&mouse(MouseEventKind::ScrollDown)), None);
        assert_eq!(handle(&key(KeyCode::Char('m'))), None);
        assert_eq!(handle(&Event::Resize(80, 24)), None);
    }

    #[test]
    fn any_size_draws() {
        for (w, h) in [(1, 1), (20, 10), (44, 15), (240, 100), (300, 8)] {
            let buf = frame(RENDER_SEED, w, h, 7.5);
            assert_eq!(buf.area, Rect::new(0, 0, w, h));
        }
    }
}
