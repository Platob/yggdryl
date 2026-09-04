//! The record terminator: optional, flexible when unset, exact when pinned.
//!
//! "Separator" is deliberately *not* the word. It is reserved for the field
//! separation delimited formats will need - the comma of a CSV, the tab of a
//! TSV - and using it for the record terminator now would collide with that
//! work and read ambiguously the moment both exist. Here, **`linesep`
//! terminates records** and `separator` will one day delimit fields within one.

use smol_str::format_smolstr;
use std::sync::Arc;

use crate::{Error, Result};

/// One record terminator, pinned explicitly.
///
/// Built by [`LineSep::new`]; the common spellings have constants. A pinned
/// terminator is the **only** one recognized - a lone `\n` inside a
/// `\r\n`-pinned resource is content, not a break - which is what a caller
/// needs when a record's own text may legitimately contain a newline.
///
/// ```
/// use yggdryl::media::text::LineSep;
///
/// # fn main() -> yggdryl::Result<()> {
/// assert_eq!(LineSep::LF.as_bytes(), b"\n");
/// assert_eq!(LineSep::CRLF.as_bytes(), b"\r\n");
/// // `find -print0` writes NUL-delimited records.
/// assert_eq!(LineSep::NUL.as_bytes(), b"\0");
/// // Any non-empty byte string works.
/// assert_eq!(LineSep::new("<END>")?.as_bytes(), b"<END>");
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct LineSep(Held);

/// Where a terminator's bytes live.
///
/// The common spellings are `'static` so the constants are const-constructible
/// and cost no allocation; anything else is one shared allocation. The
/// distinction is representation only - two terminators with the same bytes are
/// equal, hash alike, and order alike whichever half holds them, which is why
/// the value traits are written out rather than derived.
#[derive(Clone, Debug)]
enum Held {
    Static(&'static [u8]),
    Owned(Arc<[u8]>),
}

impl Held {
    fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Static(bytes) => bytes,
            Self::Owned(bytes) => bytes,
        }
    }
}

impl PartialEq for LineSep {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for LineSep {}

impl std::hash::Hash for LineSep {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state);
    }
}

impl Ord for LineSep {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_bytes().cmp(other.as_bytes())
    }
}

impl PartialOrd for LineSep {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl LineSep {
    /// A bare line feed, the platform-neutral terminator writes default to.
    pub const LF: Self = Self::inline(b"\n");
    /// A carriage return and line feed, as Windows tools write.
    pub const CRLF: Self = Self::inline(b"\r\n");
    /// A lone carriage return, as classic Mac OS wrote.
    pub const CR: Self = Self::inline(b"\r");
    /// A NUL byte, as `find -print0` and `xargs -0` exchange.
    pub const NUL: Self = Self::inline(b"\0");
    /// ASCII record separator, `0x1e`.
    pub const RS: Self = Self::inline(b"\x1e");

    /// A constant terminator, without allocating.
    const fn inline(bytes: &'static [u8]) -> Self {
        Self(Held::Static(bytes))
    }

    /// Pin a terminator to an exact byte string.
    ///
    /// # Errors
    ///
    /// Returns an error when the terminator is empty: a zero-length terminator
    /// would match everywhere and terminate nothing.
    pub fn new(terminator: impl AsRef<[u8]>) -> Result<Self> {
        let bytes = terminator.as_ref();
        if bytes.is_empty() {
            return Err(Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$.linesep"),
                reason: crate::text::expected_got(
                    "a non-empty record terminator",
                    "an empty one, which would match everywhere and terminate nothing",
                ),
            });
        }
        Ok(Self(Held::Owned(Arc::from(bytes))))
    }

    /// Borrow the terminator's bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Return how many bytes the terminator occupies.
    #[must_use]
    pub fn len(&self) -> usize {
        self.as_bytes().len()
    }

    /// Return whether the terminator is empty, which construction refuses.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.as_bytes().is_empty()
    }
}

impl std::fmt::Display for LineSep {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The escaped spelling, so a terminator is legible in an error.
        match std::str::from_utf8(self.as_bytes()) {
            Ok(text) => write!(formatter, "{text:?}"),
            Err(_) => write!(formatter, "{:?}", self.as_bytes()),
        }
    }
}

impl std::str::FromStr for LineSep {
    type Err = Error;

