//! [`IOKind`] — what kind of thing an I/O source is (missing / file / directory / heap).

use core::fmt;

use super::IoError;

/// The kind of an I/O source — an **int enum** (`#[repr(u8)]`, stable numeric values) naming
/// what the source physically is: [`Missing`](IOKind::Missing) (nothing at the address),
/// [`File`](IOKind::File), [`Directory`](IOKind::Directory), the in-memory
/// [`Heap`](IOKind::Heap), or [`Unknown`](IOKind::Unknown) (something exists but its type is
/// not one of the above — a special file, an object-store entry of an unrecognized type).
/// Every [`IOBase`] source reports one via [`kind`](crate::io::memory::IOBase::kind).
///
/// The numeric values are wire-stable: `Unknown = 0` (the **default**), `Missing = 1`,
/// `File = 2`, `Directory = 3`, `Heap = 4` — see [`to_u8`](IOKind::to_u8) /
/// [`from_u8`](IOKind::from_u8) and the [`parse_str`](IOKind::parse_str) name parser.
///
/// [`IOBase`]: crate::io::memory::IOBase
///
/// ```
/// use yggdryl_core::io::IOKind;
///
/// assert_eq!(IOKind::parse_str("heap").unwrap(), IOKind::Heap);
/// assert_eq!(IOKind::Heap.to_u8(), 4);
/// assert_eq!(IOKind::from_u8(3).unwrap(), IOKind::Directory);
/// assert_eq!(IOKind::Missing.to_string(), "missing");
/// assert_eq!(IOKind::default(), IOKind::Unknown); // the zero value
/// assert!(IOKind::Unknown.exists()); // exists, but of an undetermined kind
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum IOKind {
    /// Something exists at the address but its type is not `File` / `Directory` / `Heap` — a
    /// special file (socket, device), a symlink left unclassified, or an object-store entry
    /// of an unrecognized type. The **default** (zero) value. Value `0`.
    #[default]
    Unknown = 0,
    /// Nothing exists at the source's address. Value `1`.
    Missing = 1,
    /// A regular file. Value `2`.
    File = 2,
    /// A directory. Value `3`.
    Directory = 3,
    /// An in-memory heap buffer. Value `4`.
    Heap = 4,
}

impl IOKind {
    /// Parses a kind **name** (ASCII case-insensitive): `"missing"`, `"file"`, `"directory"`
    /// (or `"dir"`), `"heap"`.
    ///
    /// # Errors
    /// [`IoError::UnknownName`] naming the offending input and every accepted token.
    ///
    /// ```
    /// use yggdryl_core::io::IOKind;
    ///
    /// assert_eq!(IOKind::parse_str("DIR").unwrap(), IOKind::Directory);
    /// assert!(IOKind::parse_str("bogus").is_err());
    /// ```
    pub fn parse_str(s: &str) -> Result<IOKind, IoError> {
        let matches = |token: &str| s.eq_ignore_ascii_case(token);
        if matches("missing") {
            Ok(IOKind::Missing)
        } else if matches("file") {
            Ok(IOKind::File)
        } else if matches("directory") || matches("dir") {
            Ok(IOKind::Directory)
        } else if matches("heap") {
            Ok(IOKind::Heap)
        } else if matches("unknown") {
            Ok(IOKind::Unknown)
        } else {
            Err(IoError::UnknownName {
                kind: "IOKind",
                input: s.to_string(),
                expected: "missing, file, directory/dir, heap, unknown",
            })
        }
    }

    /// The canonical lowercase name (`"directory"`), the exact inverse of
    /// [`parse_str`](IOKind::parse_str).
    pub fn name(self) -> &'static str {
        match self {
            IOKind::Missing => "missing",
            IOKind::File => "file",
            IOKind::Directory => "directory",
            IOKind::Heap => "heap",
            IOKind::Unknown => "unknown",
        }
    }

    /// The stable numeric value (`Unknown = 0`, `Missing = 1`, … `Heap = 4`).
    pub fn to_u8(self) -> u8 {
        self as u8
    }

    /// The kind for a stable numeric value, or [`IoError::UnknownName`] for a value outside
    /// `0..=4` — a **checked** match, never a transmute.
    pub fn from_u8(value: u8) -> Result<IOKind, IoError> {
        match value {
            0 => Ok(IOKind::Unknown),
            1 => Ok(IOKind::Missing),
            2 => Ok(IOKind::File),
            3 => Ok(IOKind::Directory),
            4 => Ok(IOKind::Heap),
            _ => Err(IoError::UnknownName {
                kind: "IOKind",
                input: value.to_string(),
                expected: "0 (unknown), 1 (missing), 2 (file), 3 (directory), 4 (heap)",
            }),
        }
    }

    /// Whether the source exists at all (everything except `Missing`).
    pub fn exists(self) -> bool {
        !matches!(self, IOKind::Missing)
    }
}

impl fmt::Display for IOKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
