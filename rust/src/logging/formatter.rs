//! How a handler spells a record: Python's `%`-style format and its
//! `strftime` date format, each parsed once.

use std::fmt::{self, Write as _};
use std::ops::Range;
use std::str::FromStr;
use std::sync::{Arc, LazyLock};

use smol_str::format_smolstr;

use super::Record;
use crate::timezone::{civil_from_days, days_from_civil};
use crate::{Error, Result, Timezone};

/// How a handler spells a record: a Python `%`-style format over the
/// record's attributes, with the date `asctime` takes in `strftime`
/// directives.
///
/// | Key | Is |
/// | --- | --- |
/// | `name` | the logger's name |
/// | `levelno`, `levelname` | the level's number and name - the one the record states, else the level's own, `Level 15` for an unnamed one |
/// | `message` | the message |
/// | `asctime` | the creation instant as `datefmt` spells it, `2026-10-03 14:05:09,123` when none is given |
/// | `created`, `msecs`, `relativeCreated` | seconds since the Unix epoch, its milliseconds, and milliseconds since the logging tree first answered |
/// | `pathname`, `filename`, `module`, `lineno` | where the record was raised: `(unknown file)` and `0` when it says nothing |
/// | `funcName` | the function a record states - a binding's call site - else `(unknown function)`: Rust names no function |
/// | `caller` | the call site as one token: `function:line` when the record states its function, else the Rust module's last segment or the file's stem and the line - `open:42`, `table:227` - and `-` when nothing is known |
/// | `thread`, `threadName`, `process`, `processName` | the emitting thread's number; the thread a record states, else the name [`set_thread_name`] gave the emitting thread, else the one it was spawned with, else `Thread-N`; the process id and `MainProcess` |
/// | `levelglyph` | the level's [glyph](super::Level::glyph): `·` `•` `!` `✗` `‼` |
/// | `levelcolor`, `dim`, `bold`, `reset` | the styles a colour terminal shows - the level's colour, faint, bold, none - spelled only by [`Self::format_colored_into`] |
///
/// A key takes Python's conversion spec - `%(levelname)-8s`,
/// `%(msecs)03d`, `%(created).3f` - and the spec is checked against the
/// key's kind when the format is parsed, so a format that would fail on
/// every record is refused once, by byte position. The instant is rendered
/// in UTC unless [`Self::with_timezone`] names a zone: a log read on
/// another machine names the same instant.
///
/// ```
/// use yggdryl::logging::{Formatter, Level, Record};
///
/// let formatter = Formatter::from_str("%(levelname)-8s %(name)s: %(message)s")?;
/// let record = Record::new("trades.feed", Level::INFO, &"opened");
/// assert_eq!(formatter.format(&record), "INFO     trades.feed: opened");
///
/// let dated = Formatter::from_str("%(asctime)s %(message)s")?.with_datefmt("%Y-%m-%dT%H:%M:%SZ")?;
/// let record = Record::new("trades", Level::INFO, &"up").with_created(1_700_000_000_123_000_000);
/// assert_eq!(dated.format(&record), "2023-11-14T22:13:20Z up");
///
/// assert!(Formatter::from_str("%(levelname)d").is_err());
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone)]
pub struct Formatter {
    inner: Arc<Compiled>,
}

/// A parsed format: the text it was spelled as, and the parts every record
/// is rendered through.
struct Compiled {
    format: Box<str>,
    parts: Box<[Part]>,
    datefmt: Option<Box<str>>,
    date: Option<Box<[DatePart]>>,
    timezone: Timezone,
    /// Whether a message's later lines start under its first: the terminal
    /// format's layout.
    hanging: bool,
}

/// One piece of a `%`-style format.
#[derive(Clone)]
enum Part {
    /// A run of the format's own text.
    Text(Range<usize>),
    /// `%(key)spec`.
    Field(Key, Spec),
}

/// A record attribute a format names.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Key {
    Name,
    LevelNo,
    LevelName,
    Message,
    AscTime,
    Created,
    Msecs,
    RelativeCreated,
    PathName,
    FileName,
    Module,
    LineNo,
    FuncName,
    Thread,
    ThreadName,
    Process,
    ProcessName,
    Caller,
    LevelGlyph,
    LevelColor,
    Dim,
    Bold,
    Reset,
}

impl Key {
    /// Every key and its spelling, as Python's `LogRecord` names it.
    const ALL: [(&'static str, Self); 23] = [
        ("name", Self::Name),
        ("levelno", Self::LevelNo),
        ("levelname", Self::LevelName),
        ("message", Self::Message),
        ("asctime", Self::AscTime),
        ("created", Self::Created),
        ("msecs", Self::Msecs),
        ("relativeCreated", Self::RelativeCreated),
        ("pathname", Self::PathName),
        ("filename", Self::FileName),
        ("module", Self::Module),
        ("lineno", Self::LineNo),
        ("funcName", Self::FuncName),
        ("thread", Self::Thread),
        ("threadName", Self::ThreadName),
        ("process", Self::Process),
        ("processName", Self::ProcessName),
        ("caller", Self::Caller),
        ("levelglyph", Self::LevelGlyph),
        ("levelcolor", Self::LevelColor),
        ("dim", Self::Dim),
        ("bold", Self::Bold),
        ("reset", Self::Reset),
    ];

    /// The escape sequence a style key turns on in colour; `None` for a key
    /// that is a value.
    fn style(self, level: super::Level) -> Option<&'static str> {
        match self {
            Self::LevelColor => Some(level.color()),
            Self::Dim => Some(super::terminal::DIM),
            Self::Bold => Some(super::terminal::BOLD),
            Self::Reset => Some(super::terminal::RESET),
            _ => None,
        }
    }

    /// Whether the key is a whole number, which `%x`, `%o` and `%c` take.
    const fn is_integer(self) -> bool {
        matches!(
            self,
            Self::LevelNo | Self::LineNo | Self::Thread | Self::Process
        )
    }

    /// Whether the key's value is a number, which takes every conversion a
    /// text value takes and the numeric ones besides.
    const fn is_numeric(self) -> bool {
        matches!(
            self,
            Self::LevelNo
                | Self::Created
                | Self::Msecs
                | Self::RelativeCreated
                | Self::LineNo
                | Self::Thread
                | Self::Process
        )
    }
}

/// A `%`-style conversion spec: its flags, its width and precision, and
/// the conversion.
#[derive(Clone, Copy, Debug, Default)]
struct Spec {
    left: bool,
    zero: bool,
    /// `#`: `0x`/`0X`/`0o` before a hexadecimal or octal number, and the
    /// trailing zeros `%g` drops kept.
    alternate: bool,
    plus: bool,
    space: bool,
    width: usize,
    precision: Option<usize>,
    conversion: Conversion,
}

