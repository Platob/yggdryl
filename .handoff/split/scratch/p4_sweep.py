#!/usr/bin/env python3
"""P4 / D38: the event's two instants are `transunix` and `sendunix`, and the
element's own names drop `curr`.

`currunix` becomes `transunix` - the transaction instant, when the operation
really happened, the identity and order axis - `recdunix` becomes `sendunix` -
the technical wire clock, when the message crossed the wire, the merge
reference - `curruuid` becomes `uuid` and `currhashcode` becomes `hashcode`,
the element's own identifier and content code, at every door of the tree, in
one pass. `prevuuid`, `crossuuid`, `crosshashcode`, `srcuuids` and `currency`
are not touched.

Usage: python3 p4_sweep.py <tree root> [--check] [--emit-table]

Four phases, every one validated before any file is written, so a run either
writes the whole sweep or writes nothing:

1. The meaning, re-written by hand: `EDITS`, exact anchors in the tree's
   original spelling, each asserted to occur exactly the count it states, each
   replacement already in the new spelling.
2. The hash sentence: the newest `It last moved when` of the dictionary-hash
   rustdoc, added below the earlier ones - the rustdoc reads oldest first -
   directly above the test, every earlier sentence kept as written.
3. The bare names: every case spelling of the four names, and every longer
   identifier they open or close (`get_currunix`, `walked_currunix`,
   `EventColumn::CurrUnix`, `ElementColumn::CurrUuid`, `CURRUUID_TAG_NAME`,
   `eventCurrunix`, `fix_event_recdunix`, `curruuids`), in every text file but
   the excluded paths, each file asserted to hold the count `BULK_EXPECTED`
   states. A lower-case or upper-case name is matched where no letter or digit
   precedes it and a capitalised one where no upper-case letter or digit does,
   so `refrecdunix` - the retired FIX field 65_064, a name of its own - keeps
   its spelling; so does the dictionary-hash rustdoc, whose earlier sentences
   are history.
4. The layout rustfmt gives the swept Rust: `FMT_EDITS`, exact anchors in the
   swept spelling - a line the shorter `uuid` and `hashcode` let rustfmt join,
   one the longer `transunix` pushes past the width, a `use` list whose order
   the new names move - each asserted to occur the count it states.

Idempotent: an applied edit is recognised by its replacement and skipped, and
a file whose names are already swept holds none, so a second run writes
nothing. `--check` validates and writes nothing; `--emit-table` prints the
`BULK_EXPECTED` table the tree answers after phases 1 and 2.
"""

import fnmatch
import re
import subprocess
import sys
from pathlib import Path

OLD_TO_NEW = {
    # The event's two instants.
    "currunix": "transunix",
    "recdunix": "sendunix",
    "CurrUnix": "TransUnix",
    "RecdUnix": "SendUnix",
    "CURRUNIX": "TRANSUNIX",
    "RECDUNIX": "SENDUNIX",
    "Currunix": "Transunix",
    "Recdunix": "Sendunix",
    # The element's own identifier and content code.
    "curruuid": "uuid",
    "currhashcode": "hashcode",
    "CurrUuid": "Uuid",
    "CurrHashCode": "HashCode",
    "CURRUUID": "UUID",
    "CURRHASHCODE": "HASHCODE",
    "Curruuid": "Uuid",
    "Currhashcode": "Hashcode",
}

# A lower-case or upper-case name where no letter or digit precedes it, a
# capitalised one where no upper-case letter or digit does (`eventCurrunix`);
# what follows is free (`get_currunix`, `CURRUUID_TAG_NAME`, `curruuids`).
# `refrecdunix` - the retired FIX field 65_064 - is the one longer name
# holding an old one that keeps its spelling, and no rule reaches it.
_PLAIN = [name for name in OLD_TO_NEW if name.islower() or name.isupper()]
_CAPITAL = [name for name in OLD_TO_NEW if not (name.islower() or name.isupper())]
BULK = re.compile(
    r"(?<![A-Za-z0-9])(" + "|".join(_PLAIN) + r")"
    r"|(?<![A-Z0-9])(?<!Ref)(" + "|".join(_CAPITAL) + r")"
)
OLD_ANY = re.compile(r"(?i)(?<!ref)(currunix|recdunix|curruuid|currhashcode)")

HASH_TEST = "    #[test]\n    fn the_committed_dictionary_hashes_to_one_pinned_value() {\n"
# The design's sentence, its tags spelled as the rustdoc's earlier ones spell
# theirs; `hashcode` is 65004 (65002 is `crossuuid`).
HASH_SENTENCE = (
    "    /// It last moved when `curruuid`, `currhashcode`, `currunix` and\n"
    "    /// `recdunix` became `uuid`, `hashcode`, `transunix` and `sendunix`:\n"
    "    /// 65001, 65004, 65007 and 65009 re-spelled with their descriptions, so\n"
    "    /// the crate's field shard and the fixed row component were written\n"
    "    /// again. No count of the census below moved.\n"
)

EXCLUDED_PREFIXES = (
    "target/",
    ".git/",
    "node/node_modules/",
    "python/.venv",
    ".handoff/",
    "config/fix/",
)
EXCLUDED_PATHS = {
    "rust/tests/fix/equivalence.snapshot",
    "node/index.d.ts",
    "node/index.js",
}
EXCLUDED_GLOBS = ("docs/assets/*.json",)

# A region of a file the bulk sweep leaves as written, from its start anchor up
# to its end anchor: the dictionary-hash rustdoc, whose `It last moved when`
# sentences say what the names were when each moved the hash.
KEPT_REGIONS = {
    "rust/tests/fix/store.rs": (
        "    /// The committed dictionary's hash, pinned as a literal.\n",
        "    fn the_committed_dictionary_hashes_to_one_pinned_value() {\n",
    ),
}


# Phase 1. (path, old, new, count): `old` in the tree's spelling, `new` in the
# swept one.
EDITS = []


def edit(path, old, new, count=1):
    EDITS.append((path, old, new, count))


# rust/src/graph/column.rs - the two columns' docs, displays and descriptions.
edit(
    "rust/src/graph/column.rs",
    "/// generated schema use: **when** it happened - the instant, then the\n",
    "/// generated schema use: **when** it happened - the transaction instant, then the\n",
)
edit(
    "rust/src/graph/column.rs",
    "    /// When the event happened, a nanosecond UTC clock; never absent.\n",
    "    /// When the operation happened - the transaction instant, a nanosecond\n"
    "    /// UTC clock; never absent.\n",
)
edit(
    "rust/src/graph/column.rs",
    "    /// When it was recorded, where that is known.\n",
    "    /// When the message crossed the wire - the technical clock - where that\n"
    "    /// is known.\n",
)
edit("rust/src/graph/column.rs", '"Current Time"', '"Transaction Time"')
edit("rust/src/graph/column.rs", '"Recording Time"', '"Sending Time"')
edit(
    "rust/src/graph/column.rs",
    '"When the event happened: the settled instant, UTC."',
    '"When the operation happened: the settled transaction instant, UTC."',
)
edit(
    "rust/src/graph/column.rs",
    '"When this event was recorded, where that is known; the earliest its statements know."',
    '"When the message crossed the wire, where that is known; the earliest its statements know."',
)

# rust/src/graph/element.rs - the `Event` trait's two instants and the merge
# reference over them.
E = "rust/src/graph/element.rs"
edit(
    E,
    "/// The element facts of two event statements, with the recording-selected\n"
    "/// reference leading conflicts and list order. An unstated fact on the\n"
    "/// reference is still filled by the other statement.\n",
    "/// The element facts of two event statements, with the reference their\n"
    "/// wire clocks select leading conflicts and list order. An unstated fact on\n"
    "/// the reference is still filled by the other statement.\n",
)
edit(
    E,
    "/// `recdunix`. The most recently recorded statement leads; a stated recording\n"
    "/// clock leads an unstated one, and equal or absent recording clocks fall\n"
    "/// back to the later event instant. Exact ties keep `left`.\n"
    "///\n"
    "/// A folded statement keeps the earliest recording its statements know, so\n",
    "/// `sendunix`. The statement sent last leads; a stated wire clock leads an\n"
    "/// unstated one, and equal or absent wire clocks fall back to the later\n"
    "/// `transunix`. Exact ties keep `left`.\n"
    "///\n"
    "/// A folded statement keeps the earliest `sendunix` its statements know, so\n",
)
edit(
    E,
    "/// earliest recording either statement knows; whether it moved. It never\n",
    "/// earliest `sendunix` either statement knows; whether it moved. It never\n",
)
edit(
    E,
    "/// earliest recording instant, the reference statement's\n"
    "/// instant and code, the higher place, the lifecycle folded,\n",
    "/// earliest `sendunix`, the reference statement's\n"
    "/// `transunix` and code, the higher place, the lifecycle folded,\n",
)
edit(
    E,
    "/// The instant is `currunix`: a count of nanoseconds since the Unix epoch, UTC,\n"
    "/// held as an `i64`, the count every clock this crate reads states. Coupled\n",
    "/// The instant is `transunix`, the transaction instant - when the operation\n"
    "/// really happened: a count of nanoseconds since the Unix epoch, UTC, held\n"
    "/// as an `i64`, the count every clock this crate reads states. Coupled\n",
)
edit(
    E,
    "/// states them only where it knows them: when it was created, when it was\n"
    "/// recorded and when it expires, each an instant in the same count; the\n",
    "/// states them only where it knows them: when it was created, when its\n"
    "/// message crossed the wire - `sendunix`, the technical clock - and when it\n"
    "/// expires, each an instant in the same count; the\n",
)
edit(
    E,
    "/// expiration, including one that shortens the lifetime. Recording belongs\n"
    "/// to one observation and never follows, while two observations of the same\n"
    "/// event keep the earliest. When a market event last executed is a market\n"
    "/// fact, [`Market::get_execunix`](super::Market::get_execunix), and its\n"
    "/// readings fold it.\n",
    "/// expiration, including one that shortens the lifetime. The wire clock\n"
    "/// belongs to one observation and never follows, while two observations of\n"
    "/// the same event keep the earliest. When a market event last executed is a\n"
    "/// market fact, [`Market::get_execunix`](super::Market::get_execunix), and\n"
    "/// its readings fold it.\n",
)
edit(
    E,
    "/// The order an event states through [`Element::is_after`] is its instant:\n"
    "/// later is after.\n",
    "/// The order an event states through [`Element::is_after`] is its\n"
    "/// transaction instant: later is after.\n",
)
edit(
    E,
    "    /// When this event happened: nanoseconds since the Unix epoch, UTC.\n",
    "    /// When the operation this event states really happened - its\n"
    "    /// transaction instant: nanoseconds since the Unix epoch, UTC.\n",
)
edit(
    E,
    "    /// Records when this event happened, as nanoseconds since the Unix\n"
    "    /// epoch, UTC.\n",
    "    /// Records when the operation this event states really happened, as\n"
    "    /// nanoseconds since the Unix epoch, UTC.\n",
)
edit(
    E,
    "    /// When this event was recorded, in the same count as\n"
    "    /// [`Self::get_currunix`], where it knows.\n",
    "    /// When this event's message crossed the wire - the technical clock - in\n"
    "    /// the same count as [`Self::get_transunix`], where it knows.\n",
)
edit(
    E,
    "    /// Records when this event was recorded; `None` states it does not know.\n",
    "    /// Records when this event's message crossed the wire; `None` states it\n"
    "    /// does not know.\n",
)
edit(
    E,
    "    /// of one chain share it. What the event itself says - its instant, recording clock,\n",
    "    /// of one chain share it. What the event itself says - its instant, wire clock,\n",
)
edit(
    E,
    "    /// one identity and the chain grows by nothing. Their recording instants\n",
    "    /// one identity and the chain grows by nothing. Their wire clocks\n",
)
edit(
    E,
    "    /// `recdunix`; a stated clock leads an unstated one, a tie falls back\n"
    "    /// to the later event instant, and an exact tie keeps this one. The\n",
    "    /// `sendunix` - the statement sent last; a stated clock leads an unstated\n"
    "    /// one, a tie falls back to the later `transunix`, and an exact tie keeps\n"
    "    /// this one. The\n",
)
edit(
    E,
    "    /// latest expiration, furthest state; the recording clock is the\n",
    "    /// latest expiration, furthest state; the wire clock is the\n",
)
edit(
    E,
    "    /// earliest recording it knows against a third, so which of three leads\n",
    "    /// earliest `sendunix` it knows against a third, so which of three leads\n",
)
edit(
    E,
    "    /// content behind. The instants - when it happened, was created,\n"
    "    /// executed, recorded, expires, the predecessor's and the snapshot's -\n"
    "    /// are left out, so the code says what an event states and not when.\n",
    "    /// content behind. The instants - when it happened, was created,\n"
    "    /// executed, crossed the wire, expires, the predecessor's and the\n"
    "    /// snapshot's - are left out, so the code says what an event states and\n"
    "    /// not when.\n",
)
edit(
    E,
    "    /// The instant and the code coupled: a [`TxHash`] of [`Self::get_currunix`]\n"
    "    /// at nanosecond resolution and [`Element::get_currhashcode`] as the XXH3-64\n"
    "    /// digest it is, which is the crate's own time-ordered identity.\n",
    "    /// The transaction instant and the code coupled: a [`TxHash`] of\n"
    "    /// [`Self::get_transunix`] at nanosecond resolution and\n"
    "    /// [`Element::get_hashcode`] as the XXH3-64 digest it is, which is the\n"
    "    /// crate's own time-ordered identity.\n",
)
edit(
    E,
    "    /// The generic event identity: RFC 9562 UUIDv7 with\n"
    "    /// [`Self::get_currunix`] floored to milliseconds in its timestamp,\n",
    "    /// The generic event identity: RFC 9562 UUIDv7 with the transaction\n"
    "    /// instant [`Self::get_transunix`] floored to milliseconds in its timestamp,\n",
)

