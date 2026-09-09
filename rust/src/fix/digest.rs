//! A message's value digest, and the adapter that drops a republished line.
//!
//! # What it is over
//!
//! The [entries](super::FixEntry), in arrival order. Not the row: the row is
//! an interpretation, and two readers at two versions interpret one wire
//! record into two rows. The entries are what arrived, so a digest over them
//! answers "the same message?" and nothing about how it was read.
//!
//! Arrival order, never sorted. Order carries meaning inside a repeating
//! group, and sorting would cost an allocation per message to produce a
//! worse answer.
//!
//! # Why the lengths are written down
//!
//! Each value is fed as its length and then its bytes, rather than separated
//! by anything. A FIX value may contain any byte at all - `data` fields exist
//! precisely for that - so a separator-framed digest would make two different
//! messages equal. `("1", "23")` and `("12", "3")` are the shortest case and
//! the test that pins it.
//!
//! # The standard header and trailer are excluded, and only those
//!
//! This is a digest of what a message *says*, not of the frame it said it in,
//! so the two components FIX wraps every message in are left out and the body
//! is what remains. Read from [`STANDARD_HEADER_TAGS`] and
//! [`STANDARD_TRAILER_TAGS`] rather than from a list of this module's own, so
//! a tag either component gains is excluded here without a second listing
//! learning about it - and so the rule is nameable in one sentence instead of
//! being three groups a reader has to check.
//!
//! What that covers, by the roles the header and trailer play: `BeginString`,
//! `BodyLength`, `CheckSum`, `Signature` and `SignatureLength` describe how it
//! was written down; `SenderCompID`, `TargetCompID` and their sub- and
//! location- variants describe who wrote it; `MsgSeqNum`, `SendingTime`,
//! `OrigSendingTime`, `PossDupFlag` and `PossResend` describe *this* delivery
//! of it; `ApplVerID`, `CstmApplVerID` and `MessageEncoding` describe how to
//! read it. None of them is the message.
//!
//! `MsgType` is the header's one exception and stays in, because a message
//! type is what a message *is* rather than how it travelled: an order and a
//! report that happen to carry the same tags are not one message.
//!
//! The consequence is the point and is worth stating: two identical orders
//! sent a second apart hash equal, and so do the same order relayed through
//! two sessions or replayed on a resend. That is what a *value* digest is
//! for - it answers "the same message?" about content, which is what
//! deduplicating a capture, joining a relayed message to its original, and
//! recognising a redelivery all need. A consumer that wants to tell two
//! deliveries apart reads the sequence number and the time, which are columns
//! of their own beside it.
//!
//! This crate's own derived fields are excluded too, for the plainer reason
//! that a value cannot cover itself: the digest is one of them.

use crate::digest::DigestAlgorithm;

use super::msg::FixMsg;
use super::{MSGTYPE_TAG, STANDARD_HEADER_TAGS, STANDARD_TRAILER_TAGS};

/// Whether one tag belongs to the envelope rather than the message.
///
/// The envelope is the standard header and the standard trailer, whole, less
/// the one tag in them that says what the message is.
fn is_envelope(tag: i32) -> bool {
    if tag == MSGTYPE_TAG {
        return false;
    }
    STANDARD_HEADER_TAGS.contains(&tag)
        || STANDARD_TRAILER_TAGS.contains(&tag)
        // A value cannot cover itself, and every derived field is computed
        // from this one or beside it.
        || super::fix_crate_fields()
            .unwrap_or_default()
            .iter()
            .any(|field| field.as_fix().tag().ok().flatten() == Some(tag))
}

impl FixMsg {
    /// This message's value digest.
    ///
    /// Computed on every call and stored nowhere. Two calls answer the same
    /// value because the entries do not change, and a cached digest is a fact
    /// that a later edit makes a lie - the invalidation rule that would
    /// prevent it costs more than the walk it saves. [`Direction`] is the
    /// opposite case and *is* stored, because the bytes it is read from are
    /// gone by the time anyone could ask again.
    ///
    /// 128 bits rather than 64. A day of capture is comfortably a billion
    /// messages, and the birthday bound puts a 64-bit digest into collision
    /// around ten times that - so a 64-bit dedup key silently drops a real
    /// message roughly once per large capture. Sixteen bytes a row is the
    /// price.
    ///
    /// Nothing is materialized: the entries feed a resumable state as they
    /// are walked, and the length prefixes come from a stack array, so
    /// hashing a million messages allocates nothing.
    ///
    /// A message built from a schema and a value has no entries and digests
    /// as the empty walk - the same answer for every such message, which is
    /// correct: none of them arrived.
    ///
    /// [`Direction`]: crate::types::MsgDirection
    ///
    /// ```
    /// # fn main() -> yggdryl::Result<()> {
    /// # use std::sync::Arc;
    /// # use yggdryl::holder::local::Folder;
    /// # use yggdryl::{FixCodec, FixRegistry};
    /// # let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    /// # let registry = FixRegistry::from_handle(&Folder::new(root)?)?;
    /// let reader = FixCodec::new(Arc::new(registry));
    /// let sent = reader.transform_fix_line(b"8=FIX.4.4|9=64|35=D|11=A|55=AAPL|10=203|", false)?;
    ///
    /// // The frame is not the message: a different separator, a recomputed
    /// // body length and a different checksum are the same message.
    /// let again = reader.transform_fix_line(b"8=FIX.4.4|9=99|35=D|11=A|55=AAPL|10=000|", false)?;
    /// assert_eq!(sent.digest(), again.digest());
    ///
    /// // A different value is a different message.
    /// let other = reader.transform_fix_line(b"8=FIX.4.4|35=D|11=A|55=MSFT|10=203|", false)?;
    /// assert_ne!(sent.digest(), other.digest());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn digest(&self) -> u128 {
        let mut state = DigestAlgorithm::Xxh128.digester();
        walk(&mut state, self.entries());
        state
            .as_digest()
            .as_u128()
            .expect("the 128-bit algorithm answers 128 bits")
    }
}