/// What a value is converted as.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Conversion {
    /// `s`: the value as text.
    #[default]
    Str,
    /// `r`: the value as Python's `repr` spells it.
    Repr,
    /// `a`: the value as Python's `ascii` spells it - `repr` with every
    /// character past ASCII escaped.
    Ascii,
    /// `d`, `i` and `u`: an integer, truncated toward zero.
    Int,
    /// `f` and `F`: a fixed-point number, six places unless a precision says.
    Float,
    /// `x` and `X`: a whole number in hexadecimal, its digits upper case
    /// under `X`.
    Hex { upper: bool },
    /// `o`: a whole number in octal.
    Octal,
    /// `e` and `E`: a number in exponent notation, `1.500000e+03`.
    Exponent { upper: bool },
    /// `g` and `G`: fixed-point or exponent notation, whichever Python's
    /// rule picks for the precision, trailing zeros dropped.
    General { upper: bool },
    /// `c`: the character a whole number is the code point of.
    Char,
}

impl Conversion {
    /// Whether the conversion spells text rather than a number.
    const fn is_textual(self) -> bool {
        matches!(self, Self::Str | Self::Repr | Self::Ascii)
    }

    /// Whether the conversion takes only a whole number.
    const fn is_integral(self) -> bool {
        matches!(self, Self::Hex { .. } | Self::Octal | Self::Char)
    }
}

/// The widest field and the longest precision a spec states: a padded or
/// truncated field past this is no log line's, and the bound keeps a
/// format from costing more than its records.
const MAX_WIDTH: usize = 4096;

/// One piece of a `strftime` date format.
#[derive(Clone, Copy, Debug)]
enum DatePart {
    /// A run of the date format's own text.
    Text(usize, usize),
    /// A separator a composite directive (`%F`, `%T`, `%D`, `%R`) spells.
    Literal(&'static str),
    Year,
    YearOfCentury,
    Century,
    Month,
    Day,
    DaySpaced,
    DayOfYear,
    Hour,
    HourSpaced,
    Hour12,
    Hour12Spaced,
    AmPm,
    AmPmLower,
    Minute,
    Second,
    Micros,
    WeekdayShort,
    WeekdayLong,
    WeekdayMonday,
    WeekdaySunday,
    /// `%U`: the week of the year, its first Sunday starting week one.
    WeekSunday,
    /// `%W`: the week of the year, its first Monday starting week one.
    WeekMonday,
    /// `%V`: the ISO 8601 week.
    IsoWeek,
    /// `%G`: the year the ISO 8601 week belongs to.
    IsoYear,
    /// `%g`: that year's last two digits.
    IsoYearOfCentury,
    MonthShort,
    MonthLong,
    Offset,
    Zone,
    Epoch,
    Percent,
}

impl Formatter {
    /// Python's `BASIC_FORMAT`, for a handler that should read as Python's
    /// `basicConfig` writes: `WARNING:trades.book:crossed`.
    pub const BASIC_FORMAT: &'static str = "%(levelname)s:%(name)s:%(message)s";

    /// The terminal format [`Self::terminal`] spells: the timestamp, the
    /// level's glyph and name in its colour, the thread, the logger in bold
    /// and the call site, then the message - `2026-10-03 14:05:09,123 •
    /// INFO     [main] trades.feed open:42 › opened 3 venues`.
    pub const TERMINAL_FORMAT: &'static str = "%(dim)s%(asctime)s%(reset)s \
        %(levelcolor)s%(levelglyph)s %(levelname)-8s%(reset)s \
        %(dim)s[%(threadName)s]%(reset)s %(bold)s%(name)s%(reset)s \
        %(dim)s%(caller)s ›%(reset)s %(message)s";

    /// The prebuilt terminal formatter, [`Self::TERMINAL_FORMAT`]: the
    /// timestamp as Python spells `asctime`, the level, the thread, the
    /// logger, and the call site as `%(caller)s` reads it - the function
    /// and line a record states, else its Rust module's last segment or its
    /// file's stem and the line. A message's lines after the first start
    /// under the first, so a multi-line message stays one block. A handler
    /// writing to a colour terminal shows its styles; anywhere else the
    /// same line is plain text. The last resort and `basic_config` use it
    /// unless told otherwise.
    ///
    /// ```
    /// use yggdryl::logging::{Formatter, Level, Record};
    ///
    /// let record = Record::new("trades.feed", Level::WARNING, &"crossed\nat 101.5")
    ///     .with_location("src/feed.rs", 42)
    ///     .with_function("open")
    ///     .with_thread("main")
    ///     .with_created(1_700_000_000_123_000_000);
    /// let line = Formatter::terminal().format(&record);
    /// let [first, second] = line.lines().collect::<Vec<_>>()[..] else { panic!("two lines") };
    /// assert_eq!(first, "2023-11-14 22:13:20,123 ! WARNING  [main] trades.feed open:42 › crossed");
    /// assert_eq!(second.trim_start(), "at 101.5");
    /// assert_eq!(second.len() - "at 101.5".len(), first.find("crossed").unwrap() - 2);
    /// let mut colored = String::new();
    /// Formatter::terminal().format_colored_into(&record, &mut colored);
    /// assert!(colored.starts_with("\x1b[2m2023-11-14 22:13:20,123\x1b[0m \x1b[33m! WARNING "));
    /// ```
    pub fn terminal() -> Self {
        Self::terminal_ref().clone()
    }

    /// The one terminal formatter the process builds.
    pub(super) fn terminal_ref() -> &'static Self {
        static TERMINAL: LazyLock<Formatter> = LazyLock::new(|| {
            let formatter =
                Formatter::from_str(Formatter::TERMINAL_FORMAT).expect("the terminal format");
            let compiled = &formatter.inner;
            Formatter {
                inner: Arc::new(Compiled {
                    format: compiled.format.clone(),
                    parts: compiled.parts.clone(),
                    datefmt: compiled.datefmt.clone(),
                    date: compiled.date.clone(),
                    timezone: compiled.timezone,
                    hanging: true,
                }),
            }
        });
        &TERMINAL
    }

