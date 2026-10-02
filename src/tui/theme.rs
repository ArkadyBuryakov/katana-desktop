//! Colours: the web UI's light and dark palettes, for terminals with and without truecolor.

use ratatui::style::Color;

pub use crate::game::Rgb;

pub const BLACK: Rgb = [0, 0, 0];
pub const WHITE: Rgb = [255, 255, 255];

/// a over b: t = 0 gives a, t = 1 gives b
pub fn blend(a: Rgb, b: Rgb, t: f32) -> Rgb {
    std::array::from_fn(|i| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round() as u8)
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    truecolor: bool,
    pub bg: Rgb,
    pub panel: Rgb,
    pub ink: Rgb,
    pub muted: Rgb,
    pub card: Rgb,
    pub error: Rgb,
    pub accent: Rgb,
    pub accent_ink: Rgb,
    pub ok: Rgb,
}

impl Theme {
    pub fn new(dark: bool) -> Theme {
        let truecolor = matches!(
            std::env::var("COLORTERM").as_deref(),
            Ok("truecolor" | "24bit")
        ) || cfg!(windows);
        let (accent, accent_ink, ok) = ([0xb5, 0x45, 0x2b], WHITE, [0x30, 0xa4, 0x6c]);
        if dark {
            Theme {
                dark,
                truecolor,
                bg: [0x1d, 0x1b, 0x18],
                panel: [0x26, 0x23, 0x1f],
                ink: [0xec, 0xe4, 0xd6],
                muted: [0x9d, 0x92, 0x82],
                card: [0x2b, 0x28, 0x23],
                error: [0xff, 0x8a, 0x80],
                accent,
                accent_ink,
                ok,
            }
        } else {
            Theme {
                dark,
                truecolor,
                bg: [0xf3, 0xed, 0xe2],
                panel: [0xff, 0xfa, 0xf2],
                ink: [0x2b, 0x26, 0x20],
                muted: [0x8a, 0x7f, 0x70],
                card: WHITE,
                error: [0xc6, 0x28, 0x28],
                accent,
                accent_ink,
                ok,
            }
        }
    }

    /// The terminal colour for an RGB one: itself, or the nearest of the 256-colour palette.
    pub fn c(&self, [r, g, b]: Rgb) -> Color {
        if self.truecolor {
            return Color::Rgb(r, g, b);
        }
        // the 6x6x6 cube has levels 0, 95, 135, 175, 215, 255; the grey ramp is 8, 18, ..., 238
        let cube = |v: u8| if v < 48 { 0 } else { (v.max(75) - 35) / 40 };
        let level = |i: u8| if i == 0 { 0 } else { 55 + 40 * i };
        let (ri, gi, bi) = (cube(r), cube(g), cube(b));
        let grey = ((r as u16 + g as u16 + b as u16) / 3)
            .saturating_sub(3)
            .min(235) as u8
            / 10;
        let dist = |c: Rgb| {
            (0..3)
                .map(|k| (c[k] as i32 - [r, g, b][k] as i32).pow(2))
                .sum::<i32>()
        };
        if dist([8 + 10 * grey; 3]) < dist([level(ri), level(gi), level(bi)]) {
            Color::Indexed(232 + grey)
        } else {
            Color::Indexed(16 + 36 * ri + 6 * gi + bi)
        }
    }
}

/// The terminal says whether it is dark in COLORFGBG ("fg;bg"), when it says anything.
pub fn terminal_is_dark() -> bool {
    let bg = std::env::var("COLORFGBG").ok();
    let bg = bg
        .as_deref()
        .and_then(|v| v.rsplit(';').next()?.parse::<u8>().ok());
    !matches!(bg, Some(7 | 9..=15))
}
