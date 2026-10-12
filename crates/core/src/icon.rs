//! Icon rendering, as arithmetic.
//!
//! `tray-icon` accepts a raw RGBA buffer, so the tray artwork is computed rather than
//! loaded. That keeps the repository free of binary assets, lets the installers derive
//! every icon size from one source of truth (`cargo xtask icons`), and makes the artwork
//! unit testable.
//!
//! Two shapes are produced:
//!
//! * A glyph - an eye, drawn with 3x3 supersampling so the curves are smooth.
//! * A badge - digits from a crisp 5x7 bitmap font, deliberately *not* antialiased. This
//!   is how the countdown is shown on hosts whose tray has no text label, which on Windows
//!   is the only way to put a number in the notification area. It is either one number as
//!   large as will fit, or minutes stacked above seconds.

use crate::engine::{Badge, IconKind};

/// Pure black: on macOS the tray image is installed as a template image, where only the
/// alpha channel matters and the system recolours the glyph for the current menu bar.
pub const INK_TEMPLATE: [u8; 3] = [0, 0, 0];

/// A mid-tone blue that stays legible on both the light and the dark Windows taskbar,
/// where the icon is composited as-is.
pub const INK_TRAY: [u8; 3] = [0x3d, 0xa9, 0xf0];

/// Width of one bitmap-font glyph, in font pixels.
const GLYPH_W: u32 = 5;
/// Height of one bitmap-font glyph, in font pixels.
const GLYPH_H: u32 = 7;

/// Digits 0-9 as 7 rows of 5 bits, most significant bit leftmost.
const DIGITS: [[u8; 7]; 10] = [
    [
        0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
    ],
    [
        0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
    ],
    [
        0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
    ],
    [
        0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110,
    ],
    [
        0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
    ],
    [
        0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
    ],
    [
        0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
    ],
    [
        0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
    ],
    [
        0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
    ],
    [
        0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
    ],
];

/// A premultiplication-free RGBA image, row major, four bytes per pixel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgba {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Rgba {
    /// A fully transparent image.
    #[must_use]
    pub fn transparent(width: u32, height: u32) -> Self {
        let len = (width as usize) * (height as usize) * 4;
        Self {
            width,
            height,
            pixels: vec![0; len],
        }
    }

    /// Image width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Image height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// The raw buffer, four bytes per pixel, `width * height * 4` long.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.pixels
    }

    /// Consumes the image and returns the raw buffer.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.pixels
    }

    /// The pixel at `(x, y)`, or fully transparent when out of bounds.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.width || y >= self.height {
            return [0; 4];
        }
        let i = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        [
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        ]
    }

    fn set(&mut self, x: u32, y: u32, px: [u8; 4]) {
        if x >= self.width || y >= self.height {
            return;
        }
        let i = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        self.pixels[i..i + 4].copy_from_slice(&px);
    }

    /// Sum of the alpha channel. Used by tests as a cheap "how much ink is on the page".
    #[must_use]
    pub fn alpha_sum(&self) -> u64 {
        self.pixels
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| u64::from(p[3]))
            .sum()
    }
}

/// Renders the tray image.
///
/// When `badge` is `Some`, the countdown is drawn instead of the glyph: a host that cannot
/// show text next to the icon has nowhere else to put it.
#[must_use]
pub fn render(kind: IconKind, size: u32, ink: [u8; 3], badge: Option<Badge>) -> Rgba {
    match badge {
        Some(Badge::Number(value)) => render_badge(kind, size, ink, value),
        Some(Badge::Stacked { minutes, seconds }) => render_stacked(size, ink, minutes, seconds),
        None => render_glyph(kind, size, ink),
    }
}