    /// Parses `format`, a Python `%`-style format naming at least one key.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] at the byte that does not read: an unknown
    /// key, an unclosed `%(`, a `%` naming no key, an unknown conversion, a
    /// conversion a key's kind does not take - a number's on text, `x`, `o`
    /// or `c` on a fraction - a width or a precision past 4096, or a format
    /// naming no key at all.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(format: &str) -> Result<Self> {
        <Self as FromStr>::from_str(format)
    }

    /// The formatter spelling `asctime` as `datefmt` - `strftime`
    /// directives `%Y %y %C %G %g %m %d %e %j %U %W %V %H %k %I %l %p %P %M
    /// %S %f %a %A %u %w %b %B %h %z %Z %s %c %x %X %F %T %D %R %r %n %t %%`,
    /// the C locale's spellings Python's `time.strftime` writes - with no
    /// milliseconds appended, as in Python.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] at a directive outside that list.
    pub fn with_datefmt(self, datefmt: &str) -> Result<Self> {
        let compiled = &self.inner;
        if datefmt.is_empty() {
            // Python reads an empty date format as none at all.
            return Ok(Self {
                inner: Arc::new(Compiled {
                    format: compiled.format.clone(),
                    parts: compiled.parts.clone(),
                    datefmt: None,
                    date: None,
                    timezone: compiled.timezone,
                    hanging: compiled.hanging,
                }),
            });
        }
        let date = parse_datefmt(datefmt)?;
        Ok(Self {
            inner: Arc::new(Compiled {
                format: compiled.format.clone(),
                parts: compiled.parts.clone(),
                datefmt: Some(datefmt.into()),
                date: Some(date),
                timezone: compiled.timezone,
                hanging: compiled.hanging,
            }),
        })
    }

    /// The formatter rendering instants in `timezone` rather than UTC; a
    /// zone with no rules renders as UTC.
    #[must_use]
    pub fn with_timezone(self, timezone: Timezone) -> Self {
        let compiled = &self.inner;
        Self {
            inner: Arc::new(Compiled {
                format: compiled.format.clone(),
                parts: compiled.parts.clone(),
                datefmt: compiled.datefmt.clone(),
                date: compiled.date.clone(),
                timezone,
                hanging: compiled.hanging,
            }),
        }
    }

    /// The format, as it was spelled.
    pub fn as_str(&self) -> &str {
        &self.inner.format
    }

    /// The date format `asctime` is spelled in, `None` for Python's default.
    pub fn datefmt(&self) -> Option<&str> {
        self.inner.datefmt.as_deref()
    }

    /// The zone instants are rendered in.
    pub fn timezone(&self) -> Timezone {
        self.inner.timezone
    }

    /// `record` as this format spells it.
    pub fn format(&self, record: &Record<'_>) -> String {
        let mut line = String::new();
        self.format_into(record, &mut line);
        line
    }

    /// Appends `record` as this format spells it to `line`, allocating only
    /// what `line` must grow by. The style keys - `levelcolor`, `dim`,
    /// `bold`, `reset` - spell nothing.
    pub fn format_into(&self, record: &Record<'_>, line: &mut String) {
        self.inner.render(record, line, false);
    }

    /// Appends `record` to `line` as a colour terminal shows it: the style
    /// keys spell their escape sequences, `levelcolor` the record level's.
    pub fn format_colored_into(&self, record: &Record<'_>, line: &mut String) {
        self.inner.render(record, line, true);
    }
}

impl Default for Formatter {
    /// `%(message)s`: the message alone, Python's default formatter.
    fn default() -> Self {
        static DEFAULT: LazyLock<Formatter> =
            LazyLock::new(|| Formatter::from_str("%(message)s").expect("the default format"));
        DEFAULT.clone()
    }
}

impl FromStr for Formatter {
    type Err = Error;

    fn from_str(format: &str) -> Result<Self> {
        Ok(Self {
            inner: Arc::new(Compiled {
                format: format.into(),
                parts: parse_format(format)?,
                datefmt: None,
                date: None,
                timezone: Timezone::UTC,
                hanging: false,
            }),
        })
    }
}

impl PartialEq for Formatter {
    fn eq(&self, other: &Self) -> bool {
        self.inner.format == other.inner.format
            && self.inner.datefmt == other.inner.datefmt
            && self.inner.timezone == other.inner.timezone
            && self.inner.hanging == other.inner.hanging
    }
}

impl Eq for Formatter {}

impl fmt::Debug for Formatter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Formatter")
            .field("format", &self.as_str())
            .field("datefmt", &self.datefmt())
            .field("timezone", &self.timezone().as_str())
            .finish()
    }
}

impl fmt::Display for Formatter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

fn format_error(position: usize, reason: impl fmt::Display) -> Error {
    Error::Parse {
        target: "log format",
        position,
        reason: format_smolstr!("{reason}"),
    }
}

/// Parses a `%`-style format into its parts.
fn parse_format(format: &str) -> Result<Box<[Part]>> {
    let bytes = format.as_bytes();
    let mut parts = Vec::new();
    let mut text = 0;
    let mut at = 0;
    let mut fields = 0;
    while at < bytes.len() {
        if bytes[at] != b'%' {
            at += 1;
            continue;
        }
        if text < at {
            parts.push(Part::Text(text..at));
        }
        let start = at;
        at += 1;
        match bytes.get(at) {
            Some(b'%') => {
                parts.push(Part::Text(at..at + 1));
                at += 1;
            }
            Some(b'(') => {
                let close = format[at..]
                    .find(')')
                    .map(|offset| at + offset)
                    .ok_or_else(|| format_error(start, "expected ')' closing the key"))?;
                let name = &format[at + 1..close];
                let key = Key::ALL
                    .iter()
                    .find(|(spelling, _)| *spelling == name)
                    .map(|(_, key)| *key)
                    .ok_or_else(|| {
                        format_error(
                            at + 1,
                            format_args!(
                                "expected one of {}, got {name:?}",
                                Key::ALL.map(|(spelling, _)| spelling).join(", ")
                            ),
                        )
                    })?;
                let (spec, next) = parse_spec(format, close + 1)?;
                if !key.is_numeric() && !spec.conversion.is_textual() {
                    return Err(format_error(
                        next - 1,
                        format_args!("{name} is text and takes the s, r or a conversion"),
                    ));
                }
                if !key.is_integer() && spec.conversion.is_integral() {
                    return Err(format_error(
                        next - 1,
                        format_args!(
                            "{name} is a fraction and takes the s, r, a, d, i, u, f, F, e, \
                             E, g or G conversion"
                        ),
                    ));
                }
                parts.push(Part::Field(key, spec));
                fields += 1;
                at = next;
            }
            _ => return Err(format_error(start, "expected '(' or '%' after '%'")),
        }
        text = at;
    }
    if text < bytes.len() {
        parts.push(Part::Text(text..bytes.len()));
    }
    if fields == 0 {
        return Err(format_error(
            0,
            "expected a format naming at least one %(key)",
        ));
    }
    Ok(parts.into_boxed_slice())
}

