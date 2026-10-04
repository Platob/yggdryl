//! The level a record is logged at, and the level a logger or a handler
//! handles from: Python's numbers.

use std::fmt;
use std::str::FromStr;

use smol_str::format_smolstr;

use crate::{Error, Result};

/// How severe a record is, on Python's `logging` scale: `DEBUG` is `10`,
/// `INFO` `20`, `WARNING` `30`, `ERROR` `40`, `CRITICAL` `50`, and `NOTSET`
/// `0` - on a logger, "take the level of the nearest ancestor that states
/// one", and on a handler, "handle everything".
///
/// Any number is a level, as in Python: a record at `15` is handled by a
/// logger at `DEBUG` and not by one at `INFO`. `TRACE` (`5`) is the level a
/// [`log::Level::Trace`] record arrives at; Python names no level there and
/// spells it `Level 5`.
///
/// ```
/// use yggdryl::logging::Level;
///
/// assert_eq!(Level::from_str("warn")?, Level::WARNING);
/// assert_eq!(Level::from_str("15")?.to_string(), "Level 15");
/// assert!(Level::ERROR > Level::WARNING);
/// assert_eq!(Level::from(log::Level::Trace), Level::TRACE);
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Level(u8);

impl Level {
    /// On a logger, the level of the nearest ancestor that states one; on a
    /// handler, every record.
    pub const NOTSET: Self = Self(0);
    /// What a [`log::Level::Trace`] record arrives at.
    pub const TRACE: Self = Self(5);
    /// An operation starting.
    pub const DEBUG: Self = Self(10);
    /// An operation done, carrying the counts a monitor watches.
    pub const INFO: Self = Self(20);
    /// Something passed over or done instead of what was asked; the level a
    /// logger stating none answers at the root.
    pub const WARNING: Self = Self(30);
    /// An operation that failed.
    pub const ERROR: Self = Self(40);
    /// A failure the process may not survive.
    pub const CRITICAL: Self = Self(50);

    /// Every named level, from the least severe.
    pub const ALL: [Self; 7] = [
        Self::NOTSET,
        Self::TRACE,
        Self::DEBUG,
        Self::INFO,
        Self::WARNING,
        Self::ERROR,
        Self::CRITICAL,
    ];

    /// The level numbered `value`, named or not.
    pub const fn new(value: u8) -> Self {
        Self(value)
    }

    /// The level's number.
    pub const fn get(self) -> u8 {
        self.0
    }

    /// The level's name - `DEBUG`, `WARNING` - or `None` for a number no
    /// level is named at.
    pub const fn name(self) -> Option<&'static str> {
        match self.0 {
            0 => Some("NOTSET"),
            5 => Some("TRACE"),
            10 => Some("DEBUG"),
            20 => Some("INFO"),
            30 => Some("WARNING"),
            40 => Some("ERROR"),
            50 => Some("CRITICAL"),
            _ => None,
        }
    }

    /// Parses a level: a name in any case - Python's `WARN` and `FATAL`
    /// among them - or its number.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming every accepted name for anything else,
    /// a number past `255` included.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }
}

impl FromStr for Level {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let text = value.trim();
        if !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()) {
            return crate::integer::integer_from_text_as::<u8>(text)
                .map(Self)
                .ok_or_else(|| Error::Parse {
                    target: "log level",
                    position: 0,
                    reason: format_smolstr!("expected a level from 0 to 255, got {value:?}"),
                });
        }
        let named = match text.to_ascii_uppercase().as_str() {
            "NOTSET" => Self::NOTSET,
            "TRACE" => Self::TRACE,
            "DEBUG" => Self::DEBUG,
            "INFO" => Self::INFO,
            "WARN" | "WARNING" => Self::WARNING,
            "ERROR" => Self::ERROR,
            "FATAL" | "CRITICAL" => Self::CRITICAL,
            _ => {
                return Err(Error::Parse {
                    target: "log level",
                    position: 0,
                    reason: format_smolstr!(
                        "expected one of NOTSET, TRACE, DEBUG, INFO, WARNING, ERROR, CRITICAL \
                         or a number, got {value:?}"
                    ),
                });
            }
        };
        Ok(named)
    }
}

/// The name, or `Level N` for a number no level is named at - Python's
/// `getLevelName`.
impl fmt::Display for Level {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(name) => formatter.write_str(name),
            None => write!(formatter, "Level {}", self.0),
        }
    }
}

impl From<log::Level> for Level {
    fn from(level: log::Level) -> Self {
        match level {
            log::Level::Error => Self::ERROR,
            log::Level::Warn => Self::WARNING,
            log::Level::Info => Self::INFO,
            log::Level::Debug => Self::DEBUG,
            log::Level::Trace => Self::TRACE,
        }
    }
}

impl Level {
    /// The one character a terminal marks a record at this level with, the
    /// command line's own: `·` below `INFO`, `•` for `INFO`, `!` for
    /// `WARNING`, `✗` for `ERROR` and `‼` from `CRITICAL` - a number between
    /// two named levels takes the lower one's.
    pub const fn glyph(self) -> &'static str {
        match self.0 {
            0..=19 => "·",
            20..=29 => "•",
            30..=39 => "!",
            40..=49 => "✗",
            _ => "‼",
        }
    }

    /// The escape sequence a coloured terminal shows this level in: faint
    /// below `DEBUG`, cyan, green, yellow, red, and bold red from
    /// `CRITICAL`.
    pub(crate) const fn color(self) -> &'static str {
        match self.0 {
            0..=9 => super::terminal::DIM,
            10..=19 => "\x1b[36m",
            20..=29 => "\x1b[32m",
            30..=39 => "\x1b[33m",
            40..=49 => "\x1b[31m",
            _ => "\x1b[1;31m",
        }
    }
}

/// A threshold: the lowest level something handles, as a number one wider
/// than a level so that [`NEVER`] - nothing handled - has a place.
pub(crate) type Threshold = u16;

/// The threshold that handles no level at all.
pub(crate) const NEVER: Threshold = 256;

impl Level {
    /// The facade's ceiling for a threshold: the most verbose `log` level
    /// whose records a threshold admits.
    pub(crate) const fn filter_for(threshold: Threshold) -> log::LevelFilter {
        match threshold {
            0..=5 => log::LevelFilter::Trace,
            6..=10 => log::LevelFilter::Debug,
            11..=20 => log::LevelFilter::Info,
            21..=30 => log::LevelFilter::Warn,
            31..=40 => log::LevelFilter::Error,
            _ => log::LevelFilter::Off,
        }
    }
}
