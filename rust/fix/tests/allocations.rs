//! The allocation claims over what `yggdryl-fix` owns, counted by the
//! same allocator as the core's own in `rust/tests/allocations.rs`.
//!
//! What a borrowed protocol view allocates, counted rather than asserted.
//!
//! A view is one pointer plus a `Scheme`, built per call rather than stored,
//! and that is only defensible if building one and reading through it costs
//! nothing. A comment saying so is not evidence, and neither is a timing: a
//! stray `String` in an accessor hides easily inside a map lookup. So this
//! counts them, and pins the three places a protocol read does allocate - a
//! key handed back to the caller, a lookup key too long for `SmolStr`'s inline
//! buffer, and a value that is a serie.
//!
//! It also pins the no-op write. A rewrite of a value a field already carries
//! must cost the same however much metadata surrounds it, because it stops
//! before copying the map; nothing else fails when that short-circuit is lost,
//! so nothing else would notice.
//!
//! The counting allocator is a global and a program has exactly one, which is
//! why this is its own test target rather than a case in another file.

#[path = "support/install.rs"]
mod install;
#[path = "support/ulbridge.rs"]
mod ulbridge;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::sync::Mutex;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};

use std::sync::Arc;
use yggdryl::StructType;
use yggdryl::graph::{Element, Event};
use yggdryl::holder::Buffer;
use yggdryl::text::{TextBytes, TextLine, TextOptions, read_text_lines};
use yggdryl::{DataType, Field, FieldPath, MediaType, Scalar, Serie, TimeUnit, Timezone};
use yggdryl_fix::{FixCode, FixCodec, FixFieldMut, FixId, FixMsg, FixRegistry};
use yggdryl_market::graph::Market;

/// A pass-through allocator that counts allocations, and the bytes they
/// hold, while armed.
struct Counting;

/// Allocations since the counter was armed.
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

/// Bytes the armed thread allocated less the bytes it freed since it was
/// armed - below zero where it frees what it held before - and the most
/// that difference reached.
static LIVE: AtomicIsize = AtomicIsize::new(0);
static PEAK: AtomicIsize = AtomicIsize::new(0);

/// Move the armed thread's live bytes by `delta`, keeping the peak.
fn held(delta: isize) {
    let live = LIVE.fetch_add(delta, Ordering::Relaxed) + delta;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

/// A size as a byte delta: an allocation never exceeds `isize::MAX`.
fn signed(size: usize) -> isize {
    isize::try_from(size).unwrap_or(isize::MAX)
}

// Armed *per thread*, and const-initialized so reading it inside the allocator
// cannot itself allocate. Cargo runs the cases in this file concurrently, and a
// process-wide flag would charge one case for another's setup - which looks
// exactly like the stray accessor allocation these cases exist to catch.
thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
}

/// Held across a counted section, so only one thread is ever armed.
static COUNTING: Mutex<()> = Mutex::new(());

// SAFETY: every method forwards to `System`, which upholds the contract; the
// counter is an atomic and the flag is a const-initialized thread local, so
// neither adds aliasing nor re-enters the allocator.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.get() {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            held(signed(layout.size()));
        }
        // SAFETY: `layout` is forwarded unchanged to the system allocator.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if ARMED.get() {
            held(-signed(layout.size()));
        }
        // SAFETY: the pointer came from `System.alloc` with this same layout.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if ARMED.get() {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            held(signed(size) - signed(layout.size()));
        }
        // SAFETY: the pointer and layout came from this allocator.
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Count the allocations `work` performs, and return them with its answer.
fn counted<T>(work: impl FnOnce() -> T) -> (usize, T) {
    let (counted, _, _, answer) = armed(work);
    (counted, answer)
}

/// The most bytes `work` held at once beyond what its thread held before,
/// with its answer.
fn peaked<T>(work: impl FnOnce() -> T) -> (usize, T) {
    let (_, peak, _, answer) = armed(work);
    (peak, answer)
}

/// The bytes `work` left allocated when it returned - what it made and
/// kept, its answer's included, less what it freed of what its thread held
/// before - with its answer.
fn retained<T>(work: impl FnOnce() -> T) -> (isize, T) {
    let (_, _, live, answer) = armed(work);
    (live, answer)
}

/// Run `work` armed: its allocations, the most bytes it held at once, the
/// bytes it still held when it returned and its answer.
fn armed<T>(work: impl FnOnce() -> T) -> (usize, usize, isize, T) {
    let guard = COUNTING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // The machine's name is read once per process, by whichever surface asks
    // first; read here, outside every count, it is never any one test's cost.
    black_box(yggdryl::HOSTNAME.len());
    ALLOCATIONS.store(0, Ordering::Relaxed);
    LIVE.store(0, Ordering::Relaxed);
    PEAK.store(0, Ordering::Relaxed);
    ARMED.set(true);
    let answer = work();
    ARMED.set(false);
    let counted = ALLOCATIONS.load(Ordering::Relaxed);
    let peak = usize::try_from(PEAK.load(Ordering::Relaxed)).unwrap_or(0);
    let live = LIVE.load(Ordering::Relaxed);
    drop(guard);
    (counted, peak, live, answer)
}

/// Count `work` run once and run a thousand times.
///
/// One run alone cannot tell a per-call cost from a one-time initialization,
/// so every case reports both and asserts the relationship between them.
fn counted_once_and_repeated(mut work: impl FnMut()) -> (usize, usize) {
    work();
    let (once, ()) = counted(&mut work);
    let (repeated, ()) = counted(|| {
        for _ in 0..1_000 {
            work();
        }
    });
    (once, repeated)
}

/// Assert a read costs nothing, whether it runs once or a thousand times.
fn free(what: &str, work: impl FnMut()) {
    let (once, repeated) = counted_once_and_repeated(work);
    assert_eq!(once, 0, "{what} allocated on a single read");
    assert_eq!(repeated, 0, "{what} allocated over a thousand reads");
}

/// Count `work` over an input `setup` builds outside the count, once and
/// then sixty-four times over inputs built ahead: a stage that takes a
/// fresh message or row by value is charged for what it does with it and
/// never for the clone that hands it one. Sixty-four rather than a thousand,
/// because a packed message is tens of kilobytes.
fn counted_each<I, O>(
    mut setup: impl FnMut() -> I,
    mut work: impl FnMut(I) -> O,
) -> (usize, usize) {
    drop(work(setup()));
    let one = setup();
    let (once, out) = counted(|| work(one));
    drop(out);
    let inputs: Vec<I> = (0..64).map(|_| setup()).collect();
    let mut outputs = Vec::with_capacity(64);
    let (repeated, ()) = counted(|| outputs.extend(inputs.into_iter().map(&mut work)));
    drop(outputs);
    (once, repeated)
}

/// Assert a read costs exactly `each` allocations every time it runs.
fn costs(what: &str, each: usize, work: impl FnMut()) {
    let (once, repeated) = counted_once_and_repeated(work);
    assert_eq!(once, each, "{what} cost changed for a single read");
    assert_eq!(
        repeated,
        each * 1_000,
        "{what} did not cost {each} per read over a thousand"
    );
}

/// The dialect the venue field below is a member of.
///
/// Membership is provenance a caller filters on; resolution never consults
/// it, so a member field costs what a standard one does in every probe.
const VENUE: &str = "venue";

/// A FIX registry of `extra` generated fields around the keyed ones every
/// probe below lands on: a standard scalar with alternate tags and aliases,
/// a venue's member, a message type, and a repeating group beside its
/// counter.
///
/// The generated fields are what a probe walks past in the maps; the keyed
/// ones are what every hit lands on.
///
/// They are tagged from 1100 rather than from 1000 so that no generated tag
/// is one this crate types: a message lifts a typed tag onto a holder
/// instead of leaving it among the entries, which is a different cost from
/// what a pair adds, and the probes below measure the latter.
fn fix_registry(extra: usize) -> FixRegistry {
    let item = StructType::from_fields([DataType::utf8().nullable_field("PartyID")])
        .map(DataType::from)
        .expect("a struct item")
        .required_field("item");
    let mut parties = DataType::serie(item).nullable_field("Parties");
    FixFieldMut::new(&mut parties)
        .set_counter(453)
        .expect("a static counter");
    let mut counter = DataType::Int32.nullable_field("NoPartyIDs");
    FixFieldMut::new(&mut counter)
        .set_tag(453)
        .expect("a static tag");
    let mut symbol = DataType::utf8().nullable_field("Symbol");
    FixFieldMut::new(&mut symbol)
        .set_tag(55)
        .expect("a static tag");
    FixFieldMut::new(&mut symbol)
        .set_tags(&[65])
        .expect("a static alternate tag");
    FixFieldMut::new(&mut symbol)
        .set_names(["Ticker", "SecuritySymbolIdentifier"])
        .expect("static aliases");
    let mut msgtype = DataType::utf8().nullable_field("MsgType");
    FixFieldMut::new(&mut msgtype)
        .set_tag(35)
        .expect("a static tag");
    let mut trade = DataType::utf8().nullable_field("TradeID");
    FixFieldMut::new(&mut trade)
        .set_tag(5_001)
        .expect("a static tag");
    FixFieldMut::new(&mut trade)
        .set_sources([VENUE])
        .expect("a static membership");
    FixFieldMut::new(&mut trade)
        .set_names(["TradeIdentifier"])
        .expect("a static alias");
    let generated = (0..extra).map(|index| {
        let mut field = DataType::Int64.nullable_field(format!("Generated{index:04}"));
        let tag = i32::try_from(1_100 + index).expect("a small tag");
        FixFieldMut::new(&mut field)
            .set_tag(tag)
            .expect("a generated tag");
        FixFieldMut::new(&mut field)
            .set_names([format!("GeneratedAlias{index:04}")])
            .expect("a generated alias");
        field
    });
    let mut registry = FixRegistry::from_fields(
        [symbol, msgtype, trade, counter]
            .into_iter()
            .chain(generated),
    )
    .expect("the generated dictionary has no conflict");
    registry.insert(parties).expect("the group definition");
    registry
}