    /// Read a terminator from its text, accepting the usual escapes.
    ///
    /// `\n`, `\r\n`, `\r`, `\0`, `\t`, and `\xNN` are read as the bytes they
    /// spell, so a configuration document can pin a terminator that has no
    /// printable form.
    fn from_str(value: &str) -> Result<Self> {
        let mut bytes = Vec::with_capacity(value.len());
        let mut rest = value.chars();
        while let Some(character) = rest.next() {
            if character != '\\' {
                let mut buffer = [0_u8; 4];
                bytes.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
                continue;
            }
            match rest.next() {
                Some('n') => bytes.push(b'\n'),
                Some('r') => bytes.push(b'\r'),
                Some('t') => bytes.push(b'\t'),
                Some('0') => bytes.push(0),
                Some('\\') => bytes.push(b'\\'),
                Some('x') => {
                    let high = rest.next();
                    let low = rest.next();
                    let spelled: String = [high, low].into_iter().flatten().collect();
                    let byte =
                        u8::from_str_radix(&spelled, 16).map_err(|_| Error::InvalidRecord {
                            path: smol_str::SmolStr::new_static("$.linesep"),
                            reason: crate::text::expected_got(
                                "two hexadecimal digits after \\x",
                                format_smolstr!("{spelled:?}"),
                            ),
                        })?;
                    bytes.push(byte);
                }
                other => {
                    return Err(Error::InvalidRecord {
                        path: smol_str::SmolStr::new_static("$.linesep"),
                        reason: crate::text::expected_got(
                            "a known escape (\\n, \\r, \\t, \\0, \\\\, \\xNN)",
                            format_smolstr!("\\{}", other.unwrap_or(' ')),
                        ),
                    });
                }
            }
        }
        Self::new(bytes)
    }
}

/// Where one record's terminator sits, and how wide it is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Break {
    /// The offset the terminator starts at, relative to the scanned slice.
    pub(crate) at: usize,
    /// How many bytes the terminator occupies. A `\r\n` counts two.
    pub(crate) width: usize,
}

impl Break {
    /// The offset just past the terminator, where the next record opens.
    pub(crate) const fn end(self) -> usize {
        self.at + self.width
    }
}

/// Find the next record break in `window`, under an optional pinned terminator.
///
/// Unset - the default and the recommended path - this accepts `\n`, `\r\n`,
/// and a lone `\r`, **mixed within one resource**, because real corpora are
/// mixed: a log rotated on Windows and concatenated on Linux, a file with CRLF
/// headers and LF bodies. Detection is per terminator, not a sniff of the first
/// line applied to the rest, which is exactly how a mixed file gets misparsed.
///
/// The flexible path is the *fast* path, not a fallback for convenience: one
/// scan for either candidate byte, and `\r` versus `\r\n` resolved by looking
/// at the single next byte. No backtracking, and no per-line branching on
/// configuration beyond one already-predicted match.
///
/// `complete` says whether `window` ends at the resource's end. When it does
/// not, a `\r` as the very last byte is *ambiguous* - the next byte, not yet
/// read, may be the `\n` that makes it a `\r\n` - so no break is reported and
/// the caller refills.
pub(crate) fn next_break(
    window: &[u8],
    linesep: Option<&LineSep>,
    complete: bool,
) -> Option<Break> {
    let Some(linesep) = linesep else {
        return flexible_break(window, complete);
    };
    let needle = linesep.as_bytes();
    if let [byte] = needle {
        // A pinned single-byte terminator is the same scan with one needle.
        return find_byte(window, *byte).map(|at| Break { at, width: 1 });
    }
    // Only a multi-byte pinned terminator needs a general search.
    find_slice(window, needle).map(|at| Break {
        at,
        width: needle.len(),
    })
}

/// The flexible scan: `\n`, `\r\n`, or a lone `\r`, whichever comes next.
fn flexible_break(window: &[u8], complete: bool) -> Option<Break> {
    let at = find_either(window, b'\n', b'\r')?;
    if window[at] == b'\n' {
        return Some(Break { at, width: 1 });
    }
    match window.get(at + 1) {
        Some(b'\n') => Some(Break { at, width: 2 }),
        Some(_) => Some(Break { at, width: 1 }),
        // A trailing `\r` with more bytes to come is undecided: the next byte
        // may be the `\n` that makes this one terminator rather than two.
        None if complete => Some(Break { at, width: 1 }),
        None => None,
    }
}

/// The first offset of `needle` in `window`.
///
/// `memchr` rather than a scalar loop: the byte scan is the floor under every
/// record read, the crate is already in the tree under the regex engine, and
/// its vectorized search is what keeps line splitting off the profile.
fn find_byte(window: &[u8], needle: u8) -> Option<usize> {
    memchr::memchr(needle, window)
}

/// The first offset of either needle in `window`, in one pass.
fn find_either(window: &[u8], first: u8, second: u8) -> Option<usize> {
    memchr::memchr2(first, second, window)
}

/// The first offset of `needle` in `window`, for a multi-byte terminator.
fn find_slice(window: &[u8], needle: &[u8]) -> Option<usize> {
    memchr::memmem::find(window, needle)
}
