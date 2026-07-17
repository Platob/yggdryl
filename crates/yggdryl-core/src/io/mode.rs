//! [`IOMode`] — how a source may be accessed (read / write / append / overwrite).

use core::fmt;

use super::IoError;

/// The access mode of an I/O source — an **int enum** (`#[repr(u8)]`, stable numeric values)
/// naming how the source may be used: [`Read`](IOMode::Read), [`Write`](IOMode::Write),
/// [`ReadWrite`](IOMode::ReadWrite), [`Append`](IOMode::Append) (writes go to the end), and
/// [`Overwrite`](IOMode::Overwrite) (truncate-then-write). Every [`IOBase`] source reports one
/// via [`mode`](crate::io::memory::IOBase::mode).
///
/// The numeric values are wire-stable: `Read = 1`, `Write = 2`, `ReadWrite = 3`
/// (= `Read | Write`), `Append = 4`, `Overwrite = 5` — see [`to_u8`](IOMode::to_u8) /
/// [`from_u8`](IOMode::from_u8) and the [`parse_str`](IOMode::parse_str) name parser.
///
/// [`IOBase`]: crate::io::memory::IOBase
///
/// ```
/// use yggdryl_core::io::IOMode;
///
/// assert_eq!(IOMode::parse_str("rw").unwrap(), IOMode::ReadWrite);
/// assert_eq!(IOMode::ReadWrite.to_u8(), 3);
/// assert!(IOMode::Append.is_writable());
/// assert!(!IOMode::Read.is_writable());
/// assert_eq!(IOMode::Overwrite.to_string(), "overwrite");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum IOMode {
    /// Read-only access — `"read"` / `"r"`. Value `1`.
    Read = 1,
    /// Write-only access — `"write"` / `"w"`. Value `2`.
    Write = 2,
    /// Read and write access — `"read_write"` / `"rw"` / `"+"`. Value `3` (`Read | Write`).
    ReadWrite = 3,
    /// Write-only, every write lands at the end — `"append"` / `"a"`. Value `4`.
    Append = 4,
    /// Write that truncates existing content first — `"overwrite"` / `"o"` / `"truncate"`.
    /// Value `5`.
    Overwrite = 5,
}

impl IOMode {
    /// Parses a mode **name** (ASCII case-insensitive): the canonical snake_case name
    /// (`"read_write"`) or its short POSIX-style alias (`"rw"`, `"r"`, `"a"`, …).
    ///
    /// # Errors
    /// [`IoError::UnknownName`] naming the offending input and every accepted token.
    ///
    /// ```
    /// use yggdryl_core::io::IOMode;
    ///
    /// assert_eq!(IOMode::parse_str("READ").unwrap(), IOMode::Read);
    /// assert_eq!(IOMode::parse_str("a").unwrap(), IOMode::Append);
    /// let err = IOMode::parse_str("bogus").unwrap_err();
    /// assert!(err.to_string().contains("bogus"));
    /// assert!(err.to_string().contains("read_write"));
    /// ```
    pub fn parse_str(s: &str) -> Result<IOMode, IoError> {
        // ASCII-lowercase comparison without allocating for the common already-lowercase case.
        let matches = |token: &str| s.eq_ignore_ascii_case(token);
        if matches("read") || matches("r") {
            Ok(IOMode::Read)
        } else if matches("write") || matches("w") {
            Ok(IOMode::Write)
        } else if matches("read_write") || matches("rw") || matches("+") {
            Ok(IOMode::ReadWrite)
        } else if matches("append") || matches("a") {
            Ok(IOMode::Append)
        } else if matches("overwrite") || matches("o") || matches("truncate") {
            Ok(IOMode::Overwrite)
        } else {
            Err(IoError::UnknownName {
                kind: "IOMode",
                input: s.to_string(),
                expected: "read/r, write/w, read_write/rw/+, append/a, overwrite/o/truncate",
            })
        }
    }

    /// The canonical snake_case name (`"read_write"`), the exact inverse of
    /// [`parse_str`](IOMode::parse_str).
    pub fn name(self) -> &'static str {
        match self {
            IOMode::Read => "read",
            IOMode::Write => "write",
            IOMode::ReadWrite => "read_write",
            IOMode::Append => "append",
            IOMode::Overwrite => "overwrite",
        }
    }

    /// The stable numeric value (`Read = 1`, … `Overwrite = 5`).
    pub fn to_u8(self) -> u8 {
        self as u8
    }

    /// The mode for a stable numeric value, or [`IoError::UnknownName`] for a value outside
    /// `1..=5` — a **checked** match, never a transmute.
    ///
    /// ```
    /// use yggdryl_core::io::IOMode;
    ///
    /// assert_eq!(IOMode::from_u8(4).unwrap(), IOMode::Append);
    /// assert!(IOMode::from_u8(9).is_err());
    /// ```
    pub fn from_u8(value: u8) -> Result<IOMode, IoError> {
        match value {
            1 => Ok(IOMode::Read),
            2 => Ok(IOMode::Write),
            3 => Ok(IOMode::ReadWrite),
            4 => Ok(IOMode::Append),
            5 => Ok(IOMode::Overwrite),
            _ => Err(IoError::UnknownName {
                kind: "IOMode",
                input: value.to_string(),
                expected: "1 (read), 2 (write), 3 (read_write), 4 (append), 5 (overwrite)",
            }),
        }
    }

    /// Whether this mode allows reading (`Read` / `ReadWrite`).
    pub fn is_readable(self) -> bool {
        matches!(self, IOMode::Read | IOMode::ReadWrite)
    }

    /// Whether this mode allows writing (everything except `Read`).
    pub fn is_writable(self) -> bool {
        !matches!(self, IOMode::Read)
    }
}

impl fmt::Display for IOMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