# rust/src/implementer.rs - the reference and the fold, for the split crates.
I = "rust/src/implementer.rs"
edit(
    I,
    "/// whether the right of two statements of one event is the reference, by\n"
    "/// their recording clocks, then their instants.\n",
    "/// whether the right of two statements of one event is the reference, by\n"
    "/// their `sendunix`, then their `transunix`.\n",
)
edit(
    I,
    "/// `graph::element::fold_event_instants`, for the market crate: the earliest\n"
    "/// recording two statements of one event know; whether it moved.\n",
    "/// `graph::element::fold_event_instants`, for the market crate: the earliest\n"
    "/// `sendunix` two statements of one event know; whether it moved.\n",
)

# rust/src/fix/crated.rs - 65_007 and 65_009, their docs and descriptions.
C = "rust/src/fix/crated.rs"
edit(
    C,
    "//! digests to, when it happened, was created, executed, recorded and was read\n",
    "//! digests to, when it happened, was created, executed, crossed the wire and was read\n",
)
edit(
    C,
    "/// The tag and name carrying when the message happened: the settled\n"
    "/// instant, nanoseconds since the Unix epoch, UTC.\n",
    "/// The tag and name carrying when the operation the message states really\n"
    "/// happened: the settled transaction instant, nanoseconds since the Unix\n"
    "/// epoch, UTC.\n",
)
edit(
    C,
    "/// The tag and name carrying when the message was recorded: by its carrier,\n"
    "/// where a carrier states one, else by its sender, where it states its\n"
    "/// `SendingTime(52)`.\n",
    "/// The tag and name carrying when the message crossed the wire - the\n"
    "/// technical clock: its carrier's, where a carrier states one, else its\n"
    "/// sender's, where it states its `SendingTime(52)`.\n",
)
edit(
    C,
    "            \"When the message was created: what it states, else when it \\\n"
    "             happened; once walked,",
    "            \"When the message was created: what it states, else its \\\n"
    "             transunix; once walked,",
)
edit(
    C,
    "        \"When the message was recorded: by its carrier, where the carrier \\\n"
    "         states one, else by its sender, where it states a SendingTime; the \\\n"
    "         earliest its statements know.\",\n",
    "        \"When the message crossed the wire: its carrier's clock where the \\\n"
    "         carrier states one, else the sender's SendingTime; the earliest its \\\n"
    "         statements know.\",\n",
)

# rust/src/fix/msg.rs - the wire clock a row states, and the merge reference.
M = "rust/src/fix/msg.rs"
edit(M, "ROW_STATED_RECORDING", "ROW_STATED_SENDUNIX", 4)
edit(M, "carries_recording", "carries_sendunix", 3)
edit(
    M,
    "        // Whether the row has a place for the recording at all: a row that\n",
    "        // Whether the row has a place for the wire clock at all: a row that\n",
)
edit(
    M,
    "    /// The instant this message happened: the best official clock standing\n"
    "    /// within `delay` of the sending clock, else the sending clock itself.\n",
    "    /// When the operation this message states happened - its `transunix`:\n"
    "    /// the best official clock standing within `delay` of the sending clock,\n"
    "    /// else the sending clock itself.\n",
)
edit(
    M,
    "    /// latest recording as the reference row and the earliest precise facts.\n",
    "    /// one sent last as the reference row and the earliest precise facts.\n",
)
edit(
    M,
    "    /// which stays the reference whatever the two recording clocks say: for\n"
    "    /// a caller that already chose it, as a run of observations sorted\n"
    "    /// latest recording first does, where re-deciding at every pair would\n"
    "    /// let a later one lead against the earliest recording a fold keeps.\n",
    "    /// which stays the reference whatever the two wire clocks say: for\n"
    "    /// a caller that already chose it, as a run of observations sorted\n"
    "    /// latest `sendunix` first does, where re-deciding at every pair would\n"
    "    /// let a later one lead against the earliest `sendunix` a fold keeps.\n",
)
edit(
    M,
    "    /// Records the message as recorded when it was sent, where its carrier\n"
    "    /// stated no recording and the message states its `SendingTime(52)`:\n"
    "    /// the one recording the message itself states is its sender's. Read\n"
    "    /// once, where the message is built from a row with no `recdunix`\n"
    "    /// column - a row that has one states its own, a null included.\n",
    "    /// Records the message as crossing the wire when it was sent, where its\n"
    "    /// carrier stated no wire clock and the message states its\n"
    "    /// `SendingTime(52)`: the one wire clock the message itself states is\n"
    "    /// its sender's. Read once, where the message is built from a row with\n"
    "    /// no `sendunix` column - a row that has one states its own, a null\n"
    "    /// included.\n",
)

# rust/src/fix/build.rs - the carrier's clock.
edit(
    "rust/src/fix/build.rs",
    "    /// When the carrier recorded the row - the line's own `currunix` - in\n"
    "    /// nanoseconds since the Unix epoch. Applied after the row's explicit\n"
    "    /// fills, so a stated crate `recdunix` stands; it is also the sending\n"
    "    /// clock a message stating no `SendingTime(52)` is dated by where no\n"
    "    /// fill reaches tag 52, ahead of the codec's default.\n",
    "    /// When the message crossed the wire as its carrier saw it - the line's\n"
    "    /// own `transunix` - in nanoseconds since the Unix epoch. Applied after\n"
    "    /// the row's explicit fills, so a stated crate `sendunix` stands; it is\n"
    "    /// also the sending clock a message stating no `SendingTime(52)` is\n"
    "    /// dated by where no fill reaches tag 52, ahead of the codec's default.\n",
)

# rust/src/fix/codec.rs - the line's clock as the message's wire clock.
D = "rust/src/fix/codec.rs"
edit(
    D,
    "    /// from raw bytes states none. The line's `mtime` - its `currunix` -\n"
    "    /// fills the message's `recdunix` as the carrier's recording clock, and\n",
    "    /// from raw bytes states none. The line's `mtime` - its `transunix` -\n"
    "    /// fills the message's `sendunix` as the carrier's wire clock, and\n",
)
edit(
    D,
    "    /// A line whose `mtime` capture does not read states no recording clock,\n",
    "    /// A line whose `mtime` capture does not read states no wire clock,\n",
)
edit(
    D,
    "        // The recording clock is the carrier's statement about the line,\n",
    "        // The wire clock is the carrier's statement about the line,\n",
)
edit(
    D,
    '"FIX recording clock defaulted to none: the line\'s mtime capture does not read"',
    '"FIX wire clock defaulted to none: the line\'s mtime capture does not read"',
)
edit(
    D,
    "    /// that recording clock - and the sending clock of a message stating\n",
    "    /// that wire clock - and the sending clock of a message stating\n",
)
edit(
    D,
    "    /// because a merge keeps the earliest recording either side knows and\n",
    "    /// because a merge keeps the earliest `sendunix` either side knows and\n",
)
edit(
    D,
    "        // the instant the line was recorded at, which is nearer the send\n",
    "        // the instant the line was written at, which is nearer the send\n",
)

# rust/src/fix/enrich.rs - the reference order of a delivery's observations.
N = "rust/src/fix/enrich.rs"
edit(
    N,
    "/// Latest recording first; the event instant breaks absent/equal recording\n"
    "/// ties exactly as the graph's reference selection does.\n",
    "/// Latest `sendunix` first; the `transunix` breaks absent or equal wire\n"
    "/// clock ties exactly as the graph's reference selection does.\n",
)
edit(
    N,
    "/// the lifecycle walk can mistake them for successive events. The most\n"
    "/// recently recorded message is the retained FIX row; the graph fold unions\n",
    "/// the lifecycle walk can mistake them for successive events. The\n"
    "/// message sent last is the retained FIX row; the graph fold unions\n",
)
edit(
    N,
    "/// latest recorded - with the others folded in. An observation whose\n",
    "/// one sent last - with the others folded in. An observation whose\n",
)
edit(
    N,
    "    // each fold keeps the earliest recording, so deciding again at\n",
    "    // each fold keeps the earliest `sendunix`, so deciding again at\n",
)

# rust/src/fix/ulbridge.rs - the line's instant as its messages' wire clock.
edit(
    "rust/src/fix/ulbridge.rs",
    "/// it, never by the file's modification time. The line's instant is its messages'\n"
    "/// `recdunix` and the sending clock of any that states no\n",
    "/// it, never by the file's modification time. The line's instant is its messages'\n"
    "/// `sendunix` - the clock their carrier saw them cross the wire by - and the\n"
    "/// sending clock of any that states no\n",
)

# rust/src/graph/book.rs, market.rs, facts.rs - the merge reference's clock.
edit(
    "rust/src/graph/book.rs",
    "/// book's own at zero, the earliest creation and recording, and the latest\n"
    "/// execution.\n",
    "/// book's own at zero, the earliest creation and `sendunix`, and the\n"
    "/// latest execution.\n",
)
edit(
    "rust/src/graph/book.rs",
    "    /// reference chosen by its recording clock: two complete books' sides\n",
    "    /// reference chosen by its wire clock: two complete books' sides\n",
)
edit(
    "rust/src/graph/market.rs",
    "    /// chosen by its recording clock; `None` where they differ or nothing\n",
    "    /// chosen by its wire clock; `None` where they differ or nothing\n",
)
edit(
    "rust/src/graph/market.rs",
    "    /// reference chosen by its recording clock; `None` where they differ or\n",
    "    /// reference chosen by its wire clock; `None` where they differ or\n",
)
edit(
    "rust/src/graph/facts.rs",
    "    /// and quantity its chain gave it, its instant, place, recording,\n",
    "    /// and quantity its chain gave it, its instant, place, wire clock,\n",
)

# rust/src/text/line.rs, options.rs - the line's one instant and its wire
# clock capture.
T = "rust/src/text/line.rs"
edit(
    T,
    "    /// When the record was written: the header's `mtime` capture, else the\n"
    "    /// handle's own modification time, in nanoseconds since the Unix epoch,\n"
    "    /// UTC; `None` where neither dates it. [`Event::set_currunix`] states\n"
    "    /// the instant, and this answers it.\n",
    "    /// When the record was written, the line's transaction instant: the\n"
    "    /// header's `mtime` capture, else the handle's own modification time, in\n"
    "    /// nanoseconds since the Unix epoch, UTC; `None` where neither dates it.\n"
    "    /// [`Event::set_transunix`] states the instant, and this answers it.\n",
)
edit(
    T,
    "    /// When the line's event was recorded: a `recdunix` capture, else none.\n",
    "    /// When the line's message crossed the wire: a `sendunix` capture, else\n"
    "    /// none - never the `mtime`.\n",
)
edit(
    "rust/src/text/options.rs",
    "/// The row-header capture that dates a line, feeding `currunix`.\n",
    "/// The row-header capture that dates a line, feeding its transaction\n"
    "/// instant, `transunix`.\n",
)

# The bindings' getters.
for path, old, new in [
    (
        "python/src/graph/mod.rs",
        "            /// When this happened: nanoseconds since the Unix epoch, UTC.\n",
        "            /// When the operation happened - the transaction instant:\n"
        "            /// nanoseconds since the Unix epoch, UTC.\n",
    ),
    (
        "python/src/graph/mod.rs",
        "            /// When this was recorded, where stated.\n",
        "            /// When the message crossed the wire - the technical clock -\n"
        "            /// where stated.\n",
    ),
    (
        "node/src/graph/mod.rs",
        "            /// When this happened: nanoseconds since the Unix epoch, UTC.\n",
        "            /// When the operation happened - the transaction instant:\n"
        "            /// nanoseconds since the Unix epoch, UTC.\n",
    ),
    (
        "node/src/graph/mod.rs",
        "            /// When this was recorded, where stated.\n",
        "            /// When the message crossed the wire - the technical clock -\n"
        "            /// where stated.\n",
    ),
    (
        "python/src/fix.rs",
        "    /// When the message happened: nanoseconds since the Unix epoch, UTC -\n",
        "    /// When the operation the message states happened - its transaction\n"
        "    /// instant: nanoseconds since the Unix epoch, UTC -\n",
    ),
    (
        "python/src/fix.rs",
        "    /// When the message was recorded, where stated.\n",
        "    /// When the message crossed the wire - its carrier's clock, else its\n"
        "    /// stated `SendingTime` - where stated.\n",
    ),
    (
        "node/src/fix.rs",
        "    /// When the event happened, nanoseconds since the Unix epoch, UTC.\n",
        "    /// When the operation the message states happened - its transaction\n"
        "    /// instant: nanoseconds since the Unix epoch, UTC.\n",
    ),
    (
        "node/src/fix.rs",
        "    /// When the message was recorded, where stated.\n",
        "    /// When the message crossed the wire - its carrier's clock, else its\n"
        "    /// stated `SendingTime` - where stated.\n",
    ),
]:
    edit(path, old, new)