#[test]
fn a_fix_registry_lookup_borrows_whatever_the_catalog_walks_past() {
    crate::install::installed();
    // A wide dictionary: a hit must cost the same however much it walks past.
    let registry = fix_registry(512);
    let vendor = FixId::of(5_001, "TradeID").expect("a vendor identifier");
    // Another name on the same tag is another identity: an exact miss.
    let foreign = FixId::of(5_001, "OtherTradeID").expect("a foreign identifier");

    for (what, key) in [
        ("a scalar tag", yggdryl_fix::FixKey::Tag(55)),
        ("an alternate tag", yggdryl_fix::FixKey::Tag(65)),
        ("a counter tag", yggdryl_fix::FixKey::Tag(453)),
        ("a tag nothing declares", yggdryl_fix::FixKey::Tag(9_999)),
        ("a canonical name", yggdryl_fix::FixKey::Name("symbol")),
        ("an alias", yggdryl_fix::FixKey::Name("ticker")),
        ("a counter name", yggdryl_fix::FixKey::Name("nopartyids")),
        ("a group name", yggdryl_fix::FixKey::Name("parties")),
        ("a venue's own name", yggdryl_fix::FixKey::Name("tradeid")),
        ("an identity", yggdryl_fix::FixKey::Id(vendor)),
        (
            "an identity nothing holds",
            yggdryl_fix::FixKey::Id(foreign),
        ),
    ] {
        free(what, || {
            let _ = black_box(registry.get_field(black_box(key)));
        });
    }
    // A name nothing declares is the one probe that is not free: the fold
    // parses the spelling it looks up as a path - the tokens, the segments
    // and the path they make - before it can say there is no field under
    // it. It last moved down by one when the reserved-word check began to
    // compare a word case-insensitively rather than render it lower-cased.
    costs("a name nothing declares", 3, || {
        let _ = black_box(registry.get_field(black_box(yggdryl_fix::FixKey::Name("absent"))));
    });
    free("the counter door", || {
        let _ = black_box(registry.get_field_by_counter(black_box(453)));
    });
    // The walk is the one probe that is not free: it holds the cursor it
    // hands each field from, and that is one allocation however many it
    // walks past.
    costs("the walk", 1, || {
        let _ = black_box(registry.iter().count());
    });
}

#[test]
fn reading_which_way_a_line_moved_allocates_nothing() {
    crate::install::installed();
    // The defaults, and a table the dictionary states: both are compiled
    // when the reading is built, so neither costs a match.
    let registry = fix_registry(8);
    let reading = registry.msgdirection();
    for line in [
        b"sending >> 8=FIX.4.4|35=D|10=0|".as_slice(),
        b"2026-08-14 03:03:13.314 [23] [Jolokia] (DEBUG) Response: 8=FIX.4.4|35=0|10=0|",
        b"sending and receiving 8=FIX.4.4|35=D|10=0|",
        b"no verb printed by this plugin 8=FIX.4.4|35=D|10=0|",
    ] {
        free("the default reading", || {
            let _ = black_box(reading.read_bytes(black_box(line)));
        });
    }
    let mut ruled = FixRegistry::new();
    let mut field = DataType::utf8().nullable_field("MsgDirection");
    FixFieldMut::new(&mut field).set_tag(385).unwrap();
    FixFieldMut::new(&mut field)
        .set_directions(&[
            yggdryl_fix::FixDirection::new("S", [r"(?i)(?:^|\s)tx\s", ">>>"]),
            yggdryl_fix::FixDirection::new("R", [r"(?i)(?:^|\s)rx\s", "<<<"]),
        ])
        .unwrap();
    ruled.insert(field).unwrap();
    let ruled = ruled.msgdirection();
    for line in [
        b"09:12:03 TX 8=FIX.4.4|35=D|10=0|".as_slice(),
        b"09:12:03 <<< 8=FIX.4.4|35=0|10=0|",
        b"TX <<< 8=FIX.4.4|35=D|10=0|",
        b"09:12:03 8=FIX.4.4|35=D|10=0|",
    ] {
        free("a stated table", || {
            let _ = black_box(ruled.read_bytes(black_box(line)));
        });
    }
}

#[test]
fn a_fix_code_lookup_allocates_nothing() {
    crate::install::installed();
    let mut registry = FixRegistry::new();
    registry
        .set_codeset(
            "sidecodeset",
            &[FixCode::new("Buy", "1"), FixCode::new("Sell", "2")],
        )
        .expect("a static code set");
    let mut field = DataType::utf8().nullable_field("Side");
    FixFieldMut::new(&mut field)
        .set_tag(9_995)
        .expect("a static tag");
    FixFieldMut::new(&mut field)
        .set_codeset("sidecodeset")
        .expect("the set the field reads by");
    // The field names the set and the dictionary holds its members, so the
    // name is resolved here, once: what is counted below is the scan over
    // the set alone.
    let set = registry.codeset_of(&field).expect("a held code set");
    free("a code by its wire value", || {
        let _ = black_box(set.code(black_box("1")));
    });
    free("a code by its name", || {
        let _ = black_box(set.code_by_name(black_box("buy")));
    });
    free("a name for a wire value", || {
        let _ = black_box(set.code_name(black_box("2")));
    });
    free("a value no code spells", || {
        let _ = black_box(set.code(black_box("9")));
    });
    free("the walk over the set", || {
        let _ = black_box(set.codes().count());
    });
}

#[test]
fn a_fix_message_tag_lookup_allocates_nothing() {
    crate::install::installed();
    let registry = Arc::new(fix_registry(64));
    let vendor = FixId::of(5_001, "TradeID").expect("a vendor identifier");
    let foreign = FixId::of(5_001, "OtherTradeID").expect("a foreign identifier");
    let mut symbol = DataType::utf8().nullable_field("Symbol");
    FixFieldMut::new(&mut symbol)
        .set_tag(55)
        .expect("a static tag");
    let mut trade = DataType::utf8().nullable_field("TradeID");
    FixFieldMut::new(&mut trade)
        .set_tag(5_001)
        .expect("a static tag");
    FixFieldMut::new(&mut trade)
        .set_sources([VENUE])
        .expect("a static membership");
    let root = StructType::from_fields([symbol, trade, DataType::utf8().nullable_field("9999")])
        .map(DataType::from)
        .expect("three children")
        .required_field("row");
    let value = Scalar::from_sequence([
        Scalar::from("AAPL"),
        Scalar::from("T-1"),
        Scalar::from("custom"),
    ]);
    let msg = FixMsg::with_registry(registry, root, value).expect("a valid message");

    // One namespace: a venue's field by its own name and a standard field by
    // its tag are both reachable from the same message, and neither costs
    // an allocation.
    assert!(
        msg.get_by_name("tradeid").is_some(),
        "the venue field by name"
    );
    assert!(msg.get_by_tag(55).is_some(), "the standard field by tag");
    free("get_by_name vendor", || {
        let _ = black_box(msg.get_by_name(black_box("tradeid")));
    });
    free("get_by_tag vendor", || {
        let _ = black_box(msg.get_by_tag(black_box(5_001)));
    });
    free("get_by_tag known", || {
        let _ = black_box(msg.get_by_tag(black_box(55)));
    });
    free("get_by_id vendor", || {
        let _ = black_box(msg.get_by_id(black_box(vendor)));
    });
    free("get_by_id foreign", || {
        let _ = black_box(msg.get_by_id(black_box(foreign)));
    });
    // An unknown tag is rendered on the stack and looked up by that name.
    free("get_by_tag unknown retained", || {
        let _ = black_box(msg.get_by_tag(black_box(9999)));
    });
    free("get_by_tag unknown absent", || {
        let _ = black_box(msg.get_by_tag(black_box(1234)));
    });
    free("get_by_name", || {
        let _ = black_box(msg.get_by_name(black_box("ticker")));
    });
    let absent_member = FieldPath::from_str("Symbol.absent").expect("a path");
    free("get_by_path", || {
        let _ = black_box(msg.get_by_path(black_box(&absent_member)));
    });
}

#[test]
fn the_typed_facts_of_a_message_are_borrowed_at_every_row_width() {
    crate::install::installed();
    // The header, the capture and the event are held beside the row rather
    // than in it, so reading one is a borrow whatever the row carries.
    for width in [0, 64, 1_024] {
        let field = StructType::from_fields(
            (0..width).map(|index| DataType::Int64.required_field(format!("datum{index}"))),
        )
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        let value = Scalar::from_sequence((0..width).map(Scalar::from));
        let message = FixMsg::with_registry(Arc::new(FixRegistry::new()), field, value).unwrap();
        free("the typed holders", || {
            let held = black_box(&message);
            black_box((
                held.header().msgtype(),
                held.header().beginstring(),
                held.header().sendingtime(),
                held.capture().msgpluginid(),
                held.text(),
                held.metadata().len(),
            ));
        });
        free("the settled identity", || {
            let held = black_box(&message);
            black_box((
                held.get_transunix(),
                held.get_hashcode(),
                held.get_uuid(),
                held.get_crosscode(),
            ));
        });
    }
}

#[test]
fn direct_fix_operation_conversion_needs_no_intermediate_allocation() {
    crate::install::installed();
    let codec = FixCodec::new(Arc::new(fix_registry(0)));
    for wire in [b"35=D|55=AAPL|".as_slice(), b"35=S|55=AAPL|"] {
        let message = codec.parse_line(wire).unwrap().next().unwrap().unwrap();
        let (allocations, operation) =
            counted(|| yggdryl_fix::FixMsg::into_market_leaf(message).unwrap());
        assert_eq!(operation.get_ticker(), Some("AAPL"));
        assert_eq!(
            allocations, 0,
            "a direct operation moves its existing holder"
        );
    }
}

