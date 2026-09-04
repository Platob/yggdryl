//! The one iterator every listing answers with.
//!
//! A listing says what is there; it must never require holding all of it. So
//! [`IOBase::ls`](super::IOBase::ls), [`glob`](super::IOBase::glob), and the
//! predicate listings all hand back a [`Listing`]: one entry at a time, from a
//! walk that is still running.
//!
//! [`IOBase`](super::IOBase) is object-safe and stays so, which is why this is
//! one *named* type rather than `impl Iterator` or a bare `Box<dyn Iterator>`
//! in a public signature - a `dyn IOBase` keeps working and a binding can name
//! what it wraps. There is exactly one such type, because there is exactly one
//! item kind: a [`Holder`].

use std::collections::VecDeque;

use crate::generic::Holder;
use crate::{Error, IOBase, Result};

/// The entries of one listing, yielded one at a time.
///
/// The item is a [`Result`], so a listing fails *at* the failing entry, naming
/// it, without discarding what it already yielded. After that first failure the
/// iterator is fused: it yields `None` forever rather than looping against a
/// backend that is already refusing.
///
/// Order is deterministic and documented by whoever built the listing: the same
/// listing over the same state yields the same sequence.
///
/// ```
/// use yggdryl::io::{Buffer, IOBase};
///
/// // A resource that cannot contain others lists nothing rather than failing.
/// let buffer = Buffer::new();
/// assert_eq!(buffer.ls(false, false).count(), 0);
/// ```
pub struct Listing {
    /// The walk still running. `None` once the listing is spent.
    entries: Option<Box<dyn Iterator<Item = Result<Holder>> + Send + Sync>>,
}

impl Listing {
    /// A listing of nothing, which is what a leaf answers.
    ///
    /// Boxing an empty iterator allocates nothing: it is zero-sized.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            entries: Some(Box::new(std::iter::empty())),
        }
    }

    /// Wrap a walk that is already lazy.
    ///
    /// The walk is consumed one entry at a time, so whatever it holds is its
    /// own business - and whatever that is must be bounded and say so.
    pub fn new(entries: impl Iterator<Item = Result<Holder>> + Send + Sync + 'static) -> Self {
        Self {
            entries: Some(Box::new(entries)),
        }
    }

    /// A listing that reports one failure and then ends.
    ///
    /// This is how a walk whose *first* step fails - an unreadable directory, a
    /// pattern the backend refuses - stays a listing rather than becoming a
    /// second return shape.
    #[must_use]
    pub fn failing(error: Error) -> Self {
        Self::new(std::iter::once(Err(error)))
    }

    /// Descend every container this listing yields, depth first, pre-order.
    ///
    /// Each entry is yielded before the subtree beneath it, exactly as the
    /// eager walk did. What is held is the *frontier*: one level's remaining
    /// entries per open depth, and nothing of the result. A ten-thousand-entry
    /// folder therefore costs one entry list, not ten thousand holders.
    #[must_use]
    pub fn descending(self, include_private: bool) -> Self {
        Self::new(Descent {
            stack: VecDeque::from([self]),
            include_private,
            done: false,
        })
    }

    /// Keep only the entries a predicate accepts, without listing more.
    ///
    /// The predicate sees each entry as it arrives, so a losing entry is
    /// dropped before the next one is fetched.
    #[must_use]
    pub fn keeping(self, keep: impl FnMut(&Holder) -> bool + Send + Sync + 'static) -> Self {
        let mut keep = keep;
        Self::new(self.filter(move |entry| match entry {
            Ok(holder) => keep(holder),
            // A failure is never filtered out: it is the listing's answer.
            Err(_) => true,
        }))
    }
}

impl Iterator for Listing {
    type Item = Result<Holder>;

    fn next(&mut self) -> Option<Self::Item> {
        let entries = self.entries.as_mut()?;
        match entries.next() {
            Some(Ok(entry)) => Some(Ok(entry)),
            // Fused at the first failure: a listing that has started refusing
            // will keep refusing, and spinning against it helps nobody.
            Some(Err(error)) => {
                self.entries = None;
                Some(Err(error))
            }
            None => {
                self.entries = None;
                None
            }
        }
    }
}

impl std::iter::FusedIterator for Listing {}

impl std::fmt::Debug for Listing {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Listing")
            .field("spent", &self.entries.is_none())
            .finish()
    }
}

impl Default for Listing {
    fn default() -> Self {
        Self::empty()
    }
}

impl FromIterator<Result<Holder>> for Listing {
    fn from_iter<I: IntoIterator<Item = Result<Holder>>>(entries: I) -> Self {
        Self::new(entries.into_iter().collect::<Vec<_>>().into_iter())
    }
}

/// A depth-first pre-order walk that holds its frontier and not its result.
///
/// The stack holds one partially-drained [`Listing`] per open depth level, so
/// what is retained is bounded by the tree's *depth* times one directory's
/// entry cursor - never by the number of entries the walk will yield.
struct Descent {
    /// One partially-drained listing per open level; the front is the deepest.
    stack: VecDeque<Listing>,
    /// Whether private entries are descended into and yielded.
    include_private: bool,
    /// Fused after the first failure, exactly as [`Listing`] is.
    done: bool,
}

impl Iterator for Descent {
    type Item = Result<Holder>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        loop {
            let level = self.stack.front_mut()?;
            let Some(entry) = level.next() else {
                self.stack.pop_front();
                continue;
            };
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    self.done = true;
                    return Some(Err(error));
                }
            };
            if entry.is_container() {
                // Pre-order: the container is yielded first, and its own
                // listing goes on the front so the subtree comes next.
                self.stack.push_front(entry.ls(false, self.include_private));
            }
            return Some(Ok(entry));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Listing;
    use crate::Error;

    #[test]
    fn a_listing_is_fused_after_the_first_failure() {
        let mut listing = Listing::new(
            [
                Err(Error::absent("file", "a")),
                Err(Error::absent("file", "b")),
            ]
            .into_iter(),
        );
        assert!(listing.next().is_some_and(|entry| entry.is_err()));
        assert!(listing.next().is_none());
        assert!(listing.next().is_none());
    }

    #[test]
    fn an_empty_listing_yields_nothing() {
        assert_eq!(Listing::empty().count(), 0);
        assert_eq!(Listing::default().count(), 0);
    }

    #[test]
    fn a_failing_listing_reports_once_and_ends() {
        let entries: Vec<_> = Listing::failing(Error::absent("folder", "gone")).collect();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].as_ref().is_err_and(Error::is_absent));
    }
}