/// Parses the flags, width, precision and conversion after `%(key)`,
/// answering the spec and the byte after it.
fn parse_spec(format: &str, mut at: usize) -> Result<(Spec, usize)> {
    let bytes = format.as_bytes();
    let mut spec = Spec::default();
    while let Some(flag) = bytes.get(at) {
        match flag {
            b'-' => spec.left = true,
            b'0' => spec.zero = true,
            b'+' => spec.plus = true,
            b' ' => spec.space = true,
            b'#' => spec.alternate = true,
            _ => break,
        }
        at += 1;
    }
    let digits = |at: &mut usize, what: &str| {
        let start = *at;
        while bytes.get(*at).is_some_and(u8::is_ascii_digit) {
            *at += 1;
        }
        if start == *at {
            return Ok(0);
        }
        format[start..*at]
            .parse::<usize>()
            .ok()
            .filter(|digits| *digits <= MAX_WIDTH)
            .ok_or_else(|| {
                format_error(
                    start,
                    format_args!(
                        "expected a {what} of at most {MAX_WIDTH}, got {}",
                        &format[start..*at]
                    ),
                )
            })
    };
    spec.width = digits(&mut at, "width")?;
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        spec.precision = Some(digits(&mut at, "precision")?);
    }
    if matches!(bytes.get(at), Some(b'h' | b'l' | b'L')) {
        at += 1;
    }
    spec.conversion = match bytes.get(at) {
        Some(b's') => Conversion::Str,
        Some(b'r') => Conversion::Repr,
        Some(b'a') => Conversion::Ascii,
        Some(b'd' | b'i' | b'u') => Conversion::Int,
        Some(b'f' | b'F') => Conversion::Float,
        Some(b'x') => Conversion::Hex { upper: false },
        Some(b'X') => Conversion::Hex { upper: true },
        Some(b'o') => Conversion::Octal,
        Some(b'e') => Conversion::Exponent { upper: false },
        Some(b'E') => Conversion::Exponent { upper: true },
        Some(b'g') => Conversion::General { upper: false },
        Some(b'G') => Conversion::General { upper: true },
        Some(b'c') => Conversion::Char,
        Some(other) => {
            return Err(format_error(
                at,
                format_args!(
                    "expected one of the conversions s, r, a, d, i, u, o, x, X, e, E, f, \
                     F, g, G, c, got {:?}",
                    char::from(*other)
                ),
            ));
        }
        None => return Err(format_error(at, "expected a conversion after the key")),
    };
    Ok((spec, at + 1))
}

/// Parses a `strftime` date format into its parts.
fn parse_datefmt(datefmt: &str) -> Result<Box<[DatePart]>> {
    let bytes = datefmt.as_bytes();
    let mut parts = Vec::new();
    let mut text = 0;
    let mut at = 0;
    let date_error = |position: usize, reason: String| Error::Parse {
        target: "log date format",
        position,
        reason: reason.into(),
    };
    while at < bytes.len() {
        if bytes[at] != b'%' {
            at += 1;
            continue;
        }
        if text < at {
            parts.push(DatePart::Text(text, at));
        }
        let Some(&directive) = bytes.get(at + 1) else {
            return Err(date_error(at, "expected a directive after '%'".to_owned()));
        };
        let expanded: &[DatePart] = match directive {
            b'Y' => &[DatePart::Year],
            b'y' => &[DatePart::YearOfCentury],
            b'C' => &[DatePart::Century],
            b'm' => &[DatePart::Month],
            b'd' => &[DatePart::Day],
            b'e' => &[DatePart::DaySpaced],
            b'j' => &[DatePart::DayOfYear],
            b'H' => &[DatePart::Hour],
            b'k' => &[DatePart::HourSpaced],
            b'I' => &[DatePart::Hour12],
            b'l' => &[DatePart::Hour12Spaced],
            b'p' => &[DatePart::AmPm],
            b'P' => &[DatePart::AmPmLower],
            b'M' => &[DatePart::Minute],
            b'S' => &[DatePart::Second],
            b'f' => &[DatePart::Micros],
            b'a' => &[DatePart::WeekdayShort],
            b'A' => &[DatePart::WeekdayLong],
            b'u' => &[DatePart::WeekdayMonday],
            b'w' => &[DatePart::WeekdaySunday],
            b'U' => &[DatePart::WeekSunday],
            b'W' => &[DatePart::WeekMonday],
            b'V' => &[DatePart::IsoWeek],
            b'G' => &[DatePart::IsoYear],
            b'g' => &[DatePart::IsoYearOfCentury],
            b'n' => &[DatePart::Literal("\n")],
            b't' => &[DatePart::Literal("\t")],
            // The C locale's spellings, which Python's `time.strftime` writes.
            b'c' => &[
                DatePart::WeekdayShort,
                DatePart::Literal(" "),
                DatePart::MonthShort,
                DatePart::Literal(" "),
                DatePart::DaySpaced,
                DatePart::Literal(" "),
                DatePart::Hour,
                DatePart::Literal(":"),
                DatePart::Minute,
                DatePart::Literal(":"),
                DatePart::Second,
                DatePart::Literal(" "),
                DatePart::Year,
            ],
            b'x' => &[
                DatePart::Month,
                DatePart::Literal("/"),
                DatePart::Day,
                DatePart::Literal("/"),
                DatePart::YearOfCentury,
            ],
            b'X' => &[
                DatePart::Hour,
                DatePart::Literal(":"),
                DatePart::Minute,
                DatePart::Literal(":"),
                DatePart::Second,
            ],
            b'r' => &[
                DatePart::Hour12,
                DatePart::Literal(":"),
                DatePart::Minute,
                DatePart::Literal(":"),
                DatePart::Second,
                DatePart::Literal(" "),
                DatePart::AmPm,
            ],
            b'b' | b'h' => &[DatePart::MonthShort],
            b'B' => &[DatePart::MonthLong],
            b'z' => &[DatePart::Offset],
            b'Z' => &[DatePart::Zone],
            b's' => &[DatePart::Epoch],
            b'%' => &[DatePart::Percent],
            b'F' => &[
                DatePart::Year,
                DatePart::Literal("-"),
                DatePart::Month,
                DatePart::Literal("-"),
                DatePart::Day,
            ],
            b'T' => &[
                DatePart::Hour,
                DatePart::Literal(":"),
                DatePart::Minute,
                DatePart::Literal(":"),
                DatePart::Second,
            ],
            b'D' => &[
                DatePart::Month,
                DatePart::Literal("/"),
                DatePart::Day,
                DatePart::Literal("/"),
                DatePart::YearOfCentury,
            ],
            b'R' => &[DatePart::Hour, DatePart::Literal(":"), DatePart::Minute],
            other => {
                return Err(date_error(
                    at,
                    format!(
                        "expected one of the directives %Y %y %C %G %g %m %d %e %j %U %W %V \
                         %H %k %I %l %p %P %M %S %f %a %A %u %w %b %B %h %z %Z %s %c %x %X \
                         %F %T %D %R %r %n %t %%, got %{}",
                        char::from(other)
                    ),
                ));
            }
        };
        parts.extend_from_slice(expanded);
        at += 2;
        text = at;
    }
    if text < bytes.len() {
        parts.push(DatePart::Text(text, bytes.len()));
    }
    Ok(parts.into_boxed_slice())
}