/// Renders the eye glyph, antialiased.
#[must_use]
pub fn render_glyph(kind: IconKind, size: u32, ink: [u8; 3]) -> Rgba {
    let mut img = Rgba::transparent(size, size);
    if size == 0 {
        return img;
    }
    // Paused is the same shape at 40% strength.
    let dim = if kind == IconKind::Paused { 0.4 } else { 1.0 };
    for y in 0..size {
        for x in 0..size {
            let coverage = supersample(size, x, y, |nx, ny| glyph_hit(kind, nx, ny));
            img.set(x, y, [ink[0], ink[1], ink[2], alpha_byte(coverage * dim)]);
        }
    }
    img
}

/// Renders a one or two digit countdown, crisply.
///
/// During a break the digits are punched out of a filled slab, so that work and break are
/// distinguishable at a glance without reading the number.
#[must_use]
pub fn render_badge(kind: IconKind, size: u32, ink: [u8; 3], value: u16) -> Rgba {
    let mut img = Rgba::transparent(size, size);
    if size == 0 {
        return img;
    }
    let value = value.min(99);
    let digits: Vec<u8> = if value >= 10 {
        vec![
            u8::try_from(value / 10).unwrap_or(0),
            u8::try_from(value % 10).unwrap_or(0),
        ]
    } else {
        vec![u8::try_from(value).unwrap_or(0)]
    };

    let count = u32::try_from(digits.len()).unwrap_or(1);
    let columns = GLYPH_W * count + (count - 1);
    let scale = (size / columns).min(size / GLYPH_H).max(1);
    let block_w = columns * scale;
    let block_h = GLYPH_H * scale;
    let x0 = (size.saturating_sub(block_w)) / 2;
    let y0 = (size.saturating_sub(block_h)) / 2;

    let inverted = kind == IconKind::Break;
    if inverted {
        // A filled slab with one pixel of padding around the digits.
        let pad = scale;
        for y in y0.saturating_sub(pad)..(y0 + block_h + pad).min(size) {
            for x in x0.saturating_sub(pad)..(x0 + block_w + pad).min(size) {
                img.set(x, y, [ink[0], ink[1], ink[2], 255]);
            }
        }
    }

    for (index, digit) in digits.iter().enumerate() {
        let rows = DIGITS[usize::from(*digit)];
        let dx = x0 + u32::try_from(index).unwrap_or(0) * (GLYPH_W + 1) * scale;
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..GLYPH_W {
                let on = bits & (1 << (GLYPH_W - 1 - col)) != 0;
                if on == inverted {
                    continue;
                }
                let px = [ink[0], ink[1], ink[2], 255];
                for sy in 0..scale {
                    for sx in 0..scale {
                        let x = dx + col * scale + sx;
                        let y = y0 + u32::try_from(row).unwrap_or(0) * scale + sy;
                        if inverted {
                            img.set(x, y, [0, 0, 0, 0]);
                        } else {
                            img.set(x, y, px);
                        }
                    }
                }
            }
        }
    }
    img
}

/// Renders minutes above seconds, both as two digits: `19:42` for a tray that has only a
/// square to show it in.
///
/// Two digits are 11 font pixels wide and two rows are 15 tall, so the smallest tray icon
/// there is (16 pixels) holds it at one pixel per font pixel - small, but whole. Both rows
/// are zero-padded so that nothing moves as the numbers change.
#[must_use]
pub fn render_stacked(size: u32, ink: [u8; 3], minutes: u8, seconds: u8) -> Rgba {
    const BLOCK_W: u32 = GLYPH_W * 2 + 1;
    const BLOCK_H: u32 = GLYPH_H * 2 + 1;

    let mut img = Rgba::transparent(size, size);
    let scale = (size / BLOCK_W).min(size / BLOCK_H).max(1);
    let x0 = size.saturating_sub(BLOCK_W * scale) / 2;
    let y0 = size.saturating_sub(BLOCK_H * scale) / 2;
    let px = [ink[0], ink[1], ink[2], 255];

    for (row, value) in (0u32..).zip([minutes.min(99), seconds.min(99)]) {
        let y = y0 + row * (GLYPH_H + 1) * scale;
        for (column, digit) in (0u32..).zip([value / 10, value % 10]) {
            let x = x0 + column * (GLYPH_W + 1) * scale;
            stamp_digit(&mut img, digit, x, y, scale, px);
        }
    }
    img
}

