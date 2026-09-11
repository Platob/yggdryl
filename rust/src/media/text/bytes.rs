//! A counted range of bytes over one retained page.

use std::borrow::Cow;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use smol_str::format_smolstr;

use crate::{Charset, Error, Result};

/// A counted range of bytes over one retained page.
///
/// The text reader fills one page-sized buffer, seals it, and hands out ranges
/// into it, so a line costs one reference count rather than a copy of its own
/// bytes. Sealing is what makes this work: a shared page has no interior
/// mutability, so a page is filled while nothing points into it and is shared
/// only once it is complete.
///
/// The page is also what a binary-view column is built from: a view names a
/// block, an offset and a length, and this carries the page, the offset, and
/// the length as the difference of its two bounds.
///
/// A borrowed slice would be cheaper still and is not an option: an iterator
/// cannot yield items that outlive the buffer they point into, and a borrowed
/// row could not be stored, collected, or crossed to a binding. One atomic
/// increment per line buys all of that.
#[derive(Clone, Default)]
pub struct TextBytes {
    page: Option<Arc<Vec<u8>>>,
    start: u32,
    end: u32,
}

impl TextBytes {
    /// The empty range, which retains no page.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            page: None,
            start: 0,
            end: 0,
        }
    }

    /// Take a range of one page.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when the range is inverted, reaches
    /// past the page, or does not fit the 32-bit offsets an Arrow view uses.
    pub fn from_page(page: &Arc<Vec<u8>>, start: usize, end: usize) -> Result<Self> {
        if start > end || end > page.len() {
            return Err(Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$.body"),
                reason: format_smolstr!(
                    "expected a range inside a page of {} bytes, got {start}..{end}",
                    page.len()
                ),
            });
        }
        let (Ok(start), Ok(end)) = (u32::try_from(start), u32::try_from(end)) else {
            return Err(Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$.body"),
                reason: format_smolstr!(
                    "expected a range addressable in 32 bits, got {start}..{end}"
                ),
            });
        };
        if start == end {
            return Ok(Self::new());
        }
        Ok(Self {
            page: Some(Arc::clone(page)),
            start,
            end,
        })
    }

    /// Take a whole page.
    ///
    /// # Errors
    ///
    /// Returns the same refusal as [`Self::from_page`] for a page longer than
    /// 32-bit offsets reach.
    pub fn from_whole_page(page: Arc<Vec<u8>>) -> Result<Self> {
        let len = page.len();
        Self::from_page(&page, 0, len)
    }

    /// Copy bytes into a page of their own.
    ///
    /// The allocating constructor, for a value that was not read into a page:
    /// a line assembled across a page boundary, a framed record joining several
    /// physical lines, or a caller building a line by hand.
    ///
    /// # Errors
    ///
    /// Returns the same refusal as [`Self::from_page`] for input longer than
    /// 32-bit offsets reach.
    pub fn from_bytes(bytes: impl AsRef<[u8]>) -> Result<Self> {
        let bytes = bytes.as_ref();
        if bytes.is_empty() {
            return Ok(Self::new());
        }
        Self::from_whole_page(Arc::new(bytes.to_vec()))
    }

    /// Borrow the bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        match &self.page {
            Some(page) => &page[self.start as usize..self.end as usize],
            None => &[],
        }
    }

    /// The number of bytes.
    #[must_use]
    pub const fn len(&self) -> usize {
        (self.end - self.start) as usize
    }

    /// Whether this range holds no bytes.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Borrow the page these bytes are a range of.
    ///
    /// The Arrow builder registers a page once and then appends one view per
    /// line, so it needs the page itself and not only the bytes.
    #[must_use]
    pub const fn page(&self) -> Option<&Arc<Vec<u8>>> {
        self.page.as_ref()
    }

    /// Where this range starts in its page.
    #[must_use]
    pub const fn start(&self) -> u32 {
        self.start
    }

    /// Where this range ends in its page.
    #[must_use]
    pub const fn end(&self) -> u32 {
        self.end
    }

    /// A sub-range of these bytes, sharing the same page.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when the range reaches outside this
    /// one.
    pub fn slice(&self, start: usize, end: usize) -> Result<Self> {
        if start > end || end > self.len() {
            return Err(Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$.body"),
                reason: format_smolstr!(
                    "expected a range inside {} bytes, got {start}..{end}",
                    self.len()
                ),
            });
        }
        let Some(page) = &self.page else {
            return Ok(Self::new());
        };
        let base = self.start as usize;
        Self::from_page(page, base + start, base + end)
    }

    /// The bytes as text, when they are valid UTF-8.
    ///
    /// This is [`Self::decode`] under [`Charset::Utf8`] with the refusal
    /// dropped, kept because most callers here only ask whether a range is
    /// already text.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(self.as_bytes()).ok()
    }

    /// The bytes as text, read in one charset.
    ///
    /// The answer borrows these bytes whenever they are already UTF-8 - which
    /// an all-ASCII range is under every ASCII-compatible charset - so the
    /// common capture costs no allocation.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Codec`] naming the charset, the byte position within
    /// this range, and what was found there.
    pub fn decode(&self, charset: Charset) -> Result<Cow<'_, str>> {
        charset.decode(self.as_bytes())
    }

    /// Copy the bytes into an owned vector.
    ///
    /// The allocating counterpart of [`Self::as_bytes`].
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.as_bytes().to_vec()
    }
}

/// Identity is the bytes, never the page or the offsets.
///
/// Two lines carrying the same bytes in different pages are the same value, and
/// anything comparing rows - adjacent deduplication, a test, a sort - depends on
/// that being true. Deriving these would compare the whole page and both
/// offsets, so the same bytes at different offsets of different pages would
/// come out unequal and hash apart.
impl PartialEq for TextBytes {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for TextBytes {}

impl Ord for TextBytes {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_bytes().cmp(other.as_bytes())
    }
}

impl PartialOrd for TextBytes {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Hash for TextBytes {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state);
    }
}

impl fmt::Debug for TextBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The bytes, not the pointer: a debug rendering naming a page address
        // is neither stable nor useful.
        fmt::Debug::fmt(self.as_bytes(), formatter)
    }
}

impl AsRef<[u8]> for TextBytes {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl std::ops::Deref for TextBytes {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_bytes()
    }
}

impl TryFrom<&[u8]> for TextBytes {
    type Error = Error;

    fn try_from(bytes: &[u8]) -> Result<Self> {
        Self::from_bytes(bytes)
    }
}

impl TryFrom<Vec<u8>> for TextBytes {
    type Error = Error;

    fn try_from(bytes: Vec<u8>) -> Result<Self> {
        Self::from_bytes(bytes)
    }
}