/// A capture expands into its sorted leaves at a cost per message that does
/// not grow with the capture - no row parsed again, no metadata key built
/// per leaf beyond its own, one sort of the leaves - whether the leaves
/// carry their unmapped fields or not, and carrying them costs something.
#[test]
fn fix_market_operations_cost_is_linear_in_the_leaves() {
    crate::install::installed();
    let registry = Arc::new(committed_registry().clone());
    // Orders, quotes, a fill and a two-entry snapshot in turn, a second
    // apart, each stating fields no typed column reads. The quote is
    // two-sided: one leaf holding both its legs.
    let capture = |codec: &FixCodec, messages: usize| -> Vec<FixMsg> {
        (0..messages)
            .map(|index| {
                let clock = format!("20260921-10:{:02}:{:02}", index / 60, index % 60);
                let line = match index % 4 {
                    0 => format!(
                        "8=FIX.4.4|35=D|52={clock}|11=C{index}|55=AAPL|54=1|44=100|38=5|40=2|21=1|10=0|"
                    ),
                    1 => format!(
                        "8=FIX.4.4|35=S|52={clock}|117=Q{index}|55=AAPL|132=99|134=7|133=101|135=8|537=1|10=0|"
                    ),
                    2 => format!(
                        "8=FIX.4.4|35=8|52={clock}|17=E{index}|37=O{index}|55=AAPL|54=1|31=100|32=2|150=F|40=2|10=0|"
                    ),
                    _ => format!(
                        "8=FIX.4.4|35=W|52={clock}|55=AAPL|1180=MDP|268=2|269=0|278=B{index}|270=100|271=10|83=1|269=1|278=A{index}|270=101|271=12|83=2|10=0|"
                    ),
                };
                codec
                    .parse_fix_line(line.as_bytes())
                    .expect("a synthetic message")
            })
            .collect()
    };
    let cost = |market_metadata: bool, messages: usize| {
        let codec = FixCodec::new(Arc::clone(&registry))
            .with_threads(1)
            .with_market_metadata(market_metadata);
        let held = capture(&codec, messages);
        // Settle the codec's and the registry's first-use state first.
        black_box(codec.market_data(held.clone()).count());
        let (allocations, count) = counted(|| codec.market_data(held.clone()).count());
        assert_eq!(count, messages / 4 * 5, "a leaf each, two for a snapshot");
        allocations
    };
    let mut costs = Vec::new();
    for market_metadata in [true, false] {
        let (small, large) = (cost(market_metadata, 8), cost(market_metadata, 64));
        // Per message, the larger capture costs at most what the smaller
        // does plus one allocation: the sort's buffer and the leaves'
        // doublings, spread thinner, and nothing that grows per message.
        assert!(
            large <= small * 8 + 64,
            "market_metadata={market_metadata}: 64 messages cost {large}, 8 cost {small}"
        );
        costs.push((small, large));
    }
    let [(filled_small, filled_large), (bare_small, bare_large)] = costs[..] else {
        unreachable!("two settings")
    };
    assert!(
        bare_small < filled_small && bare_large < filled_large,
        "carrying the metadata costs something: {costs:?}"
    );
}

#[test]
fn a_line_of_several_frames_costs_its_messages_and_nothing_per_line() {
    crate::install::installed();
    let codec = FixCodec::new(Arc::new(fix_registry(64)));
    let frame = "8=FIX.4.4|35=D|11=A|10=001|";
    let read = |frames: usize| {
        let line = frame.repeat(frames).into_bytes();
        // The row is read outside the count: what is measured is draining
        // the messages it carries, which is where a collection would show.
        let messages = codec.parse_line(black_box(&line)).expect("a row");
        let (allocations, count) = counted(move || {
            messages
                .inspect(|message| {
                    black_box(message.as_ref().expect("a message").as_field().name());
                })
                .count()
        });
        assert_eq!(
            count, frames,
            "a line of {frames} frames is {frames} messages"
        );
        allocations
    };
    // What a row of several frames costs is its messages and nothing per
    // line: the source holds the row's entries and re-enters the frame
    // reader where each opens, so draining is proportional to the messages
    // and never to the line - which is what a collection of the results
    // would break.
    // Settle the shared schema/registry plan before counting the repeated path.
    // Cold plan construction belongs to the boundary, not to each frame.
    black_box(read(1));
    let each = read(4) / 4;
    for frames in [4, 8, 16] {
        assert_eq!(read(frames), each * frames, "{frames} frames");
    }
}

/// A framed body of `pairs` pairs, every one of them a field the dictionary
/// holds.
///
/// The keys are the generated tags [`fix_registry`] writes, so each pair
/// resolves to its own child and no two repeat: what grows between the sizes
/// below is the width of the message and nothing else.
fn fix_pairs_line(pairs: usize) -> Vec<u8> {
    let mut line = b"35=D".to_vec();
    for index in 0..pairs.saturating_sub(1) {
        line.extend_from_slice(format!("|{}={index}", 1_100 + index).as_bytes());
    }
    line.push(b'|');
    line
}

/// What one framed line costs to read as a message, by how many pairs it
/// carries.
///
/// Three widths, because one could not tell a per-message cost from a
/// per-pair one: the constant is what a message costs whatever it carries -
/// the page the line is read into, the row's schema and value, the typed
/// facts the parse settles and the arrival record - and the slope is what a
/// pair adds, which is its column and nothing for its entry, because a key
/// and a value are ranges of that one page.
///
/// A caller who decoded the line already owns the page, and
/// [`FIX_TEXT_LINE_COSTS`] is the same three widths through the door that
/// takes it: the page's own vector saved at each, and the one source the
/// message states - the line it was read from - paid instead.
///
/// Per-slot `Vec` buffers are gone: every unique ordinary field has one
/// inline scalar, and the fallback `BeginString` contributes once, saving
/// `pairs + 1` allocations from this path.
///
/// The arrival record the parse settles is held behind one shared
/// allocation, so a clone of the message - every walk keeps one - shares the
/// record rather than deriving it again: one more per message, whatever its
/// width. The table of names a lookup past the tag index falls back to is
/// sized once for every child rather than grown a doubling at a time, which
/// is nothing at four pairs, three fewer at sixteen and five at sixty-four.
///
/// It last moved when four allocations a message paid went: the settle's
/// list of the registry fields a level reached, whose name test already
/// keeps each reached once; the digest's cells, on the stack now up to
/// thirty-two; the builder's two vectors of slots sorted into header, body
/// and trailer, one vector of ranks now with the slots taken in rank order
/// out of the vector they arrived in; and the frame walk's vector of
/// arrivals, an unmarked frame now walked straight into its pairs: 32 to
/// 28 at four pairs, 33 to 29 at sixteen and 34 to 30 at sixty-four.
///
/// It rose by one at every width when that table of names came to be held
/// in the `Arc` a clone of the message shares it through, rather than built
/// again on the clone's first miss: 28 to 29, 29 to 30 and 30 to 31.
///
/// It rose by one at sixty-four alone, 31 to 32, when the displayed quantity
/// became a market fact: that line's tags run from 1100 to 1162 and so state
/// `DisplayQty(1138)`, which the market facts hold in the one boxed record
/// of rarely stated quantities they allocate on the first such fact.
///
/// It fell by one at every width when a group became its list alone: the
/// builder's finish no longer collects each child's tag and counter to
/// restate a counter beside its group, there being none: 29 to 28, 30 to 29
/// and 32 to 31.
const FIX_LINE_COSTS: [(usize, usize); 3] = [(4, 28), (16, 29), (64, 31)];

/// A dictionary of `count` `Utf8` fields, tagged from 2000.
///
/// Text rather than integers, because what is measured next door is a
/// *value's width*: a field that only types a number never carries three
/// kilobytes, so a numeric dictionary could not state the case at all.
fn fix_text_registry(count: usize) -> FixRegistry {
    let mut msgtype = DataType::utf8().nullable_field("MsgType");
    FixFieldMut::new(&mut msgtype)
        .set_tag(35)
        .expect("a static tag");
    let generated = (0..count).map(|index| {
        let mut field = DataType::utf8().nullable_field(format!("Text{index:04}"));
        let tag = i32::try_from(2_000 + index).expect("a small tag");
        FixFieldMut::new(&mut field)
            .set_tag(tag)
            .expect("a generated tag");
        field
    });
    FixRegistry::from_fields(std::iter::once(msgtype).chain(generated))
        .expect("the generated dictionary has no conflict")
}

/// A framed body of `pairs` pairs whose every value is `width` bytes wide.
fn fix_text_line(pairs: usize, width: usize) -> Vec<u8> {
    let mut line = b"35=D".to_vec();
    for index in 0..pairs.saturating_sub(1) {
        line.extend_from_slice(format!("|{}=", 2_000 + index).as_bytes());
        line.resize(line.len() + width, b'x');
    }
    line.push(b'|');
    line
}

/// What the *width* of a value costs the arrival record: nothing.
///
/// This is the claim the entries-over-ranges change exists for, and the one
/// [`FIX_LINE_COSTS`] cannot state, because its keys and values all fitted
/// `SmolStr`'s inline buffer and so were never allocations to begin with.
/// Here they do not fit: the same pair counts, once with two-byte values and
/// once with values three kilobytes wide.
///
/// The wide reading costs allocations the narrow one does not, and they are
/// the row's own: a typed `Utf8` column holds the value it was given, and a
/// column is what a row is for. The entry beside it adds nothing at all,
/// because a key and a value are ranges of the page the line was read into
/// and a range is two offsets whatever it spans.
///
/// Three pair counts and two widths, because one of each could tell neither a
/// per-message cost from a per-pair one nor a cost that scales with a value
/// from one that does not. The narrow column of this table is
/// [`FIX_LINE_COSTS`] at the same widths, and moves with it.
///
/// It last moved with [`FIX_LINE_COSTS`], by the same one in both columns.
const WIDE_VALUE_COSTS: [(usize, (usize, usize)); 3] =
    [(4, (28, 34)), (16, (29, 59)), (64, (30, 156))];

#[test]
fn a_wide_value_costs_the_entries_nothing_and_the_row_one_column() {
    crate::install::installed();
    let codec = FixCodec::new(Arc::new(fix_text_registry(64)));
    for (pairs, (narrow, wide)) in WIDE_VALUE_COSTS {
        for (width, each) in [(2, narrow), (3_072, wide)] {
            let line = fix_text_line(pairs, width);
            costs(
                &format!("a {pairs}-pair line whose values are {width} bytes wide"),
                each,
                || {
                    black_box(
                        codec
                            .parse_fix_line(black_box(&line))
                            .expect("a readable line"),
                    );
                },
            );
        }
        // The whole of the difference, stated as the rule rather than as two
        // numbers a reader has to subtract: a fixed cost per wide value,
        // the same at every width, and nothing that scales with the bytes.
        assert_eq!(
            wide - narrow,
            2 * (pairs - 1),
            "a {pairs}-pair line's wide values cost more than two each"
        );
    }
}

/// A dictionary whose `Parties` group declares `members` members.
///
/// A packed occurrence is read against what the group declares, so the
/// declaration is what decides how many rendered keys one packed value
/// becomes - which is the number the case below grows against.
fn fix_group_registry(members: usize) -> FixRegistry {
    let declared = (0..members).map(|index| {
        let mut field = DataType::utf8().nullable_field(format!("Member{index:04}"));
        let tag = i32::try_from(3_000 + index).expect("a small tag");
        FixFieldMut::new(&mut field)
            .set_tag(tag)
            .expect("a generated tag");
        field
    });
    let item = StructType::from_fields(declared)
        .map(DataType::from)
        .expect("a struct item")
        .required_field("item");
    let mut parties = DataType::serie(item).nullable_field("Parties");
    FixFieldMut::new(&mut parties)
        .set_counter(453)
        .expect("a static counter");
    let mut counter = DataType::Int32.nullable_field("NoPartyIDs");
    FixFieldMut::new(&mut counter)
        .set_tag(453)
        .expect("a static tag");
    let mut msgtype = DataType::utf8().nullable_field("MsgType");
    FixFieldMut::new(&mut msgtype)
        .set_tag(35)
        .expect("a static tag");
    let mut registry = FixRegistry::from_fields([msgtype, counter])
        .expect("the generated dictionary has no conflict");
    registry.insert(parties).expect("the group definition");
    registry
}