# .api-inventory.txt - the entries that state what the two instants mean.
V = ".api-inventory.txt"
edit(
    V,
    "the right of two statements of one event is the reference, by their recording clocks, then their instants)",
    "the right of two statements of one event is the reference, by their `sendunix`, then their `transunix`)",
)
edit(
    V,
    "for the market crate: the earliest recording two statements of one event know; whether it moved)",
    "for the market crate: the earliest `sendunix` two statements of one event know; whether it moved)",
)
edit(
    V,
    '(65_007, "currunix");  (non-null nanosecond UTC clock: when the event happened - a stated',
    '(65_007, "transunix");  (non-null nanosecond UTC clock: the transaction instant, when the operation really happened - a stated',
)
edit(
    V,
    '(65_009, "recdunix");  (nullable nanosecond UTC clock: the precise recording instant directly stated by the row, else the text carrier\'s `mtime`; raw bytes have none, and no FIX sending clock stands in)',
    '(65_009, "sendunix");  (nullable nanosecond UTC clock: when the message crossed the wire - the technical clock directly stated by the row, else the text carrier\'s `mtime`, else a stated `SendingTime(52)`; raw bytes stating none have none, and no stand-in sending clock stands in)',
)
edit(
    V,
    "  fn get_currunix(&self) -> i64  (nanoseconds since the Unix epoch, UTC)",
    "  fn get_transunix(&self) -> i64  (the transaction instant - when the operation really happened, the identity and order axis - nanoseconds since the Unix epoch, UTC)",
)
edit(
    V,
    "  fn get_recdunix(&self) -> Option<i64>  (the precise recording instant, in the same count, where known)",
    "  fn get_sendunix(&self) -> Option<i64>  (the technical wire clock - when the message crossed the wire - in the same count, where known; the merge reference, outside identity and every digest)",
)
edit(
    V,
    "(provided: the element-level merge with the later `recdunix` as reference - stated recording beats absent, ties fall back to later `currunix`, an exact tie keeps this one - then that reference's instant, code, predecessor and snapshot leading; the higher place, folded lifecycle, and the earliest recording instant retained, so a merged statement ranks by its earliest recording against a third",
    "(provided: the element-level merge with the statement sent last, the later `sendunix`, as reference - a stated wire clock beats absent, ties fall back to later `transunix`, an exact tie keeps this one - then that reference's instant, code, predecessor and snapshot leading; the higher place, folded lifecycle, and the earliest `sendunix` retained, so a merged statement ranks by its earliest `sendunix` against a third",
)

# AGENTS.md - the graph and fix rows, the registry row and the text-row
# contract.
A = "AGENTS.md"
edit(
    A,
    "an element with an instant (`currunix`, `i64` nanoseconds since the epoch, UTC), a precise optional recording instant, a state",
    "an element with an instant (`transunix`, the transaction instant - when the operation really happened - `i64` nanoseconds since the epoch, UTC), a technical wire clock (`sendunix`, optional, the merge reference), a state",
)
edit(
    A,
    "tagged contiguously from `65_001` to `65_052` - `origccy` at `65_018`,",
    "tagged contiguously from `65_001` to `65_052` - the event's `transunix` at `65_007` and `sendunix` at `65_009`, `origccy` at `65_018`,",
)
edit(
    A,
    "`firstunix` and `lastunix` (the earliest and the latest instants an event stated the ISIN,",
    "`firstunix` and `lastunix` (the earliest and the latest `transunix` an event stated the ISIN at,",
)
edit(
    A,
    "  place - and refused where a count cannot hold it; when the record was\n"
    "  written is `currunix`, the row\n",
    "  place - and refused where a count cannot hold it; when the operation\n"
    "  happened is `transunix`, the row\n",
)

# docs/fix/capture.md - the crate's columns and the dating rule.
F = "docs/fix/capture.md"
edit(
    F,
    "| `currunix` | `Current Time` | 65007 | when the message happened, a nanosecond UTC clock:",
    "| `transunix` | `Transaction Time` | 65007 | when the operation the message states really happened - its transaction instant, a nanosecond UTC clock:",
)
edit(
    F,
    "| `recdunix` | `Recording Time` | 65009 | the precise recording instant, nanoseconds UTC: a stated crate value, else the enclosing text line's `mtime`, else - where the row carries no `recdunix` column at all - the stated `SendingTime(52)`, the one recording the message itself states;",
    "| `sendunix` | `Sending Time` | 65009 | when the message crossed the wire - the technical clock and the merge reference - nanoseconds UTC: a stated crate value, else its carrier's clock, the enclosing text line's `mtime`, else - where the row carries no `sendunix` column at all - the stated `SendingTime(52)`, the one wire clock the message itself states; the earliest its statements know;",
)
edit(
    F,
    "The line's clock leads the pin because the instant a line was recorded at is nearer the send than any clock",
    "The line's clock leads the pin because the instant a line was written at is nearer the send than any clock",
)
edit(
    F,
    "That clock is the **reference** every parse dates against, and `currunix` is a stated `currunix`,",
    "That clock is the **reference** every parse dates against, and `transunix` - when the operation happened - is a stated `transunix`,",
)
edit(
    F,
    "`recdunix` is the enclosing line's recording clock - the same line clock - else, where the message states its `SendingTime(52)`, that clock: the one recording the message itself states; a stand-in sending clock records nothing.",
    "`sendunix` - when the message crossed the wire - is the enclosing line's clock, the carrier's, else, where the message states its `SendingTime(52)`, that clock: the one wire clock the message itself states; a stand-in sending clock states none.",
)

# docs/fix/arrow.md and lifecycle.md.
edit(
    "docs/fix/arrow.md",
    "| `currunix` | when the row's line was written - the text reader's `mtime` - as the carrier's precise recording instant: written to `recdunix` unless",
    "| `transunix` | when the row's line was written - the text reader's `mtime` - as the carrier's clock of when the message crossed the wire: written to `sendunix` unless",
)
edit(
    "docs/fix/arrow.md",
    "each carrying its row header's session, context, sequence and recording clock,",
    "each carrying its row header's session, context, sequence and wire clock,",
)
L = "docs/fix/lifecycle.md"
edit(
    L,
    "and `recdunix` (65009) the earliest precise recording instant its statements know;",
    "and `sendunix` (65009) the earliest wire clock its statements know - when the message crossed the wire;",
)
edit(
    L,
    "the latest recorded observation is the reference, the later `currunix` closing a tie,",
    "the observation sent last - the latest `sendunix` - is the reference, the later `transunix` closing a tie,",
)
edit(
    L,
    "keeps the earliest execution and recording instants either observation states,",
    "keeps the earliest execution instant and wire clock either observation states,",
)

# docs/fix/explorer.md and docs/graph/schemas.md - the two displays.
edit("docs/fix/explorer.md", '"Current Time"', '"Transaction Time"', 2)
edit("docs/fix/explorer.md", "'Current Time'", "'Transaction Time'")
S = "docs/graph/schemas.md"
edit(S, "| Current Time |", "| Transaction Time |", 3)
edit(S, "| Recording Time |", "| Sending Time |", 3)

# docs/graph/event.md - the contract, the merge and the examples.
G = "docs/graph/event.md"
edit(
    G,
    "| `currunix` | `get_currunix`/`set_currunix`: nanoseconds since the Unix epoch, UTC, signed |",
    "| `transunix` | `get_transunix`/`set_transunix`: the transaction instant - when the operation really happened, the identity and order axis - nanoseconds since the Unix epoch, UTC, signed |",
)
edit(
    G,
    "`recdunix` the earliest recording (what a merge ranks by)",
    "`sendunix` the technical wire clock - when the message crossed the wire, the earliest its statements know (what a merge ranks by)",
)
edit(
    G,
    "| Nothing else | current/recording instants, sources and snapshot move nowhere |",
    "| Nothing else | the transaction instant, the wire clock, sources and snapshot move nowhere |",
)
edit(
    G,
    "lifecycle folded, earliest recording kept, then finalized",
    "lifecycle folded, earliest `sendunix` kept, then finalized",
)
edit(
    G,
    "| The reference | the statement with the later `recdunix` (stated beats unstated; equal/absent falls back to the greater `currunix`; an exact tie keeps `self`) |",
    "| The reference | the statement sent last, the later `sendunix` (stated beats unstated; equal/absent falls back to the greater `transunix`; an exact tie keeps `self`) |",
)
edit(
    G,
    "| From the reference | cross code, current instant/code, predecessor, snapshot;",
    "| From the reference | cross code, transaction instant `transunix` and `hashcode`, predecessor, snapshot;",
)
edit(
    G,
    "`fold_lifecycle`; recording = earliest of either |",
    "`fold_lifecycle`; `sendunix` = earliest of either |",
)
edit(
    G,
    "(ranked by earliest recording kept)",
    "(ranked by the earliest `sendunix` kept)",
)
edit(
    G,
    "The same fill report, recorded by a gateway at +2ms and an OMS at +5ms, each from its own log line.",
    "The same fill report, sent through a gateway at +2ms and an OMS at +5ms, each from its own log line.",
)
edit(
    G,
    "    let report = |recorded: Option<i64>, line: u128| {\n",
    "    let report = |sent: Option<i64>, line: u128| {\n",
)
edit(G, "        event.set_recdunix(recorded);\n", "        event.set_sendunix(sent);\n")
edit(
    G,
    "Recording clocks and sources are not content: one event.",
    "Wire clocks and sources are not content: one event.",
    3,
)
edit(
    G,
    "Merging: the later recording leads, the earliest recording stays,",
    "Merging: the later `sendunix` leads, the earliest one stays,",
    3,
)
edit(
    G,
    "    def report(recorded: int, line: str) -> graph.OrderEvent:\n",
    "    def report(sent: int, line: str) -> graph.OrderEvent:\n",
)
edit(
    G,
    'state="PARTIALLY_FILLED", recdunix=recorded, srcuuids=[line]',
    'state="PARTIALLY_FILLED", sendunix=sent, srcuuids=[line]',
)
edit(
    G,
    "    const report = (recorded, line) => new graph.OrderEvent(",
    "    const report = (sent, line) => new graph.OrderEvent(",
)
edit(
    G,
    "state: 'PARTIALLY_FILLED', recdunix: recorded, srcuuids: [line],",
    "state: 'PARTIALLY_FILLED', sendunix: sent, srcuuids: [line],",
)

# docs/graph/book.md, trade.md, index.md, market.md.
edit(
    "docs/graph/book.md",
    "earliest creation/recording instants,",
    "earliest creation instant and wire clock,",
)
edit(
    "docs/graph/book.md",
    "the reference chosen by its recording clock - the later `recdunix`, then `currunix`:",
    "the reference chosen by its wire clock - the later `sendunix`, then `transunix`:",
)
edit(
    "docs/graph/trade.md",
    "the earliest creation and recording instants,",
    "the earliest creation instant and wire clock,",
)
edit(
    "docs/graph/index.md",
    "clocks (creation, recording, expiration, predecessor, snapshot)",
    "clocks (creation, wire, expiration, predecessor, snapshot)",
)
edit(
    "docs/graph/market.md",
    "    // statement - here the later recording - keeps its identifiers whole.\n",
    "    // statement - here the one sent later - keeps its identifiers whole.\n",
)
edit(
    "docs/graph/market.md",
    '.expect("the later recording leads");',
    '.expect("the statement sent later leads");',
)

# skills - the merge reference and the two-hop example.
for path, comment in [
    ("skills/yggdryl-market-data/references/rust.md", "//"),
    ("skills/yggdryl-market-data/references/python.md", "#"),
    ("skills/yggdryl-market-data/references/javascript.md", "//"),
]:
    edit(
        path,
        "folds another statement of the same event (the later recording leads, sources\n",
        "folds another statement of the same event (the later `sendunix` leads, sources\n",
    )
    edit(
        path,
        comment + " One report recorded by two hops: recording clocks and sources are not content.\n",
        comment + " One report sent through two hops: wire clocks and sources are not content.\n",
    )
edit(
    "skills/yggdryl-market-data/references/rust.md",
    "let hop = |recorded: i64, line: u128| -> yggdryl::Result<OrderEvent> {\n",
    "let hop = |sent: i64, line: u128| -> yggdryl::Result<OrderEvent> {\n",
)
edit(
    "skills/yggdryl-market-data/references/rust.md",
    "    report.set_recdunix(Some(recorded));\n",
    "    report.set_sendunix(Some(sent));\n",
)
edit(
    "skills/yggdryl-market-data/references/javascript.md",
    "const hop = (recorded, line) => new graph.OrderEvent(T + 1_000_000_000n, { crosscode: 'O-1001', recdunix: recorded, srcuuids: [line] })\n",
    "const hop = (sent, line) => new graph.OrderEvent(T + 1_000_000_000n, { crosscode: 'O-1001', sendunix: sent, srcuuids: [line] })\n",
)
edit(
    "skills/yggdryl-fix/references/rust.md",
    "let recorded: Vec<Option<i64>> = messages.iter().map(Event::get_recdunix).collect();\n"
    "assert_eq!(recorded, [",
    "let sent: Vec<Option<i64>> = messages.iter().map(Event::get_sendunix).collect();\n"
    "assert_eq!(sent, [",
)