/// Draws one digit with its top left corner at `(x0, y0)`, `scale` pixels per font pixel.
fn stamp_digit(img: &mut Rgba, digit: u8, x0: u32, y0: u32, scale: u32, px: [u8; 4]) {
    let rows = DIGITS[usize::from(digit % 10)];
    for (row, bits) in (0u32..).zip(rows) {
        for col in 0..GLYPH_W {
            if bits & (1 << (GLYPH_W - 1 - col)) == 0 {
                continue;
            }
            for sy in 0..scale {
                for sx in 0..scale {
                    img.set(x0 + col * scale + sx, y0 + row * scale + sy, px);
                }
            }
        }
    }
}

/// Renders the application icon: the eye in white on a rounded accent-coloured tile.
///
/// Used for the macOS `.icns` and the Windows `.ico`, where the artwork sits on an
/// unpredictable background and so cannot rely on alpha alone.
#[must_use]
pub fn render_app_icon(size: u32) -> Rgba {
    let mut img = Rgba::transparent(size, size);
    if size == 0 {
        return img;
    }
    for y in 0..size {
        for x in 0..size {
            let tile = supersample(size, x, y, |nx, ny| rounded_square(nx, ny, 0.94, 0.28));
            if tile <= 0.0 {
                continue;
            }
            let eye = supersample(size, x, y, |nx, ny| {
                glyph_hit(IconKind::Work, nx / 0.78, ny / 0.78)
            });
            // The eye is painted in white over the accent tile, antialiased by `eye`.
            let channel =
                |c: u8| alpha_byte(eye.mul_add(1.0 - f64::from(c) / 255.0, f64::from(c) / 255.0));
            img.set(
                x,
                y,
                [
                    channel(INK_TRAY[0]),
                    channel(INK_TRAY[1]),
                    channel(INK_TRAY[2]),
                    alpha_byte(tile),
                ],
            );
        }
    }
    img
}

