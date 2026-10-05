//! Number formats: a cell format's code read once, what it says a number
//! is, and the text a value displays as under it.
//!
//! A `numFmt` code is up to four sections - positive, negative, zero and
//! text - each a run of placeholders and literals under an optional colour
//! and condition. [`FormatCode::from_code`] reads one into its sections once,
//! refusing a code Excel refuses at the byte it breaks at, and every read
//! after that is over the sections: [`FormatCode::kind`] is the one reading
//! of what a number under the code *is* ([`NumberFormat`]), and
//! [`FormatCode::render`] the text Excel's en-US display writes, into a
//! buffer on the stack, so rendering allocates the text it answers and
//! nothing else.
//!
//! | Section piece | Renders |
//! | --- | --- |
//! | `General` | Excel's eleven-character spelling of the number |
//! | `0 # ?` | a digit, a digit or nothing, a digit or a space |
//! | `.` `,` `%` | the decimal point; a thousands separator between placeholders, a thousand's scale after them; the number as a percentage |
//! | `E+ E- e+ e-` | scientific notation, the exponent's sign always or only when negative |
//! | `# ?/?`, `?/8` | a fraction, its denominator at most the placeholders' digits or fixed |
//! | `"text"`, `\x`, `_x`, `*x`, `@` | literal text, one literal character, a space, a character repeated to fill the cell, the cell's text |
//! | `[Red]`, `[Color7]`, `[>=100]` | the section's colour, and the condition choosing it |
//! | `[$€-407]`, `[$-F800]`, `[$-F400]` | a currency symbol (the locale ignored), the en-US long date and long time |
//! | `y m d h s`, `AM/PM`, `A/P`, `.0`, `[h] [m] [s]` | the date and clock of a serial, and elapsed time |

use std::fmt::{self, Write as _};
use std::sync::{Arc, OnceLock};

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result, Scalar};

use super::cell::DateSystem;
use super::styles::NumberFormat;

/// The en-US code of a number format ECMA-376 §18.8.30 builds in: the ids
/// 0 to 22, 37 to 40 and 45 to 49 - the only ids a part may name without
/// declaring them, and so the only ones this crate's writer does.
pub(crate) const fn builtin_code(id: u32) -> Option<&'static str> {
    Some(match id {
        0 => "General",
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        5 => "$#,##0_);($#,##0)",
        6 => "$#,##0_);[Red]($#,##0)",
        7 => "$#,##0.00_);($#,##0.00)",
        8 => "$#,##0.00_);[Red]($#,##0.00)",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 => "m/d/yyyy",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "m/d/yyyy h:mm",
        37 => "#,##0 ;(#,##0)",
        38 => "#,##0 ;[Red](#,##0)",
        39 => "#,##0.00;(#,##0.00)",
        40 => "#,##0.00;[Red](#,##0.00)",
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        // ECMA-376 lists `mmss.0`; every Excel displays and writes `mm:ss.0`.
        47 => "mm:ss.0",
        48 => "##0.0E+0",
        49 => "@",
        _ => return None,
    })
}

/// Whether `id` may be named without a declaration.
pub(crate) const fn in_builtin_table(id: u32) -> bool {
    matches!(id, 0..=22 | 37..=40 | 45..=49)
}

/// The code a `numFmtId` a part does not declare displays with: the
/// built-in table's, the en-US accounting formats 41 to 44 that Excel
/// declares whenever it uses them, and for an East Asian locale id (27 to
/// 36, 50 to 58) the en-US code of the date or time it reads as; General
/// for any other id.
pub(crate) const fn undeclared_code(id: u32) -> &'static str {
    if let Some(code) = builtin_code(id) {
        return code;
    }
    match id {
        41 => r#"_(* #,##0_);_(* \(#,##0\);_(* "-"_);_(@_)"#,
        42 => r#"_("$"* #,##0_);_("$"* \(#,##0\);_("$"* "-"_);_(@_)"#,
        43 => r#"_(* #,##0.00_);_(* \(#,##0.00\);_(* "-"??_);_(@_)"#,
        44 => r#"_("$"* #,##0.00_);_("$"* \(#,##0.00\);_("$"* "-"??_);_(@_)"#,
        27..=31 | 36 | 50 | 51 | 54 | 57 | 58 => "m/d/yyyy",
        32..=35 | 52 | 53 | 55 | 56 => "h:mm:ss",
        _ => "General",
    }
}

/// The en-US long date a `[$-F800]` section displays as.
const LONG_DATE: &str = "dddd, mmmm d, yyyy";

/// The en-US long time a `[$-F400]` section displays as.
const LONG_TIME: &str = "h:mm:ss AM/PM";

/// How many characters Excel's General spelling of a number takes at most,
/// a minus sign aside: the width of a default column.
const GENERAL_WIDTH: usize = 11;

/// The most characters a code holds, as Excel takes one.
const MAX_CODE_LENGTH: usize = 255;

/// The eight colours a section names by name, as RGB.
const NAMED_COLORS: [(&str, u32); 8] = [
    ("black", 0x00_0000),
    ("blue", 0x00_00FF),
    ("cyan", 0x00_FFFF),
    ("green", 0x00_FF00),
    ("magenta", 0xFF_00FF),
    ("red", 0xFF_0000),
    ("white", 0xFF_FFFF),
    ("yellow", 0xFF_FF00),
];

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// A number format code, read into its sections once.
///
/// Equality and hashing read the code as written: two codes spelling the
/// same display differently are two codes, as they are two `numFmt`s.
///
/// ```
/// use yggdryl::excel::{DateSystem, FormatCode, NumberFormat};
/// use yggdryl::Scalar;
///
/// let money = FormatCode::from_code("#,##0.00;[Red](#,##0.00)")?;
/// let shown = money.render(&Scalar::from(-1234.5), DateSystem::Year1900);
/// assert_eq!(shown.text, "(1,234.50)");
/// assert_eq!(shown.color, Some(0xFF_0000));
/// assert_eq!(money.kind(), NumberFormat::General);
///
/// let day = FormatCode::builtin(14).expect("a built-in format");
/// assert_eq!(day.code(), "m/d/yyyy");
/// assert_eq!(day.kind(), NumberFormat::Date);
/// assert_eq!(day.render(&Scalar::date32(19_724), DateSystem::Year1900).text, "1/2/2024");
///
/// // A code Excel refuses is refused at the byte it breaks at.
/// assert!(FormatCode::from_code("0.00\"x").is_err());
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone)]
pub struct FormatCode {
    code: SmolStr,
    sections: Arc<[Section]>,
    /// The code names a locale's own digits or calendar (`[DBNum1]`,
    /// `[$-2000000]`): displayed as General, and never interned.
    localized: bool,
}

impl PartialEq for FormatCode {
    fn eq(&self, other: &Self) -> bool {
        self.code == other.code
    }
}

impl Eq for FormatCode {}

impl std::hash::Hash for FormatCode {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.code.hash(state);
    }
}

impl fmt::Debug for FormatCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("FormatCode")
            .field(&self.code.as_str())
            .finish()
    }
}

impl fmt::Display for FormatCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.code)
    }
}

impl std::str::FromStr for FormatCode {
    type Err = Error;

    fn from_str(code: &str) -> Result<Self> {
        Self::from_code(code)
    }
}

impl Default for FormatCode {
    /// `General`.
    fn default() -> Self {
        Self::general()
    }
}

/// What a value displays as under a [`FormatCode`].
///
/// `text` is the display itself; `color` the RGB colour the section chose
/// names, `None` for the cell's own; `fill` the character a `*x` repeats
/// to fill the cell's width and where in `text`, counted in characters, the
/// repetition goes; `shorter` the General spellings of the number from
/// widest to narrowest that a column too narrow for `text` shows instead -
/// empty for anything but a General number.
///
/// ```
/// use yggdryl::excel::{DateSystem, FormatCode};
/// use yggdryl::Scalar;
///
/// // The accounting layout: `*` repeats a space between the symbol and
/// // the number, whatever the column's width.
/// let money = FormatCode::from_code("\"$\"* #,##0.00")?;
/// let shown = money.render(&Scalar::from(1234.5), DateSystem::Year1900);
/// assert_eq!(shown.text, "$1,234.50");
/// assert_eq!(shown.fill, Some((' ', 1)));
///
/// let general = FormatCode::general().render(&Scalar::from(1234.5678), DateSystem::Year1900);
/// assert_eq!(general.text, "1234.5678");
/// assert_eq!(general.shorter, ["1234.568", "1234.57", "1234.6", "1235"]);
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rendered {
    /// The text displayed.
    pub text: SmolStr,
    /// The section's colour as `0xRRGGBB`, `None` for none.
    pub color: Option<u32>,
    /// The fill character and the character offset in `text` it repeats at.
    pub fill: Option<(char, usize)>,
    /// Narrower General spellings, each shorter than the one before.
    pub shorter: Vec<SmolStr>,
}

impl Rendered {
    /// Plain text, no colour and no fill.
    fn plain(text: SmolStr) -> Self {
        Self {
            text,
            ..Self::default()
        }
    }
}

/// An actual rendering overflow, distinct from a legitimate hash fill token.
struct RenderFailure {
    color: Option<u32>,
}

impl RenderFailure {
    fn display(self) -> Rendered {
        Rendered {
            color: self.color,
            fill: Some(('#', 0)),
            ..Rendered::default()
        }
    }
}

/// One section of a code.
#[derive(Clone, Debug, PartialEq)]
struct Section {
    /// The byte range of the section in the code.
    span: (usize, usize),
    condition: Option<Condition>,
    color: Option<Paint>,
    tokens: Box<[Token]>,
    shape: Shape,
    /// Where the section's decimals are written, for `with_decimals`.
    decimals: Option<Decimals>,
}

/// The colour a section names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Paint {
    /// One of the eight named colours, as RGB.
    Rgb(u32),
    /// `[ColorN]`: the workbook palette's entry `N + 7`.
    Indexed(u8),
}

impl Paint {
    /// The RGB value the colour displays as, an indexed one read from
    /// `palette` ([`super::theme::indexed`]).
    fn rgb(self, palette: Option<&[u32]>) -> Option<u32> {
        match self {
            Self::Rgb(rgb) => Some(rgb),
            Self::Indexed(index) => super::theme::indexed(palette, usize::from(index)),
        }
    }
}

/// Where a numeric section's decimal places sit in the code, in bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Decimals {
    /// A `General` word, at its span.
    General(usize, usize),
    /// Digit placeholders: just past the last integer one, the decimal
    /// point, and each fraction placeholder.
    Digits {
        after_integer: usize,
        point: Option<usize>,
        fraction: Vec<usize>,
    },
}