/// One bridge row packing `members` members into a single occurrence.
fn fix_packed_line(members: usize) -> Vec<u8> {
    let mut line = b"MSGTYPE=D|NOPARTYIDS=1|NOPARTYIDS[0]=".to_vec();
    for index in 0..members {
        if index > 0 {
            line.extend_from_slice(b"\x04\x03");
        }
        line.extend_from_slice(format!("MEMBER{index:04}=v{index}").as_bytes());
    }
    line
}

/// What unpacking one packed occurrence costs, by how many members it packs.
///
/// The path packing is about, and the one the plain frame above never
/// reaches. A bridge writes a whole occurrence into one value and the reader
/// unpacks it into `NOPARTYIDS[0].MEMBER0000` and its siblings, which are
/// keys no range of the line names - so they are the one thing on this path
/// that has to be built rather than pointed at.
///
/// Each rendered key is exactly one allocation: the path, held as the bytes
/// it is, rather than a counted page that would only be borrowed back.
/// Members stay in `Member::Value` and the group keeps occurrences, so no
/// `Slot.values` buffer exists. A packed row has exactly two scalar slots:
/// `MsgType` and fallback `BeginString`; the counter opens the group and is
/// no slot of its own.
///
/// Four members and sixteen, because the number that matters is the slope,
/// and the rest of it is the row a wider group builds. The codec reads a
/// row's pairs directly and descends into none of them, so the tree the
/// packed value would have been scanned into is not among these.
/// Two member counts, because the number that matters is the slope and not
/// the constant a message pays whatever it carries.
///
/// The arrival record's one shared allocation is in both, as it is in
/// [`FIX_LINE_COSTS`].
///
/// It last moved when the group's rows were written straight into the runs
/// they are stored in with the members moved out of the occurrence rather
/// than cloned, the key that orders the rows rendered only where a second
/// occurrence exists to order against, the grouped path's top level held in
/// locals rather than a vector, the alias child's metadata shared through
/// the memo, and the occurrence index written into its path in place: 78
/// to 57 at four members and 138 to 79 at sixteen, the slope below two per
/// member being the vectors that grow by doubling across the row.
///
/// It moved again when the settle's reached list and the digest's cells
/// vector went and the builder's finish sorted ranks rather than slots -
/// one each, at both widths - and when a packed value's members were sized
/// once rather than grown by doubling, two fewer at sixteen: 57 to 54 at
/// four members and 79 to 74 at sixteen.
///
/// It rose by one at both widths with [`FIX_LINE_COSTS`], the `Arc` the
/// message's table of names is shared through: 54 to 55 and 74 to 75.
///
/// It fell by two at both widths when a group became its list alone: the
/// finish's list of each child's tag and counter, as at every width of
/// [`FIX_LINE_COSTS`], and the list of counters the entries collected at
/// the row's level to leave the counter's child out: 55 to 53 and 75 to 73.
const PACKED_MEMBER_COSTS: [(usize, usize); 2] = [(4, 53), (16, 73)];

#[test]
fn a_packed_occurrence_costs_one_allocation_for_each_key_it_renders() {
    crate::install::installed();
    for (members, each) in PACKED_MEMBER_COSTS {
        let codec = FixCodec::new(Arc::new(fix_group_registry(members)));
        let line = fix_packed_line(members);
        costs(
            &format!("a packed occurrence of {members} members"),
            each,
            || {
                black_box(
                    codec
                        .parse_ullink_line(black_box(&line))
                        .expect("a readable row"),
                );
            },
        );
    }
}

/// What the same three lines cost through the door that takes a decoded line.
///
/// One less than [`FIX_LINE_COSTS`] at every width, and three things move
/// inside it. Two allocations are saved: the page's own buffer and the
/// handle that counts it. A caller holding a [`TextLine`] already owns the
/// bytes as a range of a page it read them into, so the codec is handed
/// that page instead of making a second one - which is what the byte door
/// must do, because a bare slice is not a page and a message keeps ranges
/// of one. One allocation is paid: the source. A message read from a line
/// states that line's identity as the one element it was read from, and
/// the list holding it is the message's own; the byte door reads from no
/// element and states none.
///
/// All three are per message and not per pair, which is exactly right: a
/// page is one page and a source one source however many pairs the line
/// carries, so the slope is unchanged and only the constant moves. Three
/// widths again, so that the claim is the constant and not a number that
/// happens to be equal.
///
/// The arrival record's one shared allocation and the name table sized once
/// are in all three, as they are in [`FIX_LINE_COSTS`].
///
/// It moved with [`FIX_LINE_COSTS`], by the same four: 31 to 27 at four
/// pairs, 32 to 28 at sixteen and 33 to 29 at sixty-four; then by the
/// same one: 28, 29 and 30; and last by its displayed quantity at
/// sixty-four alone: 31; and with it by one at every width: 27, 28 and 30.
const FIX_TEXT_LINE_COSTS: [(usize, usize); 3] = [(4, 27), (16, 28), (64, 30)];

#[test]
fn a_message_read_from_a_decoded_line_does_not_pay_for_its_page_again() {
    crate::install::installed();
    let codec = FixCodec::new(Arc::new(fix_registry(64)));
    for ((pairs, each), (widest, bytes)) in FIX_TEXT_LINE_COSTS.iter().zip(FIX_LINE_COSTS) {
        assert_eq!(*pairs, widest, "the two pins measure the same widths");
        assert_eq!(
            *each + 1,
            bytes,
            "the page and its handle saved are the source paid and one more"
        );
        let held = fix_pairs_line(*pairs);
        // The page is made outside the counted closure because that is what a
        // caller reading text actually has: the decode already happened, and
        // what is measured here is what reading a message from it adds.
        let line = TextLine::from_bytes(
            0,
            TextBytes::from_bytes(&held).expect("a page"),
            std::sync::Arc::new(yggdryl::text::TextOptions::new()),
        )
        .unwrap();
        costs(
            &format!("a {pairs}-pair decoded line read as a message"),
            *each,
            || {
                black_box(
                    codec
                        .parse_text_line(black_box(&line))
                        .expect("a readable line"),
                );
            },
        );
    }
}

#[test]
fn a_fix_message_read_from_a_line_costs_what_its_pairs_cost() {
    crate::install::installed();
    let codec = FixCodec::new(Arc::new(fix_registry(64)));
    for (pairs, each) in FIX_LINE_COSTS {
        let line = fix_pairs_line(pairs);
        // The read is inside the counted closure, which is the whole point:
        // a message parsed outside one measures nothing about the parse.
        costs(
            &format!("a {pairs}-pair line read as a message"),
            each,
            || {
                black_box(
                    codec
                        .parse_fix_line(black_box(&line))
                        .expect("a readable line"),
                );
            },
        );
    }
}

/// A line carrying a `UTCTimeOnly` and a `TZTimeOnly` beside its integers
/// costs exactly what the same line carrying two more integers costs: the
/// FIX clock doors (`Time64::from_fix_text`, `DateTime64::from_fix_clock`)
/// read on the stack, the typed scalar is inline, and the restating at the
/// column's unit allocates nothing. The two dictionaries differ only in the
/// datatype and the `FIX:datatype` of two tags no market fact lifts, so the
/// comparison isolates the readers.
#[test]
fn a_fix_line_carrying_a_clock_and_a_time_of_day_costs_what_a_line_of_integers_costs() {
    crate::install::installed();
    let typed = |dtype: DataType, tag: i32, datatype: &str| {
        let mut field = dtype.nullable_field(format!("Typed{tag}"));
        FixFieldMut::new(&mut field)
            .set_tag(tag)
            .expect("a static tag");
        FixFieldMut::new(&mut field)
            .set_datatype(datatype)
            .expect("a FIX datatype name");
        field
    };
    let registry = |clocks: bool| {
        let mut msgtype = DataType::utf8().nullable_field("MsgType");
        FixFieldMut::new(&mut msgtype)
            .set_tag(35)
            .expect("a static tag");
        let (time, clock) = if clocks {
            (
                typed(
                    DataType::time64(TimeUnit::Nanosecond).expect("a clock"),
                    9_100,
                    "UTCTimeOnly",
                ),
                typed(
                    DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC).expect("an instant"),
                    9_101,
                    "TZTimeOnly",
                ),
            )
        } else {
            (
                typed(DataType::Int32, 9_100, "int"),
                typed(DataType::Int32, 9_101, "int"),
            )
        };
        let integers = (0..8).map(|index| typed(DataType::Int32, 1_100 + index, "int"));
        Arc::new(
            FixRegistry::from_fields([msgtype, time, clock].into_iter().chain(integers))
                .expect("a dictionary"),
        )
    };
    let line = |clocks: bool| {
        let value = if clocks { "093000" } else { "93000" };
        let mut line = b"35=D".to_vec();
        for index in 0..8 {
            line.extend_from_slice(format!("|{}={index}", 1_100 + index).as_bytes());
        }
        line.extend_from_slice(format!("|9100={value}|9101={value}|").as_bytes());
        line
    };
    let measure = |clocks: bool| {
        let codec = FixCodec::new(registry(clocks));
        let held = line(clocks);
        let (once, repeated) = counted_once_and_repeated(|| {
            black_box(
                codec
                    .parse_fix_line(black_box(&held))
                    .expect("a readable line"),
            );
        });
        assert_eq!(repeated, once * 1_000, "a read costs the same every time");
        once
    };
    assert_eq!(
        measure(true),
        measure(false),
        "a UTCTimeOnly and a TZTimeOnly cost what two integers cost"
    );
}