/// The instant a record names, read once in the formatter's zone.
struct Civil {
    epoch: i64,
    year: i32,
    month: u32,
    day: u32,
    day_of_year: i64,
    weekday: u32,
    hour: u32,
    minute: u32,
    second: u32,
    nanos: u32,
    offset: i32,
}

impl Civil {
    /// The ISO 8601 year and week: the week holding the year's first
    /// Thursday is week one, so early January can belong to the year
    /// before and late December to the year after.
    fn iso_week(&self) -> (i32, i64) {
        let days_in = |year: i32| {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                366
            } else {
                365
            }
        };
        // The day of the year, from zero, of this week's Thursday.
        let monday_based = i64::from((self.weekday + 6) % 7);
        let thursday = self.day_of_year - 1 - monday_based + 3;
        let (year, thursday) = if thursday < 0 {
            (self.year - 1, thursday + days_in(self.year - 1))
        } else if thursday >= days_in(self.year) {
            (self.year + 1, thursday - days_in(self.year))
        } else {
            (self.year, thursday)
        };
        (year, thursday / 7 + 1)
    }

    fn at(created: i64, timezone: Timezone) -> Self {
        let epoch = created.div_euclid(1_000_000_000);
        let nanos = u32::try_from(created.rem_euclid(1_000_000_000)).unwrap_or(0);
        let offset = timezone.offset_at(epoch).unwrap_or(0);
        let local = epoch.saturating_add(i64::from(offset));
        let days = local.div_euclid(86_400);
        let clock = u32::try_from(local.rem_euclid(86_400)).unwrap_or(0);
        let (year, month, day) = civil_from_days(days);
        Self {
            epoch,
            year,
            month,
            day,
            day_of_year: days - days_from_civil(year, 1, 1) + 1,
            weekday: u32::try_from((days + 4).rem_euclid(7)).unwrap_or(0),
            hour: clock / 3_600,
            minute: clock / 60 % 60,
            second: clock % 60,
            nanos,
            offset,
        }
    }
}

const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
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

impl Compiled {
    /// Appends `record`, its styles spelled when `colored`.
    fn render(&self, record: &Record<'_>, line: &mut String, colored: bool) {
        let origin = line.len();
        for part in &*self.parts {
            match part {
                Part::Text(range) => line.push_str(&self.format[range.clone()]),
                Part::Field(key, spec) => match key.style(record.level()) {
                    Some(style) => {
                        if colored {
                            line.push_str(style);
                        }
                    }
                    None => self.field(*key, *spec, record, line, origin),
                },
            }
        }
    }

    /// Appends one key of `record`, converted and padded as `spec` says;
    /// `origin` is where the record's line starts, which a hanging message
    /// aligns its later lines by.
    fn field(&self, key: Key, spec: Spec, record: &Record<'_>, line: &mut String, origin: usize) {
        let start = line.len();
        match key {
            Key::LevelNo => return number(line, Number::Int(record.level().get().into()), spec),
            Key::LineNo => {
                return number(line, Number::Int(record.line().unwrap_or(0).into()), spec);
            }
            Key::Process => return number(line, Number::Int(std::process::id().into()), spec),
            Key::Thread => {
                let thread = i64::try_from(thread_number()).unwrap_or(i64::MAX);
                return number(line, Number::Int(thread), spec);
            }
            #[allow(clippy::cast_precision_loss)]
            Key::Created => {
                return number(line, Number::Float(record.created() as f64 / 1e9), spec);
            }
            Key::Msecs => {
                let msecs = record.created().rem_euclid(1_000_000_000) / 1_000_000;
                #[allow(clippy::cast_precision_loss)]
                return number(line, Number::Float(msecs as f64), spec);
            }
            Key::RelativeCreated => {
                // Python's start precedes every record; this one is read when
                // the tree first answers, so a record made before is at zero.
                let elapsed = record.created().saturating_sub(*super::START).max(0);
                #[allow(clippy::cast_precision_loss)]
                return number(line, Number::Float(elapsed as f64 / 1e6), spec);
            }
            Key::Name => line.push_str(record.name()),
            Key::LevelName => match record.level_name().or_else(|| record.level().name()) {
                Some(name) => line.push_str(name),
                None => {
                    let _ = write!(line, "{}", record.level());
                }
            },
            Key::Message => {
                if self.hanging {
                    let _ = write!(
                        Hanging {
                            line,
                            prefix: origin..start,
                            column: None,
                            indent: false,
                        },
                        "{}",
                        record.message()
                    );
                } else {
                    let _ = write!(line, "{}", record.message());
                }
            }
            Key::AscTime => self.asctime(record.created(), line),
            Key::PathName => line.push_str(record.file().unwrap_or(UNKNOWN_FILE)),
            Key::FileName => line.push_str(file_name(record.file())),
            Key::Module => {
                let name = file_name(record.file());
                line.push_str(name.rsplit_once('.').map_or(name, |(stem, _)| stem));
            }
            Key::FuncName => line.push_str(record.function().unwrap_or(UNKNOWN_FUNCTION)),
            Key::ThreadName => match record.thread() {
                Some(name) => line.push_str(name),
                None => thread_name(line),
            },
            Key::Caller => {
                let place = record
                    .function()
                    .or_else(|| {
                        record
                            .module_path()
                            .map(|path| path.rsplit("::").next().unwrap_or(path))
                    })
                    .or_else(|| {
                        record.file().map(|file| {
                            let name = file_name(Some(file));
                            name.rsplit_once('.').map_or(name, |(stem, _)| stem)
                        })
                    });
                match (place, record.line()) {
                    (Some(place), Some(at)) => {
                        let _ = write!(line, "{place}:{at}");
                    }
                    (Some(place), None) => line.push_str(place),
                    (None, _) => line.push('-'),
                }
            }
            Key::ProcessName => line.push_str("MainProcess"),
            Key::LevelGlyph => line.push_str(record.level().glyph()),
            Key::LevelColor | Key::Dim | Key::Bold | Key::Reset => {}
        }
        if matches!(spec.conversion, Conversion::Repr | Conversion::Ascii) {
            quoted(line, start, spec.conversion == Conversion::Ascii);
        }
        if let Some(precision) = spec.precision
            && let Some((cut, _)) = line[start..].char_indices().nth(precision)
        {
            line.truncate(start + cut);
        }
        pad(line, start, 0, spec, false);
    }