/// A section's condition, `[>=100]`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Condition {
    comparison: Comparison,
    value: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Comparison {
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Equal,
    NotEqual,
}

impl Condition {
    /// The implicit second section's magnitude rule, confirmed against all
    /// six comparisons on either side of zero in Excel16 build20430.
    fn fallback_magnitude(self) -> bool {
        match self.comparison {
            Comparison::Less | Comparison::Greater | Comparison::GreaterEqual => self.value <= 0.0,
            Comparison::LessEqual | Comparison::NotEqual => self.value < 0.0,
            Comparison::Equal => false,
        }
    }

    fn holds(self, value: f64) -> bool {
        match self.comparison {
            Comparison::Less => value < self.value,
            Comparison::LessEqual => value <= self.value,
            Comparison::Greater => value > self.value,
            Comparison::GreaterEqual => value >= self.value,
            Comparison::Equal => value == self.value,
            Comparison::NotEqual => value != self.value,
        }
    }
}

/// A digit placeholder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Placeholder {
    /// `0`: a digit, zero where the number has none.
    Zero,
    /// `#`: a digit, nothing where the number has none.
    Hash,
    /// `?`: a digit, a space where the number has none.
    Question,
}

impl Placeholder {
    /// What the placeholder shows where the number has no digit.
    const fn pad(self) -> Option<char> {
        match self {
            Self::Zero => Some('0'),
            Self::Hash => None,
            Self::Question => Some(' '),
        }
    }
}

/// Which part of a number a placeholder spells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Integer,
    Fraction,
    Exponent,
    Numerator,
    Denominator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AmPm {
    /// `AM/PM`, always rendered in uppercase under en-US.
    Full,
    /// `A/P`.
    Letter { am_lower: bool, pm_lower: bool },
}

/// A date or clock part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DatePart {
    Year2,
    Year4,
    /// `m` to `mmmmm`, before it is known whether it is minutes.
    Month(u8),
    Minute(u8),
    Day(u8),
    Hour(u8),
    Second(u8),
    /// `.0` to `.000` after seconds.
    Subsecond(u8),
    AmPm(AmPm),
    ElapsedHours(u8),
    ElapsedMinutes(u8),
    ElapsedSeconds(u8),
}

impl DatePart {
    const fn is_date(self) -> bool {
        matches!(
            self,
            Self::Year2 | Self::Year4 | Self::Month(_) | Self::Day(_)
        )
    }

    const fn is_time(self) -> bool {
        matches!(
            self,
            Self::Minute(_) | Self::Hour(_) | Self::Second(_) | Self::Subsecond(_) | Self::AmPm(_)
        )
    }

    const fn is_elapsed(self) -> bool {
        matches!(
            self,
            Self::ElapsedHours(_) | Self::ElapsedMinutes(_) | Self::ElapsedSeconds(_)
        )
    }

    const fn is_hour(self) -> bool {
        matches!(self, Self::Hour(_) | Self::ElapsedHours(_))
    }