/// What a key costs the residual record: nothing of its own. Every pair
/// past `MsgType(35)` in [`fix_pairs_line`] is a dictionary field no column
/// of the fixed row represents, so each is filed in `fixentries` under its
/// `tag:name`; the keys are sorted on the stack and a key stated once is
/// filed as the one entry it is, so a row of fifteen such keys costs what a
/// row of three does - every key and value inside `SmolStr`'s inline width.
#[test]
fn a_residual_record_files_a_key_stated_once_at_no_cost_of_its_own() {
    crate::install::installed();
    let registry = Arc::new(fix_registry(64));
    let codec = FixCodec::new(Arc::clone(&registry))
        .try_with_default_sending_time(Some(
            Scalar::datetime64(
                1_704_190_530_000_000_000,
                TimeUnit::Nanosecond,
                Timezone::UTC,
            )
            .expect("a clock"),
        ))
        .expect("a fixed clock");
    let schema = yggdryl_fix::fix_schema(&registry, "fix").expect("the fixed schema");
    let at = schema.index_of("fixentries").expect("the residual column");
    let mut each = Vec::with_capacity(2);
    for pairs in [4, 16] {
        let message = codec
            .parse_fix_line(&fix_pairs_line(pairs))
            .expect("a readable line");
        let row = message.into_row(&schema).expect("the fixed row");
        let keys = row.as_sequence().expect("a row")[at]
            .as_mapping()
            .expect("the residual record")
            .len();
        assert!(
            keys >= pairs - 1,
            "{pairs} pairs filed {keys} residual keys"
        );
        let (once, repeated) = counted_each(
            || message.clone(),
            |held| held.into_row(&schema).expect("the fixed row"),
        );
        assert_eq!(
            repeated,
            once * 64,
            "{pairs} pairs: a row costs {once} per call"
        );
        each.push(once);
    }
    assert_eq!(
        each[0], each[1],
        "a row filing three residual keys and one filing fifteen"
    );
}

/// The bridge's own capture, the corpus `rust/fix/tests/root/ulbridge.rs` reads
/// whole and the `fix/ulbridge` benchmark times.
const ULBRIDGE: &[u8] = include_bytes!("../../tests/support/ulbridge.log");

/// One line of the capture past its row header: what the text reader hands
/// the codec as the body.
fn capture_body(index: usize) -> Vec<u8> {
    let line = ULBRIDGE
        .split(|byte| *byte == b'\n')
        .nth(index)
        .expect("a line of the capture");
    let at = line
        .windows(2)
        .position(|pair| pair == b") ")
        .expect("the row header's end")
        + 2;
    line[at..].to_vec()
}

/// What one line costs at each stage of the pipeline, per message.
#[derive(Debug, PartialEq, Eq)]
struct StageCosts {
    /// The codec over the body: the message built, settled and identified.
    parse: usize,
    /// A fresh clone read against the fixed schema.
    into_row: usize,
    /// The row landed as a one-row `Serie` under the root, its projection
    /// warm.
    landing: usize,
    /// The landed `Serie` built into a `RecordBatch`.
    batch: usize,
    /// The arrival record's hash, derived on a fresh clone.
    digest: usize,
    /// The walk over one fresh clone, chained to nothing.
    lifecycle: usize,
}