    /// Appends the instant `created` names, as `datefmt` spells it or as
    /// Python's default `2026-10-03 14:05:09,123`.
    fn asctime(&self, created: i64, line: &mut String) {
        let civil = Civil::at(created, self.timezone);
        let Some(date) = &self.date else {
            let _ = write!(
                line,
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02},{:03}",
                civil.year,
                civil.month,
                civil.day,
                civil.hour,
                civil.minute,
                civil.second,
                civil.nanos / 1_000_000
            );
            return;
        };
        let datefmt = self.datefmt.as_deref().unwrap_or_default();
        for part in &**date {
            let _ = match *part {
                DatePart::Literal(text) => {
                    line.push_str(text);
                    Ok(())
                }
                DatePart::Text(start, end) => {
                    line.push_str(&datefmt[start..end]);
                    Ok(())
                }
                DatePart::Year => write!(line, "{:04}", civil.year),
                DatePart::YearOfCentury => write!(line, "{:02}", civil.year.rem_euclid(100)),
                DatePart::Century => write!(line, "{:02}", civil.year.div_euclid(100)),
                DatePart::Month => write!(line, "{:02}", civil.month),
                DatePart::Day => write!(line, "{:02}", civil.day),
                DatePart::DaySpaced => write!(line, "{:2}", civil.day),
                DatePart::DayOfYear => write!(line, "{:03}", civil.day_of_year),
                DatePart::Hour => write!(line, "{:02}", civil.hour),
                DatePart::HourSpaced => write!(line, "{:2}", civil.hour),
                DatePart::Hour12 => write!(line, "{:02}", (civil.hour + 11) % 12 + 1),
                DatePart::Hour12Spaced => write!(line, "{:2}", (civil.hour + 11) % 12 + 1),
                DatePart::AmPm => {
                    line.push_str(if civil.hour < 12 { "AM" } else { "PM" });
                    Ok(())
                }
                DatePart::AmPmLower => {
                    line.push_str(if civil.hour < 12 { "am" } else { "pm" });
                    Ok(())
                }
                DatePart::WeekSunday => write!(
                    line,
                    "{:02}",
                    (civil.day_of_year - 1 + 7 - i64::from(civil.weekday)) / 7
                ),
                DatePart::WeekMonday => write!(
                    line,
                    "{:02}",
                    (civil.day_of_year - 1 + 7 - i64::from((civil.weekday + 6) % 7)) / 7
                ),
                DatePart::IsoWeek => write!(line, "{:02}", civil.iso_week().1),
                DatePart::IsoYear => write!(line, "{:04}", civil.iso_week().0),
                DatePart::IsoYearOfCentury => {
                    write!(line, "{:02}", civil.iso_week().0.rem_euclid(100))
                }
                DatePart::Minute => write!(line, "{:02}", civil.minute),
                DatePart::Second => write!(line, "{:02}", civil.second),
                DatePart::Micros => write!(line, "{:06}", civil.nanos / 1_000),
                DatePart::WeekdayShort => {
                    line.push_str(&WEEKDAYS[civil.weekday as usize][..3]);
                    Ok(())
                }
                DatePart::WeekdayLong => {
                    line.push_str(WEEKDAYS[civil.weekday as usize]);
                    Ok(())
                }
                DatePart::WeekdayMonday => write!(line, "{}", (civil.weekday + 6) % 7 + 1),
                DatePart::WeekdaySunday => write!(line, "{}", civil.weekday),
                DatePart::MonthShort => {
                    line.push_str(&MONTHS[civil.month as usize - 1][..3]);
                    Ok(())
                }
                DatePart::MonthLong => {
                    line.push_str(MONTHS[civil.month as usize - 1]);
                    Ok(())
                }
                DatePart::Offset => {
                    let sign = if civil.offset < 0 { '-' } else { '+' };
                    let minutes = civil.offset.unsigned_abs() / 60;
                    write!(line, "{sign}{:02}{:02}", minutes / 60, minutes % 60)
                }
                DatePart::Zone => {
                    match self.timezone.abbreviation_at(civil.epoch) {
                        Some(abbreviation) => line.push_str(abbreviation),
                        None if self.timezone.is_known() && !self.timezone.is_utc() => {
                            line.push_str(self.timezone.as_str());
                        }
                        None => line.push_str("UTC"),
                    }
                    Ok(())
                }
                DatePart::Epoch => write!(line, "{}", civil.epoch),
                DatePart::Percent => {
                    line.push('%');
                    Ok(())
                }
            };
        }
    }
}

/// A message written with each line after its first starting under the
/// first - the terminal format's hanging indent - as it is rendered, so
/// nothing is copied; a line is indented only once it has text, so a
/// message ending in a newline leaves no run of spaces behind.
struct Hanging<'a> {
    line: &'a mut String,
    /// What the line holds before the message: the indent is its width.
    prefix: Range<usize>,
    /// That width, measured at the message's first newline - a message of
    /// one line, the usual one, never measures it.
    column: Option<usize>,
    /// Whether the line being written still owes its indent.
    indent: bool,
}

impl fmt::Write for Hanging<'_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for (index, piece) in text.split('\n').enumerate() {
            if index > 0 {
                self.line.push('\n');
                self.indent = true;
            }
            if !piece.is_empty() {
                if self.indent {
                    let column = *self.column.get_or_insert_with(|| {
                        super::terminal::visible_width(&self.line[self.prefix.clone()])
                    });
                    self.line.extend(std::iter::repeat_n(' ', column));
                    self.indent = false;
                }
                self.line.push_str(piece);
            }
        }
        Ok(())
    }
}

/// What a record says when it does not say where it was raised: Python's
/// `findCaller` answer.
const UNKNOWN_FILE: &str = "(unknown file)";

/// What a record says when it does not say which function raised it.
const UNKNOWN_FUNCTION: &str = "(unknown function)";

/// The last component of `file`, either separator.
fn file_name(file: Option<&str>) -> &str {
    file.map_or(UNKNOWN_FILE, |path| {
        path.rsplit(['/', '\\']).next().unwrap_or(path)
    })
}

std::thread_local! {
    /// What [`set_thread_name`] named the current thread.
    static THREAD_NAME: std::cell::RefCell<Option<Box<str>>> =
        const { std::cell::RefCell::new(None) };
}