    const fn is_second(self) -> bool {
        matches!(self, Self::Second(_) | Self::ElapsedSeconds(_))
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    /// Text shown as written: quoted, escaped, or a character that is no
    /// placeholder.
    Literal(SmolStr),
    /// `_x`: a space as wide as `x`.
    Space,
    /// `*x`: `x` repeated to fill the cell.
    Fill(char),
    /// `@`: the cell's text.
    Text,
    /// `General`.
    General,
    Digit(Placeholder, Role),
    /// A digit `1` to `9` outside a placeholder: a fixed denominator's, or
    /// a literal.
    Numeral(char),
    Point,
    /// `,` before it is known whether it separates, scales or is a literal.
    Comma,
    Thousands,
    Scale,
    Percent,
    Exponent {
        upper: bool,
        plus: bool,
    },
    Slash,
    Date(DatePart),
}

/// What a section displays a number as.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Shape {
    /// Nothing but literals: the number is not shown.
    Literal,
    /// `@`: a text section.
    Text,
    General,
    Number {
        integer: u16,
        fraction: u16,
        thousands: bool,
        scale: u16,
        percent: u16,
    },
    Scientific {
        integer: u16,
        fraction: u16,
        engineering: bool,
        percent: u16,
    },
    Fraction {
        denominator: Denominator,
    },
    Date {
        hour12: bool,
        subsecond: u8,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Denominator {
    /// At most this many digits.
    Digits(u16),
    /// This denominator.
    Fixed(u32),
}

impl FormatCode {
    /// Read `code`: at most four sections separated by `;`, as Excel
    /// accepts them. `General` in any case, and the empty code, are the
    /// General format.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] at the byte of the first fault: a quote or
    /// a bracket left open, a bracket naming no colour, condition, elapsed
    /// unit or currency, a `\`, `_` or `*` with no character after it, a
    /// fifth section, or a 256th character - Excel takes a code of at most
    /// 255.
    pub fn from_code(code: &str) -> Result<Self> {
        if let Some((at, _)) = code.char_indices().nth(MAX_CODE_LENGTH) {
            return Err(refused(
                at,
                format_smolstr!("expected a code of at most {MAX_CODE_LENGTH} characters"),
            ));
        }
        let (sections, localized) = parse(code)?;
        Ok(Self {
            code: SmolStr::new(code),
            sections,
            localized,
        })
    }

    /// The code a styles part states, read whatever it holds: a code this
    /// reader refuses displays as General and keeps its text, with no
    /// decimals [`Self::with_decimals`] could widen.
    pub(crate) fn from_file(code: &str) -> Self {
        Self::from_code(code).unwrap_or_else(|_| Self {
            code: SmolStr::new(code),
            sections: Arc::from([Section {
                span: (0, code.len()),
                condition: None,
                color: None,
                tokens: Box::new([Token::General]),
                shape: Shape::General,
                decimals: None,
            }]),
            localized: false,
        })
    }

    /// `General`.
    #[must_use]
    pub fn general() -> Self {
        static GENERAL: OnceLock<FormatCode> = OnceLock::new();
        GENERAL
            .get_or_init(|| Self::from_code("General").expect("General is a format code"))
            .clone()
    }

    /// The format ECMA-376 §18.8.30 builds in under `id`, as en-US displays
    /// it: the ids 0 to 22, 37 to 40 and 45 to 49, `None` for any other -
    /// 14 is `m/d/yyyy`, 22 `m/d/yyyy h:mm`.
    #[must_use]
    pub fn builtin(id: u32) -> Option<Self> {
        builtin_code(id).map(Self::from_file)
    }

    /// The code as written.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Whether the code is the General format: `General` in any case, or
    /// the empty code.
    #[must_use]
    pub fn is_general(&self) -> bool {
        self.code.is_empty() || self.code.eq_ignore_ascii_case("General")
    }

    /// Whether the code names a locale's own digits or calendar, which
    /// display as General here.
    pub(crate) const fn is_localized(&self) -> bool {
        self.localized
    }

    /// Whether the code is a text format (`@`): its first section shows the
    /// cell's text, and what is typed into a cell of it stays text.
    pub(crate) fn is_text(&self) -> bool {
        self.sections
            .first()
            .is_some_and(|section| section.shape == Shape::Text)
    }

    /// Whether the first section shows the number as a percentage.
    pub(crate) fn is_percent(&self) -> bool {
        self.sections.first().is_some_and(|section| {
            matches!(
                section.shape,
                Shape::Number { percent, .. } | Shape::Scientific { percent, .. } if percent > 0
            )
        })
    }

    /// What a number under the code is: the one reading of a code's
    /// meaning.
    ///
    /// Only the first section counts. An elapsed unit (`[h]`, `[mm]`,
    /// `[s]`) makes a duration; a year, a day or a month makes a date; an
    /// hour, a minute, a second or `AM/PM` a clock; both a datetime, at the
    /// fraction when its seconds show one (`ss.000`); anything else - a
    /// number, text, General - is General.
    #[must_use]
    pub fn kind(&self) -> NumberFormat {
        let Some(section) = self.sections.first() else {
            return NumberFormat::General;
        };
        if !matches!(section.shape, Shape::Date { .. }) {
            return NumberFormat::General;
        }
        let (mut date, mut time, mut fraction) = (false, false, false);
        for token in section.tokens.iter() {
            if let Token::Date(part) = token {
                if part.is_elapsed() {
                    return NumberFormat::Duration;
                }
                date |= part.is_date();
                time |= part.is_time();
                fraction |= matches!(part, DatePart::Subsecond(_));
            }
        }
        match (date, time) {
            (true, true) if fraction => NumberFormat::DateTimeFraction,
            (true, true) => NumberFormat::DateTime,
            (true, false) => NumberFormat::Date,
            (false, true) => NumberFormat::Time,
            (false, false) => NumberFormat::General,
        }
    }

    /// The code with `delta` more decimal places (fewer when negative) in
    /// every section showing a number, as the ribbon's decimal buttons do:
    /// `0.00` by one is `0.000`, `0%` by one `0.0%`, `#,##0.0` by minus one
    /// `#,##0`, General by one `0.0`; a date, a fraction or text is left as
    /// it is.
    ///
    /// ```
    /// use yggdryl::excel::FormatCode;
    ///
    /// let money = FormatCode::from_code("#,##0.00;[Red](#,##0.00)")?;
    /// assert_eq!(money.with_decimals(1).code(), "#,##0.000;[Red](#,##0.000)");
    /// assert_eq!(FormatCode::general().with_decimals(2).code(), "0.00");
    /// assert_eq!(FormatCode::from_code("0.0%")?.with_decimals(-1).code(), "0%");
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    #[must_use]
    pub fn with_decimals(self, delta: i8) -> Self {
        if delta == 0 {
            return self;
        }
        let mut code = String::with_capacity(self.code.len() + 8);
        let mut changed = false;
        for (at, section) in self.sections.iter().enumerate() {
            if at > 0 {
                code.push(';');
            }
            let Some(text) = self.code.get(section.span.0..section.span.1) else {
                return self;
            };
            let offset = section.span.0;
            match &section.decimals {
                Some(Decimals::General(start, end)) => {
                    changed = true;
                    code.push_str(&text[..start - offset]);
                    code.push('0');
                    if delta > 0 {
                        code.push('.');
                        code.extend(std::iter::repeat_n('0', usize::from(delta.unsigned_abs())));
                    }
                    code.push_str(&text[end - offset..]);
                }
                Some(Decimals::Digits {
                    after_integer,
                    point,
                    fraction,
                }) => {
                    changed = true;
                    let magnitude = usize::from(delta.unsigned_abs());
                    if delta > 0 {
                        let (at, prefix) = match (point, fraction.last()) {
                            (_, Some(last)) => (last + 1, ""),
                            (Some(point), None) => (point + 1, ""),
                            (None, None) => (*after_integer, "."),
                        };
                        code.push_str(&text[..at - offset]);
                        code.push_str(prefix);
                        code.extend(std::iter::repeat_n('0', magnitude));
                        code.push_str(&text[at - offset..]);
                    } else {
                        // The last `magnitude` fraction placeholders go, and
                        // the point with them once none is left.
                        let kept = fraction.len().saturating_sub(magnitude);
                        let mut removed: Vec<usize> = fraction[kept..].to_vec();
                        if kept == 0 {
                            removed.extend(point.iter().copied());
                        }
                        for (index, character) in text.char_indices() {
                            if !removed.contains(&(index + offset)) {
                                code.push(character);
                            }
                        }
                    }
                }
                None => code.push_str(text),
            }
        }
        if !changed {
            return self;
        }
        Self::from_code(&code).unwrap_or(self)
    }

    /// The text `value` displays as under the code, the way Excel's en-US
    /// display writes it.
    ///
    /// A number takes the section its sign or the conditions choose; a
    /// date, time or duration is its serial under `system`; a boolean is
    /// `TRUE` or `FALSE` whatever the code says; text takes the text
    /// section, or is itself when the code has none; null is the empty
    /// text. A date section outside its date system's bounds fills the
    /// cell with `#`; the 1900 system also refuses negative dates. A number
    /// no double holds
    /// (infinite, not a number) is `#NUM!`. A section's `[ColorN]` is the
    /// default palette's colour: a workbook stating its own palette shows
    /// its own ([`Workbook::display_text`](super::Workbook::display_text)).
    #[must_use]
    pub fn render(&self, value: &Scalar, system: DateSystem) -> Rendered {
        self.render_in(value, system, None)
    }

    /// Cell display retains overflow colour and fill; formula TEXT receives
    /// the same render failure as an error instead of confusing it with *#.
    pub(crate) fn render_in(
        &self,
        value: &Scalar,
        system: DateSystem,
        palette: Option<&[u32]>,
    ) -> Rendered {
        self.render_value(value, system, palette, true)
            .unwrap_or_else(RenderFailure::display)
    }

    /// TEXT shares section selection and rendering, without allocating the
    /// shorter General spellings used only by a cell's width-dependent display.
    pub(crate) fn render_formula(
        &self,
        value: &Scalar,
        system: DateSystem,
    ) -> std::result::Result<SmolStr, super::cell::ExcelError> {
        if self.code().is_empty() {
            return Ok(SmolStr::default());
        }
        self.render_value(value, system, None, false)
            .map(|rendered| rendered.text)
            .map_err(|_| super::cell::ExcelError::Value)
    }

    fn render_value(
        &self,
        value: &Scalar,
        system: DateSystem,
        palette: Option<&[u32]>,
        alternatives: bool,
    ) -> std::result::Result<Rendered, RenderFailure> {
        Ok(match value {
            Scalar::Null => Rendered::default(),
            Scalar::Boolean(flag) => Rendered::plain(SmolStr::new_static(if flag.get() {
                "TRUE"
            } else {
                "FALSE"
            })),
            crate::string_scalars!(text) => {
                return self.render_text(text.as_str(), palette, !alternatives);
            }
            _ => match number_of(value, system) {
                Some(number) => return self.render_number(number, system, palette, alternatives),
                None => {
                    return self.render_text(
                        &super::cell::cell_text(value),
                        palette,
                        !alternatives,
                    );
                }
            },
        })
    }

    /// Render a number. A failure retains the display's section colour.
    fn render_number(
        &self,
        value: f64,
        system: DateSystem,
        palette: Option<&[u32]>,
        alternatives: bool,
    ) -> std::result::Result<Rendered, RenderFailure> {
        if self.localized || !value.is_finite() {
            return Ok(general_rendered(value, alternatives));
        }
        let Some((section, signed)) = self.choose(value) else {
            // A text-only format displays numeric values as General; a valid
            // conditional format with no matching numeric section overflows.
            return if self.sections.iter().take(2).any(|s| s.condition.is_some()) {
                Err(RenderFailure { color: None })
            } else {
                Ok(general_rendered(value, alternatives))
            };
        };
        let color = section.color.and_then(|paint| paint.rgb(palette));
        let mut out = Out::new();
        let mut fill = None;
        let negative = signed && value < 0.0;
        let valid = match section.shape {
            Shape::General if is_bare_general(section) => {
                return Ok(Rendered {
                    color,
                    ..general_rendered(if signed { value } else { value.abs() }, alternatives)
                });
            }
            Shape::Text => return Ok(general_rendered(value, alternatives)),
            Shape::Date { .. } => {
                render_date(section, value, system, negative, &mut out, &mut fill)
            }
            _ => render_numeric(section, value.abs(), negative, &mut out, &mut fill),
        };
        if !valid {
            return Err(RenderFailure { color });
        }
        Ok(out.rendered(color, fill))
    }
    /// Text uses the existing section tokens; TEXT alone enforces the formula
    /// cell limit before copying a repeated @ source. Display stays unbounded.
    fn render_text(
        &self,
        text: &str,
        palette: Option<&[u32]>,
        bounded: bool,
    ) -> std::result::Result<Rendered, RenderFailure> {
        let section = match self.sections.len() {
            4 => self.sections.get(3),
            _ => self
                .sections
                .last()
                .filter(|section| section.shape == Shape::Text),
        };
        let Some(section) = section else {
            return Ok(Rendered::plain(SmolStr::new(text)));
        };
        let color = section.color.and_then(|paint| paint.rgb(palette));
        let mut out = Out::new();
        let mut fill = None;
        let source_units = if bounded {
            text.encode_utf16().count()
        } else {
            0
        };
        let mut units = 0;
        for token in section.tokens.iter() {
            let before = out.len();
            match token {
                Token::Text => {
                    if bounded && source_units > super::cell::MAX_CELL_TEXT - units {
                        return Err(RenderFailure { color });
                    }
                    out.push_str(text);
                    units += source_units;
                }
                Token::Space => out.push(' '),
                Token::Fill(character) => set_fill(&mut fill, *character, &out),
                other => write_literal(other, &mut out),
            }
            if bounded && !matches!(token, Token::Text) {
                // A literal is bounded by the 255-character format code. It
                // uses the sole token writer before its units are counted.
                units += out.as_str()[before..].encode_utf16().count();
                if units > super::cell::MAX_CELL_TEXT {
                    return Err(RenderFailure { color });
                }
            }
        }
        Ok(out.rendered(color, fill))
    }

    /// The section a number takes, and whether it shows the number's sign.
    fn choose(&self, value: f64) -> Option<(&Section, bool)> {
        let sections = &self.sections[..];
        let count = match sections.len() {
            4 => 3,
            length
                if sections
                    .last()
                    .is_some_and(|last| last.shape == Shape::Text) =>
            {
                length - 1
            }
            length => length,
        };
        let numeric = &sections[..count];
        let first = numeric.first()?;
        let second = numeric.get(1);
        let conditional =
            first.condition.is_some() || second.is_some_and(|s| s.condition.is_some());
        if conditional {
            // A missing first predicate is Excel's implicit positive slot.
            // A stated predicate owns its sign rule; the unconditional second
            // slot has the complementary rule of the first predicate.
            if first
                .condition
                .map_or(value > 0.0, |condition| condition.holds(value))
            {
                return Some((first, !first.is_negative_only()));
            }
            if let Some(second) =
                second.filter(|second| second.condition.is_some_and(|c| c.holds(value)))
            {
                return Some((second, !second.is_negative_only()));
            }
            return match (first.condition, second.and_then(|second| second.condition)) {
                (_, Some(_)) => numeric.get(2).map(|third| (third, true)),
                (Some(condition), None) => {
                    second.map(|second| (second, !condition.fallback_magnitude()))
                }
                (None, None) => None,
            };
        }
        Some(if count == 1 {
            (first, true)
        } else if value < 0.0 {
            (second?, false)
        } else if count == 3 && value == 0.0 {
            (numeric.get(2)?, true)
        } else {
            (first, true)
        })
    }
}

impl Section {
    /// A matching condition renders a magnitude only when it names a
    /// strictly negative slot. `[<=0]` includes zero and keeps the sign.
    fn is_negative_only(&self) -> bool {
        self.condition
            .is_some_and(|condition| match condition.comparison {
                Comparison::Less => condition.value <= 0.0,
                Comparison::LessEqual | Comparison::Equal => condition.value < 0.0,
                _ => false,
            })
    }
}

/// Whether a General section holds the word and nothing else.
fn is_bare_general(section: &Section) -> bool {
    matches!(&*section.tokens, [Token::General])
}

/// The number `value` is displayed from: a number as itself, a temporal
/// as its serial.
fn number_of(value: &Scalar, system: DateSystem) -> Option<f64> {
    super::cell::number_of(value).or_else(|| {
        system
            .serial_of(value)
            .ok()
            .flatten()
            .map(|(serial, _)| serial)
    })
}

/// A number as a cell's entry spells it, as Excel's formula bar shows it:
/// at most fifteen significant digits, in full from a billionth to below
/// 10^20 (a twenty-digit number typed shows its zeros past the fifteenth)
/// and in scientific notation past either; a percentage times a hundred
/// with its `%`, the point moved rather than multiplied.
pub(crate) fn entry_number(value: f64, percent: bool) -> SmolStr {
    let mut digits = Digits::from_f64(value);
    if percent {
        digits.shift(2);
    }
    let mut out = Out::new();
    if value < 0.0 && !digits.is_zero() {
        out.push('-');
    }
    if digits.is_zero() {
        out.push('0');
    } else if (-9..=20).contains(&digits.point) {
        digits.write_fixed(&mut out);
    } else {
        digits.write_mantissa(&mut out);
        let exponent = digits.point - 1;
        let _ = write!(
            out,
            "E{}{:02}",
            if exponent < 0 { '-' } else { '+' },
            exponent.abs()
        );
    }
    if percent {
        out.push('%');
    }
    out.finish()
}

/// Formula numeric-to-text uses General's twenty-character budget, excluding
/// the sign. It shares the fifteen-digit decimal and notation rules with
/// display formatting, without constructing shorter display alternatives.
pub(crate) fn formula_number(value: f64) -> SmolStr {
    let mut out = Out::new();
    let _ = general(value, 20, &mut out);
    out.finish()
}

/// A number under General, with its narrower spellings.
fn general_rendered(value: f64, alternatives: bool) -> Rendered {
    if !value.is_finite() {
        return Rendered::plain(SmolStr::new_static("#NUM!"));
    }
    let mut out = Out::new();
    if !general(value, GENERAL_WIDTH, &mut out) {
        return Rendered::plain(SmolStr::default());
    }
    let text = out.finish();
    if !alternatives {
        return Rendered::plain(text);
    }
    let mut shorter = Vec::new();
    let sign = usize::from(value < 0.0);
    let mut last = text.len() - sign;
    for width in (1..last).rev() {
        let mut narrower = Out::new();
        if general(value, width, &mut narrower) && narrower.len() - sign < last {
            last = narrower.len() - sign;
            // One list, sized for every width narrower than the text.
            if shorter.is_empty() {
                shorter.reserve_exact(width);
            }
            shorter.push(narrower.finish());
        }
    }
    Rendered {
        text,
        color: None,
        fill: None,
        shorter,
    }
}

/// Write `value` as Excel's General does in `width` characters, a minus
/// sign aside; `false` when not even its scientific spelling fits.
///
/// The number is first rounded to fifteen significant digits. A number of
/// at most `width` integer digits is written in full, its decimals rounded
/// to what fits; one below 0.0001 only when every digit fits, else in
/// scientific notation; anything wider in scientific notation, its
/// mantissa rounded to what fits beside a two-digit exponent.
fn general(value: f64, width: usize, out: &mut Out) -> bool {
    let digits = Digits::from_f64(value.abs());
    if digits.is_zero() {
        out.push('0');
        return true;
    }
    let width = i32::try_from(width).unwrap_or(i32::MAX);
    let exponent = digits.point - 1;
    let negative = value < 0.0;
    // Fixed notation, when the number fits it.
    let fixed = if exponent >= 0 {
        (digits.point <= width).then(|| {
            let decimals = (width - digits.point - 1).max(0);
            let mut rounded = digits;
            rounded.round(decimals);
            rounded
        })
    } else {
        let zeros = -digits.point;
        let available = width - 2 - zeros;
        if available <= 0 {
            None
        } else if exponent >= -4 {
            let mut rounded = digits;
            rounded.round(width - 2);
            Some(rounded)
        } else {
            (i32::from(digits.len) <= available).then_some(digits)
        }
    };
    if let Some(rounded) = fixed.filter(|rounded| rounded.point <= width && !rounded.is_zero()) {
        if negative {
            out.push('-');
        }
        rounded.write_fixed(out);
        return true;
    }
    // Scientific notation.
    let mut rounded = digits;
    for _ in 0..2 {
        let exponent = rounded.point - 1;
        let exponent_digits = if exponent.abs() >= 100 { 3 } else { 2 };
        let mantissa = width - 2 - exponent_digits;
        if mantissa < 1 {
            return false;
        }
        let decimals = if mantissa >= 3 { mantissa - 2 } else { 0 };
        let mut candidate = rounded;
        candidate.round_significant(decimals + 1);
        if candidate.point != rounded.point {
            // Rounding carried into another digit: read the exponent again.
            rounded = candidate;
            continue;
        }
        if negative {
            out.push('-');
        }
        candidate.write_mantissa(out);
        let _ = write!(
            out,
            "E{}{:02}",
            if exponent < 0 { '-' } else { '+' },
            exponent.abs()
        );
        return true;
    }
    false
}

/// Render a number section: `value` is the number's magnitude, `negative`
/// whether a minus sign leads. Refuses a fraction beyond Excel's bounds.
fn render_numeric(
    section: &Section,
    value: f64,
    negative: bool,
    out: &mut Out,
    fill: &mut Option<(char, usize)>,
) -> bool {
    match section.shape {
        Shape::Number {
            fraction,
            thousands,
            scale,
            percent,
            ..
        } => {
            let mut digits = Digits::from_f64(value);
            digits.shift(2 * i32::from(percent) - 3 * i32::from(scale));
            digits.round(i32::from(fraction));
            // Excel16 confirms implicit negative zero loses its sign after
            // scaling and rounding; a literal minus in the code is untouched.
            if negative && !digits.is_zero() {
                out.push('-');
            }
            write_placeholders(section, &digits, None, thousands, out, fill);
        }
        Shape::Scientific {
            integer,
            fraction,
            engineering,
            percent,
        } => {
            let mut digits = Digits::from_f64(value);
            digits.shift(2 * i32::from(percent));
            let (mantissa, exponent) = scientific(digits, integer, fraction, engineering);
            if negative {
                out.push('-');
            }
            write_placeholders(section, &mantissa, Some(exponent), false, out, fill);
        }
        Shape::Fraction { denominator } => {
            return write_fraction(section, value, denominator, negative, out, fill);
        }
        Shape::General => {
            if negative {
                out.push('-');
            }
            for token in section.tokens.iter() {
                match token {
                    Token::General => {
                        let _ = general(value, GENERAL_WIDTH, out);
                    }
                    Token::Space => out.push(' '),
                    Token::Fill(character) => set_fill(fill, *character, out),
                    other => write_literal(other, out),
                }
            }
        }
        Shape::Literal | Shape::Text | Shape::Date { .. } => {
            // A literal-only section still receives an implicit minus when
            // sign selection requires one (Excel16: `"yes"` over -5).
            if negative {
                out.push('-');
            }
            for token in section.tokens.iter() {
                match token {
                    Token::Space => out.push(' '),
                    Token::Fill(character) => set_fill(fill, *character, out),
                    Token::Text => {}
                    other => write_literal(other, out),
                }
            }
        }
    }
    true
}

/// The mantissa and exponent of a scientific section's number.
fn scientific(digits: Digits, integer: u16, fraction: u16, engineering: bool) -> (Digits, i32) {
    if digits.is_zero() {
        return (digits, 0);
    }
    let integer = i32::from(integer);
    let mut value = digits;
    for _ in 0..3 {
        let leading = value.point - 1;
        let exponent = if integer == 0 {
            leading + 1
        } else if engineering {
            leading.div_euclid(integer) * integer
        } else {
            leading - (integer - 1)
        };
        let mut mantissa = value;
        mantissa.shift(-exponent);
        mantissa.round(i32::from(fraction));
        let allowed = integer.max(0);
        if !mantissa.is_zero() && mantissa.point > allowed {
            // Rounding carried past the digits the mantissa holds.
            value = mantissa;
            value.shift(exponent);
            continue;
        }
        return (mantissa, exponent);
    }
    (value, 0)
}

/// The digits one run of placeholders shows - an integer part, an
/// exponent, a numerator - right-aligned: the number's own digits, padded
/// on the left with the zeros its `0` placeholders ask for, the leftmost
/// placeholder taking every digit the run has no place for.
struct Run {
    digits: Digits,
    /// Digits the number has.
    own: i32,
    /// Leading zeros the `0` placeholders add.
    zeros: i32,
    /// How many placeholders the run has.
    placeholders: i32,
}

impl Run {
    /// The run of `role` placeholders in `section` over the integer part
    /// of `digits`.
    fn new(digits: Digits, section: &Section, role: Role) -> Self {
        let own = if digits.is_zero() {
            0
        } else {
            digits.point.max(0)
        };
        let mut placeholders = 0_i32;
        let mut leftmost_zero = None;
        for token in section.tokens.iter() {
            if let Token::Digit(kind, held) = token
                && *held == role
            {
                if *kind == Placeholder::Zero && leftmost_zero.is_none() {
                    leftmost_zero = Some(placeholders);
                }
                placeholders += 1;
            }
        }
        // From the leftmost `0` placeholder to the end, every place shows.
        let shown = leftmost_zero.map_or(0, |at| placeholders - at);
        Self {
            digits,
            own,
            zeros: (shown - own).max(0),
            placeholders,
        }
    }

    /// The digits shown, zeros included.
    const fn total(&self) -> i32 {
        self.own + self.zeros
    }

    /// The digit at index `at` of the digits shown.
    fn digit(&self, at: i32) -> char {
        let digit = if at < self.zeros {
            0
        } else {
            self.digits.digit(at - self.zeros)
        };
        char::from(b'0' + digit)
    }

    /// Write digit `at`, a thousands separator after it where one falls.
    fn push(&self, out: &mut Out, at: i32, thousands: bool) {
        out.push(self.digit(at));
        let after = self.total() - 1 - at;
        if thousands && after > 0 && after % 3 == 0 {
            out.push(',');
        }
    }

    /// Write the placeholder `index` of the run, counted from the left.
    fn write(&self, out: &mut Out, index: i32, kind: Placeholder, thousands: bool) {
        let total = self.total();
        let from_right = self.placeholders - 1 - index;
        if index == 0 && total > self.placeholders {
            for at in 0..=(total - self.placeholders) {
                self.push(out, at, thousands);
            }
        } else if from_right < total {
            self.push(out, total - 1 - from_right, thousands);
        } else if let Some(pad) = kind.pad() {
            out.push(pad);
        }
    }

    /// Write every digit, for a number with no placeholder to hold them.
    fn write_all(&self, out: &mut Out, thousands: bool) {
        for at in 0..self.total() {
            self.push(out, at, thousands);
        }
    }
}

/// Write a number section's tokens: the integer placeholders and the
/// fraction from `digits`, the exponent when given.
fn write_placeholders(
    section: &Section,
    digits: &Digits,
    exponent: Option<i32>,
    thousands: bool,
    out: &mut Out,
    fill: &mut Option<(char, usize)>,
) {
    let mut integers = Run::new(*digits, section, Role::Integer);
    // Excel keeps the complete mantissa width for a scientific zero.
    if exponent.is_some() && digits.is_zero() {
        integers.zeros = integers.placeholders;
    }
    let exponents = Run::new(
        Digits::from_integer(u64::from(exponent.unwrap_or(0).unsigned_abs())),
        section,
        Role::Exponent,
    );
    // The fraction digits shown, and where the last one not zero is.
    let fraction_count = section
        .tokens
        .iter()
        .filter(|token| matches!(token, Token::Digit(_, Role::Fraction)))
        .count();
    let fraction_digit = |at: usize| -> u8 {
        let index = digits.point + i32::try_from(at).unwrap_or(i32::MAX);
        if index < 0 { 0 } else { digits.digit(index) }
    };
    let last_significant = (0..fraction_count)
        .rev()
        .find(|at| fraction_digit(*at) != 0);
    let (mut integer_index, mut exponent_index, mut fraction_index) = (0_i32, 0_i32, 0_usize);
    let mut integers_written = integers.placeholders > 0;
    for token in section.tokens.iter() {
        match token {
            Token::Digit(kind, Role::Integer) => {
                integers.write(out, integer_index, *kind, thousands);
                integer_index += 1;
            }
            Token::Point => {
                if !integers_written {
                    // No integer placeholder: the integer shows before the point.
                    integers.write_all(out, thousands);
                    integers_written = true;
                }
                out.push('.');
            }
            Token::Digit(kind, Role::Fraction) => {
                if last_significant.is_some_and(|last| fraction_index <= last) {
                    out.push(char::from(b'0' + fraction_digit(fraction_index)));
                } else if let Some(pad) = kind.pad() {
                    out.push(pad);
                }
                fraction_index += 1;
            }
            Token::Exponent { upper, plus } => {
                out.push(if *upper { 'E' } else { 'e' });
                match exponent {
                    Some(exponent) if exponent < 0 => out.push('-'),
                    _ if *plus => out.push('+'),
                    _ => {}
                }
            }
            Token::Digit(kind, Role::Exponent) => {
                exponents.write(out, exponent_index, *kind, false);
                exponent_index += 1;
            }
            Token::Percent => out.push('%'),
            Token::Thousands | Token::Scale | Token::Text => {}
            Token::Space => out.push(' '),
            Token::Fill(character) => set_fill(fill, *character, out),
            other => write_literal(other, out),
        }
    }
}

/// Note the fill character at the current end of `out`, once.
fn set_fill(fill: &mut Option<(char, usize)>, character: char, out: &Out) {
    if fill.is_none() {
        *fill = Some((character, out.len()));
    }
}

/// Write a fraction section.
fn write_fraction(
    section: &Section,
    value: f64,
    denominator: Denominator,
    negative: bool,
    out: &mut Out,
    fill: &mut Option<(char, usize)>,
) -> bool {
    let mixed = section
        .tokens
        .iter()
        .any(|token| matches!(token, Token::Digit(_, Role::Integer)));
    let (mut whole, part) = if mixed {
        (value.trunc(), value - value.trunc())
    } else {
        (0.0, value)
    };
    let (mut top, bottom) = match denominator {
        Denominator::Fixed(fixed) => {
            let top = (part * f64::from(fixed)).round();
            // Excel16 applies the numerator bound after rounding, including
            // fixed denominators 1, 2, 8, 100 and either sign at 32767.5.
            if top > f64::from(i16::MAX) {
                return false;
            }
            (top, u64::from(fixed))
        }
        Denominator::Digits(digits) => {
            // A mixed fraction passes only its fractional remainder here.
            if part >= f64::from(i32::MAX) {
                return false;
            }
            let most = 10_u64.pow(u32::from(digits.min(7))) - 1;
            continued_fraction(part, most.max(1))
        }
    };
    let over = bottom as f64;
    if mixed && top >= over {
        whole += (top / over).floor();
        top %= over;
    }
    // With no fraction left, a mixed number shows its whole part and
    // blanks where the fraction would be.
    let blank = mixed && top == 0.0;
    let nothing = whole == 0.0 && top == 0.0;
    if negative && (mixed || !nothing) {
        out.push('-');
    }
    let integers = Run::new(Digits::from_f64(whole), section, Role::Integer);
    let mut numerators = Run::new(Digits::from_f64(top), section, Role::Numerator);
    if top == 0.0 {
        // A zero numerator still shows its zero.
        numerators.zeros = numerators.zeros.max(1);
    }
    let bottom_digits = Digits::from_integer(bottom);
    let bottom_len = bottom_digits.point.max(1);
    let (mut integer_index, mut numerator_index, mut denominator_index) = (0_i32, 0_i32, 0_i32);
    let mut in_fraction = false;
    for token in section.tokens.iter() {
        match token {
            Token::Digit(kind, Role::Integer) => {
                if nothing {
                    // Zero shows as one zero, in the last integer place.
                    if integer_index == integers.placeholders - 1 {
                        out.push('0');
                    }
                } else {
                    integers.write(out, integer_index, *kind, false);
                }
                integer_index += 1;
            }
            Token::Digit(kind, Role::Numerator) => {
                in_fraction = true;
                if blank {
                    out.push(' ');
                } else {
                    numerators.write(out, numerator_index, *kind, false);
                }
                numerator_index += 1;
            }
            Token::Slash => out.push(if blank { ' ' } else { '/' }),
            Token::Digit(kind, Role::Denominator) => {
                if blank {
                    out.push(' ');
                } else if denominator_index < bottom_len {
                    out.push(char::from(b'0' + bottom_digits.digit(denominator_index)));
                } else if let Some(pad) = kind.pad() {
                    out.push(pad);
                }
                denominator_index += 1;
            }
            Token::Numeral(numeral) => out.push(if blank && in_fraction { ' ' } else { *numeral }),
            Token::Space => out.push(' '),
            Token::Fill(character) => set_fill(fill, *character, out),
            Token::Percent => out.push('%'),
            Token::Text => {}
            other => write_literal(other, out),
        }
    }
    true
}

/// The last continued-fraction convergent whose denominator and partial
/// numerator product fit Excel's bounds. Excel does not choose a closer
/// semiconvergent: `# ??/??` displays pi as `3  1/7 `, not `3 14/99`.
/// The caller proves the nonnegative fractional value is below 2^31-1.
fn continued_fraction(value: f64, most: u64) -> (f64, u64) {
    if value <= 0.0 || !value.is_finite() {
        return (0.0, 1);
    }
    let (mut p0, mut q0, mut p1, mut q1) = (0_u64, 1_u64, 1_u64, 0_u64);
    let mut rest = value;
    // Until a proper convergent fits, Excel uses the nearest integer,
    // not the floor term with which the recurrence starts.
    let mut answer = (value.round(), 1);
    for _ in 0..64 {
        let whole = rest.floor();
        if whole > 1e18 {
            break;
        }
        let a = whole as u64;
        let Some(q2) = a
            .checked_mul(q1)
            .and_then(|product| product.checked_add(q0))
        else {
            break;
        };
        if q2 > most {
            break;
        }
        // Excel16 bounds the product before adding p0, not the final
        // numerator: 2147483646.625 renders as 4294967293/2. The independent
        // 64-case probe covers this rule at four different magnitudes.
        let Some(product) = a.checked_mul(p1).filter(|p| *p <= i32::MAX as u64) else {
            break;
        };
        let Some(p2) = product.checked_add(p0) else {
            break;
        };
        (p0, q0, p1, q1) = (p1, q1, p2, q2);
        if q0 != 0 {
            answer = (p1 as f64, q1);
        }
        let fraction = rest - whole;
        if fraction < 1e-12 {
            break;
        }
        rest = 1.0 / fraction;
    }
    answer
}

/// Write a date section over `serial`; `false` when no day spells it.
/// Only 1904 permits a negative serial; its section sees the magnitude and
/// the selection policy supplies an automatic sign when `negative` is true.
fn render_date(
    section: &Section,
    serial: f64,
    system: DateSystem,
    negative: bool,
    out: &mut Out,
    fill: &mut Option<(char, usize)>,
) -> bool {
    let Shape::Date { hour12, subsecond } = section.shape else {
        return false;
    };
    if serial < 0.0 && system == DateSystem::Year1900 {
        return false;
    }
    let serial = serial.abs();
    if !(0.0..(system.last_serial() + 1) as f64).contains(&serial) {
        return false;
    }
    if negative {
        out.push('-');
    }
    let per_second = 10_i64.pow(u32::from(subsecond));
    let per_day = 86_400 * per_second;
    // Rounded to the finest unit shown.
    let units = (serial * per_day as f64).round() as i64;
    let day = units / per_day;
    let clock = units % per_day;
    let seconds_of_day = clock / per_second;
    let below = clock % per_second;
    let (hour, minute, second) = (
        seconds_of_day / 3_600,
        (seconds_of_day / 60) % 60,
        seconds_of_day % 60,
    );
    let calendar = system.civil_day(day);
    let (year, month, day_of_month, weekday) = (
        calendar.year,
        calendar.month,
        calendar.day,
        calendar.weekday,
    );
    for token in section.tokens.iter() {
        match token {
            Token::Date(part) => match *part {
                DatePart::Year2 => {
                    let _ = write!(out, "{:02}", year % 100);
                }
                DatePart::Year4 => {
                    let _ = write!(out, "{year:04}");
                }
                DatePart::Month(width) => {
                    let name = MONTHS[(month - 1) as usize];
                    match width {
                        1 => {
                            let _ = write!(out, "{month}");
                        }
                        2 => {
                            let _ = write!(out, "{month:02}");
                        }
                        3 => out.push_str(&name[..3]),
                        4 => out.push_str(name),
                        _ => out.push_str(&name[..1]),
                    }
                }
                DatePart::Day(width) => {
                    let name = WEEKDAYS[weekday as usize];
                    match width {
                        1 => {
                            let _ = write!(out, "{day_of_month}");
                        }
                        2 => {
                            let _ = write!(out, "{day_of_month:02}");
                        }
                        3 => out.push_str(&name[..3]),
                        _ => out.push_str(name),
                    }
                }
                DatePart::Hour(width) => {
                    let shown = if hour12 {
                        match hour % 12 {
                            0 => 12,
                            other => other,
                        }
                    } else {
                        hour
                    };
                    pad(out, shown, width);
                }
                DatePart::Minute(width) => pad(out, minute, width),
                DatePart::Second(width) => pad(out, second, width),
                DatePart::Subsecond(width) => {
                    out.push('.');
                    let mut rest = below;
                    let mut scale = per_second;
                    for _ in 0..width {
                        scale /= 10;
                        let digit = if scale > 0 { rest / scale } else { 0 };
                        if scale > 0 {
                            rest %= scale;
                        }
                        out.push(char::from(b'0' + digit as u8));
                    }
                }
                DatePart::AmPm(kind) => {
                    let morning = hour < 12;
                    out.push_str(match (kind, morning) {
                        (AmPm::Full, true) => "AM",
                        (AmPm::Full, false) => "PM",
                        (
                            AmPm::Letter {
                                am_lower: false, ..
                            },
                            true,
                        ) => "A",
                        (
                            AmPm::Letter {
                                pm_lower: false, ..
                            },
                            false,
                        ) => "P",
                        (AmPm::Letter { am_lower: true, .. }, true) => "a",
                        (AmPm::Letter { pm_lower: true, .. }, false) => "p",
                    });
                }
                DatePart::ElapsedHours(width) => pad(out, units / (3_600 * per_second), width),
                DatePart::ElapsedMinutes(width) => pad(out, units / (60 * per_second), width),
                DatePart::ElapsedSeconds(width) => pad(out, units / per_second, width),
            },
            Token::Space => out.push(' '),
            Token::Fill(character) => set_fill(&mut *fill, *character, out),
            Token::Text => {}
            other => write_literal(other, out),
        }
    }
    true
}

/// Write `value` with at least `width` digits.
fn pad(out: &mut Out, value: i64, width: u8) {
    if width >= 2 {
        let _ = write!(out, "{value:02}");
    } else {
        let _ = write!(out, "{value}");
    }
}

/// Write a token that shows as written.
fn write_literal(token: &Token, out: &mut Out) {
    match token {
        Token::Literal(text) => out.push_str(text),
        Token::Numeral(character) => out.push(*character),
        Token::Point => out.push('.'),
        Token::Comma => out.push(','),
        Token::Percent => out.push('%'),
        Token::Slash => out.push('/'),
        Token::General => out.push_str("General"),
        Token::Exponent { upper, plus } => {
            out.push(if *upper { 'E' } else { 'e' });
            out.push(if *plus { '+' } else { '-' });
        }
        Token::Digit(kind, _) => out.push(match kind {
            Placeholder::Zero => '0',
            Placeholder::Hash => '#',
            Placeholder::Question => '?',
        }),
        _ => {}
    }
}

/// A non-negative decimal of at most seventeen significant digits,
/// `0.d₁d₂… × 10^point`: the fifteen-digit form Excel rounds a number
/// through, held on the stack.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Digits {
    digits: [u8; 20],
    /// Significant digits held; `0` is zero.
    len: u8,
    /// Where the point is: the number of integer digits when positive.
    point: i32,
}

impl PartialEq for Digits {
    fn eq(&self, other: &Self) -> bool {
        // Rounding/trim deliberately leave unused trailing storage intact.
        // Only the canonical active decimal is a fact of this value.
        self.point == other.point
            && self.digits[..usize::from(self.len)] == other.digits[..usize::from(other.len)]
    }
}

impl Eq for Digits {}

/// Decimal components after the caller has proved their grammar. An entry
/// may contain validated thousands separators; a formula lexeme may not.
pub(crate) struct DecimalParts<'a> {
    pub(crate) integer: &'a str,
    pub(crate) fraction: Option<&'a str>,
    pub(crate) exponent: i64,
}