# Tests and benchmarks: the two displays the crate fields carry, and the
# wire clock where a comment or a message names the merge reference's clock
# (the retired 65_064's history - `refrecdunix` - keeps its words).
edit("rust/tests/fix/digest.rs", 'Some("Current Time"),', 'Some("Transaction Time"),')
edit("rust/tests/fix/digest.rs", 'Some("Recording Time"),', 'Some("Sending Time"),')
edit(
    "rust/tests/fix/schema.rs",
    '(yggdryl::CURRUNIX_TAG_NAME.0, "Current Time"),',
    '(yggdryl::TRANSUNIX_TAG_NAME.0, "Transaction Time"),',
)
edit(
    "rust/benchmarks/fix/pipeline.rs",
    "    // captures state each hop's session event and recording clock, so this\n",
    "    // captures state each hop's session event and wire clock, so this\n",
)
edit(
    "rust/tests/fix/batch.rs",
    '"the recording clock survives lifecycle Arrow reread"',
    '"the wire clock survives lifecycle Arrow reread"',
)
edit(
    "rust/tests/graph/iterator.rs",
    "\"following does not inherit the predecessor's recording clock\"",
    "\"following does not inherit the predecessor's wire clock\"",
)
X = "rust/tests/graph/element.rs"
edit(
    X,
    "    // With no recording clocks, the later event is the reference: its cross\n",
    "    // With no wire clocks, the later event is the reference: its cross\n",
)
edit(
    X,
    "        // The reference selects conflicts; the recording clock still folds\n",
    "        // The reference selects conflicts; the wire clock still folds\n",
)
edit(
    X,
    '.expect("a stated recording clock selects the reference");',
    '.expect("a stated wire clock selects the reference");',
)
edit(
    X,
    "    // Equal recording clocks, or none on either side, fall back to the later\n",
    "    // Equal wire clocks, or none on either side, fall back to the later\n",
)
edit(
    X,
    "    // own and read from lines that overlap: with no recording clocks the\n",
    "    // own and read from lines that overlap: with no wire clocks the\n",
)

edit(
    "rust/tests/text/line.rs",
    "        /// The execution and recording instants, then a capture named after\n",
    "        /// The execution instant and the wire clock, then a capture named after\n",
)
edit(
    "rust/tests/text/line.rs",
    "            // The recording capture feeds the event column itself, not a\n",
    "            // The wire clock capture feeds the event column itself, not a\n",
)


# Lists kept in name order, where the new names sort elsewhere than the old:
# the bindings inventory's `TextLine` members (`hashcode` after
# `get_entry_by_path`, `transunix` and `uuid` last) and the book JSON a
# sorted struct renders.
B = ".api-bindings.txt"
edit(
    B,
    "crossuuid: Scalar (uuid), currhashcode: int, currunix: int, curruuid: Scalar (uuid), "
    "decoded_byte_size, dropped_byte_size, entries, entry_by_path, get_entry_by_path, index: int",
    "crossuuid: Scalar (uuid), decoded_byte_size, dropped_byte_size, entries, entry_by_path, "
    "get_entry_by_path, hashcode: int, index: int",
)
edit(
    B,
    "under the working directory where it is a name), sourceurl\n",
    "under the working directory where it is a name), sourceurl, transunix: int, uuid: Scalar (uuid)\n",
)
edit(
    B,
    "crossuuid: string, currhashcode: bigint, currunix: bigint, curruuid: string, "
    "decodedByteSize, droppedByteSize, entries, entryByPath, getEntryByPath, index: bigint",
    "crossuuid: string, decodedByteSize, droppedByteSize, entries, entryByPath, "
    "getEntryByPath, hashcode: bigint, index: bigint",
)
edit(
    B,
    "where it is a name), sourceurl, toString\n",
    "where it is a name), sourceurl, toString, transunix: bigint, uuid: string\n",
)
edit(
    "docs/graph/serve.md",
    ' "bidqty":"50","complete":true,"crosscode":"3:0:CH0012214059","currunix":"2026-08-14T14:46:40.020+02:00[Europe/Zurich]","delta":1,\n'
    ' "events":0,"imbalance":"1","iscrossed":false,"isincode":"CH0012214059","islocked":false,"midpoint":null,"spread":null,"ticker":"HOLN"}\n',
    ' "bidqty":"50","complete":true,"crosscode":"3:0:CH0012214059","delta":1,\n'
    ' "events":0,"imbalance":"1","iscrossed":false,"isincode":"CH0012214059","islocked":false,"midpoint":null,"spread":null,"ticker":"HOLN",\n'
    ' "transunix":"2026-08-14T14:46:40.020+02:00[Europe/Zurich]"}\n',
)

# The element's own two names: `uuid`, its identifier, and `hashcode`, its
# content code - their docs, displays and descriptions.
EC = "rust/src/graph/element_column.rs"
edit(
    EC,
    "    /// The element's identity; never absent.\n    CurrUuid,\n",
    "    /// The element's own UUID - its identity; never absent.\n    Uuid,\n",
)
edit(
    EC,
    "    /// The XXH3-64 the element's content digests to; never absent.\n    CurrHashCode,\n",
    "    /// The element's hash code: the XXH3-64 its content digests to; never\n"
    "    /// absent.\n    HashCode,\n",
)
edit(EC, '            Self::CurrUuid => "Current UUID",\n', '            Self::Uuid => "UUID",\n')
edit(
    EC,
    '            Self::CurrHashCode => "Current Hash Code",\n',
    '            Self::HashCode => "Hash Code",\n',
)
edit(
    EC,
    "\"The element's identity: UUIDv7 ordered by millisecond and sequence, with a content payload seeded by its cross hash.\"",
    "\"The element's own UUID: UUIDv7 ordered by millisecond and sequence, with a content payload seeded by its cross hash.\"",
)
edit(
    E,
    "    /// This element's identity.\n    fn get_curruuid(&self) -> Uuid;\n",
    "    /// This element's own UUID: its identity.\n    fn get_uuid(&self) -> Uuid;\n",
)
edit(
    E,
    "    /// Records this element's identity.\n    fn set_curruuid(&mut self, curruuid: Uuid);\n",
    "    /// Records this element's own UUID.\n    fn set_uuid(&mut self, uuid: Uuid);\n",
)
edit(
    E,
    "    /// The code this element's content digests to.\n    fn get_currhashcode(&self) -> u64;\n",
    "    /// This element's hash code: the code its content digests to.\n"
    "    fn get_hashcode(&self) -> u64;\n",
)
edit(
    E,
    "    /// Records the code this element's content digests to.\n"
    "    fn set_currhashcode(&mut self, hashcode: u64);\n",
    "    /// Records this element's hash code.\n    fn set_hashcode(&mut self, hashcode: u64);\n",
)
edit(
    E,
    "    /// derives from its content, and resets the current identity where it\n",
    "    /// derives from its content, and resets its own `uuid` where it\n",
)
edit(
    E,
    "    /// is derived is never fed: not the current identity, not the cross hash\n",
    "    /// is derived is never fed: not its own `uuid`, not the cross hash\n",
)
edit(
    E,
    "/// An event whose current identity is [`Self::time_uuid`] keeps that identity\n",
    "/// An event whose `uuid` is [`Self::time_uuid`] keeps that identity\n",
)
edit(
    "rust/src/graph/facts.rs",
    "    /// changes. A UUIDv7 refusal retains the current identity, as\n",
    "    /// changes. A UUIDv7 refusal retains the element's `uuid`, as\n",
)
edit(
    "rust/src/graph/facts.rs",
    "    /// resulting current identity and cross hash.\n",
    "    /// resulting `uuid` and cross hash.\n",
)
edit(
    "rust/src/text/line.rs",
    "/// identity moves neither the content code nor current identity.\n",
    "/// identity moves neither the content code nor the line's `uuid`.\n",
)
edit(
    "rust/src/text/line.rs",
    "    /// cross hash, current identity and cross identity. An explicitly stated\n",
    "    /// cross hash, `uuid` and cross identity. An explicitly stated\n",
)

# rust/src/fix/crated.rs - 65_001 and 65_004, and the creation and execution
# clocks that fall back to the transaction instant.
edit(
    C,
    "/// The tag and name carrying the code the message's content digests to:\n"
    "/// the XXH3-64 of what the event states and the named FIX content behind it.\n",
    "/// The tag and name carrying the message's hash code, the code its content\n"
    "/// digests to: the XXH3-64 of what the event states and the named FIX\n"
    "/// content behind it.\n",
)
edit(
    C,
    "/// The tag and name carrying the message's identity: UUIDv7 ordered by its\n"
    "/// millisecond and sequence, with a content payload seeded by its cross hash.\n",
    "/// The tag and name carrying the message's own UUID, its identity: UUIDv7\n"
    "/// ordered by the millisecond of its `transunix` and its sequence, with a\n"
    "/// content payload seeded by its cross hash.\n",
)
edit(
    C,
    "/// else when it happened. Once walked, a message stating no\n",
    "/// else its `transunix`. Once walked, a message stating no\n",
)
edit(
    C,
    "/// none and follows nothing - when the message happened; the latest its\n",
    "/// none and follows nothing - its `transunix`; the latest its\n",
)
edit(
    C,
    '/// assert_eq!(held[0].display(), Some("Current UUID"));\n',
    '/// assert_eq!(held[0].display(), Some("UUID"));\n',
)
edit(
    V,
    "resets the current identity to `time_uuid`",
    "resets the `uuid` to `time_uuid`",
)
edit(
    V,
    '(65_004, "currhashcode");  (non-null uint64: the XXH3-64',
    '(65_004, "hashcode");  (non-null uint64: the hash code, the XXH3-64',
)
edit(
    V,
    '(65_001, "curruuid");  (non-null uuid: the event\'s identity,',
    '(65_001, "uuid");  (non-null uuid: the event\'s own UUID, its identity,',
)

# docs - the two displays and the two crate rows.
edit(
    F,
    "| `curruuid` | `Current UUID` | 65001 | the message's identity, a `uuid`:",
    "| `uuid` | `UUID` | 65001 | the message's own UUID, its identity, a `uuid`:",
)
edit(
    F,
    "| `currhashcode` | `Current Hash Code` | 65004 | the code the message's content digests to,",
    "| `hashcode` | `Hash Code` | 65004 | the message's hash code, the code its content digests to,",
)
edit(S, "| Current UUID |", "| UUID |", 3)
edit(S, "| Current Hash Code |", "| Hash Code |", 3)

# Tests: the two displays, and the comments that called `uuid` the current
# identity.
edit("rust/tests/fix/digest.rs", 'Some("Current UUID"),', 'Some("UUID"),')
edit("rust/tests/fix/digest.rs", 'Some("Current Hash Code"),', 'Some("Hash Code"),')
edit(
    "rust/tests/fix/schema.rs",
    '(yggdryl::CURRHASHCODE_TAG_NAME.0, "Current Hash Code"),',
    '(yggdryl::HASHCODE_TAG_NAME.0, "Hash Code"),',
)
edit(
    X,
    "    // The cross code moves both its cross element and the current identity,\n"
    "    // because the derived cross hash seeds the current payload.\n",
    "    // The cross code moves both its cross element and its `uuid`, because\n"
    "    // the derived cross hash seeds the payload.\n",
)
edit(
    X,
    "    // current identity deliberately uses the cross hash as its payload seed;\n",
    "    // `uuid` deliberately uses the cross hash as its payload seed;\n",
)