/// Names the calling thread in every record logged from it that names no
/// thread of its own, in `%(threadName)s` and the terminal format's
/// `[thread]`, in place of the name the thread was spawned with, or the
/// `Thread-N` Python numbers an unnamed one by. A binding names the threads its
/// runtime owns this way: the Node.js addon calls its main thread `main`
/// and a worker `worker-N`. An empty `name` forgets the one set.
///
/// ```
/// use yggdryl::logging::{self, Formatter, Level, Record};
///
/// let formatter = Formatter::from_str("%(threadName)s")?;
/// std::thread::spawn(move || {
///     let record = Record::new("feed", Level::INFO, &"opened");
///     logging::set_thread_name("ingest");
///     assert_eq!(formatter.format(&record), "ingest");
///     logging::set_thread_name("");
///     assert!(formatter.format(&record).starts_with("Thread-"));
/// })
/// .join()
/// .unwrap();
/// # Ok::<(), yggdryl::Error>(())
/// ```
pub fn set_thread_name(name: &str) {
    // A thread being torn down has nothing left to name.
    let _ = THREAD_NAME.try_with(|named| {
        *named.borrow_mut() = (!name.is_empty()).then(|| name.into());
    });
}

/// Appends the current thread's name: the one [`set_thread_name`] gave it,
/// else the one it was spawned with, else `Thread-N`.
fn thread_name(line: &mut String) {
    // A record logged while the thread's storage is torn down - from a
    // thread-local's `Drop` - reads past the name it can no longer reach.
    let named = THREAD_NAME
        .try_with(|named| match named.borrow().as_deref() {
            Some(name) => {
                line.push_str(name);
                true
            }
            None => false,
        })
        .unwrap_or(false);
    if !named {
        match std::thread::current().name() {
            Some(name) => line.push_str(name),
            None => {
                let _ = write!(line, "Thread-{}", thread_number());
            }
        }
    }
}

/// The emitting thread's number: assigned once per thread, from one.
pub(super) fn thread_number() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    thread_local! {
        static NUMBER: u64 = NEXT.fetch_add(1, Ordering::Relaxed);
    }
    NUMBER.with(|number| *number)
}

/// A numeric attribute's value.
#[derive(Clone, Copy)]
enum Number {
    Int(i64),
    Float(f64),
}

/// Appends a number converted and padded as `spec` says: `%d` truncates
/// a float toward zero, `%f` reads an integer as one, `%s` and `%r` spell a
/// float the shortest way that reads back, with Python's `.0` on a whole one.
fn number(line: &mut String, value: Number, spec: Spec) {
    let start = line.len();
    let conversion = spec.conversion;
    if conversion == Conversion::Char {
        // A code point no character has spells the replacement character.
        let character = match value {
            Number::Int(value) => u32::try_from(value).ok().and_then(char::from_u32),
            Number::Float(_) => None,
        };
        line.push(character.unwrap_or(char::REPLACEMENT_CHARACTER));
        pad(line, start, 0, spec, false);
        return;
    }
    let textual = conversion.is_textual();
    // The sign of what is written: `%d` of -0.5 writes `0`, as Python does.
    let negative = match value {
        Number::Int(value) => value < 0,
        Number::Float(value) if matches!(conversion, Conversion::Int) => value.trunc() < 0.0,
        Number::Float(value) => value.is_sign_negative() && value != 0.0,
    };
    if negative {
        line.push('-');
    } else if !textual && spec.plus {
        line.push('+');
    } else if !textual && spec.space {
        line.push(' ');
    }
    if spec.alternate {
        match conversion {
            Conversion::Hex { upper: false } => line.push_str("0x"),
            Conversion::Hex { upper: true } => line.push_str("0X"),
            Conversion::Octal => line.push_str("0o"),
            _ => {}
        }
    }
    // Zero padding goes after the sign and the prefix.
    let sign = line.len() - start;
    #[allow(clippy::cast_precision_loss)]
    let magnitude = match value {
        Number::Int(value) => value.unsigned_abs() as f64,
        Number::Float(value) => value.abs(),
    };
    let _ = match (conversion, value) {
        (Conversion::Str | Conversion::Repr | Conversion::Ascii, Number::Int(value))
        | (Conversion::Int, Number::Int(value)) => write!(line, "{}", value.unsigned_abs()),
        (Conversion::Str | Conversion::Repr | Conversion::Ascii, Number::Float(_)) => {
            let written = write!(line, "{magnitude}");
            if magnitude.is_finite() && magnitude.fract() == 0.0 {
                line.push_str(".0");
            }
            written
        }
        #[allow(clippy::cast_possible_truncation)]
        (Conversion::Int, Number::Float(value)) => {
            write!(line, "{}", (value.trunc() as i64).unsigned_abs())
        }
        (Conversion::Float, _) => {
            if magnitude.is_finite() {
                write!(line, "{:.*}", spec.precision.unwrap_or(6), magnitude)
            } else {
                not_finite(line, magnitude, false);
                Ok(())
            }
        }
        (Conversion::Hex { upper: false }, Number::Int(value)) => {
            write!(line, "{:x}", value.unsigned_abs())
        }
        (Conversion::Hex { upper: true }, Number::Int(value)) => {
            write!(line, "{:X}", value.unsigned_abs())
        }
        (Conversion::Octal, Number::Int(value)) => write!(line, "{:o}", value.unsigned_abs()),
        (Conversion::Exponent { upper }, _) => {
            exponent(
                line,
                magnitude,
                spec.precision.unwrap_or(6),
                upper,
                spec.alternate,
            );
            Ok(())
        }
        (Conversion::General { upper }, _) => {
            general(line, magnitude, spec.precision, upper, spec.alternate);
            Ok(())
        }
        // The parse refuses a fraction under these, and `c` returned above.
        (Conversion::Hex { .. } | Conversion::Octal, Number::Float(_)) | (Conversion::Char, _) => {
            Ok(())
        }
    };
    if matches!(
        conversion,
        Conversion::Int | Conversion::Hex { .. } | Conversion::Octal
    ) && let Some(precision) = spec.precision
    {
        let digits = line.len() - start - sign;
        if digits < precision {
            insert_run(line, start + sign, '0', precision - digits);
        }
    } else if textual
        && let Some(precision) = spec.precision
        && let Some((cut, _)) = line[start..].char_indices().nth(precision)
    {
        line.truncate(start + cut);
    }
    pad(line, start, sign, spec, !textual);
}

/// Appends an infinity or a not-a-number as Python spells it, upper case
/// under an upper-case conversion.
fn not_finite(line: &mut String, magnitude: f64, upper: bool) {
    line.push_str(match (magnitude.is_nan(), upper) {
        (true, false) => "nan",
        (true, true) => "NAN",
        (false, false) => "inf",
        (false, true) => "INF",
    });
}

/// Appends `magnitude` in exponent notation with `precision` places, the
/// exponent signed and at least two digits: `1.500000e+03`.
fn exponent(line: &mut String, magnitude: f64, precision: usize, upper: bool, alternate: bool) {
    if !magnitude.is_finite() {
        return not_finite(line, magnitude, upper);
    }
    let at = line.len();
    let _ = write!(line, "{magnitude:.precision$e}");
    // Rust spells the exponent bare - `1.5e3` - where Python signs it.
    let Some(mark) = line[at..].rfind('e').map(|offset| at + offset) else {
        return;
    };
    let power = line[mark + 1..].parse::<i32>().unwrap_or(0);
    line.truncate(mark);
    if alternate && precision == 0 {
        line.push('.');
    }
    line.push(if upper { 'E' } else { 'e' });
    let _ = write!(
        line,
        "{}{:02}",
        if power < 0 { '-' } else { '+' },
        power.unsigned_abs()
    );
}