impl Digits {
    const ZERO: Self = Self {
        digits: [0; 20],
        len: 0,
        point: 0,
    };

    /// `value`'s magnitude rounded to fifteen significant digits.
    pub(crate) fn from_f64(value: f64) -> Self {
        let value = value.abs();
        if value == 0.0 || !value.is_finite() {
            return Self::ZERO;
        }
        let mut buffer = StackText::<32>::new();
        let _ = write!(buffer, "{value:.14e}");
        Self::from_decimal(
            std::str::from_utf8(buffer.as_bytes()).expect("number formatter emits ASCII"),
        )
    }

    /// The magnitude of a lexer-proven decimal, keeping its first fifteen
    /// significant digits without rounding. Exponent saturation preserves
    /// overflow/underflow classification without an input-sized buffer.
    pub(crate) fn from_decimal(text: &str) -> Self {
        let (mantissa, exponent) = match text.find(['e', 'E']) {
            Some(at) => {
                let exponent = &text[at + 1..];
                let exponent = exponent.parse::<i64>().unwrap_or_else(|_| {
                    if exponent.starts_with('-') {
                        i64::MIN
                    } else {
                        i64::MAX
                    }
                });
                (&text[..at], exponent)
            }
            None => (text, 0),
        };
        let (integer, fraction) = match mantissa.split_once('.') {
            Some((integer, fraction)) => (integer, Some(fraction)),
            None => (mantissa, None),
        };
        Self::from_parts(DecimalParts {
            integer,
            fraction,
            exponent,
        })
    }