/// Three real lines of the bridge's capture through every stage of the
/// pipeline, on the committed dictionary: a bridge row of named keys
/// (`bridge_pipe`, line 1), a frame spelled with `|` (`frame_pipe`, line
/// 72) and the `35=UL` frame packing a group inside a group
/// (`frame_packed`, line 111). [`FIX_LINE_COSTS`] pins the slope per pair
/// over a synthetic dictionary; these pin the constant a real shape pays
/// at each stage, so a change that moves one stage is read at that stage.
///
/// Every stage runs over what the one before it made, set up outside the
/// count: the clone a stage takes by value is the caller's. The landing is
/// on a root whose projection is warm, because the root is a fact of the
/// schema and the first landing under it fills every level's cache; the
/// cold landing is the warm one plus that projection, which
/// [`projecting_a_root_projects_every_level_below_it_into_its_own_cache`]
/// in `rust/tests/root/field.rs` pins. A bridge row's plan is found again
/// by its shape once the alias children it makes share their metadata, so
/// its parse is as linear as a frame's. Each landing fell by four when the
/// `state` column became the `int32` code of its member: one primitive
/// buffer where a text column built its offsets and its bytes; and by four
/// again when the `side` column did the same. Every parse fell when the
/// derivations and the retired fields' restatements became native code, no
/// rule compiled or bound per registry (718 to 559, 254 to 206, 1471 to
/// 1005, and a bridge row's walk 38 to 36, which enriches again). The
/// residual record became one `map<utf8, utf8>` and its counter column
/// went: the landing fell by about seventy and the batch by thirteen, fewer
/// arrays than three levels of entry structs; `into_row` now renders a
/// group's JSON, which allocates by contract - the packed frame's nested
/// groups 174 to 244 - while a flat row's map is about what its list cost
/// (83 to 78 with the unresolved keys moved into `metadata`, 55 to 58). The
/// split at the parse, the six bid and ask facts and the token aliases
/// (A12-A22) moved every parse up (559 to 653, 206 to 254, 1005 to 1036),
/// each landing by seven and each batch by one with the six new market
/// columns, and a bridge row's walk to 43. Reading the four word aliases on
/// the stack, where the token lookup cut a name into a `Vec` of words and
/// spelled each alternative as a `String`, took 24, 24 and 8 back off the
/// parse (629, 230, 1028) and 8 off the bridge row's walk (35); and a key no
/// dictionary resolves that a row states empty is kept in `metadata` as
/// `{}` rather than dropped, the bridge row's `into_row` 78 to 79. A
/// message's table of names came to be shared by its clones through one
/// `Arc`: a parse building one pays that `Arc` (1028 to 1029), a clone read
/// or walked no longer builds its own (`into_row` 79 to 77, 58 to 57 and
/// 244 to 243), and a bridge row's parse, whose split execution shares its
/// report's table, fell by one net (629 to 628). Each walk pays one table
/// for the identities its deduplication window keeps and, walking a clone,
/// no longer pays a table of names: a frame's walk stands at 7; and a bridge
/// row's fell from 35 to 8 when redating a message a parse built settled
/// what its clock moved alone. A trade side stating no `Side(54)` came to
/// split off an execution of side `UKNW` where it noted an anomaly: the
/// packed frame's parse 1029 to 1132, that execution's 106 less the
/// anomaly's 3. Reading the accounts off the parties at every settle took
/// the bridge row to 641 and the packed frame to 1145, thirteen each for
/// six and seven parties per message; the frame's regulatory `TVTIC`, a
/// fifth alternate identifier growing its map at each of three settles,
/// took it to 233. Reading the accounts level by level - a trade side's
/// parties, then the message's - rather than gathered into one list that
/// spilled past its eight inline occurrences took two back off the packed
/// frame at its two settles (1143); its `Account(1)` costs these lines
/// nothing. An execution split off a report came to be chained under its
/// `ExecID(17)` as given rather than `ExecID=` before it: the code is the
/// identifier copied once where `format!` reserved twice its seven-byte
/// literal and grew once for the sixteen-byte identifier, so the bridge
/// row's parse fell to 640 and the frame's to 232.
///
/// The row gained twenty-five columns when every generated schema came to
/// open with the element, event, market and operation facts - the market
/// data type, the stop price, the displayed, hidden and cancelled
/// quantities, the ordered quantity and the rest - 133 columns to 158: each
/// landing rose by about eleven a column (1381 to 1663, 1362 to 1642, 1398
/// to 1682), what a nullable leaf column costs a one-row landing, and each
/// batch by forty-five (190 to 235). The parse and the row moved with the
/// facts a message now settles and states: the type its typing field reads
/// as, the quantities its state implies and the boxed record of rarely
/// stated quantities, against a time in force held as a one-byte member
/// rather than text - the bridge row's parse 640 to 629 and its row 77 to
/// 82, a frame's 232 to 239 and 57 to 60, the packed frame's 1143 to 1148
/// and 243 to 246.
///
/// The bridge row's parse fell to 623 when the instrument key a bridge's
/// `*INSTRUMENTID` names came to be read into its three typed parts - inline
/// codes - where it copied each part into a `String` at each of the three
/// facts that read it, less what a ticker of an identifier's own shape
/// derives; a frame's parse rose to 241 and its walk to 8 with the RIC its
/// ticker is, derived and carried along its chain.
///
/// The identifier maps became one sorted vector of identifiers each -
/// `secaltids`, `altids`, `parties` - typed, sourced and valued apart.
/// Each parse fell where an entry held its key and its value packed in one
/// buffer past `SmolStr`'s inline width (24 for the bridge row's alternate
/// identifiers and 6 for its accounts), a type and a source being static
/// words and most values inline; it rose by one per settle for a set of one
/// or two identifiers, which takes its one backing where two inline slots
/// held it, and by the parties now kept apart under their role and source:
/// the bridge row's two parties sourced by a description the code set does
/// not resolve - typed by that spelling - stand beside the proprietary ones
/// of their role, where a map keyed by role kept the first. The bridge
/// row's parse is 602, a frame's 237, the packed frame's 1150. The row moved by what `SecurityID(48)`,
/// `SecurityIDSource(22)` and `Parties(453)` cost once they left its
/// columns for `fixentries` - a group's JSON allocating by contract, about
/// seventy for the bridge row's eight parties and thirty-five for a frame's
/// three - and by one record per identifier where a map held two texts
/// (twenty-six for the bridge row, eleven for a frame): `into_row` 82 to
/// 175, 60 to 106, 246 to 273. With four columns fewer and the identifier
/// sets laid out as a list of five-text records rather than maps, each
/// landing fell by about seventy-three (1590, 1571, 1608) and each batch by
/// seventeen (218). A walk no longer copies the instrument's identifiers to
/// learn and fill them, which took each walk to 7. The sets then became
/// `securityids`, `identifiers` and `partyids`, each identifier a `src`,
/// `type` and `value` of lower-case words - the members of two enums where
/// they are named, an inline word where they are not - and moved none of
/// these counts.
///
/// An identifier then lost its `parent` and `orig`, each set came to be laid
/// out as a map from the key `src:type` to the identifier, and the stored
/// cross code took its `{kind}:{side}:{base}` prefix. Each batch rose by six
/// to 224: an identifier column is a map node and its entries struct (four)
/// over the key text (one) and the identifier struct (two) of three texts
/// (three), ten arrays where a list node (one) over a struct (two) of five
/// texts (five) was eight. A frame's parse rose to 241 by the six times its
/// two messages spell the stored code - as it is set, then as the side and
/// the category it is stored under land - each one allocation at its exact
/// length, less the two the sided code cost before; the packed frame's to
/// 1153 by its three spellings. A frame's row rose to 109 by its three keys
/// past `SmolStr`'s inline width, one allocation each, the map's entries
/// costing a set what the list did; the packed frame's to 279 by its five
/// such keys and the party `client:clientid` its unmapped `client.clientid`
/// entry now states. The bridge row's nine crate fields went (65051 to
/// 65060), taking 34 off its parse: the restatement of those fields (27),
/// the entries its digest rendered for them (6), two typed translations and
/// two where its identifiers are read from the unmapped entries rather than
/// built from those fields, less the three a key resolved through a field
/// path costs; it gained four for its six spellings of the stored code over
/// the two before and nine for the parents filled at each of its three
/// settles - the set copied, the fills gathered, one insert grown. Its row
/// lost the `SecAltIDGrp(454)` entry the two instrument keys among those
/// fields made and their own nine entries, Username(553)'s taking one back
/// (34), and gained five identifier rows - nine read from unmapped entries
/// for the four those fields held - and six keys past the inline width: 175
/// to 152. An alias stating another
/// value than the field it lost to came to be kept in the message's
/// metadata beside an anomaly, and the bridge row's `OMSDealerAccount`,
/// `ULTraderClOrdID`, `MarketOrderID` and `OMSDealerOrderID` do: fourteen
/// to its parse - each anomaly's reason formatted and copied into its
/// string (eight), the alias that had agreed before both lost (one), the
/// anomaly list past eight (one), and the metadata past eleven keys, two
/// more B-tree nodes where the parse builds it and two where the split
/// execution clones it - so the parse stands at 595. `ParentClOrdID` came to
/// reach `OrigClOrdID(41)` as its other spelling, so the bridge row lands
/// that column's sixteen bytes where it landed a validity bitmap and its
/// buffer: 1589. Splitting a trade into its sides cuts each side out of
/// the sides group, and the whole serie is the serie: the packed frame's
/// one side is that group itself, where the cut copied it, so its parse
/// fell by that copy to 1152.
///
/// A decimal came to read and to write its text with no heap string built on
/// the way - the coefficient's spelling joined on the stack, the pointed text
/// written straight to its sink - and nothing else moved: each parse fell by
/// the allocations the decimals it reads used to cost, ten for the bridge row
/// (585), seven for a frame (234) and nineteen for the packed frame (1134),
/// and each digest, which renders the decimals it holds, by eight (16), eight
/// (16) and six (10).
///
/// Every decimal then came to write one text, the shortest that states it,
/// built on the stack whichever leaf holds it - the fixed leaves' text had
/// built a heap string and trimmed it, and the wire and the digest spelled
/// each `decimal128(38, 18)` field through it - and a float came to be read
/// off a spelling on the stack: each digest renders its decimals with no
/// allocation and stands at one, and each parse fell by the allocations its
/// decimal spellings used to cost, four for the bridge row (581), two for a
/// frame (232) and twenty for the packed frame (1114).
///
/// Each identifier set then became one sorted `map<utf8, utf8>` from the key
/// (a base key spelled as its type alone, a key of two words the crate
/// names one static string) to the value, and every type a named source
/// states came to hold its base key too. The bridge row measured a parse of
/// 596 and a row of 151 before it, where this pin stated 595 and 152. Each
/// batch fell by twelve to 212: an identifier column is a map node and its
/// entries struct (four) over two texts (two), six arrays where ten were.
/// Each landing fell by sixty-nine with the layout - twenty-three a column,
/// the identifier struct and its three texts gone - and by two more for
/// each set it lands, its entries three allocations where they were five:
/// 1515, 1496 and 1533, the bridge row's eighteen party keys growing their
/// text once more. `into_row` lost the record each identifier was and every
/// key spelled past `SmolStr`'s inline width, a key of two words the crate
/// names being a static string and only a key naming a source it does not -
/// `omsdealer:account` among them - spelled: 151 to 116 for the bridge row's
/// thirty identifiers, thirty records and eight texts past the inline width
/// where its thirty-seven entries now spell three, 109 to 95 for a frame's
/// eleven (eleven and three, none now) and 279 to 262 for the packed
/// frame's thirteen (thirteen and five, one now). The parses moved by the
/// sets the rule
/// keeps: a frame's rose to 245, three for the base keys its three sourced
/// parties fill growing its party map at its three settles and one for the
/// base keys its two derivations fill growing its security identifiers;
/// the bridge row's fell to 592, nine for the parent fills its
/// `ParentOrderID` keys no longer make at its three settles - the base
/// `orderid` is the wire's `OrderID(37)` - against three for its sourced
/// parties' base keys, one for its security identifiers' and one for the
/// `DETAILEDCFICODE` it states, now a name of `CFICode(461)` folded where
/// the message is built; the packed frame's did not move. Each walk rose to
/// 8: its instrument registry takes its own table on its first learn, the
/// empty registry sharing one static table until then.
///
/// A fill's report and the execution split off it then came to settle by
/// what the split recorded alone - the category, the state, the chain, the
/// sources - which moves no field, so neither rebuilds the identifier maps
/// a whole settle rebuilt off the fields its parse had already read. The
/// bridge row's parse fell by eighteen to 574, the nine that rebuild cost
/// at each of its two settles after the first: its identifier set grown
/// twice (to four, then eight) by what its fields state and once past
/// eight by the keys its unmapped entries name; its party set grown three
/// times (to four, eight and sixteen) by the fifteen its group and its
/// `Account(1)` state and once past sixteen by its unmapped entries; and
/// the two occurrences stating the source
/// `generallyacceptedmarketparticipantidentifier`, a text past `SmolStr`'s
/// inline width, read again. A frame's fell by eight to 237, the four at
/// each: its five identifiers grown twice, the fifth a regulatory trade
/// identifier, and its five parties twice. The packed frame is a trade,
/// whose sides settle whole, and did not move.
///
/// A map whose keys stand in strictly ascending order then came to prove
/// them distinct in the one pass that proves them sorted, and a map past
/// sixteen entries no longer builds the set its duplicate search held. A
/// map is checked where its entries are gathered and again where its row is
/// canonicalized, a map built in the metadata or residual's own order at
/// both and a party map at the second alone, and a landing checks each once
/// more. The bridge row's `into_row` fell by five to 111, its metadata's
/// thirty-three keys and its residual's eighteen twice each and its eighteen
/// parties once, and its landing by three to 1512; a frame's fell by four
/// to 91, its metadata's seventeen keys and its residual's twenty-two twice
/// each, and by two to 1494; the packed frame's by two to 260, its
/// metadata's eighteen keys twice, and by one to 1532.
///
/// A boolean's text then came to be read as a column cast reads it, so the
/// packed frame's `ManualOrderIndicator(1028)=no` types as false where it
/// was refused: its parse fell by sixteen to 1137, the located error, the
/// anomaly and the warning the refusal built, and its `into_row` rose by
/// three to 263, the entry the typed flag now adds to the row's
/// `fixentries`.
///
/// A row then came to hold each arrival once: a key no dictionary resolved
/// that an identifier map holds with its value rides the residual record
/// under `0:key` and leaves the `metadata` cell, which is built sorted in
/// one pass rather than by cloning the message's map and inserting each
/// unresolved name into the clone. A frame's `into_row` fell by two to 89 -
/// the three nodes its seventeen names split the cloned map into and the
/// one its cell collected from them, against the one vector the cell now
/// fills and the one its map keeps - and captured nothing; the packed
/// frame's fell by one to 262, the same two saved against the one vector
/// its one captured key adds to the residual's keyed tree; the bridge
/// row's rose by nine to 120: ten vectors for the ten keys captured out of
/// its metadata's thirty-three into its residual's eighteen and two for the
/// one key past the inline width - `0:omsdealerparentorderid` - against
/// three fewer between the residual's tree growing a node for them and the
/// metadata's clone and its splits going: ten and two, less three. Each walk
/// rose by two to 10 when the instrument registry gained its ticker index:
/// the index shares one static empty table until a walk's first learn
/// lists a ticker, which takes the index's own Arc and its own table beside
/// the rows'.
///
/// A decimal's text then came to be read with no string of its own: the
/// strict reader's `format!` of the sign, the whole and the fraction into
/// one digit string went with its fold into the one decimal reader, so each
/// parse fell by one for every decimal text it reads - eight for the bridge
/// row, six for a frame, nine for the packed frame - and no other stage
/// moved.
///
/// The two lines of history then met: the decimal reading above was the
/// one this file's earlier paragraph already counted from the other side,
/// so the merged parse is each side's fall summed less that one saving -
/// 595 less fourteen and twenty-nine plus eight is the bridge row's 560,
/// 241 less nine and ten plus six a frame's 228, 1153 less forty and
/// twenty-five plus nine the packed frame's 1097 - and every digest stands
/// at the one its stack-built decimal text leaves, every other stage as
/// this line left it.
///
/// A key stated once then came to be filed in the residual record as the
/// one entry it is, the keys sorted on the stack rather than grouped in a
/// tree of vectors: each `into_row` fell by its record's keys and the
/// tree's nodes, and no other stage moved - the bridge row's twenty-eight
/// keys and four nodes to 88, a frame's twenty-two and three to 64, the
/// packed frame's eleven and one to 250.
///
/// A group then became its list alone, no counter beside it at any level.
/// Each parse fell by the list of every child's tag and counter the
/// builder's finish collected to restate a counter: one each, and three
/// more for the bridge row, whose `NoPartyIDs` stated two beside the eight
/// occurrences the group held, for the anomaly that disagreement built.
/// Each fell by the list of counters every entries walk collected at
/// a level holding groups to leave a counter's child out: one for the bridge
/// row, four for a frame and eleven for the packed frame. The packed frame
/// fell by sixty-two more, the one write per split side restating
/// `NoSides(552)=1` beside the side's group, and by one where its root's six
/// groups no longer spilled the four-wide list of counters the member walk
/// kept; the bridge row rose by one, the builder holding three slots fewer
/// and crossing one growth boundary the other way - what the former reading
/// paid for this line with any one of its three named counters dropped. So
/// the bridge row's parse fell by four to 556, a frame's by five to 223,
/// the packed frame's by seventy-five to 1022. Each landing fell by what the
/// fixed row's two counter columns, `notrdregtimestamps` and
/// `noregulatorytradeids`, cost to lay out - ten for the bridge row, twelve
/// for each frame - and each batch by the two arrays it no longer gathers,
/// to 210; no `into_row` moved.
///
/// A place then came to be never absent: each of these messages is the
/// first at its instant, and its `seqnum` cell states zero where it was a
/// null. A column holding no null lands with no validity beside its values,
/// so each landing fell by the two that validity cost -
/// [`a_null_cell_costs_the_validity_a_stated_one_does_not`] pins the pair on
/// its own - the bridge row's to 1500, a frame's to 1480, the packed
/// frame's to 1518, and no other stage moved.
///
/// A datetime then came to be read by the datetime's own reader, and no
/// text is rendered for it: the ISO spelling the codec restated each wire
/// timestamp as - built in a string builder, finished as a shared string
/// and copied into the value the contract then read - went with the
/// rewrite, so each parse fell by what each of its datetime fields paid for
/// a rendering past the twenty-three bytes a string holds inline: three for
/// a reading of milliseconds under a UTC column - the builder's one spill
/// to the heap, the string it finished as and the value's own - and four
/// for one of microseconds, whose builder grew once more to take the `Z`.
/// The bridge row states three digit runs of milliseconds and one of
/// microseconds, thirteen, to 543; a frame a `SendingTime(52)` of
/// milliseconds and a `TransactTime(60)` and an `ExpireTime(126)` of
/// microseconds, eleven, to 212; the packed frame two digit runs of
/// milliseconds, six, to 1016, its two readings of whole seconds having
/// rendered inline. A reading builds no refusal either - a digit run is
/// read before the general reader is asked, where asking first cost two
/// for the refusal it then dropped - so a datetime field costs a parse
/// nothing, and no other stage moved.
///
/// The strike then came to be a market fact, `strikepx`, a column of the
/// fixed row's shared prefix none of these messages states: each landing
/// rose by the seven a nullable decimal column holding a null costs to lay
/// out - its values, its validity and the array around them - to 1507, 1487
/// and 1525, and each batch by the one array it gathers more, to 211; no
/// other stage moved.
/// Main also adds the required `msgpluginside` column (65043), whose
/// five landing allocations combine with strike's seven: 1512, 1492 and
/// 1530. Both added arrays make each batch 212; no per-row stage moved.
///
/// The origin currency then came to be a market fact, `origccy` (65018), a
/// column of the fixed row's shared prefix none of these messages states:
/// each landing rose by the eleven a nullable `ccy` column holding a null
/// costs to lay out - eleven over a lone column's landing, where a
/// nullable decimal holding a null costs the seven above - to 1523, 1503
/// and 1541, and each batch by the one array it gathers more, to 213. And
/// the instrument registry gained its lookup-code index beside its ticker
/// index: each walk's first learn takes the index's own `Arc` off the
/// static empty one every registry shares - one more on every walk - and
/// its own table where the row it learns holds a code a lookup reads. The
/// bridge row's and a frame's Swiss ISINs embed a Valor, so their walks
/// rose by two to 12; the packed frame's `EZ` ISIN embeds none, and its
/// walk rose by one to 11. No other stage moved.
///
/// Every FIX value then came to be read by its type's own door - a
/// `TZTimeOnly` stating no zone as the wall clock in its column's zone, a
/// bridge's `Aggressor` as the flag `Y` is, a word no code of its set spells
/// refused naming the set rather than the integer it is not - and only the
/// parse and the packed frame's row moved, each value measured against the
/// same line with that one value spelled as both readings type it. The
/// packed frame's two `AGGRESSORINDICATOR=Aggressor` and its
/// `LEGMATURITYTIME=093000` type now, and the refusals they cost went -
/// fifteen, sixteen and eighteen - and what the line's refusals cost
/// together past their sum fell by one, four to three; its four words
/// still refused (`TraderName`,
/// `publishername`, `UniqueTransactionIDLeg`, `UniqueTransactionIDHedge`)
/// cost one more each, the refusal naming the set where it named the
/// integer, but the last, whose cost stands: 1016 less fifty plus three is
/// 969. The bridge row's four words still refused cost the same one more
/// each, two for `orderoriginatorsystem`, as each measures on a line of
/// that one pair: 543 to 548. The packed frame's row rose by the one its
/// side's typed `AggressorIndicator(1057)` costs - the former reading's row
/// of the same line with that value spelled `Y` stood at 251 too - to 251.
/// A frame's line states none of these values, and no other stage moved.
///
/// The `parentclordid` spelling of `OrigClOrdID(41)` then retired, a
/// bridge's `PARENTCLORDID` a word naming no field: the bridge row's
/// `OrigClOrdID(41)` column holds a null where it held that key's value, so
/// its landing lays out the validity a null cell costs and a stated one
/// does not ([`a_null_cell_costs_the_validity_a_stated_one_does_not`]), one
/// more, to 1524 - the same line with the key spelled `ORIGCLORDID` lands at
/// 1523. The walk filing the chain identities alone moved no lifecycle: the
/// three lines are terminal, so the walk retires each and files no name,
/// and 12, 12 and 11 stand. No other stage moved.
///
/// [`projecting_a_root_projects_every_level_below_it_into_its_own_cache`]: ../root/field.rs
const FIX_PIPELINE_COSTS: [(&str, usize, StageCosts); 3] = [
    (
        "bridge_pipe",
        1,
        StageCosts {
            parse: 548,
            into_row: 88,
            landing: 1524,
            batch: 213,
            digest: 1,
            lifecycle: 12,
        },
    ),
    (
        "frame_pipe",
        72,
        StageCosts {
            parse: 212,
            into_row: 64,
            landing: 1503,
            batch: 213,
            digest: 1,
            lifecycle: 12,
        },
    ),
    (
        "frame_packed",
        111,
        StageCosts {
            parse: 969,
            into_row: 251,
            landing: 1541,
            batch: 213,
            digest: 1,
            lifecycle: 11,
        },
    ),
];