/// Appends `magnitude` as Python's `%g` does: exponent notation when the
/// exponent is below -4 or reaches the precision (six, one at least),
/// fixed-point otherwise, trailing zeros dropped unless `alternate`.
fn general(
    line: &mut String,
    magnitude: f64,
    precision: Option<usize>,
    upper: bool,
    alternate: bool,
) {
    if !magnitude.is_finite() {
        return not_finite(line, magnitude, upper);
    }
    let digits = precision.map_or(6, |precision| precision.max(1));
    let at = line.len();
    // The exponent the value rounds to at that many digits.
    exponent(line, magnitude, digits - 1, false, false);
    let power = line[at..]
        .rfind('e')
        .and_then(|offset| line[at + offset + 1..].parse::<i64>().ok())
        .unwrap_or(0);
    let fixed = (-4..i64::try_from(digits).unwrap_or(i64::MAX)).contains(&power);
    if fixed {
        line.truncate(at);
        let places = usize::try_from(i64::try_from(digits).unwrap_or(0) - 1 - power).unwrap_or(0);
        let _ = write!(line, "{magnitude:.places$}");
    } else if upper && let Some(offset) = line[at..].rfind('e') {
        line.replace_range(at + offset..=at + offset, "E");
    }
    if alternate {
        if !line[at..].contains('.') {
            let mark = line[at..]
                .find(['e', 'E'])
                .map_or(line.len(), |offset| at + offset);
            line.insert(mark, '.');
        }
        return;
    }
    // Trailing zeros of the fraction go, and a point left alone with them.
    let end = line[at..]
        .find(['e', 'E'])
        .map_or(line.len(), |offset| at + offset);
    if line[at..end].contains('.') {
        let kept = line[at..end]
            .trim_end_matches('0')
            .trim_end_matches('.')
            .len();
        line.replace_range(at + kept..end, "");
    }
}

/// Inserts `count` of `fill` at `at`, a chunk at a time: a wide field
/// shifts what follows it a few times, never once per character.
fn insert_run(line: &mut String, at: usize, fill: char, count: usize) {
    const SPACES: &str = "                                                                ";
    const ZEROS: &str = "0000000000000000000000000000000000000000000000000000000000000000";
    let run = if fill == '0' { ZEROS } else { SPACES };
    let mut left = count;
    while left > 0 {
        let chunk = left.min(run.len());
        line.insert_str(at, &run[..chunk]);
        left -= chunk;
    }
}

/// Pads what was appended since `start` to the spec's width: on the right
/// when left-aligned, with zeros after the sign for a number under `0`,
/// otherwise with spaces on the left.
fn pad(line: &mut String, start: usize, sign: usize, spec: Spec, numeric: bool) {
    let written = line[start..].chars().count();
    if written >= spec.width {
        return;
    }
    let missing = spec.width - written;
    if spec.left {
        line.extend(std::iter::repeat_n(' ', missing));
    } else if spec.zero && numeric {
        insert_run(line, start + sign, '0', missing);
    } else {
        insert_run(line, start, ' ', missing);
    }
}

/// Re-spells what was appended to `line` since `start` as Python's `repr`,
/// or `ascii`, spells a string, through a buffer the thread reuses, so a
/// quoted field allocates nothing once warm.
fn quoted(line: &mut String, start: usize, ascii: bool) {
    thread_local! {
        static SCRATCH: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    }
    let requote = |line: &mut String, scratch: &mut String| {
        scratch.clear();
        scratch.push_str(&line[start..]);
        line.truncate(start);
        repr(line, scratch, ascii);
    };
    let reused = SCRATCH.try_with(|cell| match cell.try_borrow_mut() {
        Ok(mut scratch) => {
            requote(line, &mut scratch);
            if scratch.capacity() > 64 * 1024 {
                *scratch = String::new();
            }
            true
        }
        Err(_) => false,
    });
    // The buffer is in use - a field quoted while one is - or the thread is
    // being torn down: a buffer of this field's own.
    if !matches!(reused, Ok(true)) {
        requote(line, &mut String::new());
    }
}

/// Appends `text` as Python's `repr` spells a string: single-quoted unless
/// it holds a single quote and no double one, `\\`, the quote, `\n`,
/// `\r` and `\t` escaped, and every character Python does not print -
/// controls, format characters, separators other than the space, private
/// use - as `\xhh`, `\uhhhh` or `\Uhhhhhhhh`; under `ascii`, every character
/// past ASCII too.
fn repr(line: &mut String, text: &str, ascii: bool) {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    line.push(quote);
    for character in text.chars() {
        match character {
            '\\' => line.push_str("\\\\"),
            '\n' => line.push_str("\\n"),
            '\r' => line.push_str("\\r"),
            '\t' => line.push_str("\\t"),
            character if character == quote => {
                line.push('\\');
                line.push(character);
            }
            character if !is_printable(character) || (ascii && !character.is_ascii()) => {
                let _ = match u32::from(character) {
                    code @ 0..=0xFF => write!(line, "\\x{code:02x}"),
                    code @ 0x100..=0xFFFF => write!(line, "\\u{code:04x}"),
                    code => write!(line, "\\U{code:08x}"),
                };
            }
            character => line.push(character),
        }
    }
    line.push(quote);
}

/// Whether Python's `str.isprintable` holds for `character`: not a control,
/// a format character, a separator other than the space, a private-use
/// character or a noncharacter. Unassigned code points, which only a
/// Unicode table names, print.
fn is_printable(character: char) -> bool {
    if character == ' ' {
        return true;
    }
    if character.is_control() || character.is_whitespace() {
        return false;
    }
    !matches!(
        u32::from(character),
        0xAD | 0x600..=0x605
            | 0x61C
            | 0x6DD
            | 0x70F
            | 0x890..=0x891
            | 0x8E2
            | 0x180E
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x2064
            | 0x2066..=0x206F
            | 0xFEFF
            | 0xFFF9..=0xFFFB
            | 0x110BD
            | 0x110CD
            | 0x13430..=0x1343F
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0001
            | 0xE0020..=0xE007F
            | 0xE000..=0xF8FF
            | 0xF0000..=0xFFFFD
            | 0x100000..=0x10FFFD
            | 0xFDD0..=0xFDEF
    ) && u32::from(character) & 0xFFFE != 0xFFFE
}