    /// Assemble a caller-proven decimal without copying its spelling.
    /// Trailing digits beyond the fifteenth still advance the decimal point.
    pub(crate) fn from_parts(parts: DecimalParts<'_>) -> Self {
        let mut digits = Self::ZERO;
        let mut point = 0_i64;
        let mut started = false;
        for byte in parts.integer.bytes().filter(|byte| *byte != b',') {
            debug_assert!(
                byte.is_ascii_digit(),
                "decimal grammar is proven by its caller"
            );
            if !started {
                if byte == b'0' {
                    continue;
                }
                started = true;
            }
            point += 1;
            if digits.len < 15 {
                digits.digits[usize::from(digits.len)] = byte - b'0';
                digits.len += 1;
            }
        }
        if let Some(fraction) = parts.fraction {
            for byte in fraction.bytes() {
                debug_assert!(
                    byte.is_ascii_digit(),
                    "decimal grammar is proven by its caller"
                );
                if !started {
                    if byte == b'0' {
                        point -= 1;
                        continue;
                    }
                    started = true;
                }
                if digits.len < 15 {
                    digits.digits[usize::from(digits.len)] = byte - b'0';
                    digits.len += 1;
                }
            }
        }
        let point = point.saturating_add(parts.exponent);
        digits.point = i32::try_from(point).unwrap_or(if point < 0 { i32::MIN } else { i32::MAX });
        digits.trim();
        digits
    }