#[test]
fn a_real_line_costs_the_same_at_every_stage_every_time() {
    crate::install::installed();
    let registry = Arc::new(committed_registry().clone());
    let codec = FixCodec::new(Arc::clone(&registry))
        .with_threads(1)
        .with_exclude_msgtypes::<[&str; 0], &str>([])
        .try_with_default_sending_time(Some(
            Scalar::datetime64(
                1_704_190_530_000_000_000,
                TimeUnit::Nanosecond,
                Timezone::UTC,
            )
            .expect("a clock"),
        ))
        .expect("a fixed clock");
    let schema = yggdryl_fix::fix_schema(&registry, "fix").expect("the fixed schema");
    let root = Arc::new(schema.clone());
    let mut measured = Vec::with_capacity(FIX_PIPELINE_COSTS.len());
    for (name, index, _) in &FIX_PIPELINE_COSTS {
        let body = capture_body(*index);
        let parse = || -> Vec<FixMsg> {
            codec
                .parse_line(black_box(&body))
                .expect("a readable line")
                .collect::<yggdryl::Result<Vec<_>>>()
                .expect("the line's messages")
        };
        let message = parse().swap_remove(0);
        let row = message.into_row(&schema).expect("the fixed row");
        let serie = Serie::from_scalars(Arc::clone(&root), [row.clone()]).expect("the row lands");
        let each = |what: &str, (once, repeated): (usize, usize)| {
            assert_eq!(
                repeated,
                once * 64,
                "{name}/{what} did not cost {once} per call over sixty-four"
            );
            once
        };
        measured.push((
            *name,
            *index,
            StageCosts {
                parse: each("parse", counted_each(|| (), |()| parse())),
                into_row: each(
                    "into_row",
                    counted_each(
                        || message.clone(),
                        |held| held.into_row(&schema).expect("the fixed row"),
                    ),
                ),
                landing: each(
                    "landing",
                    counted_each(
                        || row.clone(),
                        |held| Serie::from_scalars(Arc::clone(&root), [held]).expect("a serie"),
                    ),
                ),
                batch: each(
                    "batch",
                    counted_each(
                        || serie.clone(),
                        |held| held.into_arrow_batch().expect("a batch"),
                    ),
                ),
                digest: each(
                    "digest",
                    counted_each(|| message.clone(), |held| held.digest()),
                ),
                lifecycle: each(
                    "lifecycle",
                    counted_each(
                        || vec![message.clone()],
                        |held| codec.lifecycle(held).filter(Result::is_ok).count(),
                    ),
                ),
            },
        ));
    }
    assert_eq!(measured, FIX_PIPELINE_COSTS, "the pipeline's stage costs");
}

/// The committed dictionary, read once for the tests that start from it.
fn committed_registry() -> &'static FixRegistry {
    static REGISTRY: std::sync::OnceLock<FixRegistry> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config/fix");
        let folder = yggdryl::local::LocalFolder::new(root).expect("the local seed path");
        FixRegistry::from_handle(&folder).expect("the committed dictionary loads")
    })
}

#[test]
fn a_word_alias_lookup_allocates_nothing() {
    crate::install::installed();
    // The words are read on the stack: a spelling reached through them, and
    // one no spelling reaches, cost no allocation at any dictionary size.
    let registry = committed_registry();
    let (allocations, found) = counted(|| {
        (
            registry
                .get_field_by_name(black_box("AskPrice"))
                .map(Field::name),
            registry
                .get_field_by_name(black_box("DemandQty"))
                .map(Field::name),
            registry
                .get_field_by_name(black_box("HedgePriceQty"))
                .is_none(),
        )
    });
    assert_eq!(found, (Some("offerpx"), Some("bidsize"), true));
    assert_eq!(allocations, 0);
}

/// The crate's derivations are native: no registry compiles or keeps a plan
/// of them, so a report implying five fields costs every parse the same, and
/// what the first parse warms is the dictionary's own memo, never a rule.
#[test]
fn the_native_derivations_cost_every_parse_the_same() {
    crate::install::installed();
    let codec = FixCodec::new(Arc::new(committed_registry().clone()));
    let line = b"8=FIX.4.4|35=8|37=A|15=CHF|38=100|14=20|39=1|32=20|31=2.5|48=US0378331005|22=4|150=F|10=0|";
    let parse = || {
        codec
            .parse_line(black_box(line))
            .and_then(|mut messages| messages.next().expect("one frame"))
            .expect("a readable report")
    };
    let held = parse();
    for (tag, implied) in [(151, "80"), (381, "50")] {
        let value = held.get_by_tag(tag).expect("the report implies it");
        let implied = yggdryl::Decimal::parse(implied).expect("an exact number");
        assert_eq!(value, Scalar::from(implied), "tag {tag}");
    }
    for (tag, implied) in [(470, "US"), (2897, "6"), (120, "CHF")] {
        let value = held.get_by_tag(tag).expect("the report implies it");
        assert_eq!(value.as_str(), Some(implied), "tag {tag}");
    }
    let (once, repeated) = counted_once_and_repeated(|| {
        black_box(parse());
    });
    assert_eq!(
        repeated,
        once * 1_000,
        "a parse keeps nothing per message: {once} once, {repeated} over a thousand"
    );
    eprintln!("native_derivations: {once} allocations per parse");
}

