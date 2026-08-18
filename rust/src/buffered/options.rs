//! The three knobs a page cache has, and the rules that keep them coherent.

use std::time::Duration;

/// Page size when nothing else is said: 64 KiB, one readahead-sized unit.
const DEFAULT_PAGE_SIZE: usize = 64 * 1024;
/// Byte budget when nothing else is said: 8 MiB, 128 default-sized pages.
const DEFAULT_MAX_BYTES: u64 = 8 * 1024 * 1024;
/// Time to live when nothing else is said, measured from the last access.
const DEFAULT_TTL: Duration = Duration::from_secs(30);
/// The smallest page the cache will use.
///
/// Anything below this makes the per-page bookkeeping cost more than the read
/// it saves; tests that want many pages over a small value still get them.
const MIN_PAGE_SIZE: usize = 64;
/// The largest page the cache will use, so one page cannot be the whole budget.
const MAX_PAGE_SIZE: usize = 1024 * 1024 * 1024;

/// How a [`Buffered`](super::Buffered) handle caches pages.
///
/// Three knobs, each normalized on the way in so an impossible combination
/// cannot be held:
///
/// - **Page size** is the unit every miss fetches, rounded *up* to a power of
///   two and clamped to `64 ..= 1 GiB`. Powers of two are what make an offset
///   turn into a page index with a shift rather than a division.
/// - **Max bytes** is the budget the cached pages share. It is **clamped up**
///   to `2 * page_size` rather than rejected, because the header and footer
///   pages are pinned and a budget below two pages would leave the cache
///   thrashing on every read. Changing the page size re-applies the clamp.
/// - **Time to live** is measured from a page's *last access*, so a page that
///   keeps being read never expires and one nothing touches is discarded the
///   next time the table is swept.
///
/// ```
/// use std::time::Duration;
///
/// use yggdryl::buffered::BufferedOptions;
///
/// let options = BufferedOptions::default();
/// assert_eq!(options.page_size(), 64 * 1024);
/// assert_eq!(options.max_bytes(), 8 * 1024 * 1024);
/// assert_eq!(options.ttl(), Duration::from_secs(30));
///
/// // A page size that is not a power of two is rounded up to one, and a
/// // budget below two pages is raised to exactly that.
/// let tight = BufferedOptions::default()
///     .with_page_size(1_000)
///     .with_max_bytes(1);
/// assert_eq!(tight.page_size(), 1_024);
/// assert_eq!(tight.max_bytes(), 2_048);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BufferedOptions {
    page_size: usize,
    /// `page_size.trailing_zeros()`, kept so an offset turns into a page index
    /// with a shift. This is what the power-of-two rule is *for*: a read that
    /// hits does one shift rather than two divisions.
    page_shift: u32,
    max_bytes: u64,
    ttl: Duration,
}

impl BufferedOptions {
    /// Build the default options: 64 KiB pages, an 8 MiB budget, a 30 s life.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            page_size: DEFAULT_PAGE_SIZE,
            page_shift: DEFAULT_PAGE_SIZE.trailing_zeros(),
            max_bytes: DEFAULT_MAX_BYTES,
            ttl: DEFAULT_TTL,
        }
    }

    /// Return these options with a different page size.
    ///
    /// The value is rounded up to a power of two, clamped to
    /// `64 ..= 1 GiB`, and the byte budget is re-clamped to at least two of
    /// the resulting pages.
    #[must_use]
    pub const fn with_page_size(mut self, page_size: usize) -> Self {
        self.page_size = normalize_page_size(page_size);
        self.page_shift = self.page_size.trailing_zeros();
        self.max_bytes = normalize_max_bytes(self.max_bytes, self.page_size);
        self
    }

    /// Return these options with a different byte budget.
    ///
    /// A budget below `2 * page_size` is clamped up to it, so the pinned
    /// header and footer pages can never starve the cache.
    #[must_use]
    pub const fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = normalize_max_bytes(max_bytes, self.page_size);
        self
    }

    /// Return these options with a different time to live.
    ///
    /// The life of a page is counted from the last read that touched it, not
    /// from when it was fetched.
    #[must_use]
    pub const fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Return the bytes one page holds.
    #[must_use]
    pub const fn page_size(&self) -> usize {
        self.page_size
    }

    /// Return the bytes every cached page may occupy together.
    #[must_use]
    pub const fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    /// Return how long a page outlives its last access.
    #[must_use]
    pub const fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Return the index of the page holding `offset`.
    #[must_use]
    pub const fn page_index(&self, offset: u64) -> u64 {
        offset >> self.page_shift
    }

    /// Return the offset the page at `index` starts at.
    ///
    /// An index beyond the addressable range saturates rather than wrapping,
    /// which reads as "past the end" everywhere an offset is used.
    #[must_use]
    pub const fn page_start(&self, index: u64) -> u64 {
        if index > (u64::MAX >> self.page_shift) {
            return u64::MAX;
        }
        index << self.page_shift
    }

    /// Return the page size as the width offsets are measured in.
    pub(crate) const fn page_size_u64(&self) -> u64 {
        self.page_size as u64
    }
}

impl Default for BufferedOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// Round a requested page size up to a clamped power of two.
const fn normalize_page_size(page_size: usize) -> usize {
    if page_size <= MIN_PAGE_SIZE {
        return MIN_PAGE_SIZE;
    }
    if page_size >= MAX_PAGE_SIZE {
        return MAX_PAGE_SIZE;
    }
    // `next_power_of_two` is not const-callable on every supported release, so
    // the doubling walk stands in for it; it runs at most 30 steps.
    let mut size = MIN_PAGE_SIZE;
    while size < page_size {
        size *= 2;
    }
    size
}

/// Raise a byte budget to hold at least the two pinned pages.
const fn normalize_max_bytes(max_bytes: u64, page_size: usize) -> u64 {
    let floor = (page_size as u64) * 2;
    if max_bytes < floor { floor } else { max_bytes }
}