    /// `value` exactly.
    fn from_integer(value: u64) -> Self {
        let mut digits = Self::ZERO;
        if value == 0 {
            return digits;
        }
        let mut buffer = StackText::<24>::new();
        let _ = write!(buffer, "{value}");
        for byte in buffer.as_bytes() {
            digits.digits[usize::from(digits.len)] = byte - b'0';
            digits.len += 1;
        }
        digits.point = i32::from(digits.len);
        digits.trim();
        digits
    }

    const fn is_zero(&self) -> bool {
        self.len == 0
    }

    /// The significant digit at `index`, zero past the ones held.
    fn digit(&self, index: i32) -> u8 {
        usize::try_from(index)
            .ok()
            .filter(|index| *index < usize::from(self.len))
            .map_or(0, |index| self.digits[index])
    }

    /// Drop trailing zeros.
    fn trim(&mut self) {
        while self.len > 0 && self.digits[usize::from(self.len) - 1] == 0 {
            self.len -= 1;
        }
        if self.len == 0 {
            self.point = 0;
        }
    }

    /// Multiply by `10^places`.
    fn shift(&mut self, places: i32) {
        if !self.is_zero() {
            self.point += places;
        }
    }

    /// Round half away from zero to `decimals` places after the point.
    pub(crate) fn round(&mut self, decimals: i32) {
        if self.is_zero() {
            return;
        }
        let keep = self.point.saturating_add(decimals);
        if keep >= i32::from(self.len) {
            return;
        }
        if keep < 0 {
            *self = Self::ZERO;
            return;
        }
        let keep = keep as usize;
        let up = self.digits[keep] >= 5;
        self.len = keep as u8;
        if up {
            let mut at = keep;
            loop {
                if at == 0 {
                    // Every digit carried: the number is one more power.
                    self.digits[0] = 1;
                    self.len = 1;
                    self.point += 1;
                    break;
                }
                at -= 1;
                if self.digits[at] == 9 {
                    self.digits[at] = 0;
                } else {
                    self.digits[at] += 1;
                    break;
                }
            }
        }
        self.trim();
    }

    /// Advance to the next nonzero decimal quantum, away from zero.
    /// The caller owns sign; this value holds only the magnitude.
    pub(crate) fn round_away(&mut self, decimals: i32) {
        if self.is_zero() {
            return;
        }
        let keep = self.point.saturating_add(decimals);
        if keep >= i32::from(self.len) {
            return;
        }
        if keep <= 0 {
            self.digits[0] = 1;
            self.len = 1;
            self.point = 1_i32.saturating_sub(decimals);
            return;
        }
        let keep = keep as usize;
        self.len = keep as u8;
        let mut at = keep;
        loop {
            if at == 0 {
                self.digits[0] = 1;
                self.len = 1;
                self.point += 1;
                break;
            }
            at -= 1;
            if self.digits[at] == 9 {
                self.digits[at] = 0;
            } else {
                self.digits[at] += 1;
                break;
            }
        }
        self.trim();
    }

    /// Discard decimal places toward zero without a binary multiply/divide.
    pub(crate) fn truncate(&mut self, decimals: i32) {
        if self.is_zero() {
            return;
        }
        let keep = self.point.saturating_add(decimals);
        if keep >= i32::from(self.len) {
            return;
        }
        if keep <= 0 {
            *self = Self::ZERO;
            return;
        }
        self.len = keep as u8;
        self.trim();
    }

    /// Round half away from zero to `significant` digits.
    fn round_significant(&mut self, significant: i32) {
        let point = self.point;
        self.round(significant - point);
    }

    /// Parse the active decimal without rounding it to a second decimal shape.
    pub(crate) fn as_f64(&self) -> Option<f64> {
        if self.is_zero() {
            return Some(0.0);
        }
        let mut out = Out::new();
        self.write_mantissa(&mut out);
        write!(out, "e{}", self.point.saturating_sub(1)).ok()?;
        out.as_str().parse().ok()
    }

    /// Write the number in fixed notation, its trailing zeros dropped.
    fn write_fixed(&self, out: &mut Out) {
        if self.point <= 0 {
            out.push('0');
        } else {
            for at in 0..self.point {
                out.push(char::from(b'0' + self.digit(at)));
            }
        }
        if i32::from(self.len) > self.point {
            out.push('.');
            for at in self.point.min(0)..0 {
                let _ = at;
                out.push('0');
            }
            for at in self.point.max(0)..i32::from(self.len) {
                out.push(char::from(b'0' + self.digit(at)));
            }
        }
    }

    /// Write the significant digits as a mantissa: `d.ddd`, the point only
    /// when a digit follows it.
    fn write_mantissa(&self, out: &mut Out) {
        out.push(char::from(b'0' + self.digit(0)));
        if self.len > 1 {
            out.push('.');
            for at in 1..i32::from(self.len) {
                out.push(char::from(b'0' + self.digit(at)));
            }
        }
    }
}

/// A short text on the stack.
struct StackText<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> StackText<N> {
    const fn new() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl<const N: usize> fmt::Write for StackText<N> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let end = self.len + text.len();
        if end > N {
            return Err(fmt::Error);
        }
        self.bytes[self.len..end].copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }
}

/// The text a render writes: on the stack, spilled to the heap only past
/// its bound.
struct Out {
    stack: StackText<256>,
    spill: Option<String>,
}

impl Out {
    const fn new() -> Self {
        Self {
            stack: StackText::new(),
            spill: None,
        }
    }

    fn push_str(&mut self, text: &str) {
        if let Some(spill) = self.spill.as_mut() {
            spill.push_str(text);
            return;
        }
        if self.stack.write_str(text).is_err() {
            let mut spill = String::with_capacity(self.stack.len + text.len() + 64);
            spill.push_str(self.as_str());
            spill.push_str(text);
            self.spill = Some(spill);
        }
    }

    fn push(&mut self, character: char) {
        let mut buffer = [0_u8; 4];
        self.push_str(character.encode_utf8(&mut buffer));
    }

    /// Bytes written so far.
    fn len(&self) -> usize {
        self.spill.as_ref().map_or(self.stack.len, String::len)
    }

    fn as_str(&self) -> &str {
        match &self.spill {
            Some(spill) => spill,
            // Only whole `&str`s are ever written.
            None => std::str::from_utf8(self.stack.as_bytes()).unwrap_or_default(),
        }
    }