# Tests: the names and the comments that call the wire clock a recording -
# the merge reference is the statement sent last, and a fold keeps the
# earliest `sendunix`. (`refrecdunix`'s history keeps its words.)
FB = "rust/tests/fix/batch.rs"
edit(
    FB,
    "fn lifecycle_fully_merges_one_session_event_on_the_latest_recording_base() {\n",
    "fn lifecycle_fully_merges_one_session_event_on_the_base_sent_last() {\n",
)
edit(
    FB,
    "    // The later recording (200) is the reference, and the merged event\n"
    "    // keeps the earliest recording either statement knows.\n",
    "    // The statement sent later (200) is the reference, and the merged event\n"
    "    // keeps the earliest `sendunix` either statement knows.\n",
)
edit(
    FB,
    "    // Either way round: the reference is the later recording, not the\n"
    "    // statement merged into.\n",
    "    // Either way round: the reference is the statement sent later, not the\n"
    "    // statement merged into.\n",
)
edit(
    FB,
    "    // Recorded at one instant, the later event instant is the reference; a\n"
    "    // stated recording leads an unstated one whatever its instant; and an\n"
    "    // exact tie keeps the statement merged into.\n",
    "    // Sent at one instant, the later `transunix` is the reference; a stated\n"
    "    // wire clock leads an unstated one whatever its instant; and an exact\n"
    "    // tie keeps the statement merged into.\n",
)
edit(
    FB,
    '"the latest recording selects the base while the merged fact remains the earliest"',
    '"the statement sent last selects the base while the merged fact remains the earliest"',
)
edit(
    FB,
    "    // latest-recorded first (200, 150, 100): the fold's earliest recording\n"
    "    // is then never earlier than the statement folded next, so the latest\n"
    "    // recording stays the reference whatever order the rows arrived in.\n",
    "    // sent last first (200, 150, 100): the fold's earliest `sendunix` is\n"
    "    // then never earlier than the statement folded next, so the statement\n"
    "    // sent last stays the reference whatever order the rows arrived in.\n",
)
edit(
    FB,
    "    // A folded delivery keeps no trace of the recording its reference was\n"
    "    // chosen by: it ranks by the earliest recording it keeps (100) against\n",
    "    // A folded delivery keeps no trace of the wire clock its reference was\n"
    "    // chosen by: it ranks by the earliest `sendunix` it keeps (100) against\n",
)
edit(
    FB,
    "    // reference is the latest-recorded of the pair folded last - and the\n",
    "    // reference is the one of the pair folded last sent later - and the\n",
)
edit(FB, '"latest recording wins a conflict"', '"the last sent wins a conflict"', 2)
edit(
    "rust/tests/graph/book.rs",
    "fn merging_books_uses_the_latest_recording_as_reference_and_keeps_earliest_clocks() {\n",
    "fn merging_books_uses_the_book_sent_last_as_reference_and_keeps_earliest_clocks() {\n",
)
edit(
    "rust/tests/graph/book.rs",
    "    // earliest recording (20) it holds: a third book recorded at 25 leads\n",
    "    // earliest `sendunix` (20) it holds: a third book sent at 25 leads\n",
)
edit(
    X,
    "fn following_never_carries_the_recording_clock() {\n",
    "fn following_never_carries_the_wire_clock() {\n",
)
edit(X, '"no predecessor recording carries"', '"no predecessor wire clock carries"', 2)
edit(
    X,
    "fn merging_uses_the_latest_recording_as_the_reference_but_keeps_earliest_clocks() {\n",
    "fn merging_uses_the_statement_sent_last_as_the_reference_but_keeps_earliest_clocks() {\n",
)
edit(
    X,
    "    // Only the earliest recording survives either fold: `one` is the\n"
    "    // reference (recorded at 30, after 25), but no separate reference clock\n",
    "    // Only the earliest `sendunix` survives either fold: `one` is the\n"
    "    // reference (sent at 30, after 25), but no separate reference clock\n",
)
edit(
    X,
    "fn a_folded_statement_ranks_by_its_earliest_recording_so_three_way_folds_depend_on_order() {\n"
    "    // A fold keeps the earliest recording its statements know and no\n",
    "fn a_folded_statement_ranks_by_its_earliest_sendunix_so_three_way_folds_depend_on_order() {\n"
    "    // A fold keeps the earliest `sendunix` its statements know and no\n",
)
edit(
    X,
    "    // statement it ranks by that earliest recording. The reference of three\n",
    "    // statement it ranks by that earliest `sendunix`. The reference of three\n",
)
edit(
    X,
    '"every order keeps the earliest recording, order {order:?}"',
    '"every order keeps the earliest sendunix, order {order:?}"',
)
edit(
    X,
    "fn merging_a_market_event_lets_the_latest_recording_lead_event_time() {\n",
    "fn merging_a_market_event_lets_the_statement_sent_last_lead_event_time() {\n",
)
TR = "rust/tests/graph/trade.rs"
edit(
    TR,
    "fn merge_deduplicates_by_crosscode_and_the_latest_recording_leads() {\n",
    "fn merge_deduplicates_by_crosscode_and_the_statement_sent_last_leads() {\n",
)
edit(
    TR,
    "    // A merged trade and each merged child keep only the earliest recording\n"
    "    // their statements know - the trade 10, its E-1 child 30 - so against a\n"
    "    // third statement they rank by it: a trade recorded at 15 whose E-1 was\n"
    "    // recorded at 35 leads the merged trade and its child, although it\n",
    "    // A merged trade and each merged child keep only the earliest `sendunix`\n"
    "    // their statements know - the trade 10, its E-1 child 30 - so against a\n"
    "    // third statement they rank by it: a trade sent at 15 whose E-1 was\n"
    "    // sent at 35 leads the merged trade and its child, although it\n",
)
edit(
    "rust/tests/text/line.rs",
    "        fn a_recording_capture_is_a_typed_event_instant_and_execution_is_no_event_fact() {\n",
    "        fn a_sendunix_capture_is_a_typed_event_instant_and_execution_is_no_event_fact() {\n",
)
edit(
    "rust/tests/text/line.rs",
    "        fn a_recording_capture_refuses_a_bad_instant_by_name() {\n",
    "        fn a_sendunix_capture_refuses_a_bad_instant_by_name() {\n",
)
edit(
    "node/tests/fix.test.js",
    "    // latest recording is the reference and earlier observations fill it.\n",
    "    // observation sent last is the reference and earlier ones fill it.\n",
)
edit(
    FB,
    '"the later recording of the pair folded last is the reference"',
    '"the one of the pair folded last sent later is the reference"',
)
edit(
    "rust/tests/fix/codec.rs",
    '"a message no carrier recorded was recorded when it was sent"',
    '"a message no carrier dated crossed the wire when it was sent"',
)
edit(
    "rust/tests/fix/codec.rs",
    "    // A message stating no sending clock is dated by a stand-in, which\n"
    "    // records nothing.\n",
    "    // A message stating no sending clock is dated by a stand-in, which\n"
    "    // states no wire clock.\n",
)
edit(
    "rust/tests/fix/ulbridge.rs",
    '"the document was recorded when its line says"',
    '"the document crossed the wire when its line says"',
)
GB = "rust/tests/graph/book.rs"
edit(
    GB,
    "        // The later recording (30) is the reference and has the word on a\n",
    "        // The book sent later (30) is the reference and has the word on a\n",
)
edit(GB, "    // The later recording is the reference.\n", "    // The statement sent later is the reference.\n")
edit(
    GB,
    "    // Two statements of one book, the later recording the reference: the\n",
    "    // Two statements of one book, the one sent later the reference: the\n",
)
edit(
    "rust/tests/graph/market.rs",
    '.expect("the later recording leads");',
    '.expect("the statement sent later leads");',
)
edit(
    "node/tests/fix.test.js",
    "    // and so the instant, the creation and the recording.\n",
    "    // and so the `transunix`, the creation and the `sendunix`.\n",
)
edit(
    "python/tests/test_fix.py",
    "    # and so the instant, the creation and the recording.\n",
    "    # and so the `transunix`, the creation and the `sendunix`.\n",
)

edit(
    A,
    "`element.rs` holds `Element` - an element's `Uuid`, its cross identity and code, its codes and",
    "`element.rs` holds `Element` - an element's own identifier `uuid`, its cross identity and code, its content code `hashcode` and cross hash code, and",
)

# Phase 2, after phase 1 and before the bulk; it sits in a kept region.
HASH_EDIT = ("rust/tests/fix/store.rs", HASH_TEST, HASH_SENTENCE + HASH_TEST, 1)

# Phase 3: per file, the number of bulk matches the tree holds after phases 1
# and 2 - regenerate with --emit-table.
BULK_EXPECTED = {
    '.api-bindings.txt': 70,
    '.api-inventory.txt': 98,
    'AGENTS.md': 5,
    'docs/assets/fix-explorer.js': 4,
    'docs/fix/arrow.md': 24,
    'docs/fix/capture.md': 86,
    'docs/fix/cli.md': 1,
    'docs/fix/encode.md': 2,
    'docs/fix/explorer.md': 21,
    'docs/fix/lifecycle.md': 121,
    'docs/fix/message.md': 65,
    'docs/graph/book.md': 32,
    'docs/graph/candle.md': 3,
    'docs/graph/element.md': 22,
    'docs/graph/event.md': 76,
    'docs/graph/execution.md': 3,
    'docs/graph/index.md': 3,
    'docs/graph/isin-registry.md': 3,
    'docs/graph/market-data.md': 37,
    'docs/graph/market.md': 3,
    'docs/graph/order.md': 27,
    'docs/graph/schemas.md': 13,
    'docs/graph/serve.md': 13,
    'docs/graph/trade.md': 14,
    'docs/hashing.md': 5,
    'docs/media/iceberg.md': 1,
    'docs/media/text.md': 9,
    'docs/testing.md': 3,
    'docs/types/datatype.md': 8,
    'docs/types/enum/index.md': 5,
    'docs/types/enum/marketdatakind.md': 2,
    'docs/types/enum/side.md': 2,
    'docs/types/enum/timeinforce.md': 5,
    'docs/types/protocol.md': 3,
    'node/benchmarks/fix.js': 4,
    'node/binding.d.ts': 13,
    'node/binding.js': 2,
    'node/book/audit.js': 3,
    'node/src/fix.rs': 30,
    'node/src/graph/book.rs': 14,
    'node/src/graph/candle.rs': 1,
    'node/src/graph/iterator.rs': 2,
    'node/src/graph/market_data.rs': 2,
    'node/src/graph/mod.rs': 22,
    'node/src/graph/operation.rs': 7,
    'node/src/hashing/txhash.rs': 1,
    'node/src/isin_registry.rs': 2,
    'node/src/text/line.rs': 7,
    'node/tests/book.test.js': 5,
    'node/tests/fix.test.js': 178,
    'node/tests/fix.types.ts': 15,
    'node/tests/graph/book.test.js': 25,
    'node/tests/graph/candle.test.js': 3,
    'node/tests/graph/index.test.js': 1,
    'node/tests/graph/index.types.ts': 6,
    'node/tests/graph/iterator.test.js': 16,
    'node/tests/graph/market_data.test.js': 20,
    'node/tests/graph/operation.test.js': 25,
    'node/tests/graph/trade.test.js': 5,
    'node/tests/holder/fs.test.js': 1,
    'node/tests/iobase.test.js': 4,
    'node/tests/isin_registry.test.js': 13,
    'node/tests/records.test.js': 15,
    'node/tests/records.types.ts': 3,
    'node/tests/text/line.test.js': 17,
    'python/src/fix.rs': 26,
    'python/src/graph/book.rs': 6,
    'python/src/graph/iterator.rs': 2,
    'python/src/graph/market_data.rs': 1,
    'python/src/graph/mod.rs': 19,
    'python/src/graph/operation.rs': 5,
    'python/src/hashing/txhash.rs': 1,
    'python/src/isin_registry.rs': 1,
    'python/src/text/line.rs': 7,
    'python/tests/enums/test_init.py': 3,
    'python/tests/graph/test_book.py': 16,
    'python/tests/graph/test_candle.py': 2,
    'python/tests/graph/test_iterator.py': 18,
    'python/tests/graph/test_market_data.py': 21,
    'python/tests/graph/test_operation.py': 26,
    'python/tests/graph/test_trade.py': 5,
    'python/tests/holder/test_fs.py': 4,
    'python/tests/holder/test_init.py': 1,
    'python/tests/medallion.py': 12,
    'python/tests/test_fix.py': 225,
    'python/tests/test_isin_registry.py': 4,
    'python/tests/text/test_init.py': 26,
    'python/tests/text/test_line.py': 24,
    'python/tests/typing_bindings.py': 49,
    'python/yggdryl/_native.pyi': 74,
    'python/yggdryl/fix.py': 15,
    'rust/benchmarks/fix/pipeline.rs': 10,
    'rust/benchmarks/fix/resolve.rs': 2,
    'rust/benchmarks/graph/book.rs': 1,
    'rust/benchmarks/media/iceberg.rs': 4,
    'rust/examples/fix_capture.rs': 5,
    'rust/src/expression/mod.rs': 4,
    'rust/src/fix/batch.rs': 11,
    'rust/src/fix/build.rs': 8,
    'rust/src/fix/codec.rs': 28,
    'rust/src/fix/crated.rs': 30,
    'rust/src/fix/enrich.rs': 35,
    'rust/src/fix/identity.rs': 4,
    'rust/src/fix/market.rs': 6,
    'rust/src/fix/mod.rs': 4,
    'rust/src/fix/msg.rs': 53,
    'rust/src/fix/schema.rs': 6,
    'rust/src/fix/ulbridge.rs': 2,
    'rust/src/graph/arrow.rs': 68,
    'rust/src/graph/book.rs': 104,
    'rust/src/graph/candle.rs': 4,
    'rust/src/graph/column.rs': 29,
    'rust/src/graph/element.rs': 90,
    'rust/src/graph/element_column.rs': 21,
    'rust/src/graph/facts.rs': 89,
    'rust/src/graph/iterator.rs': 51,
    'rust/src/graph/market.rs': 25,
    'rust/src/graph/market_data.rs': 12,
    'rust/src/graph/mod.rs': 12,
    'rust/src/graph/operation.rs': 30,
    'rust/src/graph/serve.rs': 34,
    'rust/src/graph/trade.rs': 40,
    'rust/src/graph/view.rs': 3,
    'rust/src/hashing/mod.rs': 2,
    'rust/src/implementer.rs': 12,
    'rust/src/isin_registry.rs': 6,
    'rust/src/lib.rs': 4,
    'rust/src/limit.rs': 1,
    'rust/src/protocol.rs': 1,
    'rust/src/text/batch.rs': 2,
    'rust/src/text/line.rs': 48,
    'rust/src/text/options.rs': 10,
    'rust/src/text/plan.rs': 1,
    'rust/src/txhash/value.rs': 1,
    'rust/tests/allocations.rs': 16,
    'rust/tests/expression/eval.rs': 2,
    'rust/tests/expression/pushdown.rs': 3,
    'rust/tests/expression/transform.rs': 11,
    'rust/tests/fix.rs': 1,
    'rust/tests/fix/batch.rs': 93,
    'rust/tests/fix/build.rs': 1,
    'rust/tests/fix/codec.rs': 61,
    'rust/tests/fix/crated.rs': 2,
    'rust/tests/fix/digest.rs': 35,
    'rust/tests/fix/enrich.rs': 72,
    'rust/tests/fix/entry.rs': 31,
    'rust/tests/fix/identity.rs': 12,
    'rust/tests/fix/market.rs': 61,
    'rust/tests/fix/messages.rs': 8,
    'rust/tests/fix/msg.rs': 62,
    'rust/tests/fix/schema.rs': 52,
    'rust/tests/fix/securityids.rs': 3,
    'rust/tests/fix/store.rs': 2,
    'rust/tests/fix/ulbridge.rs': 56,
    'rust/tests/graph/arrow.rs': 59,
    'rust/tests/graph/book.rs': 147,
    'rust/tests/graph/candle.rs': 3,
    'rust/tests/graph/column.rs': 16,
    'rust/tests/graph/element.rs': 239,
    'rust/tests/graph/element_column.rs': 21,
    'rust/tests/graph/facts.rs': 4,
    'rust/tests/graph/iterator.rs': 109,
    'rust/tests/graph/market.rs': 18,
    'rust/tests/graph/market_data.rs': 19,
    'rust/tests/graph/operation.rs': 32,
    'rust/tests/graph/serve.rs': 22,
    'rust/tests/graph/trade.rs': 37,
    'rust/tests/graph/view.rs': 14,
    'rust/tests/iceberg/mod_.rs': 1,
    'rust/tests/medallion_ledger.rs': 14,
    'rust/tests/parquet/mod_.rs': 4,
    'rust/tests/scale_ulbridge.rs': 32,
    'rust/tests/text/arrow.rs': 17,
    'rust/tests/text/handle.rs': 2,
    'rust/tests/text/line.rs': 106,
    'rust/tests/text/options.rs': 7,
    'rust/tests/text/plan.rs': 14,
    'skills/yggdryl-fix/SKILL.md': 7,
    'skills/yggdryl-fix/references/javascript.md': 13,
    'skills/yggdryl-fix/references/python.md': 13,
    'skills/yggdryl-fix/references/rust.md': 13,
    'skills/yggdryl-market-data/SKILL.md': 15,
    'skills/yggdryl-market-data/references/javascript.md': 28,
    'skills/yggdryl-market-data/references/python.md': 32,
    'skills/yggdryl-market-data/references/rust.md': 32,
    'skills/yggdryl-records/SKILL.md': 7,
}