/// Feeds one level of entries, pre-order, children under their parent.
///
/// After each entry's length-prefixed value bytes comes its child count as
/// four big-endian bytes - zero included - and then the children themselves,
/// recursed directly over the slice with nothing allocated. The count is what
/// separates a parent of two from two flat siblings, so two messages that
/// differ only below the Arrow materialization depth still hash apart: the
/// digest walks the Rust tree, which is never truncated.
///
/// An excluded envelope tag excludes its entire subtree, exactly as it
/// excluded its flat entry before entries nested.
fn walk(state: &mut crate::digest::Digester, entries: &[super::FixEntry]) {
    for entry in entries {
        let tag = entry.tag();
        if is_envelope(tag) {
            continue;
        }
        state.write_bytes(&tag.to_be_bytes());
        // The key only where the tag named no field. A resolved entry is
        // identified by its tag, and two spellings of one tag are one
        // field; an unresolved one has nothing but its key, so two rows
        // whose unknown keys differ are two messages.
        if tag == 0 {
            let key = entry.key().as_bytes();
            state.write_bytes(&length_of(key));
            state.write_bytes(key);
        }
        let value = entry.value().as_bytes();
        state.write_bytes(&length_of(value));
        state.write_bytes(value);
        state.write_bytes(&(entry.children().len() as u32).to_be_bytes());
        walk(state, entry.children());
    }
}

/// One length as four big-endian bytes, saturating rather than wrapping.
///
/// A value longer than four gigabytes cannot be distinguished from one at the
/// bound, which no FIX field is and no capture line could be.
fn length_of(bytes: &[u8]) -> [u8; 4] {
    u32::try_from(bytes.len()).unwrap_or(u32::MAX).to_be_bytes()
}

/// Drops each message whose digest equals the one before it.
///
/// A line a capture tool published twice is adjacent, and the second is
/// dropped. Two identical heartbeats an hour apart are two events and both
/// survive: non-adjacent identity is a question about a *window* - how wide,
/// measured how - and that policy belongs to the caller.
///
/// One `u128` of state whatever the stream's length. Never a set: a set over
/// a day's capture grows without bound, which is the exact failure a
/// byte-shaped batch bound exists to avoid, reintroduced one layer up.
///
/// A resend survives. A message replayed under `PossDupFlag` carries a fresh
/// `SendingTime` and so digests differently - deliberately, because dedup
/// catches the same bytes twice and the `resent` facet catches the replay,
/// and neither is made to do the other's job. A dedup that folded resends
/// would drop exactly the recovery traffic a sequence-gap check reads.
///
/// ```
/// # fn main() -> yggdryl::Result<()> {
/// # use std::sync::Arc;
/// # use yggdryl::holder::local::Folder;
/// # use yggdryl::{FixDedup, FixCodec, FixRegistry};
/// # let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
/// # let registry = FixRegistry::from_handle(&Folder::new(root)?)?;
/// let reader = FixCodec::new(Arc::new(registry));
/// let rows = [
///     "8=FIX.4.4|35=D|11=A|10=0|",
///     "8=FIX.4.4|35=D|11=A|10=0|",
///     "8=FIX.4.4|35=D|11=B|10=0|",
///     "8=FIX.4.4|35=D|11=A|10=0|",
/// ];
/// let read = rows.iter().map(|row| reader.transform_fix_line(row.as_bytes(), false).expect("a readable row"));
///
/// let mut dedup = FixDedup::new(read);
/// let kept: Vec<String> = dedup.by_ref().map(|held| held.into_text('|').unwrap()).collect();
/// // The republished line goes; the one that comes back after B does not,
/// // because it is no longer adjacent to its twin.
/// assert_eq!(kept.len(), 3);
/// assert_eq!(dedup.dropped(), 1);
/// # Ok(())
/// # }
/// ```
pub struct FixDedup<I> {
    inner: I,
    last: Option<u128>,
    dropped: u64,
}

impl<I> FixDedup<I> {
    /// Wraps one stream of messages.
    pub const fn new(inner: I) -> Self {
        Self {
            inner,
            last: None,
            dropped: 0,
        }
    }

    /// How many messages this adapter has dropped so far.
    ///
    /// Dropping is counted because nothing here loses data quietly, and an
    /// opt-in filter is not an exception to that - it is the one place the
    /// loss has to be reported instead of avoided.
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    /// The stream underneath, given back.
    pub fn into_inner(self) -> I {
        self.inner
    }
}

impl<I: Iterator<Item = FixMsg>> Iterator for FixDedup<I> {
    type Item = FixMsg;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let message = self.inner.next()?;
            let digest = message.digest();
            if self.last == Some(digest) {
                self.dropped += 1;
                continue;
            }
            self.last = Some(digest);
            return Some(message);
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        // Everything may be dropped and nothing may be, so only the upper
        // bound survives the filter.
        (0, self.inner.size_hint().1)
    }
}

impl<I: Iterator<Item = FixMsg>> std::iter::FusedIterator for FixDedup<I> {}
