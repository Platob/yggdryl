//! The workbook's theme: the twelve colours of its `a:clrScheme`, which a
//! style's theme colours index.
//!
//! A cell's font, fill or border names a colour by its place in the theme
//! (`<color theme="4" tint="0.4"/>`) as often as by its value. The theme is
//! the part the workbook's theme relationship names (`xl/theme/theme1.xml`);
//! a workbook without one displays with Office's default theme, the one
//! Excel 2013 and later write. [`StyleSheet::resolve`] turns any colour a
//! style states into the RGB value it displays as.
//!
//! [`StyleSheet::resolve`]: super::StyleSheet::resolve

use quick_xml::events::Event;
use smol_str::format_smolstr;

use crate::Result;

use super::package::{attribute, codec_error, local_name};

/// The scheme's slots, in the order `a:clrScheme` lists them.
const SLOTS: [&[u8]; 12] = [
    b"dk1",
    b"lt1",
    b"dk2",
    b"lt2",
    b"accent1",
    b"accent2",
    b"accent3",
    b"accent4",
    b"accent5",
    b"accent6",
    b"hlink",
    b"folHlink",
];

/// A workbook's theme colours.
///
/// The scheme holds twelve colours as RGB, in the order the theme part
/// lists them: dark 1, light 1, dark 2, light 2, the six accents, the
/// hyperlink and the followed hyperlink. A style's `theme` index counts
/// the first four the other way round in each pair - index 0 is light 1,
/// the background, and 1 is dark 1, the text - which [`Theme::color`]
/// answers for.
///
/// ```
/// use yggdryl::excel::{Theme, Workbook};
///
/// let theme = Theme::default();
/// assert_eq!(theme.color(0), Some(0xFF_FFFF)); // Background 1: light 1
/// assert_eq!(theme.color(1), Some(0x00_0000)); // Text 1: dark 1
/// assert_eq!(theme.color(4), Some(0x44_72C4)); // Accent 1
/// assert_eq!(theme.color(12), None);
///
/// // A workbook without a theme part displays with this one.
/// assert_eq!(Workbook::new().theme()?, &theme);
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Theme {
    scheme: [u32; 12],
}

impl Theme {
    /// The Office theme Excel 2013 and later write: black and white text
    /// and background, blue-gray and light gray, the six accents from blue
    /// to green, and the two hyperlink colours.
    pub const OFFICE: Self = Self {
        scheme: [
            0x00_0000, 0xFF_FFFF, 0x44_546A, 0xE7_E6E6, 0x44_72C4, 0xED_7D31, 0xA5_A5A5, 0xFF_C000,
            0x5B_9BD5, 0x70_AD47, 0x05_63C1, 0x95_4F72,
        ],
    };

    /// The colours in the order the theme part lists them, as RGB.
    #[must_use]
    pub const fn scheme(&self) -> &[u32; 12] {
        &self.scheme
    }

    /// The colour a style's `theme` index names, as RGB: 0 and 1 are light
    /// 1 and dark 1, 2 and 3 light 2 and dark 2, 4 to 9 the accents, 10
    /// and 11 the hyperlinks; `None` past them.
    #[must_use]
    pub const fn color(&self, index: u8) -> Option<u32> {
        let slot = match index {
            0 => 1,
            1 => 0,
            2 => 3,
            3 => 2,
            4..=11 => index as usize,
            _ => return None,
        };
        Some(self.scheme[slot])
    }

    /// Read a theme part: the colours of its first `a:clrScheme`, the
    /// workbook's (the schemes an `a:extraClrSchemeLst` offers after it are
    /// not read), each slot the part leaves out, states empty or states in
    /// a form other than `a:srgbClr` or `a:sysClr` taken from
    /// [`Theme::OFFICE`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Codec`](crate::Error::Codec) when the bytes are not
    /// well-formed XML or a colour is not six hex digits.
    pub(crate) fn from_xml(bytes: &[u8]) -> Result<Self> {
        let mut reader = quick_xml::Reader::from_reader(bytes);
        let mut buffer = Vec::new();
        let mut theme = Self::OFFICE;
        let mut in_scheme = false;
        let mut slot: Option<usize> = None;
        loop {
            let position = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
            let event = reader
                .read_event_into(&mut buffer)
                .map_err(|error| codec_error(position, error.to_string()))?;
            match &event {
                Event::Start(start) | Event::Empty(start) => {
                    let qualified = start.name();
                    let name = local_name(qualified.as_ref());
                    let opened = matches!(event, Event::Start(_));
                    if name == b"clrScheme" {
                        in_scheme = opened;
                    } else if in_scheme && slot.is_none() {
                        // A slot stated empty (`<a:dk1/>`) holds no colour
                        // for what follows it.
                        slot = SLOTS
                            .iter()
                            .position(|held| *held == name)
                            .filter(|_| opened);
                    } else if let Some(at) = slot {
                        let value = match name {
                            b"srgbClr" => attribute(start, b"val", position)?,
                            b"sysClr" => attribute(start, b"lastClr", position)?,
                            _ => None,
                        };
                        if let Some(value) = value {
                            theme.scheme[at] = rgb(&value, position)?;
                        }
                    }
                }
                Event::End(end) => {
                    let qualified = end.name();
                    let name = local_name(qualified.as_ref());
                    if name == b"clrScheme" && in_scheme {
                        // The workbook's scheme is the first: the ones an
                        // `a:extraClrSchemeLst` offers after it are not.
                        break;
                    } else if slot.is_some_and(|at| SLOTS[at] == name) {
                        slot = None;
                    }
                }
                Event::Eof => break,
                _ => {}
            }
            buffer.clear();
        }
        Ok(theme)
    }
}