# Phase 4: (path, old, new, count), both in the swept spelling - what rustfmt
# makes of the swept Rust, regenerated by `p4_define/fmt_gen.py`.
FMT_EDITS = [
    (
        'rust/benchmarks/fix/pipeline.rs',
        '            message\n'
        '                .set_transunix(SNAPSHOT_BASE + i64::try_from(index).expect("sixteen rows") * MINUTE);\n'
        ,
        '            message.set_transunix(\n'
        '                SNAPSHOT_BASE + i64::try_from(index).expect("sixteen rows") * MINUTE,\n'
        '            );\n'
        ,
        1,
    ),
    (
        'rust/src/fix/market.rs',
        '        event.get_snapunix().unwrap_or_else(|| event.get_transunix())\n'
        ,
        '        event\n'
        '            .get_snapunix()\n'
        '            .unwrap_or_else(|| event.get_transunix())\n'
        ,
        1,
    ),
    (
        'rust/src/fix/mod.rs',
        '    HASHCODE_TAG_NAME, TRANSUNIX_TAG_NAME, UUID_TAG_NAME, EXECUNIX_TAG_NAME,\n'
        '    EXPRUNIX_TAG_NAME, FIGICODE_TAG_NAME, FIXMSG_TAG_NAME, FOREXCODE_TAG_NAME,\n'
        '    FORWARDPOINTS_TAG_NAME, FXRATES_TAG_NAME, HIDDENQTY_TAG_NAME, IDENTIFIERS_TAG_NAME,\n'
        '    ISINCODE_TAG_NAME, MARKETDATAKIND_TAG_NAME, MARKETDATATYPE_TAG_NAME, METADATA_TAG_NAME,\n'
        '    MICCODE_TAG_NAME, MSGCTXID_TAG_NAME, MSGDIRECTION_TAG_NAME, MSGORIGINATOR_TAG_NAME,\n'
        '    MSGPLUGINID_TAG_NAME, MSGPLUGINSIDE_TAG_NAME, MSGSESSEVENTID_TAG_NAME, MSGSESSIONID_TAG_NAME,\n'
        '    MSGTYPE_TAG_NAME, ORDQTY_TAG_NAME, ORIGCCY_TAG_NAME, PARTYIDS_TAG_NAME, PREVPX_TAG_NAME,\n'
        '    PREVQTY_TAG_NAME, PREVUNIX_TAG_NAME, PREVUUID_TAG_NAME, SENDUNIX_TAG_NAME,\n'
        '    SECURITYIDS_TAG_NAME, SEQNUM_TAG_NAME, SNAPUNIX_TAG_NAME, SOURCEURL_TAG_NAME,\n'
        '    SPOTRATE_TAG_NAME, SRCUUIDS_TAG_NAME, STATE_TAG_NAME, STRIKEPX_TAG_NAME, TICKER_TAG_NAME,\n'
        '    TRADABLE_TAG_NAME, UNIT_TAG_NAME, fix_crate_fields, is_crate_tag, is_derived_tag,\n'
        ,
        '    EXECUNIX_TAG_NAME, EXPRUNIX_TAG_NAME, FIGICODE_TAG_NAME, FIXMSG_TAG_NAME, FOREXCODE_TAG_NAME,\n'
        '    FORWARDPOINTS_TAG_NAME, FXRATES_TAG_NAME, HASHCODE_TAG_NAME, HIDDENQTY_TAG_NAME,\n'
        '    IDENTIFIERS_TAG_NAME, ISINCODE_TAG_NAME, MARKETDATAKIND_TAG_NAME, MARKETDATATYPE_TAG_NAME,\n'
        '    METADATA_TAG_NAME, MICCODE_TAG_NAME, MSGCTXID_TAG_NAME, MSGDIRECTION_TAG_NAME,\n'
        '    MSGORIGINATOR_TAG_NAME, MSGPLUGINID_TAG_NAME, MSGPLUGINSIDE_TAG_NAME, MSGSESSEVENTID_TAG_NAME,\n'
        '    MSGSESSIONID_TAG_NAME, MSGTYPE_TAG_NAME, ORDQTY_TAG_NAME, ORIGCCY_TAG_NAME, PARTYIDS_TAG_NAME,\n'
        '    PREVPX_TAG_NAME, PREVQTY_TAG_NAME, PREVUNIX_TAG_NAME, PREVUUID_TAG_NAME, SECURITYIDS_TAG_NAME,\n'
        '    SENDUNIX_TAG_NAME, SEQNUM_TAG_NAME, SNAPUNIX_TAG_NAME, SOURCEURL_TAG_NAME, SPOTRATE_TAG_NAME,\n'
        '    SRCUUIDS_TAG_NAME, STATE_TAG_NAME, STRIKEPX_TAG_NAME, TICKER_TAG_NAME, TRADABLE_TAG_NAME,\n'
        '    TRANSUNIX_TAG_NAME, UNIT_TAG_NAME, UUID_TAG_NAME, fix_crate_fields, is_crate_tag,\n'
        '    is_derived_tag,\n'
        ,
        1,
    ),
    (
        'rust/src/graph/arrow.rs',
        '        check(\n'
        '            self.hashcode,\n'
        '            canonical.get_hashcode(),\n'
        '            path,\n'
        '            "hashcode",\n'
        '        )?;\n'
        ,
        '        check(self.hashcode, canonical.get_hashcode(), path, "hashcode")?;\n'
        ,
        1,
    ),
    (
        'rust/src/graph/book.rs',
        '        event.get_snapunix().unwrap_or_else(|| event.get_transunix())\n'
        ,
        '        event\n'
        '            .get_snapunix()\n'
        '            .unwrap_or_else(|| event.get_transunix())\n'
        ,
        1,
    ),
    (
        'rust/src/graph/element.rs',
        '    changed |= moved(this.get_prevunix(), Some(previous.get_transunix()), |unix| {\n'
        '        this.set_prevunix(unix)\n'
        '    });\n'
        ,
        '    changed |= moved(\n'
        '        this.get_prevunix(),\n'
        '        Some(previous.get_transunix()),\n'
        '        |unix| this.set_prevunix(unix),\n'
        '    );\n'
        ,
        1,
    ),
    (
        'rust/src/graph/element.rs',
        '        changed |= moved(\n'
        '            this.get_hashcode(),\n'
        '            other.get_hashcode(),\n'
        '            |hashcode| this.set_hashcode(hashcode),\n'
        '        );\n'
        ,
        '        changed |= moved(this.get_hashcode(), other.get_hashcode(), |hashcode| {\n'
        '            this.set_hashcode(hashcode)\n'
        '        });\n'
        ,
        1,
    ),
    (
        'rust/src/graph/element.rs',
        '        if previous.get_uuid() == self.get_uuid()\n'
        '            || previous.get_transunix() > self.get_transunix()\n'
        ,
        '        if previous.get_uuid() == self.get_uuid() || previous.get_transunix() > self.get_transunix()\n'
        ,
        1,
    ),
    (
        'rust/src/graph/market.rs',
        '    fn following_market(mut self, previous: &Self) -> Option<Self>\n'
        '    where\n'
        '        Self: Event + Sized,\n'
        '    {\n'
        '        if previous.get_uuid() == self.get_uuid()\n'
        '            || previous.get_transunix() > self.get_transunix()\n'
        '        {\n'
        '            return None;\n'
        '        }\n'
        ,
        '    fn following_market(mut self, previous: &Self) -> Option<Self>\n'
        '    where\n'
        '        Self: Event + Sized,\n'
        '    {\n'
        '        if previous.get_uuid() == self.get_uuid() || previous.get_transunix() > self.get_transunix()\n'
        '        {\n'
        '            return None;\n'
        '        }\n'
        ,
        1,
    ),
    (
        'rust/src/graph/market.rs',
        '    fn following_operation(mut self, previous: &Self) -> Option<Self>\n'
        '    where\n'
        '        Self: Event + Sized,\n'
        '    {\n'
        '        if previous.get_uuid() == self.get_uuid()\n'
        '            || previous.get_transunix() > self.get_transunix()\n'
        '        {\n'
        '            return None;\n'
        '        }\n'
        ,
        '    fn following_operation(mut self, previous: &Self) -> Option<Self>\n'
        '    where\n'
        '        Self: Event + Sized,\n'
        '    {\n'
        '        if previous.get_uuid() == self.get_uuid() || previous.get_transunix() > self.get_transunix()\n'
        '        {\n'
        '            return None;\n'
        '        }\n'
        ,
        1,
    ),
    (
        'rust/src/graph/serve.rs',
        '        let mut terms = vec![category(), of_key(key), transunix().le(literal_instant(at)?)];\n'
        ,
        '        let mut terms = vec![\n'
        '            category(),\n'
        '            of_key(key),\n'
        '            transunix().le(literal_instant(at)?),\n'
        '        ];\n'
        ,
        1,
    ),
    (
        'rust/src/graph/serve.rs',
        '                if base.as_ref().is_none_or(|held| held.get_transunix() <= unix) {\n'
        ,
        '                if base\n'
        '                    .as_ref()\n'
        '                    .is_none_or(|held| held.get_transunix() <= unix)\n'
        '                {\n'
        ,
        1,
    ),
    (
        'rust/src/graph/serve.rs',
        '                DataType::utf8().nullable_field(crosscode),\n'
        '                DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC)?.nullable_field(transunix),\n'
        ,
        '                DataType::utf8().nullable_field(crosscode),\n'
        '                DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC)?\n'
        '                    .nullable_field(transunix),\n'
        ,
        1,
    ),
    (
        'rust/src/graph/serve.rs',
        '            vec![\n'
        '                DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC)?.nullable_field(transunix),\n'
        ,
        '            vec![\n'
        '                DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC)?\n'
        '                    .nullable_field(transunix),\n'
        ,
        1,
    ),
    (
        'rust/src/lib.rs',
        '    HASHCODE_TAG_NAME, TRANSUNIX_TAG_NAME, UUID_TAG_NAME, DEFAULT_NULL_VALUES,\n'
        '    DEFAULT_PAYLOAD_COLUMN, DEFAULT_REFUSED_MSGTYPES, EXECUNIX_TAG_NAME, EXPRUNIX_TAG_NAME,\n'
        '    FIGICODE_TAG_NAME, FIX_TYPED_TAGS, FIXMSG_TAG_NAME, FOREXCODE_TAG_NAME, FORWARDPOINTS_TAG_NAME,\n'
        '    FXRATES_TAG_NAME, FixAnomaly, FixCapture, FixCode, FixCodeSet, FixCodeValue, FixCodec,\n'
        '    FixCodes, FixCommit, FixDedup, FixDirection, FixDirectionEntry, FixDirections, FixDrop,\n'
        '    FixEntry, FixFailure, FixField, FixFieldIter, FixFieldMut, FixHeader, FixId, FixIdMapKind,\n'
        '    FixIdSource, FixIdSources, FixKey, FixLifted, FixMerge, FixMessages, FixMsg, FixPatterns,\n'
        '    FixRegistry, FixSource, FixSpellings, HIDDENQTY_TAG_NAME, IDENTIFIERS_TAG_NAME,\n'
        '    ISINCODE_TAG_NAME, MARKETDATAKIND_TAG_NAME, MARKETDATATYPE_TAG_NAME, METADATA_TAG_NAME,\n'
        '    MICCODE_TAG_NAME, MSGCTXID_TAG_NAME, MSGDIRECTION_TAG_NAME, MSGORIGINATOR_TAG_NAME,\n'
        '    MSGPLUGINID_TAG_NAME, MSGPLUGINSIDE_TAG_NAME, MSGSESSEVENTID_TAG_NAME, MSGSESSIONID_TAG_NAME,\n'
        '    ORDQTY_TAG_NAME, ORIGCCY_TAG_NAME, PARTYIDS_TAG_NAME, PREVPX_TAG_NAME, PREVQTY_TAG_NAME,\n'
        '    PREVUNIX_TAG_NAME, PREVUUID_TAG_NAME, SENDUNIX_TAG_NAME, SECURITYIDS_TAG_NAME, SEQNUM_TAG_NAME,\n'
        '    SNAPUNIX_TAG_NAME, SOH, SOURCEURL_TAG_NAME, SPOTRATE_TAG_NAME, SRCUUIDS_TAG_NAME,\n'
        '    STANDARD_HEADER_TAGS, STANDARD_TRAILER_TAGS, STATE_TAG_NAME, STRIKEPX_TAG_NAME,\n'
        '    TICKER_TAG_NAME, TRADABLE_TAG_NAME, ULBRIDGE_ROWHEADER, UNIT_TAG_NAME, Words, fix_column_of,\n'
        ,
        '    DEFAULT_NULL_VALUES, DEFAULT_PAYLOAD_COLUMN, DEFAULT_REFUSED_MSGTYPES, EXECUNIX_TAG_NAME,\n'
        '    EXPRUNIX_TAG_NAME, FIGICODE_TAG_NAME, FIX_TYPED_TAGS, FIXMSG_TAG_NAME, FOREXCODE_TAG_NAME,\n'
        '    FORWARDPOINTS_TAG_NAME, FXRATES_TAG_NAME, FixAnomaly, FixCapture, FixCode, FixCodeSet,\n'
        '    FixCodeValue, FixCodec, FixCodes, FixCommit, FixDedup, FixDirection, FixDirectionEntry,\n'
        '    FixDirections, FixDrop, FixEntry, FixFailure, FixField, FixFieldIter, FixFieldMut, FixHeader,\n'
        '    FixId, FixIdMapKind, FixIdSource, FixIdSources, FixKey, FixLifted, FixMerge, FixMessages,\n'
        '    FixMsg, FixPatterns, FixRegistry, FixSource, FixSpellings, HASHCODE_TAG_NAME,\n'
        '    HIDDENQTY_TAG_NAME, IDENTIFIERS_TAG_NAME, ISINCODE_TAG_NAME, MARKETDATAKIND_TAG_NAME,\n'
        '    MARKETDATATYPE_TAG_NAME, METADATA_TAG_NAME, MICCODE_TAG_NAME, MSGCTXID_TAG_NAME,\n'
        '    MSGDIRECTION_TAG_NAME, MSGORIGINATOR_TAG_NAME, MSGPLUGINID_TAG_NAME, MSGPLUGINSIDE_TAG_NAME,\n'
        '    MSGSESSEVENTID_TAG_NAME, MSGSESSIONID_TAG_NAME, ORDQTY_TAG_NAME, ORIGCCY_TAG_NAME,\n'
        '    PARTYIDS_TAG_NAME, PREVPX_TAG_NAME, PREVQTY_TAG_NAME, PREVUNIX_TAG_NAME, PREVUUID_TAG_NAME,\n'
        '    SECURITYIDS_TAG_NAME, SENDUNIX_TAG_NAME, SEQNUM_TAG_NAME, SNAPUNIX_TAG_NAME, SOH,\n'
        '    SOURCEURL_TAG_NAME, SPOTRATE_TAG_NAME, SRCUUIDS_TAG_NAME, STANDARD_HEADER_TAGS,\n'
        '    STANDARD_TRAILER_TAGS, STATE_TAG_NAME, STRIKEPX_TAG_NAME, TICKER_TAG_NAME, TRADABLE_TAG_NAME,\n'
        '    TRANSUNIX_TAG_NAME, ULBRIDGE_ROWHEADER, UNIT_TAG_NAME, UUID_TAG_NAME, Words, fix_column_of,\n'
        ,
        1,
    ),
    (
        'rust/tests/fix/codec.rs',
        '            for column in [\n'
        '                "uuid",\n'
        '                "hashcode",\n'
        '                "crosscode",\n'
        '                "seqnum",\n'
        '                "fixentries",\n'
        '            ] {\n'
        ,
        '            for column in ["uuid", "hashcode", "crosscode", "seqnum", "fixentries"] {\n'
        ,
        1,
    ),
    (
        'rust/tests/fix/digest.rs',
        '        [\n'
        '            yggdryl::HASHCODE_TAG_NAME,\n'
        '            yggdryl::CROSSHASHCODE_TAG_NAME\n'
        '        ],\n'
        ,
        '        [yggdryl::HASHCODE_TAG_NAME, yggdryl::CROSSHASHCODE_TAG_NAME],\n'
        ,
        1,
    ),
    (
        'rust/tests/fix/digest.rs',
        '    let hashcode_at = yggdryl::fix_column_of(&schema, yggdryl::HASHCODE_TAG_NAME.0)\n'
        '        .expect("a hashcode column");\n'
        ,
        '    let hashcode_at =\n'
        '        yggdryl::fix_column_of(&schema, yggdryl::HASHCODE_TAG_NAME.0).expect("a hashcode column");\n'
        ,
        1,
    ),
    (
        'rust/tests/fix/enrich.rs',
        '        assert_eq!(\n'
        '            stray.get_uuid(),\n'
        '            stray.time_uuid().expect("an identity")\n'
        '        );\n'
        ,
        '        assert_eq!(stray.get_uuid(), stray.time_uuid().expect("an identity"));\n'
        ,
        1,
    ),
    (
        'rust/tests/fix/enrich.rs',
        '            .with_filter("transunix >= \'2026-08-14T00:00:00Z\' and transunix < \'2026-08-15T00:00:00Z\'")\n'
        ,
        '            .with_filter(\n'
        '                "transunix >= \'2026-08-14T00:00:00Z\' and transunix < \'2026-08-15T00:00:00Z\'",\n'
        '            )\n'
        ,
        1,
    ),
    (
        'rust/tests/fix/enrich.rs',
        '    parsed.sort_by_key(|held| {\n'
        '        (\n'
        '            held.get_transunix(),\n'
        '            held.get_seqnum(),\n'
        '            held.get_hashcode(),\n'
        '        )\n'
        '    });\n'
        ,
        '    parsed.sort_by_key(|held| (held.get_transunix(), held.get_seqnum(), held.get_hashcode()));\n'
        ,
        1,
    ),
    (
        'rust/tests/fix/entry.rs',
        '        assert_ne!(\n'
        '            split_left.get_hashcode(),\n'
        '            split_right.get_hashcode()\n'
        '        );\n'
        ,
        '        assert_ne!(split_left.get_hashcode(), split_right.get_hashcode());\n'
        ,
        1,
    ),
    (
        'rust/tests/fix/market.rs',
        '    assert_eq!(\n'
        '        operation_of(changed).get_prevuuid(),\n'
        '        Some(offer.get_uuid())\n'
        '    );\n'
        ,
        '    assert_eq!(operation_of(changed).get_prevuuid(), Some(offer.get_uuid()));\n'
        ,
        1,
    ),
    (
        'rust/tests/fix/market.rs',
        '    event.get_snapunix().unwrap_or_else(|| event.get_transunix())\n'
        ,
        '    event\n'
        '        .get_snapunix()\n'
        '        .unwrap_or_else(|| event.get_transunix())\n'
        ,
        1,
    ),
    (
        'rust/tests/fix/market.rs',
        '        unsorted.iter().map(Event::get_transunix).collect::<Vec<_>>()\n'
        ,
        '        unsorted\n'
        '            .iter()\n'
        '            .map(Event::get_transunix)\n'
        '            .collect::<Vec<_>>()\n'
        ,
        1,
    ),
    (
        'rust/tests/fix/market.rs',
        '    assert_eq!(walked[cancel].get_transunix(), walked[reject].get_transunix());\n'
        ,
        '    assert_eq!(\n'
        '        walked[cancel].get_transunix(),\n'
        '        walked[reject].get_transunix()\n'
        '    );\n'
        ,
        1,
    ),
    (
        'rust/tests/fix/msg.rs',
        '    // states, so the row is read without that column.\n'
        '    let fixed = fixed_with_party_group(&registry);\n'
        '    let hashcode_at = yggdryl::fix_column_of(&fixed, yggdryl::HASHCODE_TAG_NAME.0)\n'
        '        .expect("a hashcode column");\n'
        '    let schema = StructType::from_fields(\n'
        ,
        '    // states, so the row is read without that column.\n'
        '    let fixed = fixed_with_party_group(&registry);\n'
        '    let hashcode_at =\n'
        '        yggdryl::fix_column_of(&fixed, yggdryl::HASHCODE_TAG_NAME.0).expect("a hashcode column");\n'
        '    let schema = StructType::from_fields(\n'
        ,
        1,
    ),
    (
        'rust/tests/fix/msg.rs',
        '        for (held, read) in [(&as_written, "as written"), (&as_read_back, "as read back")] {\n'
        '            assert_eq!(\n'
        '                held.get_hashcode(),\n'
        '                message.get_hashcode(),\n'
        '                "{read}"\n'
        '            );\n'
        ,
        '        for (held, read) in [(&as_written, "as written"), (&as_read_back, "as read back")] {\n'
        '            assert_eq!(held.get_hashcode(), message.get_hashcode(), "{read}");\n'
        ,
        1,
    ),
    (
        'rust/tests/fix/msg.rs',
        '    // its row states.\n'
        '    let fixed = fixed_with_party_group(&registry);\n'
        '    let hashcode_at = yggdryl::fix_column_of(&fixed, yggdryl::HASHCODE_TAG_NAME.0)\n'
        '        .expect("a hashcode column");\n'
        '    let schema = StructType::from_fields(\n'
        ,
        '    // its row states.\n'
        '    let fixed = fixed_with_party_group(&registry);\n'
        '    let hashcode_at =\n'
        '        yggdryl::fix_column_of(&fixed, yggdryl::HASHCODE_TAG_NAME.0).expect("a hashcode column");\n'
        '    let schema = StructType::from_fields(\n'
        ,
        1,
    ),
    (
        'rust/tests/fix/msg.rs',
        '            let held = FixMsg::from_row(Arc::clone(&registry), &schema, row).unwrap();\n'
        '            assert_eq!(\n'
        '                held.get_hashcode(),\n'
        '                message.get_hashcode(),\n'
        '                "{read}"\n'
        '            );\n'
        ,
        '            let held = FixMsg::from_row(Arc::clone(&registry), &schema, row).unwrap();\n'
        '            assert_eq!(held.get_hashcode(), message.get_hashcode(), "{read}");\n'
        ,
        1,
    ),
    (
        'rust/tests/fix/schema.rs',
        '    use yggdryl::{\n'
        '        CROSSHASHCODE_TAG_NAME, HASHCODE_TAG_NAME, FixMsg, IOMedia, PREVUUID_TAG_NAME,\n'
        '    };\n'
        ,
        '    use yggdryl::{CROSSHASHCODE_TAG_NAME, FixMsg, HASHCODE_TAG_NAME, IOMedia, PREVUUID_TAG_NAME};\n'
        ,
        1,
    ),
    (
        'rust/tests/fix/schema.rs',
        '    let refused =\n'
        '        PrimitiveType::from_dtype(fixed.get_field(HASHCODE_TAG_NAME.1).unwrap().dtype())\n'
        '            .map(|held| held.to_string())\n'
        '            .unwrap_err()\n'
        '            .to_string();\n'
        ,
        '    let refused = PrimitiveType::from_dtype(fixed.get_field(HASHCODE_TAG_NAME.1).unwrap().dtype())\n'
        '        .map(|held| held.to_string())\n'
        '        .unwrap_err()\n'
        '        .to_string();\n'
        ,
        1,
    ),
    (
        'rust/tests/fix/ulbridge.rs',
        '        let lines: HashSet<yggdryl::Uuid> =\n'
        '            text_lines().iter().map(TextLine::get_uuid).collect();\n'
        ,
        '        let lines: HashSet<yggdryl::Uuid> = text_lines().iter().map(TextLine::get_uuid).collect();\n'
        ,
        1,
    ),
    (
        'rust/tests/fix/ulbridge.rs',
        '                    split\n'
        '                        .entry(*source)\n'
        '                        .or_default()\n'
        '                        .insert(message.get_uuid());\n'
        ,
        '                    split.entry(*source).or_default().insert(message.get_uuid());\n'
        ,
        1,
    ),
    (
        'rust/tests/fix/ulbridge.rs',
        '            assert_eq!(\n'
        '                after.get_hashcode(),\n'
        '                before.get_hashcode(),\n'
        '                "row {index}"\n'
        '            );\n'
        ,
        '            assert_eq!(after.get_hashcode(), before.get_hashcode(), "row {index}");\n'
        ,
        1,
    ),
    (
        'rust/tests/fix/ulbridge.rs',
        '            event.get_snapunix().unwrap_or_else(|| event.get_transunix())\n'
        ,
        '            event\n'
        '                .get_snapunix()\n'
        '                .unwrap_or_else(|| event.get_transunix())\n'
        ,
        1,
    ),
    (
        'rust/tests/fix/ulbridge.rs',
        '            assert_eq!(\n'
        '                twin.get_hashcode(),\n'
        '                direct.get_hashcode(),\n'
        '                "leaf {index}"\n'
        '            );\n'
        ,
        '            assert_eq!(twin.get_hashcode(), direct.get_hashcode(), "leaf {index}");\n'
        ,
        1,
    ),
    (
        'rust/tests/graph/arrow.rs',
        '    for column in [\n'
        '        "uuid",\n'
        '        "crossuuid",\n'
        '        "crosscode",\n'
        '        "hashcode",\n'
        '        "price",\n'
        '    ] {\n'
        ,
        '    for column in ["uuid", "crossuuid", "crosscode", "hashcode", "price"] {\n'
        ,
        1,
    ),
    (
        'rust/tests/graph/arrow.rs',
        '            rebuilt\n'
        '                .alive()\n'
        '                .map(Element::get_uuid)\n'
        '                .collect::<Vec<_>>(),\n'
        '            previous\n'
        '                .alive()\n'
        '                .map(Element::get_uuid)\n'
        '                .collect::<Vec<_>>()\n'
        ,
        '            rebuilt.alive().map(Element::get_uuid).collect::<Vec<_>>(),\n'
        '            previous.alive().map(Element::get_uuid).collect::<Vec<_>>()\n'
        ,
        1,
    ),
    (
        'rust/tests/graph/arrow.rs',
        '            walked\n'
        '                .alive()\n'
        '                .map(Element::get_uuid)\n'
        '                .collect::<Vec<_>>()\n'
        ,
        '            walked.alive().map(Element::get_uuid).collect::<Vec<_>>()\n'
        ,
        1,
    ),
    (
        'rust/tests/graph/arrow.rs',
        '        assert_eq!(\n'
        '            read.get_hashcode(),\n'
        '            stated.get_hashcode(),\n'
        '            "{index}"\n'
        '        );\n'
        ,
        '        assert_eq!(read.get_hashcode(), stated.get_hashcode(), "{index}");\n'
        ,
        1,
    ),
    (
        'rust/tests/graph/book.rs',
        '    assert_eq!(\n'
        '        left_operation.get_uuid(),\n'
        '        right_operation.get_uuid()\n'
        '    );\n'
        ,
        '    assert_eq!(left_operation.get_uuid(), right_operation.get_uuid());\n'
        ,
        1,
    ),
    (
        'rust/tests/graph/book.rs',
        '                    rebuilt\n'
        '                        .alive()\n'
        '                        .map(Element::get_uuid)\n'
        '                        .collect::<Vec<_>>(),\n'
        '                    expected\n'
        '                        .alive()\n'
        '                        .map(Element::get_uuid)\n'
        '                        .collect::<Vec<_>>(),\n'
        ,
        '                    rebuilt.alive().map(Element::get_uuid).collect::<Vec<_>>(),\n'
        '                    expected.alive().map(Element::get_uuid).collect::<Vec<_>>(),\n'
        ,
        1,
    ),
    (
        'rust/tests/graph/column.rs',
        '            "transunix", "creaunix", "sendunix", "exprunix", "prevunix", "snapunix", "prevuuid",\n'
        '            "seqnum", "state",\n'
        ,
        '            "transunix",\n'
        '            "creaunix",\n'
        '            "sendunix",\n'
        '            "exprunix",\n'
        '            "prevunix",\n'
        '            "snapunix",\n'
        '            "prevuuid",\n'
        '            "seqnum",\n'
        '            "state",\n'
        ,
        1,
    ),
    (
        'rust/tests/graph/column.rs',
        '    for element in [\n'
        '        "uuid",\n'
        '        "crossuuid",\n'
        '        "crosscode",\n'
        '        "hashcode",\n'
        '        "srcuuids",\n'
        '    ] {\n'
        ,
        '    for element in ["uuid", "crossuuid", "crosscode", "hashcode", "srcuuids"] {\n'
        ,
        1,
    ),
    (
        'rust/tests/graph/element.rs',
        '    assert_ne!(\n'
        '        twin.get_uuid(),\n'
        '        fill.get_uuid(),\n'
        '        "followed, so moved"\n'
        '    );\n'
        ,
        '    assert_ne!(twin.get_uuid(), fill.get_uuid(), "followed, so moved");\n'
        ,
        1,
    ),
    (
        'rust/tests/graph/element.rs',
        '    assert_eq!(\n'
        '        (merged.get_transunix(), merged.get_hashcode()),\n'
        '        (10, 0xD)\n'
        '    );\n'
        ,
        '    assert_eq!((merged.get_transunix(), merged.get_hashcode()), (10, 0xD));\n'
        ,
        1,
    ),
    (
        'rust/tests/graph/element.rs',
        '    assert_eq!(\n'
        '        decoded(event.get_uuid()).2,\n'
        '        uuid_payload(0xCAFE, 0xBEEF, 0)\n'
        '    );\n'
        ,
        '    assert_eq!(decoded(event.get_uuid()).2, uuid_payload(0xCAFE, 0xBEEF, 0));\n'
        ,
        1,
    ),
    (
        'rust/tests/graph/element.rs',
        '    // Merged, the event is finalized: its identity is what it now says.\n'
        '    assert_eq!(\n'
        '        merged.get_uuid(),\n'
        '        merged.time_uuid().expect("an identity")\n'
        '    );\n'
        ,
        '    // Merged, the event is finalized: its identity is what it now says.\n'
        '    assert_eq!(merged.get_uuid(), merged.time_uuid().expect("an identity"));\n'
        ,
        1,
    ),
    (
        'rust/tests/graph/element.rs',
        '    assert_eq!(\n'
        '        followed.get_hashcode(),\n'
        '        followed.digest_event().as_u64()\n'
        '    );\n'
        ,
        '    assert_eq!(followed.get_hashcode(), followed.digest_event().as_u64());\n'
        ,
        1,
    ),
    (
        'rust/tests/graph/element.rs',
        '    assert_eq!(merged.get_transunix(), at(50));\n'
        '    assert_eq!(\n'
        '        merged.get_uuid(),\n'
        '        merged.time_uuid().expect("an identity")\n'
        '    );\n'
        ,
        '    assert_eq!(merged.get_transunix(), at(50));\n'
        '    assert_eq!(merged.get_uuid(), merged.time_uuid().expect("an identity"));\n'
        ,
        1,
    ),
    (
        'rust/tests/graph/element.rs',
        '    assert_eq!(\n'
        '        event.get_uuid(),\n'
        '        event.time_uuid().expect("an identity")\n'
        '    );\n'
        '    assert_eq!(\n'
        '        event.get_hashcode(),\n'
        '        event.digest_market_event().as_u64()\n'
        '    );\n'
        ,
        '    assert_eq!(event.get_uuid(), event.time_uuid().expect("an identity"));\n'
        '    assert_eq!(event.get_hashcode(), event.digest_market_event().as_u64());\n'
        ,
        1,
    ),
    (
        'rust/tests/graph/iterator.rs',
        '    assert_eq!(\n'
        '        second.get_uuid(),\n'
        '        second.time_uuid().expect("an identity")\n'
        '    );\n'
        ,
        '    assert_eq!(second.get_uuid(), second.time_uuid().expect("an identity"));\n'
        ,
        1,
    ),
    (
        'rust/tests/graph/iterator.rs',
        '    assert_eq!(\n'
        '        stray.get_uuid(),\n'
        '        stray.time_uuid().expect("an identity")\n'
        '    );\n'
        ,
        '    assert_eq!(stray.get_uuid(), stray.time_uuid().expect("an identity"));\n'
        ,
        1,
    ),
    (
        'rust/tests/graph/market_data.rs',
        '    assert_ne!(\n'
        '        leaf.get_uuid(),\n'
        '        fill.get_uuid(),\n'
        '        "finalized once more"\n'
        '    );\n'
        ,
        '    assert_ne!(leaf.get_uuid(), fill.get_uuid(), "finalized once more");\n'
        ,
        1,
    ),
    (
        'rust/tests/graph/serve.rs',
        '        expected\n'
        '            .alive()\n'
        '            .map(Element::get_uuid)\n'
        '            .collect::<Vec<_>>()\n'
        ,
        '        expected.alive().map(Element::get_uuid).collect::<Vec<_>>()\n'
        ,
        1,
    ),
    (
        'rust/tests/graph/view.rs',
        '    assert!(\n'
        '        uuids(column(&out, "uuid")).contains(&Some(quote.get_uuid().into_bytes().to_vec()))\n'
        '    );\n'
        ,
        '    assert!(uuids(column(&out, "uuid")).contains(&Some(quote.get_uuid().into_bytes().to_vec())));\n'
        ,
        1,
    ),
    (
        'rust/tests/text/line.rs',
        '            assert_eq!(\n'
        '                line.get_uuid(),\n'
        '                self::line(&body, &options).get_uuid()\n'
        '            );\n'
        ,
        '            assert_eq!(line.get_uuid(), self::line(&body, &options).get_uuid());\n'
        ,
        1,
    ),
]