/// Converts a coverage fraction to an alpha byte.
///
/// The clamp is what makes this exact: the value cannot be outside 0..=255, so the
/// conversion discards nothing.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn alpha_byte(value: f64) -> u8 {
    (value * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Fraction of a pixel covered by `hit`, sampled on a symmetric 3x3 grid so that mirrored
/// pixels always receive mirrored samples and the artwork stays exactly symmetric.
fn supersample(size: u32, x: u32, y: u32, hit: impl Fn(f64, f64) -> bool) -> f64 {
    const STEPS: u32 = 3;
    let mut covered = 0u32;
    for sy in 0..STEPS {
        for sx in 0..STEPS {
            let ox = (f64::from(sx) + 0.5) / f64::from(STEPS);
            let oy = (f64::from(sy) + 0.5) / f64::from(STEPS);
            let nx = ((f64::from(x) + ox) / f64::from(size)).mul_add(2.0, -1.0);
            let ny = ((f64::from(y) + oy) / f64::from(size)).mul_add(2.0, -1.0);
            if hit(nx, ny) {
                covered += 1;
            }
        }
    }
    f64::from(covered) / f64::from(STEPS * STEPS)
}

/// The eye: a lens formed by intersecting two circles, with a pupil or a closed lid.
fn glyph_hit(kind: IconKind, nx: f64, ny: f64) -> bool {
    const R: f64 = 0.95;
    const D: f64 = 0.62;
    const STROKE: f64 = 0.17;

    // Stretch horizontally so the almond fills the square.
    let x = nx / 1.28;
    let y = ny;

    let upper = x.hypot(y + D);
    let lower = x.hypot(y - D);
    let inside = upper <= R && lower <= R;
    if !inside {
        return false;
    }
    // Distance to the nearer of the two arcs that bound the lens.
    let edge = (R - upper).min(R - lower);
    if edge <= STROKE {
        return true;
    }
    match kind {
        IconKind::Work | IconKind::Paused => x.hypot(y) <= 0.30,
        IconKind::Break => y.abs() <= 0.08,
    }
}

/// A rounded square of half-extent `half` with corner radius `radius`.
fn rounded_square(nx: f64, ny: f64, half: f64, radius: f64) -> bool {
    let inner = half - radius;
    let dx = nx.abs() - inner;
    let dy = ny.abs() - inner;
    if dx <= 0.0 || dy <= 0.0 {
        nx.abs() <= half && ny.abs() <= half
    } else {
        dx.hypot(dy) <= radius
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: [IconKind; 3] = [IconKind::Work, IconKind::Break, IconKind::Paused];

    #[test]
    fn buffer_length_always_matches_the_dimensions() {
        for size in [1u32, 2, 16, 22, 32, 44] {
            for kind in KINDS {
                let img = render(kind, size, INK_TEMPLATE, None);
                assert_eq!(img.width(), size);
                assert_eq!(img.height(), size);
                assert_eq!(img.as_bytes().len(), (size * size * 4) as usize);
            }
        }
    }

    #[test]
    fn zero_size_is_empty_rather_than_a_panic() {
        assert!(
            render_glyph(IconKind::Work, 0, INK_TEMPLATE)
                .as_bytes()
                .is_empty()
        );
        assert!(
            render_badge(IconKind::Work, 0, INK_TEMPLATE, 20)
                .as_bytes()
                .is_empty()
        );
        assert!(render_app_icon(0).as_bytes().is_empty());
    }

    #[test]
    fn the_buffer_can_be_taken_by_value_for_handing_to_the_os() {
        let img = render_glyph(IconKind::Work, 8, INK_TEMPLATE);
        let expected = img.as_bytes().to_vec();
        assert_eq!(img.into_bytes(), expected);
    }

    #[test]
    fn out_of_bounds_access_is_transparent_and_writes_are_ignored() {
        let mut img = Rgba::transparent(2, 2);
        assert_eq!(img.pixel(5, 0), [0, 0, 0, 0]);
        img.set(5, 0, [1, 2, 3, 4]);
        img.set(0, 5, [1, 2, 3, 4]);
        assert_eq!(img.alpha_sum(), 0);
    }

    #[test]
    fn the_glyph_is_horizontally_and_vertically_symmetric() {
        let size = 32;
        for kind in KINDS {
            let img = render_glyph(kind, size, INK_TEMPLATE);
            for y in 0..size {
                for x in 0..size {
                    assert_eq!(
                        img.pixel(x, y),
                        img.pixel(size - 1 - x, y),
                        "mirror x at {x},{y}"
                    );
                    assert_eq!(
                        img.pixel(x, y),
                        img.pixel(x, size - 1 - y),
                        "mirror y at {x},{y}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_glyph_draws_ink_in_the_middle_and_leaves_the_corners_clear() {
        for kind in KINDS {
            let img = render_glyph(kind, 32, INK_TEMPLATE);
            assert!(img.pixel(16, 16)[3] > 0, "{kind:?} should mark the centre");
            for (x, y) in [(0, 0), (31, 0), (0, 31), (31, 31)] {
                assert_eq!(img.pixel(x, y)[3], 0, "{kind:?} should leave {x},{y} clear");
            }
        }
    }

    #[test]
    fn each_kind_is_visually_distinct() {
        let work = render_glyph(IconKind::Work, 32, INK_TEMPLATE);
        let brk = render_glyph(IconKind::Break, 32, INK_TEMPLATE);
        let paused = render_glyph(IconKind::Paused, 32, INK_TEMPLATE);
        assert_ne!(work, brk);
        assert_ne!(work, paused);
        assert!(
            paused.alpha_sum() < work.alpha_sum(),
            "paused is the same shape, dimmed, so it must carry less ink"
        );
        assert!(paused.alpha_sum() > 0);
    }

    #[test]
    fn the_glyph_has_smooth_edges() {
        let img = render_glyph(IconKind::Work, 32, INK_TEMPLATE);
        let partial = img
            .as_bytes()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] > 0 && p[3] < 255)
            .count();
        assert!(
            partial > 8,
            "expected antialiased edge pixels, found {partial}"
        );
    }

    #[test]
    fn ink_colour_is_applied_to_every_marked_pixel() {
        let img = render_glyph(IconKind::Work, 16, INK_TRAY);
        let marked: Vec<_> = img
            .as_bytes()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] > 0)
            .collect();
        assert!(!marked.is_empty());
        for px in marked {
            assert_eq!([px[0], px[1], px[2]], INK_TRAY);
        }
    }

    #[test]
    fn badges_render_one_and_two_digit_numbers() {
        for value in [0u16, 1, 9, 10, 20, 99] {
            let img = render_badge(IconKind::Work, 16, INK_TEMPLATE, value);
            assert!(img.alpha_sum() > 0, "{value} should draw something");
        }
        assert_ne!(
            render_badge(IconKind::Work, 16, INK_TEMPLATE, 19),
            render_badge(IconKind::Work, 16, INK_TEMPLATE, 20),
            "each minute must look different or the countdown is useless"
        );
        assert_ne!(
            render_badge(IconKind::Work, 16, INK_TEMPLATE, 9),
            render_badge(IconKind::Work, 16, INK_TEMPLATE, 19)
        );
    }

    #[test]
    fn badges_over_two_digits_are_clamped_rather_than_overflowing_the_icon() {
        assert_eq!(
            render_badge(IconKind::Work, 16, INK_TEMPLATE, 150),
            render_badge(IconKind::Work, 16, INK_TEMPLATE, 99)
        );
    }

    #[test]
    fn badge_digits_are_crisp_and_fit_inside_the_icon() {
        let size = 16;
        let img = render_badge(IconKind::Work, size, INK_TEMPLATE, 20);
        for px in img.as_bytes().as_chunks::<4>().0 {
            assert!(
                px[3] == 0 || px[3] == 255,
                "bitmap digits must not be antialiased"
            );
        }
        for i in 0..size {
            assert_eq!(img.pixel(i, 0)[3], 0, "top row must stay clear");
            assert_eq!(img.pixel(i, size - 1)[3], 0, "bottom row must stay clear");
        }
    }

    #[test]
    fn a_break_badge_is_inverted_so_it_reads_differently_at_a_glance() {
        let work = render_badge(IconKind::Work, 16, INK_TEMPLATE, 20);
        let brk = render_badge(IconKind::Break, 16, INK_TEMPLATE, 20);
        assert_ne!(work, brk);
        assert!(
            brk.alpha_sum() > work.alpha_sum(),
            "the slab is filled, the digits punched out"
        );
        assert!(
            brk.as_bytes().as_chunks::<4>().0.iter().any(|p| p[3] == 0),
            "digits must be punched through the slab"
        );
    }

    #[test]
    fn render_dispatches_on_the_badge() {
        assert_eq!(
            render(IconKind::Work, 16, INK_TEMPLATE, None),
            render_glyph(IconKind::Work, 16, INK_TEMPLATE)
        );
        assert_eq!(
            render(IconKind::Work, 16, INK_TEMPLATE, Some(Badge::Number(7))),
            render_badge(IconKind::Work, 16, INK_TEMPLATE, 7)
        );
        let stacked = Badge::Stacked {
            minutes: 19,
            seconds: 42,
        };
        assert_eq!(
            render(IconKind::Work, 16, INK_TEMPLATE, Some(stacked)),
            render_stacked(16, INK_TEMPLATE, 19, 42)
        );
    }

    /// The image as rows of `#` and `.`, for comparing against a picture.
    fn ascii(img: &Rgba) -> Vec<String> {
        (0..img.height())
            .map(|y| {
                (0..img.width())
                    .map(|x| if img.pixel(x, y)[3] == 0 { '.' } else { '#' })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn a_stacked_countdown_fits_whole_in_the_smallest_tray_icon() {
        let img = render_stacked(16, INK_TRAY, 19, 42);
        assert_eq!(
            ascii(&img),
            [
                "....#....###....",
                "...##...#...#...",
                "....#...#...#...",
                "....#....####...",
                "....#.......#...",
                "....#......#....",
                "...###...##.....",
                "................",
                ".....#...###....",
                "....##..#...#...",
                "...#.#......#...",
                "..#..#.....#....",
                "..#####...#.....",
                ".....#...#......",
                ".....#..#####...",
                "................",
            ],
            "19 above 42"
        );
        assert_eq!(
            img.pixel(4, 0),
            [INK_TRAY[0], INK_TRAY[1], INK_TRAY[2], 255]
        );
    }

    #[test]
    fn a_stacked_countdown_pads_both_rows_so_nothing_moves() {
        let rows = ascii(&render_stacked(16, INK_TRAY, 5, 7));
        let reference = ascii(&render_stacked(16, INK_TRAY, 0, 0));
        // The tens column of each row is a zero, exactly where it is in 00 over 00.
        for (row, expected) in rows.iter().zip(&reference) {
            assert_eq!(row[..8], expected[..8]);
        }
        assert_ne!(rows, reference);
    }

    #[test]
    fn a_stacked_countdown_scales_up_crisply_and_survives_odd_sizes() {
        let small = render_stacked(16, INK_TRAY, 19, 42);
        let large = render_stacked(32, INK_TRAY, 19, 42);
        assert_eq!(
            large.alpha_sum(),
            small.alpha_sum() * 4,
            "every pixel doubled"
        );
        // Too small to hold it, out of range, or empty: drawn as far as it goes, no panic.
        assert!(render_stacked(8, INK_TRAY, 19, 42).alpha_sum() > 0);
        assert_eq!(
            render_stacked(16, INK_TRAY, 200, 200),
            render_stacked(16, INK_TRAY, 99, 99)
        );
        assert!(render_stacked(0, INK_TRAY, 19, 42).as_bytes().is_empty());
    }

    #[test]
    fn badges_scale_up_with_the_icon() {
        let small = render_badge(IconKind::Work, 16, INK_TEMPLATE, 20);
        let large = render_badge(IconKind::Work, 64, INK_TEMPLATE, 20);
        assert!(large.alpha_sum() > small.alpha_sum() * 4);
    }

    #[test]
    fn the_app_icon_is_an_opaque_rounded_tile_with_a_light_eye() {
        let size = 64;
        let img = render_app_icon(size);
        assert_eq!(
            img.pixel(size / 2, size / 2)[3],
            255,
            "the tile is opaque in the middle"
        );
        assert_eq!(img.pixel(0, 0)[3], 0, "corners are rounded away");
        let centre = img.pixel(size / 2, size / 2);
        assert!(
            centre[0] > INK_TRAY[0] && centre[2] >= INK_TRAY[2],
            "the pupil is lighter than the tile: {centre:?}"
        );
        let edge = img.pixel(size / 2, size / 2 + size / 4);
        assert_eq!(
            [edge[0], edge[1], edge[2]],
            INK_TRAY,
            "the tile keeps the accent colour"
        );
    }

    #[test]
    fn rounded_square_covers_the_middle_and_cuts_the_corners() {
        assert!(rounded_square(0.0, 0.0, 0.9, 0.3));
        assert!(rounded_square(0.5, 0.0, 0.9, 0.3));
        assert!(!rounded_square(0.95, 0.0, 0.9, 0.3));
        assert!(
            !rounded_square(0.89, 0.89, 0.9, 0.3),
            "the corner is outside the radius"
        );
    }
}