/// A FIX message the walk re-keys onto its chain is settled once: over
/// followers of a `D` stored `10:1:A1`, a sided report stored `10:1:O1`
/// costs two allocations - the code's text and the one settle, which a
/// message's finalize costs alone - and a side-less cancel reject stored
/// `10:0:O1` eighteen, the side written as `Side(54)` through the provided
/// `follow_identity` before the code and the settle; a report already
/// holding the chain's side and code costs nothing and moves nothing. Read
/// off the run at 8 and 1024 followers. The side-less figure is a
/// `FixMsg`'s own `follow_identity`: the lifecycle's message, crate-private,
/// writes the side its own way.
#[cfg(feature = "internals")]
#[test]
fn a_rekey_of_a_fix_message_is_one_settle() {
    crate::install::installed();
    use yggdryl_market::internals::graph_iterator::rekeyed;

    let codec = FixCodec::new(Arc::new(committed_registry().clone())).with_threads(1);
    let parse = |line: &[u8]| -> FixMsg {
        codec
            .parse_line(line)
            .expect("a readable line")
            .next()
            .expect("one message")
            .expect("a message")
    };
    let live = parse(
        b"8=FIX.4.4|35=D|49=B|56=S|34=1|52=20260921-10:00:00|11=A1|55=AAPL|54=1|38=10|44=100|10=0|",
    );
    assert_eq!(live.get_crosscode(), "10:1:A1");
    let identity = live.get_crossuuid();
    let cases: [(&str, &[u8], &str, usize, bool); 3] = [
        (
            "a sided report under another code",
            b"8=FIX.4.4|35=8|49=S|56=B|34=2|52=20260921-10:00:01|37=O1|17=E1|150=0|39=0|54=1|55=AAPL|10=0|",
            "10:1:O1",
            2,
            true,
        ),
        (
            "a side-less cancel reject",
            b"8=FIX.4.4|35=9|49=S|56=B|34=3|52=20260921-10:00:02|11=C2|37=O1|41=A1|39=8|434=1|10=0|",
            "10:0:O1",
            18,
            true,
        ),
        (
            "a report holding the chain's side and code",
            b"8=FIX.4.4|35=8|49=S|56=B|34=4|52=20260921-10:00:03|11=A1|17=E2|150=0|39=0|54=1|55=AAPL|10=0|",
            "10:1:A1",
            0,
            false,
        ),
    ];
    for followers in [8, 1024] {
        for (what, line, parsed, each, moves) in cases {
            let message = parse(line);
            assert_eq!(message.get_crosscode(), parsed, "{what}");
            let mut held: Vec<FixMsg> = (0..followers).map(|_| message.clone()).collect();
            let (allocations, moved) = counted(|| {
                held.iter_mut()
                    .map(|message| rekeyed(message, &live, identity))
                    .filter(|moved| *moved)
                    .count()
            });
            assert_eq!(moved, if moves { followers } else { 0 }, "{what}");
            assert_eq!(
                allocations,
                each * followers,
                "{what}: {allocations} over {followers} followers"
            );
            assert!(
                held.iter()
                    .all(|message| message.get_crosscode() == "10:1:A1"
                        && message.get_crossuuid() == identity),
                "{what}: every follower stands under the chain"
            );
        }
    }
}

/// The text options a capture's copies are read under, as the scale
/// harness reads them: the bridge's row header, each line numbered and
/// classified, and its clock - which a rendered copy writes in UTC - read
/// in UTC.
fn capture_reading() -> TextOptions {
    let mut options = TextOptions::new()
        .try_with_rowheader(yggdryl_fix::ULBRIDGE_ROWHEADER)
        .expect("the row header compiles")
        .with_timezone(Timezone::UTC);
    options.start_rownum = Some(1);
    options.parse_mimetype = true;
    options
}

/// Every line of `capture`, as the text reader decodes it.
fn capture_lines(capture: &[u8]) -> Vec<TextLine> {
    let source = Buffer::from_bytes(capture.to_vec())
        .with_media_type(MediaType::from_file_name("ulbridge.log"));
    read_text_lines(&source, &capture_reading())
        .expect("a line reader")
        .map(|line| line.expect("a line"))
        .collect()
}

/// `copies` copies of the capture, each with its identifiers stepped and
/// its clocks moved by the copy, every one a second after the one before
/// inside one span as the scale run stacks them: no copy repeats another,
/// so each is chains of its own.
fn rendered_capture(copies: u64) -> Vec<u8> {
    let template = ulbridge::Template::new(copies);
    let mut rendered = Vec::new();
    for copy in 0..copies {
        template.render(copy, &mut rendered);
    }
    rendered
}

/// The codec the capture's memory pins read on: the committed dictionary
/// under the codec's own refusals, the row header's captures by name, and
/// one thread, because the counter observes the thread it is armed on.
fn capture_codec() -> FixCodec {
    FixCodec::new(Arc::new(committed_registry().clone()))
        .with_threads(1)
        .with_capture_names(capture_reading().capture_names())
}

/// What a parsed message keeps allocated while it is held: the bytes a
/// `Vec<FixMsg>` the line door parsed still holds - its own buffer, each
/// message's heap and the page its keys and values are ranges of - over the
/// capture and over two rendered copies of it. A message's bytes are its
/// own, so a second copy holds as many again; this is the figure step 0 of
/// `.design/bench/OPTIMIZATION-DESIGN.md` measures a message by.
#[test]
fn a_parsed_fix_message_holds_the_same_bytes_at_one_and_two_captures() {
    crate::install::installed();
    let codec = capture_codec();
    let bodies = |capture: &[u8]| -> Vec<Vec<u8>> {
        capture_lines(capture)
            .iter()
            .map(|line| line.body_bytes().to_vec())
            .collect()
    };
    let parse = |bodies: &[Vec<u8>]| {
        let mut messages: Vec<FixMsg> = codec
            .parse_lines(bodies)
            .collect::<yggdryl::Result<_>>()
            .expect("every line reads");
        messages.shrink_to_fit();
        messages
    };
    let one = bodies(ULBRIDGE);
    let two = bodies(&rendered_capture(2));
    // Each once outside every count, so the dictionary's memo and every
    // process-wide first use are no message's: warmed by the capture alone,
    // its next parse still kept some sixty-four kilobytes no later one does.
    drop(parse(&one));
    drop(parse(&two));
    let held = |bodies: &[Vec<u8>]| {
        let (bytes, messages) = retained(|| parse(bodies));
        let bytes = usize::try_from(bytes).expect("a parse keeps what it built");
        (bytes, messages.len())
    };
    let (one_bytes, one_messages) = held(&one);
    let (two_bytes, two_messages) = held(&two);
    assert_eq!(one_messages, 79 + 57, "the capture's messages");
    assert_eq!(
        two_messages,
        2 * one_messages,
        "two copies, twice the messages"
    );
    let per_message = |bytes: usize, messages: usize| bytes as f64 / messages as f64;
    let (one_each, two_each) = (
        per_message(one_bytes, one_messages),
        per_message(two_bytes, two_messages),
    );
    eprintln!(
        "parsed_fix_message: one capture {one_bytes} bytes over {one_messages} messages \
         ({one_each:.0} each), two copies {two_bytes} bytes over {two_messages} messages \
         ({two_each:.0} each); a FixMsg is {} bytes inline",
        std::mem::size_of::<FixMsg>()
    );
    assert!(
        (two_each - one_each).abs() <= one_each * 0.02,
        "a message holds {one_each:.0} bytes at one capture and {two_each:.0} at two"
    );
}

/// What the sorted lifecycle holds beyond its input while it walks copies
/// that repeat nothing: every copy's chains live at once, a second apart
/// inside one span, so the walk's live set - the hour it holds, the live
/// message of every chain, the identities its deduplication keeps - grows
/// with the copies, and per input message it stays where it was. The input
/// is parsed outside the walk's count and measured on its own, the way
/// [`a_parsed_fix_message_holds_the_same_bytes_at_one_and_two_captures`]
/// measures a message, so the peak is what the walk adds to holding it.
#[test]
fn a_sorted_lifecycle_walk_over_distinct_chains_holds_what_its_live_set_costs() {
    crate::install::installed();
    let codec = capture_codec();
    let hourly = codec.clone().with_sorted_lifecycle(true);
    let parse = |capture: &[u8]| {
        let mut messages: Vec<FixMsg> = codec
            .parse_text_lines(capture_lines(capture))
            .collect::<yggdryl::Result<_>>()
            .expect("every line reads");
        messages.shrink_to_fit();
        messages
    };
    let walk = |messages: Vec<FixMsg>| {
        hourly
            .lifecycle(messages)
            .try_fold(0_usize, |walked, message: yggdryl::Result<FixMsg>| {
                message.map(|_| walked + 1)
            })
            .expect("a walked message")
    };
    // Once outside every count, so every process-wide first use is no
    // walk's.
    let mut warm = parse(&rendered_capture(1));
    warm.sort_by_key(Event::get_transunix);
    walk(warm);
    let mut figures = Vec::new();
    for copies in [32_u64, 64] {
        let capture = rendered_capture(copies);
        let (input_bytes, mut messages) = retained(|| parse(&capture));
        let input_bytes = usize::try_from(input_bytes).expect("a parse keeps what it built");
        messages.sort_by_key(Event::get_transunix);
        let input = messages.len();
        let (peak, yielded) = peaked(|| walk(messages));
        let per_input = peak as f64 / input as f64;
        let per_yielded = peak as f64 / yielded as f64;
        eprintln!(
            "sorted_lifecycle_walk: {copies} copies, {input} messages holding {input_bytes} \
             bytes ({:.0} each), {yielded} yielded; the walk's peak over its input {peak} \
             bytes, {per_input:.0} per input message, {per_yielded:.0} per yielded message",
            input_bytes as f64 / input as f64
        );
        figures.push((input, yielded, per_input));
    }
    let [
        (small_input, small_yielded, small),
        (large_input, large_yielded, large),
    ] = figures[..]
    else {
        unreachable!("two sizes")
    };
    assert_eq!(
        large_input,
        2 * small_input,
        "twice the copies, twice the input"
    );
    assert!(
        large_yielded > small_yielded,
        "distinct chains yield more at more copies: {small_yielded}, {large_yielded}"
    );
    assert!(
        (large - small).abs() <= small * 0.25,
        "the walk holds {small:.0} bytes per input message at 32 copies and {large:.0} at 64"
    );
}