def excluded(path):
    if path in EXCLUDED_PATHS or path.startswith(EXCLUDED_PREFIXES):
        return True
    return any(fnmatch.fnmatchcase(path, pattern) for pattern in EXCLUDED_GLOBS)


def tracked_files(root):
    listing = subprocess.run(
        ["git", "-C", str(root), "ls-files", "-z"],
        check=True,
        capture_output=True,
    ).stdout
    return [name for name in listing.decode("utf-8").split("\0") if name]


def read_text(path):
    data = path.read_bytes()
    if b"\0" in data:
        return None
    try:
        return data.decode("utf-8")
    except UnicodeDecodeError:
        return None


def apply_edit(text, path, old, new, count, errors):
    """The text with one anchored edit applied, or as it was where it already is."""
    found_old = text.count(old)
    found_new = text.count(new)
    if found_new >= count and (old in new or found_old == 0):
        return text
    if found_old != count:
        errors.append(f"{path}: anchor expected {count}, found {found_old}: {old[:120]!r}")
        return text
    return text.replace(old, new)


def swept_spans(path, text, errors):
    """The spans of `text` the bulk sweep reads: all of it, less a kept region."""
    region = KEPT_REGIONS.get(path)
    if region is None:
        return [(0, len(text))]
    start_anchor, end_anchor = region
    if text.count(start_anchor) != 1 or text.count(end_anchor) != 1:
        errors.append(f"{path}: the kept region's anchors are not each found once")
        return [(0, len(text))]
    start = text.index(start_anchor)
    end = text.index(end_anchor)
    if end < start:
        errors.append(f"{path}: the kept region ends before it starts")
        return [(0, len(text))]
    return [(0, start), (end, len(text))]


