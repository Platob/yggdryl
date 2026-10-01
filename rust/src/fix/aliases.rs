//! The spellings every desk uses for the same field.
//!
//! A venue writes `OfferPx`, a blotter writes `AskPrice`, a risk system
//! writes `AskPx`, and FIX publishes exactly one of them. Four word pairs
//! carry all of them, each a word this industry uses two ways for one thing:
//!
//! | FIX | Also written |
//! | --- | --- |
//! | `offer` | `ask` |
//! | `size` | `qty` |
//! | `bid` | `demand` |
//! | `px` | `price` |
//!
//! Every name lookup a registry answers reads them, either way, wherever
//! one of the words stands in the name as the crate's one fold spells it -
//! case folded, `_`, `-` and spaces dropped - so `AskPx`, `ASK_PX`, `askpx`
//! and `AskPrice` reach `OfferPx(133)`, `DemandQty` reaches `BidSize(134)`
//! and a bridge's `ULLINK.OFFERPRICE` reaches `OfferPx` under its namespace.
//! Nothing is written into the dictionary: a registry's names are its own,
//! and the words are read at the lookup, after every name a field holds
//! exactly, canonically or as an alias. A name a dictionary defines as a
//! field of its own is that field, so a venue that really has an `AskPx` tag
//! keeps it; a spelling the words make reaching two different fields reaches
//! none, because an ambiguous alias is refused rather than chosen.
//!
//! # When both spellings arrive
//!
//! Namespaced bridge voices resolve to one field: conflicting voices fill
//! nothing. A row carrying `FIRM.ORIG.OFFERPX=10` and
//! `ULLINK.OFFERPRICE=11` disagrees, so composition leaves the canonical field
//! unfilled and the message's metadata keeps both voices. Agreeing voices fill
//! once. A flat alias stating what its field already holds is that field; one
//! stating another value is kept in the metadata beside an anomaly, so the
//! wire carries the dictionary's own fields alone. A write to the field
//! states every voice composed into it again.

/// The word pairs, FIX's own spelling first.
const TWINS: [(&str, &str); 4] = [
    ("offer", "ask"),
    ("size", "qty"),
    ("bid", "demand"),
    ("px", "price"),
];

/// The longest folded name the words are read in; a longer one is no
/// spelling a desk writes, and stays as it is.
const MAX_NAME: usize = 64;

/// The most words one name is read with: fifteen other spellings at most.
const MAX_WORDS: usize = 4;

/// The word of a pair `folded` opens with, as its length and its twin.
fn word_at(folded: &[u8]) -> Option<(usize, &'static str)> {
    TWINS.iter().find_map(|(fix, other)| {
        if folded.starts_with(fix.as_bytes()) {
            Some((fix.len(), *other))
        } else if folded.starts_with(other.as_bytes()) {
            Some((other.len(), *fix))
        } else {
            None
        }
    })
}

/// The one answer `find` gives for the other spellings of `name` the word
/// pairs make, every combination of its words swapped for their twins:
/// nothing where no spelling finds one, or where two find different ones.
///
/// The name is folded and every spelling built on the stack, so a lookup
/// that finds nothing allocates nothing.
pub(super) fn aliased<T: PartialEq>(
    name: &str,
    mut find: impl FnMut(&str) -> Option<T>,
) -> Option<T> {
    let mut folded = [0_u8; MAX_NAME];
    let mut length = 0;
    for byte in name.as_bytes() {
        if matches!(byte, b'_' | b'-' | b' ') {
            continue;
        }
        *folded.get_mut(length)? = byte.to_ascii_lowercase();
        length += 1;
    }
    let folded = &folded[..length];
    // The words, left to right and never overlapping: where each starts,
    // how long it is and what it is also written as.
    let mut words = [(0_usize, 0_usize, ""); MAX_WORDS];
    let mut count = 0;
    let mut at = 0;
    while at < folded.len() {
        match word_at(&folded[at..]) {
            Some((width, twin)) => {
                *words.get_mut(count)? = (at, width, twin);
                count += 1;
                at += width;
            }
            None => at += 1,
        }
    }
    let words = &words[..count];
    let mut found = None;
    for mask in 1_u32..(1 << words.len()) {
        let mut spelled = [0_u8; MAX_NAME + 3 * MAX_WORDS];
        let mut held = 0;
        let mut from = 0;
        for (bit, (start, width, twin)) in words.iter().enumerate() {
            let replaced = if mask & (1 << bit) == 0 {
                &folded[*start..start + width]
            } else {
                twin.as_bytes()
            };
            for part in [&folded[from..*start], replaced] {
                spelled[held..held + part.len()].copy_from_slice(part);
                held += part.len();
            }
            from = start + width;
        }
        let tail = &folded[from..];
        spelled[held..held + tail.len()].copy_from_slice(tail);
        held += tail.len();
        // Only ASCII words were swapped, at ASCII boundaries, so what was
        // UTF-8 still is.
        let Ok(spelled) = std::str::from_utf8(&spelled[..held]) else {
            continue;
        };
        let Some(hit) = find(spelled) else {
            continue;
        };
        match &found {
            None => found = Some(hit),
            Some(held) if *held == hit => {}
            Some(_) => return None,
        }
    }
    found
}