    fn finish(self) -> SmolStr {
        match self.spill {
            Some(spill) => SmolStr::from(spill),
            None => SmolStr::new(self.as_str()),
        }
    }

    /// The rendered text, the fill's byte offset counted in characters.
    fn rendered(self, color: Option<u32>, fill: Option<(char, usize)>) -> Rendered {
        let fill = fill.map(|(character, at)| {
            let text = self.as_str();
            (
                character,
                text.get(..at).map_or(0, |head| head.chars().count()),
            )
        });
        Rendered {
            text: self.finish(),
            color,
            fill,
            shorter: Vec::new(),
        }
    }
}

impl fmt::Write for Out {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.push_str(text);
        Ok(())
    }
}

/// A refusal of a code at byte `position`.
fn refused(position: usize, reason: SmolStr) -> Error {
    Error::Parse {
        target: "number format",
        position,
        reason,
    }
}

/// Read a code into its sections, and whether it names a locale's digits.
fn parse(code: &str) -> Result<(Arc<[Section]>, bool)> {
    let mut sections: Vec<Section> = Vec::with_capacity(1);
    let mut localized = false;
    if code.is_empty() || code.eq_ignore_ascii_case("General") {
        let section = Section {
            span: (0, code.len()),
            condition: None,
            color: None,
            tokens: Box::new([Token::General]),
            shape: Shape::General,
            decimals: code
                .eq_ignore_ascii_case("General")
                .then_some(Decimals::General(0, code.len())),
        };
        return Ok((Arc::from(vec![section]), false));
    }
    let mut start = 0;
    let bytes = code.as_bytes();
    let mut at = 0;
    let mut builder = SectionBuilder::default();
    while at < bytes.len() {
        let character = code[at..].chars().next().unwrap_or('\u{0}');
        let width = character.len_utf8();
        match character {
            '"' => {
                let Some(end) = code[at + 1..].find('"') else {
                    return Err(refused(
                        at,
                        SmolStr::new_static("expected the closing \" of a quoted text"),
                    ));
                };
                builder.literal(&code[at + 1..at + 1 + end]);
                at += end + 2;
                continue;
            }
            '\\' | '_' | '*' => {
                let Some(next) = code[at + 1..].chars().next() else {
                    return Err(refused(
                        at,
                        format_smolstr!("expected a character after {character}"),
                    ));
                };
                match character {
                    '\\' => builder.literal_char(next),
                    '_' => builder.push(Token::Space, at),
                    _ => builder.push(Token::Fill(next), at),
                }
                at += 1 + next.len_utf8();
                continue;
            }
            '[' => {
                let Some(end) = code[at + 1..].find(']') else {
                    return Err(refused(
                        at,
                        SmolStr::new_static("expected the closing ] of a bracket"),
                    ));
                };
                let content = &code[at + 1..at + 1 + end];
                localized |= builder.bracket(content, at)?;
                at += end + 2;
                continue;
            }
            ';' => {
                if sections.len() == 3 {
                    return Err(refused(
                        at,
                        SmolStr::new_static("expected at most four sections"),
                    ));
                }
                sections.push(std::mem::take(&mut builder).finish((start, at))?);
                start = at + 1;
            }
            '@' => builder.push(Token::Text, at),
            '0' => builder.push(Token::Digit(Placeholder::Zero, Role::Integer), at),
            '#' => builder.push(Token::Digit(Placeholder::Hash, Role::Integer), at),
            '?' => builder.push(Token::Digit(Placeholder::Question, Role::Integer), at),
            '1'..='9' => builder.push(Token::Numeral(character), at),
            '.' => builder.push(Token::Point, at),
            ',' => builder.push(Token::Comma, at),
            '%' => builder.push(Token::Percent, at),
            '/' => builder.push(Token::Slash, at),
            'E' | 'e' if matches!(bytes.get(at + 1), Some(b'+' | b'-')) => {
                builder.push(
                    Token::Exponent {
                        upper: character == 'E',
                        plus: bytes[at + 1] == b'+',
                    },
                    at,
                );
                at += 2;
                continue;
            }
            'G' | 'g' if starts_with_folded(&code[at..], "general") => {
                builder.general = Some((at, at + 7));
                builder.push(Token::General, at);
                at += 7;
                continue;
            }
            'A' | 'a' if starts_with_folded(&code[at..], "am/pm") => {
                builder.push(Token::Date(DatePart::AmPm(AmPm::Full)), at);
                at += 5;
                continue;
            }
            'A' | 'a' if starts_with_folded(&code[at..], "a/p") => {
                builder.push(
                    Token::Date(DatePart::AmPm(AmPm::Letter {
                        am_lower: character == 'a',
                        pm_lower: bytes[at + 2] == b'p',
                    })),
                    at,
                );
                at += 3;
                continue;
            }
            'y' | 'Y' | 'm' | 'M' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' | 'e' | 'E' => {
                let lower = character.to_ascii_lowercase();
                let run = bytes[at..]
                    .iter()
                    .take_while(|byte| byte.to_ascii_lowercase() == lower as u8)
                    .count();
                let run_width = u8::try_from(run.min(255)).unwrap_or(u8::MAX);
                let part = match lower {
                    'y' | 'e' if run <= 2 && lower == 'y' => DatePart::Year2,
                    'y' | 'e' => DatePart::Year4,
                    'm' => DatePart::Month(run_width.min(5)),
                    'd' => DatePart::Day(run_width.min(4)),
                    'h' => DatePart::Hour(run_width.min(2)),
                    _ => DatePart::Second(run_width.min(2)),
                };
                builder.push(Token::Date(part), at);
                at += run;
                continue;
            }
            other => builder.literal_char(other),
        }
        at += width;
    }
    sections.push(builder.finish((start, code.len()))?);
    Ok((Arc::from(sections), localized))
}