def bulk(path, text, errors):
    """The text with every bulk match outside a kept region renamed, and the count."""
    pieces = []
    matches = 0
    cursor = 0
    for start, end in swept_spans(path, text, errors):
        pieces.append(text[cursor:start])
        span = text[start:end]
        matches += sum(1 for _ in BULK.finditer(span))
        pieces.append(BULK.sub(lambda match: OLD_TO_NEW[match.group(0)], span))
        cursor = end
    pieces.append(text[cursor:])
    return "".join(pieces), matches


def sweep(root, flags=frozenset(), layout=True):
    """Every phase over the tree at `root`, in memory: the swept texts, the
    originals, the refusals and the bulk table. `layout` false stops before
    phase 4, which is what a regeneration of `FMT_EDITS` reads."""
    errors = []

    for path, old, new, count in EDITS:
        if OLD_ANY.search(new):
            errors.append(f"{path}: a replacement still spells an old name: {new[:120]!r}")
        if excluded(path):
            errors.append(f"{path}: an edit names an excluded path")
    for path, old, new, count in FMT_EDITS:
        if OLD_ANY.search(old) or OLD_ANY.search(new):
            errors.append(f"{path}: a layout edit spells an old name: {old[:120]!r}")
        if excluded(path):
            errors.append(f"{path}: a layout edit names an excluded path")

    texts = {}
    for name in tracked_files(root):
        if excluded(name):
            continue
        text = read_text(root / name)
        if text is not None:
            texts[name] = text
    originals = dict(texts)

    # Phases 1 and 2: the meaning, then the hash sentence.
    for path, old, new, count in EDITS + [HASH_EDIT]:
        if path not in texts:
            errors.append(f"{path}: an edited file is not in the tree")
            continue
        texts[path] = apply_edit(texts[path], path, old, new, count, errors)

    # Phase 3: the bare names.
    table = {}
    for name in sorted(texts):
        swept, matches = bulk(name, texts[name], errors)
        if matches:
            table[name] = matches
        expected = BULK_EXPECTED.get(name, 0)
        if matches == expected or (matches == 0 and "--emit-table" not in flags):
            texts[name] = swept
        else:
            errors.append(f"{name}: bulk expected {expected}, found {matches}")
    for name in sorted(set(BULK_EXPECTED) - set(texts)):
        errors.append(f"{name}: a bulk file is not in the tree")

    # Phase 4: rustfmt's layout of the swept Rust.
    if layout:
        for path, old, new, count in FMT_EDITS:
            if path not in texts:
                errors.append(f"{path}: a layout-edited file is not in the tree")
                continue
            texts[path] = apply_edit(texts[path], path, old, new, count, errors)

    return texts, originals, errors, table


def main(argv):
    args = [arg for arg in argv[1:] if not arg.startswith("--")]
    flags = {arg for arg in argv[1:] if arg.startswith("--")}
    if len(args) != 1 or flags - {"--check", "--emit-table"}:
        sys.exit(__doc__)
    root = Path(args[0]).resolve()
    texts, originals, errors, table = sweep(root, flags)

    if "--emit-table" in flags:
        print("BULK_EXPECTED = {")
        for name, matches in table.items():
            print(f"    {name!r}: {matches},")
        print("}")
        return

    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        sys.exit(f"{len(errors)} refusal(s); nothing written")

    changed = [name for name in sorted(texts) if texts[name] != originals[name]]
    if "--check" not in flags:
        for name in changed:
            (root / name).write_bytes(texts[name].encode("utf-8"))
    verb = "would change" if "--check" in flags else "changed"
    print(f"{verb} {len(changed)} file(s)")
    for name in changed:
        print(f"  {name}")


if __name__ == "__main__":
    main(sys.argv)