impl Default for Theme {
    /// [`Theme::OFFICE`].
    fn default() -> Self {
        Self::OFFICE
    }
}

/// Six hex digits of RGB.
fn rgb(value: &str, position: usize) -> Result<u32> {
    let digits = value.trim();
    u32::from_str_radix(digits, 16)
        .ok()
        .filter(|_| digits.len() == 6 && digits.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| {
            codec_error(
                position,
                format_smolstr!("expected six hex digits of RGB for a theme colour, got {value:?}"),
            )
        })
}

/// The default indexed palette, ECMA-376 §18.8.27: RGB by index, 0 to 63.
const DEFAULT_PALETTE: [u32; 64] = [
    0x00_0000, 0xFF_FFFF, 0xFF_0000, 0x00_FF00, 0x00_00FF, 0xFF_FF00, 0xFF_00FF, 0x00_FFFF,
    0x00_0000, 0xFF_FFFF, 0xFF_0000, 0x00_FF00, 0x00_00FF, 0xFF_FF00, 0xFF_00FF, 0x00_FFFF,
    0x80_0000, 0x00_8000, 0x00_0080, 0x80_8000, 0x80_0080, 0x00_8080, 0xC0_C0C0, 0x80_8080,
    0x99_99FF, 0x99_3366, 0xFF_FFCC, 0xCC_FFFF, 0x66_0066, 0xFF_8080, 0x00_66CC, 0xCC_CCFF,
    0x00_0080, 0xFF_00FF, 0xFF_FF00, 0x00_FFFF, 0x80_0080, 0x80_0000, 0x00_8080, 0x00_00FF,
    0x00_CCFF, 0xCC_FFFF, 0xCC_FFCC, 0xFF_FF99, 0x99_CCFF, 0xFF_99CC, 0xCC_99FF, 0xFF_CC99,
    0x33_66FF, 0x33_CCCC, 0x99_CC00, 0xFF_CC00, 0xFF_9900, 0xFF_6600, 0x66_6699, 0x96_9696,
    0x00_3366, 0x33_9966, 0x00_3300, 0x33_3300, 0x99_3300, 0x99_3366, 0x33_3399, 0x33_3333,
];

/// The RGB value the indexed colour `index` names: the entry of `palette` -
/// the ARGB values a styles part's `indexedColors` states - else of the
/// default palette, `None` past both. The one reading of an index, which a
/// style's colour and a format section's `[ColorN]` share.
pub(crate) fn indexed(palette: Option<&[u32]>, index: usize) -> Option<u32> {
    palette
        .and_then(|palette| palette.get(index))
        .map(|argb| argb & 0x00FF_FFFF)
        .or_else(|| DEFAULT_PALETTE.get(index).copied())
}

/// Apply the HLS luminance tint of ECMA-376 §18.8.19 with Excel's integer
/// color rounding, pinned against the desktop fixture. Negative tint moves
/// toward black, positive toward white; the bounds are `-1.0` and `1.0`.
pub(crate) fn tinted(rgb: u32, tint: f64) -> u32 {
    if tint == 0.0 || !tint.is_finite() {
        return rgb;
    }
    let tint = tint.clamp(-1.0, 1.0);
    let red = ((rgb >> 16) & 0xff) as i32;
    let green = ((rgb >> 8) & 0xff) as i32;
    let blue = (rgb & 0xff) as i32;
    // Excel's resolved colors use integer HLS240 conversion. Keep tint's
    // two positive terms separate: truncating their products independently
    // changes the result at half steps, including black +0.5.
    let maximum = red.max(green).max(blue);
    let minimum = red.min(green).min(blue);
    let source_luminance = ((maximum + minimum) * 240 + 255) / 510;
    let luminance = if tint < 0.0 {
        (f64::from(source_luminance) * (1.0 + tint)) as i32
    } else {
        let complement = 1.0 - tint;
        (f64::from(source_luminance) * complement) as i32 + 240 - (240.0 * complement) as i32
    };
    if maximum == minimum {
        let byte = ((luminance * 255 + 120) / 240) as u32;
        return byte * 0x01_0101;
    }
    let delta = maximum - minimum;
    let saturation = if source_luminance <= 120 {
        ((maximum + minimum) / 2 + delta * 240) / (maximum + minimum)
    } else {
        ((510 - maximum - minimum) / 2 + delta * 240) / (510 - maximum - minimum)
    };
    let red_norm = (delta / 2 + maximum * 40 - red * 40) / delta;
    let green_norm = (delta / 2 + maximum * 40 - green * 40) / delta;
    let blue_norm = (delta / 2 + maximum * 40 - blue * 40) / delta;
    let mut hue = if red == maximum {
        blue_norm - green_norm
    } else if green == maximum {
        80 + red_norm - blue_norm
    } else {
        160 + green_norm - red_norm
    };
    if hue < 0 {
        hue += 240;
    } else if hue > 240 {
        hue -= 240;
    }
    let upper = if luminance > 120 {
        saturation + luminance - (saturation * luminance + 120) / 240
    } else {
        ((saturation + 240) * luminance + 120) / 240
    };
    let lower = luminance * 2 - upper;
    let component = |mut shifted: i32| {
        if shifted > 240 {
            shifted -= 240;
        } else if shifted < 0 {
            shifted += 240;
        }
        let mixed = if shifted > 160 {
            lower
        } else if shifted > 120 {
            ((160 - shifted) * (upper - lower) + 20) / 40 + lower
        } else if shifted > 40 {
            upper
        } else {
            (shifted * (upper - lower) + 20) / 40 + lower
        };
        ((mixed * 255 + 120) / 240) as u32
    };
    (component(hue + 80) << 16) | (component(hue) << 8) | component(hue - 80)
}