/// Whether `text` starts with the lower-case ASCII `prefix`, in any case.
fn starts_with_folded(text: &str, prefix: &str) -> bool {
    text.len() >= prefix.len()
        && text.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// One section as it is read.
#[derive(Default)]
struct SectionBuilder {
    tokens: Vec<Token>,
    /// The byte each token starts at.
    positions: Vec<usize>,
    condition: Option<Condition>,
    color: Option<Paint>,
    general: Option<(usize, usize)>,
    /// `[$-F800]` or `[$-F400]`: the section displays as this code.
    system: Option<&'static str>,
}

impl SectionBuilder {
    fn push(&mut self, token: Token, at: usize) {
        self.tokens.push(token);
        self.positions.push(at);
    }

    fn literal(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Some(Token::Literal(held)) = self.tokens.last_mut() {
            *held = format_smolstr!("{held}{text}");
        } else {
            self.tokens.push(Token::Literal(SmolStr::new(text)));
            self.positions.push(usize::MAX);
        }
    }

    fn literal_char(&mut self, character: char) {
        let mut buffer = [0_u8; 4];
        self.literal(character.encode_utf8(&mut buffer));
    }

    /// Read a bracket's content, answering whether it names a locale's own
    /// digits or calendar.
    fn bracket(&mut self, content: &str, at: usize) -> Result<bool> {
        let folded = content.trim().to_ascii_lowercase();
        if let Some((_, color)) = NAMED_COLORS.iter().find(|(name, _)| *name == folded) {
            self.color.get_or_insert(Paint::Rgb(*color));
            return Ok(false);
        }
        if let Some(number) = folded.strip_prefix("color") {
            return match number.parse::<u8>() {
                Ok(index @ 1..=56) => {
                    self.color.get_or_insert(Paint::Indexed(index + 7));
                    Ok(false)
                }
                _ => Err(refused(
                    at,
                    format_smolstr!(
                        "expected a colour from [Color1] to [Color56], got [{content}]"
                    ),
                )),
            };
        }
        if folded.starts_with(['<', '>', '=']) {
            let (comparison, length) = [
                ("<=", Comparison::LessEqual),
                (">=", Comparison::GreaterEqual),
                ("<>", Comparison::NotEqual),
                ("<", Comparison::Less),
                (">", Comparison::Greater),
                ("=", Comparison::Equal),
            ]
            .into_iter()
            .find(|(operator, _)| folded.starts_with(operator))
            .map(|(operator, comparison)| (comparison, operator.len()))
            .unwrap_or((Comparison::Equal, 1));
            let number = folded[length..].trim();
            return match number.parse::<f64>() {
                Ok(value) if value.is_finite() => {
                    self.condition = Some(Condition { comparison, value });
                    Ok(false)
                }
                _ => Err(refused(
                    at + 1 + length,
                    format_smolstr!("expected a number in the condition [{content}]"),
                )),
            };
        }
        if !folded.is_empty()
            && folded.bytes().all(|byte| byte == folded.as_bytes()[0])
            && matches!(folded.as_bytes()[0], b'h' | b'm' | b's')
        {
            let width = u8::try_from(folded.len().min(2)).unwrap_or(2);
            let part = match folded.as_bytes()[0] {
                b'h' => DatePart::ElapsedHours(width),
                b'm' => DatePart::ElapsedMinutes(width),
                _ => DatePart::ElapsedSeconds(width),
            };
            self.push(Token::Date(part), at);
            return Ok(false);
        }
        if let Some(currency) = content.strip_prefix('$') {
            let (symbol, locale) = currency.split_once('-').unwrap_or((currency, ""));
            self.literal(symbol);
            let locale = u32::from_str_radix(locale.trim(), 16).unwrap_or(0);
            match locale & 0xFFFF {
                0xF800 => self.system = Some(LONG_DATE),
                0xF400 => self.system = Some(LONG_TIME),
                _ => {}
            }
            let digits = locale >> 24;
            let calendar = (locale >> 16) & 0xFF;
            return Ok(digits != 0 || calendar > 1);
        }
        if folded.starts_with("dbnum") || folded.starts_with("natnum") {
            return Ok(true);
        }
        Err(refused(
            at,
            format_smolstr!(
                "expected a colour, a condition, an elapsed unit or a currency in brackets, got \
                 [{content}]"
            ),
        ))
    }

    /// The section, its tokens given their roles.
    fn finish(mut self, span: (usize, usize)) -> Result<Section> {
        if self.tokens.is_empty() && self.condition.is_some() {
            // Excel normalizes `[<0];0` to `[<0]General;0`. Keep the authored
            // bytes but give its implicit General a zero-width edit position.
            self.tokens.push(Token::General);
            self.positions.push(span.1);
            self.general = Some((span.1, span.1));
        }
        if let Some(system) = self.system {
            // The locale's long date or time replaces what the section spells.
            let (mut sections, _) = parse(system)?;
            let replaced = Arc::get_mut(&mut sections)
                .and_then(|sections| sections.first_mut())
                .map(|section| std::mem::take(&mut section.tokens))
                .unwrap_or_default();
            self.tokens = replaced.into_vec();
            self.positions = vec![usize::MAX; self.tokens.len()];
            self.general = None;
        }
        let is_date = self
            .tokens
            .iter()
            .any(|token| matches!(token, Token::Date(_)));
        let has_digits = self
            .tokens
            .iter()
            .any(|token| matches!(token, Token::Digit(..)));
        let has_general = self.tokens.contains(&Token::General);
        let has_text = self.tokens.contains(&Token::Text);
        let (shape, decimals) = if is_date {
            (self.date_shape(), None)
        } else if has_digits {
            self.number_shape()
        } else if has_general {
            (
                Shape::General,
                self.general
                    .map(|(start, end)| Decimals::General(start, end)),
            )
        } else if has_text {
            (Shape::Text, None)
        } else {
            (Shape::Literal, None)
        };
        Ok(Section {
            span,
            condition: self.condition,
            color: self.color,
            tokens: self.tokens.into_boxed_slice(),
            shape,
            decimals,
        })
    }

    /// A date section: months beside a clock read as minutes, `.0` after
    /// seconds as its fraction, everything else a literal.
    fn date_shape(&mut self) -> Shape {
        let parts: Vec<(usize, DatePart)> = self
            .tokens
            .iter()
            .enumerate()
            .filter_map(|(at, token)| match token {
                Token::Date(part) => Some((at, *part)),
                _ => None,
            })
            .collect();
        for (index, (at, part)) in parts.iter().enumerate() {
            if let DatePart::Month(width @ 1..=2) = part {
                let after_hour = index > 0 && parts[index - 1].1.is_hour();
                let before_second = parts
                    .get(index + 1)
                    .is_some_and(|(_, next)| next.is_second());
                if after_hour || before_second {
                    self.tokens[*at] = Token::Date(DatePart::Minute(*width));
                }
            }
        }
        let mut subsecond = 0_u8;
        let mut at = 0;
        let mut last_part: Option<DatePart> = None;
        while at < self.tokens.len() {
            match self.tokens[at].clone() {
                Token::Date(part) => last_part = Some(part),
                Token::Point
                    if last_part.is_some_and(DatePart::is_second)
                        && matches!(
                            self.tokens.get(at + 1),
                            Some(Token::Digit(Placeholder::Zero, _))
                        ) =>
                {
                    let zeros = self.tokens[at + 1..]
                        .iter()
                        .take_while(|token| matches!(token, Token::Digit(Placeholder::Zero, _)))
                        .count();
                    let width = u8::try_from(zeros.min(3)).unwrap_or(3);
                    subsecond = subsecond.max(width);
                    self.tokens.splice(
                        at..at + 1 + zeros.min(3),
                        [Token::Date(DatePart::Subsecond(width))],
                    );
                    self.positions
                        .splice(at..at + 1 + zeros.min(3), [usize::MAX]);
                    last_part = None;
                }
                Token::Digit(kind, _) => {
                    self.tokens[at] = Token::Literal(SmolStr::new_static(match kind {
                        Placeholder::Zero => "0",
                        Placeholder::Hash => "#",
                        Placeholder::Question => "?",
                    }));
                }
                Token::General => self.tokens[at] = Token::Literal(SmolStr::new_static("General")),
                _ => {}
            }
            at += 1;
        }
        let hour12 = self
            .tokens
            .iter()
            .any(|token| matches!(token, Token::Date(DatePart::AmPm(_))));
        Shape::Date { hour12, subsecond }
    }

    /// A number section: its placeholders' roles, its commas read, and
    /// where its decimals are written.
    fn number_shape(&mut self) -> (Shape, Option<Decimals>) {
        let tokens = &mut self.tokens;
        let is_digit = |token: Option<&Token>| matches!(token, Some(Token::Digit(..)));
        // A fraction: a slash with a placeholder before it and a
        // placeholder or a fixed denominator after it, spaces allowed.
        let slash = tokens
            .iter()
            .position(|token| *token == Token::Slash)
            .filter(|at| {
                let before = tokens[..*at]
                    .iter()
                    .rev()
                    .find(|token| !is_space_literal(token));
                let after = tokens[at + 1..]
                    .iter()
                    .find(|token| !is_space_literal(token));
                matches!(before, Some(Token::Digit(..)))
                    && matches!(after, Some(Token::Digit(..) | Token::Numeral(_)))
            });
        let exponent = tokens
            .iter()
            .position(|token| matches!(token, Token::Exponent { .. }));
        let percent = u16::try_from(
            tokens
                .iter()
                .filter(|token| **token == Token::Percent)
                .count(),
        )
        .unwrap_or(u16::MAX);
        if let Some(slash) = slash {
            // Numerator: the run of placeholders right before the slash,
            // spaces between allowed; integer: any placeholders before it.
            let mut numerator_start = slash;
            while numerator_start > 0 && is_space_literal(&tokens[numerator_start - 1]) {
                numerator_start -= 1;
            }
            while numerator_start > 0 && is_digit(tokens.get(numerator_start - 1)) {
                numerator_start -= 1;
            }
            // A denominator starting with a numeral is fixed: its numerals
            // and the zeros among them spell it.
            let first_after = (slash + 1..tokens.len()).find(|at| !is_space_literal(&tokens[*at]));
            let mut fixed: Option<u32> = None;
            if let Some(first) = first_after.filter(|at| matches!(tokens[*at], Token::Numeral(_))) {
                let mut value = 0_u32;
                for token in &mut tokens[first..] {
                    let digit = match token {
                        Token::Numeral(numeral) => *numeral,
                        Token::Digit(Placeholder::Zero, _) => {
                            *token = Token::Numeral('0');
                            '0'
                        }
                        _ => break,
                    };
                    value = value
                        .saturating_mul(10)
                        .saturating_add(digit.to_digit(10).unwrap_or(0));
                }
                fixed = Some(value.max(1));
            }
            let mut denominator_digits = 0_u16;
            for (at, token) in tokens.iter_mut().enumerate() {
                match token {
                    Token::Digit(_, role) if at < numerator_start => *role = Role::Integer,
                    Token::Digit(_, role) if at < slash => *role = Role::Numerator,
                    Token::Digit(_, role) => {
                        *role = Role::Denominator;
                        denominator_digits = denominator_digits.saturating_add(1);
                    }
                    Token::Comma => *token = Token::Literal(SmolStr::new_static(",")),
                    Token::Point => *token = Token::Literal(SmolStr::new_static(".")),
                    _ => {}
                }
            }
            let denominator = match fixed {
                Some(fixed) => Denominator::Fixed(fixed),
                None => Denominator::Digits(denominator_digits.max(1)),
            };
            return (Shape::Fraction { denominator }, None);
        }
        // Commas: between two placeholders a separator, right after one a
        // scale of a thousand, anywhere else a literal.
        let mut thousands = false;
        let mut scale = 0_u16;
        for at in 0..tokens.len() {
            if tokens[at] != Token::Comma {
                continue;
            }
            let mut before = at;
            while before > 0 && matches!(tokens[before - 1], Token::Comma | Token::Scale) {
                before -= 1;
            }
            let after_placeholder = before > 0 && is_digit(tokens.get(before - 1));
            let before_placeholder = is_digit(tokens.get(at + 1));
            let in_integer = !tokens[..at]
                .iter()
                .any(|token| matches!(token, Token::Point | Token::Exponent { .. }));
            tokens[at] = if after_placeholder && before_placeholder && in_integer {
                thousands = true;
                Token::Thousands
            } else if after_placeholder && !before_placeholder {
                scale = scale.saturating_add(1);
                Token::Scale
            } else {
                Token::Literal(SmolStr::new_static(","))
            };
        }
        // Placeholders: integer before the point, fraction after it, the
        // exponent's after `E`; a second point is a literal.
        let mut seen_point = false;
        let (mut integer, mut fraction) = (0_u16, 0_u16);
        let mut after_integer = None;
        let mut point_at = None;
        let mut fraction_at = Vec::new();
        let mut engineering = false;
        for (at, token) in tokens.iter_mut().enumerate() {
            let past_exponent = exponent.is_some_and(|exponent| at > exponent);
            match token {
                Token::Digit(kind, role) => {
                    if past_exponent {
                        *role = Role::Exponent;
                    } else if seen_point {
                        *role = Role::Fraction;
                        fraction = fraction.saturating_add(1);
                        fraction_at.push(self.positions[at]);
                    } else {
                        *role = Role::Integer;
                        integer = integer.saturating_add(1);
                        engineering |= *kind == Placeholder::Hash;
                        after_integer = Some(self.positions[at] + 1);
                    }
                }
                Token::Point if !past_exponent => {
                    if seen_point {
                        *token = Token::Literal(SmolStr::new_static("."));
                    } else {
                        seen_point = true;
                        point_at = Some(self.positions[at]);
                    }
                }
                Token::Point => *token = Token::Literal(SmolStr::new_static(".")),
                Token::Slash => *token = Token::Literal(SmolStr::new_static("/")),
                Token::Numeral(numeral) => *token = Token::Literal(numeral.to_string().into()),
                _ => {}
            }
        }
        let decimals = Some(Decimals::Digits {
            after_integer: after_integer.or(point_at).unwrap_or_else(|| {
                self.positions
                    .iter()
                    .copied()
                    .filter(|position| *position != usize::MAX)
                    .max()
                    .map_or(0, |position| position + 1)
            }),
            point: point_at,
            fraction: fraction_at,
        });
        let shape = if exponent.is_some() {
            Shape::Scientific {
                integer,
                fraction,
                engineering: engineering && integer > 1,
                percent,
            }
        } else {
            Shape::Number {
                integer,
                fraction,
                thousands,
                scale,
                percent,
            }
        };
        (shape, decimals)
    }
}

/// Whether a token is a literal of spaces.
fn is_space_literal(token: &Token) -> bool {
    matches!(token, Token::Literal(text) if !text.is_empty() && text.chars().all(|c| c == ' '))
        || *token == Token::Space
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! Primitive decimal truncation, kept beside the shared Digits owner.

    /// Round a nonnegative binary64 input's fifteen-digit decimal away from zero.
    pub fn rounded_away_magnitude(value: f64, decimals: i32) -> Option<f64> {
        let mut digits = super::Digits::from_f64(value);
        digits.round_away(decimals);
        digits.as_f64()
    }

    /// Truncate a nonnegative binary64 input's fifteen-digit decimal form.
    pub fn truncated_magnitude(value: f64, decimals: i32) -> Option<f64> {
        let mut digits = super::Digits::from_f64(value);
        digits.truncate(decimals);
        digits.as_f64()
    }
}
