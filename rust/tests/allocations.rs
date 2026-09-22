//! What a borrowed protocol view allocates, counted rather than asserted.
//!
//! A view is one pointer plus a `Scheme`, built per call rather than stored,
//! and that is only defensible if building one and reading through it costs
//! nothing. A comment saying so is not evidence, and neither is a timing: a
//! stray `String` in an accessor hides easily inside a map lookup. So this
//! counts them, and pins the three places a protocol read does allocate - a
//! key handed back to the caller, a lookup key too long for `SmolStr`'s inline
//! buffer, and a value that is a list.
//!
//! It also pins the no-op write. A rewrite of a value a field already carries
//! must cost the same however much metadata surrounds it, because it stops
//! before copying the map; nothing else fails when that short-circuit is lost,
//! so nothing else would notice.
//!
//! The counting allocator is a global and a program has exactly one, which is
//! why this is its own test target rather than a case in another file.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::fmt;
use std::fmt::Write as _;
use std::hint::black_box;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use std::sync::Arc;

use yggdryl::FieldValue as _;
use yggdryl::SerieValue as _;
use yggdryl::graph::{
    Book, Element, Event, EventColumn, MarketElement, MarketEventData, MarketOperation, Order,
    Quote,
};
use yggdryl::holder::Buffer;
use yggdryl::text::{TextBytes, TextEntries, TextLine, TextOptions, read_text_lines};
use yggdryl::{
    Bytes, INLINE_BYTES, INLINE_CAPACITY, Str, StringType, StructType, UncheckedFieldScalar, Uuid,
};
use yggdryl::{
    Charset, DataType, DataTypeId, Decimal18, Field, FieldPath, FieldRecord, FieldScalar, FixCode,
    FixCodec, FixId, FixMsg, FixRegistry, Int64, MediaType, MimeType, PythonKind, PythonMetadata,
    Scalar, Serie, Side, State, TimeUnit, Timezone, Value, Variant, Version,
};

/// A pass-through allocator that counts allocations while armed.
struct Counting;

/// Allocations since the counter was armed.
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

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
        }
        // SAFETY: `layout` is forwarded unchanged to the system allocator.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the pointer came from `System.alloc` with this same layout.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if ARMED.get() {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: the pointer and layout came from this allocator.
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Count the allocations `work` performs, and return them with its answer.
fn counted<T>(work: impl FnOnce() -> T) -> (usize, T) {
    let guard = COUNTING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    ALLOCATIONS.store(0, Ordering::Relaxed);
    ARMED.set(true);
    let answer = work();
    ARMED.set(false);
    let counted = ALLOCATIONS.load(Ordering::Relaxed);
    drop(guard);
    (counted, answer)
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

/// Pin the measured codec cost after warming shared empty metadata.
fn variant_cost(what: &str, expected: usize, work: impl FnMut()) {
    let (once, repeated) = counted_once_and_repeated(work);
    assert_eq!(once, expected, "variant {what}: allocations on one call");
    assert_eq!(
        repeated,
        expected * 1_000,
        "variant {what}: repeated allocations"
    );
    eprintln!("variant_{what}: once={once} repeated={repeated}");
}

fn variant_object(fields: usize) -> Scalar {
    Scalar::from_struct((0..fields).map(|index| {
        (
            format!("field{index:04}"),
            Scalar::from(i64::try_from(index).expect("a small fixture")),
        )
    }))
    .expect("the fixture builds")
}

fn variant_nested(width: usize) -> Scalar {
    let children = Scalar::from_sequence(
        (0..width).map(|index| Scalar::from(i64::try_from(index).expect("a small fixture"))),
    );
    Scalar::from_sequence([children.clone(), children])
}

#[test]
fn variant_allocations_are_encoding_buffers_and_decoded_values() {
    let primitive = Scalar::from(7_i64);
    let object4 = variant_object(4);
    let object64 = variant_object(64);
    let nested = variant_nested(64);
    let primitive_variant = Variant::encode(&primitive).expect("the fixture encodes");
    let object4_variant = Variant::encode(&object4).expect("the fixture encodes");
    let object64_variant = Variant::encode(&object64).expect("the fixture encodes");
    let nested_variant = Variant::encode(&nested).expect("the fixture encodes");

    // Empty metadata is shared; encoding retains its payload after one scratch Vec.
    variant_cost("primitive_encode", 2, || {
        black_box(primitive.into_variant().expect("the primitive encodes"));
    });
    variant_cost("primitive_decode", 0, || {
        black_box(Scalar::from_variant(&primitive_variant).expect("the primitive decodes"));
    });
    variant_cost("object4_encode", 12, || {
        black_box(object4.into_variant().expect("the object encodes"));
    });
    variant_cost("object4_decode", 2, || {
        black_box(Scalar::from_variant(&object4_variant).expect("the object decodes"));
    });
    variant_cost("object64_encode", 25, || {
        black_box(object64.into_variant().expect("the object encodes"));
    });
    variant_cost("object64_decode", 11, || {
        black_box(Scalar::from_variant(&object64_variant).expect("the object decodes"));
    });
    // Each of the three decoded lists owns one final Arc slice, with no staging Vec.
    variant_cost("nested_decode", 3, || {
        black_box(Scalar::from_variant(&nested_variant).expect("the sequence decodes"));
    });
    let decoded = Scalar::from_variant(&nested_variant).expect("the sequence decodes");
    variant_cost("nested_scalar_clone", 0, || {
        black_box(decoded.clone());
    });
    variant_cost("nested_variant_clone", 0, || {
        black_box(nested_variant.clone());
    });
}

#[test]
fn variant_identity_projections_allocate_nothing() {
    let primitive = Int64::new(7);
    let encoded = Value::into_variant(&primitive).expect("the integer encodes");
    let wrapped = Scalar::Variant(encoded.clone());

    costs("a Variant Value into_variant projection", 0, || {
        black_box(Value::into_variant(&encoded).expect("the variant stays itself"));
    });
    costs("a Variant Value from_variant projection", 0, || {
        black_box(<Variant as Value>::from_variant(&encoded).expect("the variant stays itself"));
    });
    costs("a Scalar::Variant into_variant projection", 0, || {
        black_box(
            wrapped
                .into_variant()
                .expect("the wrapped variant stays itself"),
        );
    });
    assert_eq!(
        <Int64 as Value>::from_variant(&encoded).expect("the exact integer decodes"),
        primitive
    );
}

#[derive(Default)]
struct StackText {
    bytes: [u8; 32],
    len: usize,
}

impl fmt::Write for StackText {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let end = self.len.checked_add(value.len()).ok_or(fmt::Error)?;
        let target = self.bytes.get_mut(self.len..end).ok_or(fmt::Error)?;
        target.copy_from_slice(value.as_bytes());
        self.len = end;
        Ok(())
    }
}

#[test]
fn version_parse_compare_and_render_allocate_nothing() {
    assert_eq!(std::mem::size_of::<Version>(), 4);
    free("parsing an inline version", || {
        black_box("5.0.10".parse::<Version>().expect("a static version"));
    });
    for text in ["5.0sp250", "005.000Sp00250", "5.0SP256", "255.255sP65535"] {
        free("parsing a compact FIX version", || {
            black_box(text.parse::<Version>().expect("a static FIX version"));
        });
    }
    for (text, patch) in [
        ("1.2-rc1", 63_727),
        ("1.2SP2_EP240", 10_898),
        ("1.2.65536", 54_529),
        ("1.2界", 17_090),
    ] {
        free("parsing a version with a folded suffix", || {
            assert_eq!(
                black_box(text)
                    .parse::<Version>()
                    .expect("a folded version"),
                Version::new(1, 2, patch)
            );
        });
    }
    for tail_bytes in [16, 240, 241, 4096] {
        let text = format!("1.2-{}", "x".repeat(tail_bytes - 1));
        let expected = text.parse::<Version>().expect("a generated qualifier");
        assert_eq!(text.len() - "1.2".len(), tail_bytes);
        assert_ne!(expected.patch(), 0);
        free(
            &format!("parsing a {tail_bytes}-byte version suffix"),
            || {
                assert_eq!(
                    black_box(text.as_str())
                        .parse::<Version>()
                        .expect("a generated qualifier"),
                    expected
                );
            },
        );
    }

    let left = "5.0.2".parse::<Version>().expect("a static version");
    let right = "5.0.10".parse::<Version>().expect("a static version");
    free("comparing inline versions", || {
        black_box(left.cmp(&right));
    });

    let mut rendered = StackText::default();
    free("rendering an inline version", || {
        rendered.len = 0;
        write!(&mut rendered, "{right}").expect("the stack buffer is wide enough");
        black_box(&rendered.bytes[..rendered.len]);
    });
}

#[test]
fn uuid_version_7_and_8_construction_allocate_nothing() {
    let instants = [0, 1, 999, 281_474_976_710_655_999];
    for count in [1, 32, 1_024] {
        free(&format!("constructing {count} UUIDv7 values"), || {
            for index in 0..count {
                black_box(
                    Uuid::from_v7(
                        black_box(instants[index % instants.len()]),
                        black_box(u64::MAX - index as u64),
                    )
                    .expect("an in-range microsecond instant"),
                );
            }
        });
        free(&format!("constructing {count} UUIDv8 values"), || {
            for index in 0..count {
                black_box(Uuid::from_v8(black_box(u128::MAX - index as u128)));
            }
        });
    }
}

#[test]
fn txhash_uuid_projection_allocates_nothing_at_any_corpus_size() {
    use yggdryl::txhash::TxHash;
    use yggdryl::{Digest, DigestAlgorithm};

    let values: Vec<_> = [DigestAlgorithm::Xxh64, DigestAlgorithm::Xxh3]
        .into_iter()
        .flat_map(|algorithm| {
            [
                (0, TimeUnit::Nanosecond),
                (15, TimeUnit::Nanosecond),
                (16, TimeUnit::Nanosecond),
                (65_535, TimeUnit::Nanosecond),
                (65_536, TimeUnit::Nanosecond),
                (i64::MAX, TimeUnit::Nanosecond),
                (1_700_000_000, TimeUnit::Second),
                (1_700_000_000_000, TimeUnit::Millisecond),
                (1_700_000_000_000_000, TimeUnit::Microsecond),
            ]
            .into_iter()
            .map(move |(count, unit)| {
                TxHash::new_in(count, unit, Digest::new(algorithm, u128::MAX))
                    .expect("a clock resolution")
            })
        })
        .collect();
    for count in [1, 32, 1_024] {
        free(
            &format!("projecting {count} TxHash values to UUIDv7"),
            || {
                for index in 0..count {
                    black_box(
                        black_box(values[index % values.len()])
                            .into_uuid()
                            .expect("an in-range microsecond instant and a 64-bit digest"),
                    );
                }
            },
        );
    }
}

#[test]
fn market_event_identity_refresh_and_finalization_allocate_nothing() {
    let mut event = MarketEventData::at(1_700_000_000_000_000_000);
    event.set_currhashcode(1);
    let mut generation = 1_u64;
    free("refreshing and finalizing a market event identity", || {
        generation = generation.wrapping_add(1);
        event.set_currunix(1_700_000_000_000_000_000 + generation as i64);
        event.set_seqnum(generation);
        event.set_crosshashcode(generation);
        event.finalized(generation.rotate_left(17));
        black_box((event.get_curruuid(), event.get_crossuuid()));
    });
}

/// A field carrying HTTP headers plus `extra` unrelated metadata keys.
///
/// The extra keys sort after every `HTTP:` one, so they are what a read walks
/// past rather than something it stops at.
fn http_field(extra: usize) -> Field {
    let mut field = Field::from_parts(
        "payload",
        DataType::binary(),
        false,
        [
            ("HTTP:content-type", "application/json"),
            ("HTTP:content-encoding", "gzip, br, zstd"),
            ("HTTP:content-length", "4096"),
            ("HTTP:etag", "\"revision-1\""),
        ],
    )
    .expect("the static HTTP metadata is valid");
    field
        .update_metadata((0..extra).map(|index| (format!("zz-key-{index:04}"), index.to_string())))
        .expect("the generated metadata keys are valid");
    field
}

/// A field carrying a Python class declaration plus `extra` unrelated keys.
///
/// The declaration is the one a decorated dataclass writes, so the counts below
/// are what the Python binding pays on every schema it builds.
fn python_field(module: &str, extra: usize) -> Field {
    let declared = PythonMetadata::new(module, "Book.Quote", PythonKind::Dataclass)
        .expect("the static declaration is valid");
    let mut field = DataType::Int64.required_field("Quote");
    field
        .as_python_mut()
        .set_class(&declared)
        .expect("the static declaration remains valid");
    field
        .update_metadata((0..extra).map(|index| (format!("zz-key-{index:04}"), index.to_string())))
        .expect("the generated metadata keys are valid");
    field
}

/// A field carrying Iceberg's whole column vocabulary plus `extra` keys.
#[cfg(feature = "iceberg")]
fn iceberg_field(extra: usize) -> Field {
    use yggdryl::iceberg::Transform;

    let mut field = DataType::Int64.required_field("id");
    let mut view = field.as_iceberg_mut();
    view.set_schema_id(3).expect("a static schema identifier");
    view.set_identifier_field_ids(&[1, 2, 3])
        .expect("static identifier columns");
    view.set_doc("row identifier").expect("a static doc string");
    view.set_spec_id(7).expect("a static spec identifier");
    view.set_partition_source_id(11)
        .expect("a static source column");
    view.set_transform(&Transform::Identity)
        .expect("a static transform");
    field
        .update_metadata((0..extra).map(|index| (format!("zz-key-{index:04}"), index.to_string())))
        .expect("the generated metadata keys are valid");
    field
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
    let mut parties = DataType::list(item).nullable_field("Parties");
    parties
        .as_fix_mut()
        .set_counter(453)
        .expect("a static counter");
    let mut counter = DataType::Int32.nullable_field("NoPartyIDs");
    counter.as_fix_mut().set_tag(453).expect("a static tag");
    let mut symbol = DataType::utf8().nullable_field("Symbol");
    symbol.as_fix_mut().set_tag(55).expect("a static tag");
    symbol
        .as_fix_mut()
        .set_tags(&[65])
        .expect("a static alternate tag");
    symbol
        .as_fix_mut()
        .set_names(["Ticker", "SecuritySymbolIdentifier"])
        .expect("static aliases");
    let mut msgtype = DataType::utf8().nullable_field("MsgType");
    msgtype.as_fix_mut().set_tag(35).expect("a static tag");
    let mut trade = DataType::utf8().nullable_field("TradeID");
    trade.as_fix_mut().set_tag(5_001).expect("a static tag");
    trade
        .as_fix_mut()
        .set_branches([VENUE])
        .expect("a static membership");
    trade
        .as_fix_mut()
        .set_names(["TradeIdentifier"])
        .expect("a static alias");
    let generated = (0..extra).map(|index| {
        let mut field = DataType::Int64.nullable_field(format!("Generated{index:04}"));
        let tag = i32::try_from(1_100 + index).expect("a small tag");
        field.as_fix_mut().set_tag(tag).expect("a generated tag");
        field
            .as_fix_mut()
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
    // A wide dictionary: a hit must cost the same however much it walks past.
    let registry = fix_registry(512);
    let vendor = FixId::of(5_001, "TradeID").expect("a vendor identifier");
    // Another name on the same tag is another identity: an exact miss.
    let foreign = FixId::of(5_001, "OtherTradeID").expect("a foreign identifier");

    for (what, key) in [
        ("a scalar tag", yggdryl::FixKey::Tag(55)),
        ("an alternate tag", yggdryl::FixKey::Tag(65)),
        ("a counter tag", yggdryl::FixKey::Tag(453)),
        ("a tag nothing declares", yggdryl::FixKey::Tag(9_999)),
        ("a canonical name", yggdryl::FixKey::Name("symbol")),
        ("an alias", yggdryl::FixKey::Name("ticker")),
        ("a counter name", yggdryl::FixKey::Name("nopartyids")),
        ("a group name", yggdryl::FixKey::Name("parties")),
        ("a venue's own name", yggdryl::FixKey::Name("tradeid")),
        ("an identity", yggdryl::FixKey::Id(vendor)),
        ("an identity nothing holds", yggdryl::FixKey::Id(foreign)),
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
        let _ = black_box(registry.get_field(black_box(yggdryl::FixKey::Name("absent"))));
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
    field.as_fix_mut().set_tag(385).unwrap();
    field
        .as_fix_mut()
        .set_directions(&[
            yggdryl::FixDirection::new("S", [r"(?i)(?:^|\s)tx\s", ">>>"]),
            yggdryl::FixDirection::new("R", [r"(?i)(?:^|\s)rx\s", "<<<"]),
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
    let mut registry = FixRegistry::new();
    registry
        .set_codeset(
            "sidecodeset",
            &[FixCode::new("Buy", "1"), FixCode::new("Sell", "2")],
        )
        .expect("a static code set");
    let mut field = DataType::utf8().nullable_field("Side");
    field.as_fix_mut().set_tag(9_995).expect("a static tag");
    field
        .as_fix_mut()
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
    let registry = Arc::new(fix_registry(64));
    let vendor = FixId::of(5_001, "TradeID").expect("a vendor identifier");
    let foreign = FixId::of(5_001, "OtherTradeID").expect("a foreign identifier");
    let mut symbol = DataType::utf8().nullable_field("Symbol");
    symbol.as_fix_mut().set_tag(55).expect("a static tag");
    let mut trade = DataType::utf8().nullable_field("TradeID");
    trade.as_fix_mut().set_tag(5_001).expect("a static tag");
    trade
        .as_fix_mut()
        .set_branches([VENUE])
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
fn schema_path_hits_and_misses_on_short_names_allocate_nothing() {
    // The nested child proves the common root lookup does not fall through to
    // a traversal. This pins only one short identifier, not parsed compound or
    // long paths.
    let nested = StructType::from_fields([DataType::Int64.required_field("outside")])
        .map(DataType::from)
        .expect("a nested struct")
        .required_field("nested");
    let row = StructType::from_fields([DataType::Int64.required_field("id"), nested])
        .map(DataType::from)
        .expect("a row")
        .required_field("row");
    let dtype = row.dtype();

    assert_eq!(dtype.get_field_by_path("id").map(Field::name), Some("id"));
    assert!(dtype.get_field_by_path("missing").is_none());
    assert_eq!(row.get_field_by_path("id").map(Field::name), Some("id"));
    assert!(row.get_field_by_path("missing").is_none());

    free("a short datatype path hit and miss", || {
        black_box((
            dtype.get_field_by_path(black_box("id")).is_some(),
            dtype.get_field_by_path(black_box("missing")).is_none(),
        ));
    });
    free("a short field path hit and miss", || {
        black_box((
            row.get_field_by_path(black_box("id")).is_some(),
            row.get_field_by_path(black_box("missing")).is_none(),
        ));
    });
}

#[test]
fn the_typed_facts_of_a_message_are_borrowed_at_every_row_width() {
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
                held.event().get_currunix(),
                held.event().get_currhashcode(),
                held.event().get_curruuid(),
                held.event().get_crosscode(),
            ));
        });
    }
}

#[test]
fn typed_market_operation_and_entry_conversions_move_without_allocating() {
    let mut event = MarketEventData::at(1);
    event.set_crosscode("ORDER-1".to_owned());
    event.finalize();
    let mut operation = Some(MarketOperation::from(Order::from(event)));
    let (into_entry, entry) = counted(|| {
        operation
            .take()
            .expect("one operation")
            .into_entry()
            .expect("an order has an entry")
    });
    assert_eq!(into_entry, 0, "operation to entry allocated");

    let mut entry = Some(entry);
    let (at, operation) = counted(|| entry.take().expect("one entry").at(2));
    assert_eq!(at, 0, "entry to operation allocated");
    black_box(operation);
}

fn allocation_book_operation(
    code: impl Into<String>,
    unix: i64,
    quantity: i64,
    state: &str,
) -> MarketOperation {
    let mut event = MarketEventData::at(unix);
    event.set_crosscode(code.into());
    event.set_symbolticker(Some("ALLOC".to_owned()));
    event.set_side(Side::read("Buy").expect("the shipped buy side"));
    event.set_px(Decimal18::from_int(100));
    event.set_qty(Decimal18::from_int(quantity));
    event.set_state(State::read(state).expect("a shipped state"));
    event.finalize();
    Quote::from(event).into()
}

fn allocation_book(entries: usize) -> Book {
    let mut book = Book::new(1, "ALLOC");
    book.add_operations(
        (0..entries).map(|index| allocation_book_operation(format!("ALLOC-{index}"), 1, 1, "New")),
    )
    .expect("the initial depth");
    book
}

#[test]
fn one_book_update_does_not_allocate_per_live_entry() {
    let mut shallow = allocation_book(1);
    let shallow_update = allocation_book_operation("ALLOC-0", 2, 2, "Replaced");
    let (shallow_allocations, shallow_result) =
        counted(|| shallow.add_operations([shallow_update]));
    shallow_result.expect("the shallow update");

    let mut deep = allocation_book(128);
    let deep_update = allocation_book_operation("ALLOC-0", 2, 2, "Replaced");
    let (deep_allocations, deep_result) = counted(|| deep.add_operations([deep_update]));
    deep_result.expect("the deep update");
    assert_eq!(shallow.bid().len(), 1);
    assert_eq!(deep.bid().len(), 128);

    assert!(
        deep_allocations <= shallow_allocations + 4,
        "one update allocated {deep_allocations} times at depth 128 but {shallow_allocations} times at depth 1"
    );
    black_box((shallow, deep));
}

#[test]
fn a_line_of_several_frames_costs_its_messages_and_nothing_per_line() {
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

#[test]
fn building_a_view_and_reading_through_it_allocates_nothing() {
    // A wide map, because a view remembers its protocol rather than collecting
    // it: what surrounds the properties must not reach the count.
    let field = http_field(256);

    // Construction: the `Scheme` clone a view keeps is a well-known protocol,
    // which carries no heap payload, so building a view per call is free.
    free("as_http", || {
        let _ = black_box(field.as_http());
    });
    free("as_properties", || {
        let _ = black_box(field.as_http().as_properties());
    });
    free("scheme clone", || {
        let _ = black_box(field.as_http().scheme().clone());
    });
    free("prefix", || {
        let _ = black_box(field.as_http().prefix());
    });
    free("as_field", || {
        let _ = black_box(field.as_http().as_field().name());
    });

    // Lookup and iteration: every answer borrows out of the map the field
    // already owns.
    free("get", || {
        let _ = black_box(field.as_http().get("content-type"));
    });
    free("contains_key", || {
        let _ = black_box(field.as_http().contains_key("content-type"));
    });
    free("len", || {
        let _ = black_box(field.as_http().len());
    });
    free("is_empty", || {
        let _ = black_box(field.as_http().is_empty());
    });
    free("iter", || {
        let _ = black_box(field.as_http().iter().count());
    });
    free("next_entry", || {
        let _ = black_box(field.as_http().next_entry(Some("content-length")));
    });
    free("comment", || {
        let _ = black_box(field.as_http().comment());
    });
    free("display", || {
        let _ = black_box(field.as_http().display());
    });

    // The typed HTTP reads that answer a borrow or a copy type.
    free("content_type", || {
        let _ = black_box(field.as_http().content_type());
    });
    free("content_encoding", || {
        let _ = black_box(field.as_http().content_encoding());
    });
    free("etag", || {
        let _ = black_box(field.as_http().etag());
    });
    free("content_length", || {
        let _ = black_box(field.as_http().content_length());
    });
    free("mime_type", || {
        let _ = black_box(field.as_http().mime_type());
    });
}

#[test]
fn a_read_allocates_only_what_it_hands_back() {
    let field = http_field(0);

    // `key` is the one property method that returns an owned key, and it is
    // built once into an exactly sized `String` rather than grown.
    costs("key", 1, || {
        let _ = black_box(field.as_http().key("content-type"));
    });

    // A media type is a base plus a list of codings, so it costs the list. The
    // count is the list itself and not one per coding: three codings here cost
    // what one would.
    costs("media_type", 3, || {
        let _ = black_box(field.as_http().media_type());
    });
    let base_only = Field::from_parts(
        "payload",
        DataType::binary(),
        false,
        [("HTTP:content-type", "application/json")],
    )
    .expect("the static content type is valid");
    free("media_type without codings", || {
        let _ = black_box(base_only.as_http().media_type());
    });
}

#[cfg(feature = "iceberg")]
#[test]
fn an_iceberg_read_costs_only_what_it_hands_back() {
    let field = iceberg_field(256);

    // A lookup key is written into a stack buffer whatever its length, so no
    // read pays for its key: `ICEBERG:schema-id` and `ICEBERG:spec-id` are
    // free, and so is the 27-byte `ICEBERG:partition-source-id` that used to
    // reach the heap when the key was a `SmolStr` past its inline 23 bytes.
    free("doc", || {
        let _ = black_box(field.as_iceberg().doc());
    });
    free("schema_id", || {
        let _ = black_box(field.as_iceberg().schema_id());
    });
    free("spec_id", || {
        let _ = black_box(field.as_iceberg().spec_id());
    });
    free("transform", || {
        let _ = black_box(field.as_iceberg().transform());
    });

    free("partition_source_id", || {
        let _ = black_box(field.as_iceberg().partition_source_id());
    });

    // The identifier list costs the vector it returns, which grows by
    // doubling rather than once per identifier. The counts last moved down
    // by two when the property read stopped assembling its key on the heap.
    costs("identifier_field_ids", 1, || {
        let _ = black_box(field.as_iceberg().identifier_field_ids());
    });
    let mut wider = iceberg_field(0);
    wider
        .as_iceberg_mut()
        .set_identifier_field_ids(&[1, 2, 3, 4, 5, 6, 7, 8, 9])
        .expect("static identifier columns");
    costs("identifier_field_ids over nine", 3, || {
        let _ = black_box(wider.as_iceberg().identifier_field_ids());
    });
}

#[test]
fn a_python_read_costs_only_the_declaration_it_hands_back() {
    let field = python_field("trading.book", 256);

    // Every `PYTHON:` key is shorter than `SmolStr`'s 23-byte inline buffer -
    // `PYTHON:qualname` is the longest at 15 - so no assembled lookup key ever
    // reaches the heap, whatever the declaration says.
    free("module", || {
        black_box(field.as_python().module());
    });
    free("qualname", || {
        black_box(field.as_python().qualname());
    });
    free("class_name", || {
        black_box(field.as_python().class_name());
    });
    free("kind", || {
        let _ = black_box(field.as_python().kind());
    });

    // The whole declaration is two `SmolStr` and a form. A module and a
    // qualified name that fit inline make reading it free, which is what the
    // common case - one class, one module path - actually is.
    free("class", || {
        let _ = black_box(field.as_python().class());
    });

    // This is the boundary, pinned: a module path past the inline buffer puts
    // that half on the heap. It is a property of how long the name is, not of
    // the read.
    let deep = python_field("trading.book.execution.reporting.venue", 256);
    costs("class with a module past the inline buffer", 1, || {
        let _ = black_box(deep.as_python().class());
    });

    // The import path is the one read that assembles a value rather than
    // borrowing one, so it costs the `String` it hands back.
    costs("import_path", 1, || {
        black_box(field.as_python().import_path());
    });
}

/// What one `set_class` costs, whatever it is written over.
///
/// Three keys assembled, three values owned, the overlay that carries them,
/// and the one canonicalized form. Nothing here scales with the map.
const PYTHON_CLASS_WRITE: usize = 10;

#[test]
fn writing_a_python_declaration_costs_a_constant_whatever_surrounds_it() {
    // `set_class` is one three-property overlay rather than three writes, and
    // a field owns its own metadata map, so the write never copies what it is
    // written over. That is the claim with no other witness: the count is the
    // same over four unrelated keys and over two hundred and fifty-six, and
    // the same whether the declaration replaces itself or moves the class.
    let declared = PythonMetadata::new("trading.book", "Book.Quote", PythonKind::Dataclass)
        .expect("the static declaration is valid");
    let moved = PythonMetadata::new("trading.execution", "Book.Fill", PythonKind::Field)
        .expect("the moved static declaration is valid");
    for extra in [4_usize, 64, 256] {
        let mut field = python_field("trading.book", extra);
        let (unchanged, ()) = counted(|| {
            field
                .as_python_mut()
                .set_class(&declared)
                .expect("the identical declaration remains valid");
        });
        assert_eq!(
            unchanged, PYTHON_CLASS_WRITE,
            "rewriting the same declaration over {extra} unrelated keys grew"
        );
        let (effective, ()) = counted(|| {
            field
                .as_python_mut()
                .set_class(&moved)
                .expect("the moved declaration remains valid");
        });
        assert_eq!(
            effective, PYTHON_CLASS_WRITE,
            "moving the declaration over {extra} unrelated keys grew"
        );
    }
}

#[test]
fn a_no_op_media_type_rewrite_costs_the_same_whatever_surrounds_it() {
    let media = MediaType::from_parts(MimeType::CSV, [MimeType::GZIP])
        .expect("the static media type is valid");

    // Only the no-op is pinned. An effective write copies the map it rewrites,
    // so its count belongs to the map's size; the short-circuit is the claim
    // that has no other witness, because losing it changes no answer.
    for extra in [4_usize, 64, 256] {
        let mut field = http_field(extra);
        field
            .as_http_mut()
            .set_media_type(media.clone())
            .expect("the static media type remains valid");
        let (allocations, ()) = counted(|| {
            field
                .as_http_mut()
                .set_media_type(media.clone())
                .expect("the identical media type remains valid");
        });
        assert_eq!(
            allocations, 4,
            "rewriting the same media type over {extra} unrelated keys stopped \
             costing the two rendered headers alone"
        );
    }
}

#[cfg(feature = "iceberg")]
#[test]
fn writing_a_doc_string_costs_the_key_and_the_value_and_nothing_else() {
    // Unlike the media pair, a single property write never copies the map, so
    // both the no-op and the effective write are pinned: two allocations, the
    // assembled key and the value, however much metadata is already stored.
    for extra in [4_usize, 64, 256] {
        let mut field = iceberg_field(extra);
        let (unchanged, ()) = counted(|| {
            field
                .as_iceberg_mut()
                .set_doc("row identifier")
                .expect("the identical doc string remains valid");
        });
        assert_eq!(
            unchanged, 2,
            "rewriting the same doc over {extra} unrelated keys grew"
        );
        let (effective, ()) = counted(|| {
            field
                .as_iceberg_mut()
                .set_doc("the row identifier")
                .expect("the replacement doc string is valid");
        });
        assert_eq!(
            effective, 2,
            "replacing the doc over {extra} unrelated keys grew"
        );
    }
}

#[test]
fn timezone_handle_hits_and_copies_allocate_nothing() {
    let dynamic = Timezone::from_str("Custom/AllocationProbe").expect("a dynamic zone");
    let _ = dynamic.as_smol_str();

    for (label, spelling) in [
        ("registered zone", "Europe/Paris"),
        ("registered alias", "US/Eastern"),
        ("fixed offset", "+05:30"),
        ("interned dynamic zone", "Custom/AllocationProbe"),
    ] {
        free(label, || {
            let zone = Timezone::from_str(black_box(spelling)).expect("a valid zone");
            black_box(zone.as_str());
        });
    }

    free("copying and reading a timezone handle", || {
        let copied = black_box(dynamic);
        black_box(copied.as_str());
    });
}

/// A value of each shape the canonical feed walks differently.
///
/// A leaf, a wide record, and a deep nest: the three the benchmark measures
/// and the three where a stray allocation would hide.
fn feed_corpus() -> Vec<(&'static str, Scalar)> {
    let wide = Scalar::from_struct(
        (0..64).map(|index| (format!("column_{index:03}"), Scalar::from(index))),
    )
    .expect("the generated record names are unique");
    let mut deep = Scalar::from("leaf");
    for _ in 0..32 {
        deep = Scalar::from_sequence([deep, Scalar::from(1)]);
    }
    vec![
        ("a leaf", Scalar::from("AAPL")),
        ("an integer", Scalar::from(18_723)),
        ("a decimal", Scalar::d128(18_723, 2)),
        ("a wide record", wide),
        ("a deep nest", deep),
    ]
}

#[test]
fn the_canonical_value_feed_allocates_nothing() {
    // The feed is what every digest, row key, and `stable_hash` reads, so a
    // stray allocation in it would be paid once per value in a batch. The
    // state is built outside the counted section: XXH3 keeps its secret on the
    // heap, and that is the algorithm's cost rather than the feed's.
    for (label, value) in feed_corpus() {
        let mut sink = yggdryl::xxhash::Xxh3::new();
        free(&format!("feeding {label}"), || {
            value.write_bytes(black_box(&mut sink));
        });
    }
}

#[test]
fn borrowed_value_bytes_allocate_nothing() {
    // The payload view borrows from the value or answers an inline array, so
    // reading the bytes of a string or a 256-bit decimal copies neither.
    for (label, value) in feed_corpus() {
        if value.as_value_bytes().is_none() {
            continue;
        }
        free(&format!("reading the payload of {label}"), || {
            let bytes = value.as_value_bytes().expect("the payload is there");
            black_box(bytes.len());
        });
    }
    for value in [
        Scalar::from("a symbol long enough to outgrow any inline string buffer"),
        Scalar::d256(yggdryl::i256::from_i128(i128::MIN), -3),
    ] {
        free("reading a wide payload", || {
            black_box(value.as_value_bytes().expect("the payload is there").len());
        });
    }
}

/// A row schema over the payload-carrying columns, and one row that satisfies
/// it exactly. These are the columns whose canonical form used to be built and
/// thrown away once per row: the payload is unbounded, so the copy was too.
fn payload_row() -> (Field, Scalar) {
    let root = StructType::from_fields([
        Field::new("symbol", DataType::utf8(), false),
        Field::new("payload", DataType::binary(), false),
        Field::new("ccy", DataType::Currency, false),
        Field::new("venue", DataType::ascii(), false),
    ])
    .map(DataType::from)
    .expect("the row schema is valid")
    .required_field("row");
    let long = "a symbol far longer than any inline string buffer can hold";
    let row = Scalar::from_sequence([
        Scalar::from(long),
        Scalar::from(vec![0x42_u8; 4_096]),
        root.fields()[2]
            .scalar("USD")
            .expect("the currency code is valid"),
        root.fields()[3]
            .scalar("XNAS")
            .expect("the venue text is ASCII"),
    ]);
    let row = root
        .canonicalize_value(row)
        .expect("the row satisfies its schema");
    (root, row)
}

#[test]
fn canonicalizing_a_row_a_schema_already_holds_allocates_nothing() {
    // Ingest canonicalizes every row, and a row read back out of a batch is
    // already in its declared representation. Deciding before building is what
    // makes that case free: the alternative built each string, payload, and
    // code only to compare it against the value already in hand.
    let (root, row) = payload_row();
    free(
        "canonicalizing a row already in its declared representation",
        || {
            black_box(
                root.canonicalize_value(black_box(&row).clone())
                    .expect("the row satisfies its schema"),
            );
        },
    );
}

fn row_storage_fixture(width: usize) -> (Field, Scalar, Scalar) {
    let fields = (0..width)
        .map(|index| DataType::Int64.required_field(format!("c{index}")))
        .collect::<Vec<_>>();
    let named = Scalar::from_struct(
        fields
            .iter()
            .map(|field| (field.name(), Scalar::from(7_i32))),
    )
    .expect("unique column names");
    let row = Scalar::from_sequence((0..width).map(|_| Scalar::from(7_i32)));
    let field = DataType::from(StructType::from_fields(fields).expect("unique fields"))
        .required_field("row");
    (field, row, named)
}

#[test]
fn canonical_row_storage_is_allocated_once_when_cells_change() {
    for width in [4, 64, 1_024] {
        let (field, row, _) = row_storage_fixture(width);
        costs(&format!("canonicalizing {width} integer cells"), 1, || {
            black_box(field.canonicalize_value(black_box(&row).clone()).unwrap());
        });
    }
}

#[test]
fn canonical_row_storage_is_allocated_once_for_named_input() {
    for width in [4, 64] {
        let (field, _, named) = row_storage_fixture(width);
        costs(&format!("ordering {width} named integer cells"), 1, || {
            black_box(field.canonicalize_value(black_box(&named).clone()).unwrap());
        });
    }
}

#[test]
fn typed_row_storage_is_allocated_once_for_named_or_rewritten_input() {
    for width in [4, 64] {
        let (field, row, named) = row_storage_fixture(width);
        for (shape, value) in [("ordered", row), ("named", named)] {
            costs(&format!("reading {width} {shape} integer cells"), 1, || {
                black_box(FieldRecord::new(&field, black_box(&value).clone()).unwrap());
            });
        }
    }
}

#[test]
fn rewriting_a_layout_shares_the_storage_it_rewrites() {
    // An offset width is a layout, not a payload. Rewriting between two of
    // them retags one storage handle, so the bytes are never copied and the
    // rewritten value still points at the buffer it came from.
    let payload = vec![0x42_u8; 4_096];
    let source = Scalar::from(payload);
    let address = |value: &Scalar| value.as_bytes().expect("the payload is there").as_ptr();
    let from = address(&source);
    for dtype in [DataType::large_binary(), DataType::binary_view()] {
        let field = Field::new("payload", dtype, false);
        let rewritten = field.scalar(source.clone()).expect("the payload is bytes");
        assert_ne!(rewritten.id(), source.id());
        assert_eq!(address(&rewritten), from, "a rewrite copied the payload");
    }

    let text = Scalar::from("a symbol far longer than any inline string buffer can hold");
    let characters = |value: &Scalar| value.as_str().expect("the text is there").as_ptr();
    let from = characters(&text);
    for dtype in [DataType::large_utf8(), DataType::utf8_view()] {
        let field = Field::new("symbol", dtype, false);
        let rewritten = field.scalar(text.clone()).expect("the value is text");
        assert_ne!(rewritten.id(), text.id());
        assert_eq!(characters(&rewritten), from, "a rewrite copied the text");
    }
}

#[test]
fn a_string_value_is_inline_to_its_capacity_and_one_handle_past_it() {
    // `Str` wraps the compact string, so its threshold is that string's: a
    // value of `INLINE_CAPACITY` bytes lives in the value and one byte more
    // costs exactly the shared handle. Restating a value under other
    // parameters retags the handle, so the characters are never copied.
    let inline = "s".repeat(INLINE_CAPACITY);
    free("building a string value at the inline capacity", || {
        let value = Str::new(black_box(inline.as_str()));
        assert!(value.is_inline());
        black_box(value);
    });
    let shared = "s".repeat(INLINE_CAPACITY + 1);
    costs("building a string value one byte past it", 1, || {
        let value = Str::new(black_box(shared.as_str()));
        assert!(!value.is_inline());
        black_box(value);
    });
    let source = Str::new(&shared);
    let large = StringType::LargeUtf8String;
    free("restating a shared string value under another leaf", || {
        let restated = black_box(&source)
            .clone()
            .try_with_parameters(large)
            .expect("the leaf holds it");
        assert!(std::ptr::eq(source.as_str(), restated.as_str()));
        black_box(restated);
    });
}

#[test]
fn a_byte_value_is_inline_to_its_capacity_and_one_handle_past_it() {
    // The byte value keeps its own buffer: `INLINE_BYTES` fit in the value
    // with no heap behind them, and one byte more costs exactly the shared
    // handle.
    let inline = vec![0x42_u8; INLINE_BYTES];
    free("building a byte value at the inline capacity", || {
        let value = Bytes::new(black_box(inline.as_slice()));
        assert!(value.is_inline());
        black_box(value);
    });
    let shared = vec![0x42_u8; INLINE_BYTES + 1];
    costs("building a byte value one byte past it", 1, || {
        let value = Bytes::new(black_box(shared.as_slice()));
        assert!(!value.is_inline());
        black_box(value);
    });
}

#[test]
fn building_a_sequence_costs_one_allocation() {
    // A row is a sequence, so this runs once per row on every ingest path.
    // The children are written straight into the storage the value keeps;
    // collecting them into a `Vec` first would cost a second buffer and a
    // copy of the whole run between the two.
    for width in [4_usize, 64, 1_024] {
        let children = (0..width)
            .map(|index| Scalar::from(i64::try_from(index).expect("the index fits")))
            .collect::<Vec<_>>();
        costs(&format!("building a {width}-column row"), 1, || {
            black_box(Scalar::from_sequence(black_box(&children).iter().cloned()));
        });
    }
    free("building the shared empty sequence", || {
        black_box(Scalar::from_sequence([]));
    });
}

/// An int64 column and a utf8 column of `rows` rows, one row in three
/// absent, straight off Arrow buffers.
fn leaf_columns(rows: usize) -> (Serie, Serie) {
    use arrow_array::{Int64Array, StringArray};

    let counts = Int64Array::from(
        (0..rows)
            .map(|index| (index % 3 != 1).then(|| i64::try_from(index).expect("a row count")))
            .collect::<Vec<_>>(),
    );
    let symbols = StringArray::from(
        (0..rows)
            .map(|index| (index % 3 != 1).then(|| format!("S{index}")))
            .collect::<Vec<_>>(),
    );
    (
        Serie::from_arrow_array(Field::new("count", DataType::Int64, true), Arc::new(counts))
            .expect("an int64 column"),
        Serie::from_arrow_array(
            Field::new("symbol", DataType::utf8(), true),
            Arc::new(symbols),
        )
        .expect("a utf8 column"),
    )
}

#[test]
fn reading_a_typed_leaf_allocates_nothing() {
    // A column holds buffers and no value; a typed read lends out of them -
    // one bounds check, one bitmap read, one buffer read - and builds
    // nothing, however many rows the column holds.
    for rows in [4_usize, 1_024, 16_384] {
        let (counts, symbols) = leaf_columns(rows);
        let middle = rows / 2;
        let absent = 1;

        let leaf = counts.as_int64().expect("an int64 column");
        free(&format!("value on {rows} int64 rows"), || {
            assert_eq!(
                black_box(leaf).value(black_box(middle)),
                Some(middle as i64)
            );
            assert_eq!(black_box(leaf).value(black_box(absent)), None);
            assert_eq!(black_box(leaf).value(black_box(rows)), None);
        });
        free(&format!("values on {rows} int64 rows"), || {
            assert_eq!(black_box(leaf).values().len(), rows);
        });
        free(&format!("len and is_null on {rows} int64 rows"), || {
            assert_eq!(black_box(&counts).len(), rows);
            assert!(!black_box(&counts).is_null(middle).expect("in range"));
            assert!(black_box(leaf).is_null(absent).expect("in range"));
            assert_eq!(black_box(&counts).null_count(), (rows + 1) / 3);
            black_box(leaf.nulls());
            black_box(black_box(&counts).as_int64());
        });

        let leaf = symbols.as_utf8().expect("a utf8 column");
        free(&format!("value on {rows} utf8 rows"), || {
            assert!(black_box(leaf).value(black_box(middle)).is_some());
            assert_eq!(black_box(leaf).value(black_box(absent)), None);
        });
        free(
            &format!("the offsets and the payload of {rows} utf8 rows"),
            || {
                assert_eq!(black_box(leaf).offsets().len(), rows + 1);
                black_box(leaf.payload().as_slice());
            },
        );
        free(&format!("len and is_null on {rows} utf8 rows"), || {
            assert_eq!(black_box(&symbols).len(), rows);
            assert!(!black_box(&symbols).is_null(middle).expect("in range"));
            assert!(black_box(leaf).is_null(absent).expect("in range"));
            black_box(black_box(&symbols).as_utf8());
            black_box(black_box(&symbols).field());
        });
    }
}

#[test]
fn cloning_a_column_allocates_nothing() {
    // A column leaf sits behind one shared pointer and its Arrow buffers
    // behind one each, so a clone at any level is pointer bumps, and so is
    // the scalar carrying one.
    for rows in [4_usize, 16_384] {
        let (counts, symbols) = leaf_columns(rows);
        let records = Serie::from_scalars(
            Field::new(
                "row",
                DataType::from(
                    StructType::from_fields([
                        Field::new("count", DataType::Int64, true),
                        Field::new("symbol", DataType::utf8(), true),
                    ])
                    .expect("two named children"),
                ),
                false,
            ),
            (0..rows).map(|index| {
                Scalar::from_sequence([
                    counts.scalar(index).expect("in range"),
                    symbols.scalar(index).expect("in range"),
                ])
            }),
        )
        .expect("a record column");
        let held = Scalar::Sequence(counts.clone());

        free(&format!("cloning {rows} int64 rows"), || {
            black_box(black_box(&counts).clone());
        });
        free(&format!("cloning the int64 leaf of {rows} rows"), || {
            black_box(black_box(&counts).as_int64().expect("int64").clone());
        });
        free(&format!("cloning {rows} utf8 rows"), || {
            black_box(black_box(&symbols).clone());
        });
        free(&format!("cloning the utf8 leaf of {rows} rows"), || {
            black_box(black_box(&symbols).as_utf8().expect("utf8").clone());
        });
        free(&format!("cloning {rows} record rows"), || {
            black_box(black_box(&records).clone());
        });
        free(&format!("cloning a scalar holding {rows} rows"), || {
            black_box(black_box(&held).clone());
        });
    }
}

#[test]
fn a_second_push_into_an_owned_column_allocates_nothing() {
    // A column taken off a foreign buffer copies its rows once, on its
    // first edit, into a buffer it owns; from then on a push lands in that
    // buffer. What a push then costs is a bounded constant - the builder
    // Arrow hands the buffer back as, and the handle it is finished into -
    // and never a row: the second push costs what the thousandth does, and
    // costs the same over sixteen times the rows. A growth past the
    // capacity would show as one reallocation among the thousand, and a
    // copy would show as a count that follows the corpus.
    use arrow_array::Int64Array;

    let field = || Field::new("price", DataType::Int64, false);
    let column = |rows: usize| {
        let array = Int64Array::from(
            (0..rows)
                .map(|index| i64::try_from(index).expect("a row count"))
                .collect::<Vec<_>>(),
        );
        Serie::from_arrow_array(field(), Arc::new(array)).expect("an int64 column")
    };

    let mut native = Vec::new();
    let mut proven = Vec::new();
    for rows in [1_024_usize, 16_384] {
        // The buffer path: a native value into the values buffer.
        let mut owned = column(rows);
        let leaf = owned.get_int64_mut().expect("an int64 column");
        leaf.push_value(Some(1)).expect("the first edit");
        let (second, ()) = counted(|| leaf.push_value(Some(2)).expect("the second push"));
        let (thousand, ()) = counted(|| {
            for _ in 0..1_000 {
                leaf.push_value(Some(3)).expect("a later push");
            }
        });
        assert_eq!(
            thousand,
            second * 1_000,
            "a thousand native pushes into {rows} rows cost a thousand seconds"
        );
        assert_eq!(leaf.values().len(), rows + 1_002);
        native.push(second);

        // The proven path: a value through the field's contract on the way
        // to the same buffer.
        let mut owned = column(rows);
        owned.push(Scalar::from(1_i64)).expect("the first edit");
        let (second, ()) = counted(|| owned.push(Scalar::from(2_i64)).expect("the second push"));
        let (thousand, ()) = counted(|| {
            for _ in 0..1_000 {
                owned.push(Scalar::from(3_i64)).expect("a later push");
            }
        });
        assert_eq!(
            thousand,
            second * 1_000,
            "a thousand proven pushes into {rows} rows cost a thousand seconds"
        );
        assert_eq!(owned.len(), rows + 1_002);
        proven.push(second);
    }
    assert_eq!(
        native[0], native[1],
        "a native push costs the same at every corpus size"
    );
    assert_eq!(
        proven[0], proven[1],
        "a proven push costs the same at every corpus size"
    );
    assert!(
        native[0] <= proven[0],
        "the buffer path never costs more than the contract path: {native:?} against {proven:?}"
    );
    eprintln!("serie_push: native={} proven={}", native[0], proven[0]);
}

/// The two batches every cast-budget case reads, and the root they answer to.
///
/// The batches differ only in their values, so anything that varies between
/// casting one and casting the other is per-batch work rather than schema work.
fn cast_corpus() -> (
    arrow_schema::SchemaRef,
    [arrow_array::RecordBatch; 2],
    Field,
) {
    use arrow_array::{ArrayRef, Int32Array, RecordBatch, StringArray};
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};

    let schema = Arc::new(Schema::new(vec![
        ArrowField::new("id", ArrowDataType::Int32, false),
        ArrowField::new("symbol", ArrowDataType::Utf8, true),
    ]));
    let batch = |offset: i32| {
        RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int32Array::from(vec![offset, offset + 1])) as ArrayRef,
                Arc::new(StringArray::from(vec!["AAPL", "MSFT"])) as ArrayRef,
            ],
        )
        .expect("the benchmark batch matches its schema")
    };
    let root = Field::new(
        "row",
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
            DataType::utf8().required_field("venue"),
        ])
        .map(DataType::from)
        .expect("the root fields are valid"),
        false,
    );
    (Arc::clone(&schema), [batch(0), batch(2)], root)
}

#[test]
fn a_compiled_cast_costs_the_same_for_every_batch_it_answers() {
    use yggdryl::{ArrowCastOptions, ArrowCastPlan};

    let (schema, batches, root) = cast_corpus();
    let plan = ArrowCastPlan::compile(&schema, &root, ArrowCastOptions::new())
        .expect("the cast is plannable");

    // The budget belongs to a batch, not to the stream: applying the plan a
    // thousand times costs a thousand times one batch, because nothing about
    // the previous batch is retained. Reading it as a total would hide exactly
    // the leak this pins - a plan that grew with every batch it saw.
    let mut index = 0;
    let (once, repeated) = counted_once_and_repeated(|| {
        let batch = batches[index % batches.len()].clone();
        index += 1;
        black_box(plan.apply(batch).expect("the batch fits the plan"));
    });
    assert_eq!(
        repeated,
        once * 1_000,
        "applying one compiled plan cost {once} for one batch and {repeated} for a thousand"
    );
}

#[test]
fn planning_once_is_what_a_reused_plan_saves_per_batch() {
    use yggdryl::{ArrowCastOptions, ArrowCastPlan};

    let (schema, batches, root) = cast_corpus();
    let plan = ArrowCastPlan::compile(&schema, &root, ArrowCastOptions::new())
        .expect("the cast is plannable");
    let batch = batches[0].clone();

    // Warm both paths so neither is charged for a first-call cache fill.
    let _ = plan.apply(batch.clone());
    let _ = root.cast_arrow_batch(batch.clone(), ArrowCastOptions::new());

    let (applied, ()) = counted(|| {
        black_box(plan.apply(batch.clone()).expect("the batch fits the plan"));
    });
    let (planned, ()) = counted(|| {
        black_box(
            root.cast_arrow_batch(batch.clone(), ArrowCastOptions::new())
                .expect("the batch fits the root"),
        );
    });

    // The same batch, the same answer, and the difference is the plan: a
    // reader that compiles per batch pays that difference on every one.
    assert!(
        planned > applied,
        "compiling per batch cost {planned} and reusing one plan cost {applied}"
    );
}

#[test]
fn coupled_value_bytes_allocate_nothing() {
    // A coupled value is an inline instant and an inline digest, so laying
    // the two out, reading them back, and restating the resolution copies
    // nothing to the heap. The one-shot XXH32 answer is inline too; XXH3
    // keeps its secret on the heap, which is the algorithm's cost.
    use yggdryl::txhash::{self, TxHash};
    use yggdryl::{DigestAlgorithm, TimeUnit};

    let value = txhash::txh128(b"AAPL", 1_700_000_000_000_000);
    let bytes = value.into_bytes();
    free("laying out a coupled value", || {
        black_box(value.into_bytes().len());
    });
    free("reading a coupled value back", || {
        black_box(
            TxHash::from_bytes(TimeUnit::Microsecond, DigestAlgorithm::Xxh128, &bytes)
                .expect("the exact width"),
        );
    });
    free("restating a coupled instant", || {
        black_box(value.with_unit(TimeUnit::Second).expect("a coarser unit"));
    });
    free("coupling an XXH32 one-shot", || {
        black_box(txhash::txh32(black_box(b"AAPL"), 1));
    });
    free("projecting the instant as a datetime", || {
        black_box(value.into_datetime());
    });
    // Twenty-four bytes fit the byte value's inline buffer, so the scalar
    // costs nothing where it used to cost its one shared handle.
    free("projecting the whole value as a byte scalar", || {
        black_box(value.into_scalar());
    });
}

#[test]
fn reading_an_instant_out_of_a_value_allocates_nothing() {
    use yggdryl::txhash;
    use yggdryl::{Scalar, TimeUnit, Timezone};

    let integer = Scalar::from(1_700_000_000_000_000_i64);
    let zoned = Scalar::from_datetime(1_700_000_000, TimeUnit::Second, Timezone::UTC)
        .expect("a valid datetime");
    let day = Scalar::date32(19_723);
    for (label, value) in [
        ("an integer", &integer),
        ("a zoned datetime", &zoned),
        ("a date", &day),
    ] {
        free(&format!("reading an instant out of {label}"), || {
            black_box(
                txhash::unix_from_scalar(black_box(value), TimeUnit::Microsecond)
                    .expect("an instant"),
            );
        });
    }
    free("restating a unix count", || {
        black_box(
            txhash::restate_unix(
                black_box(1_999),
                TimeUnit::Nanosecond,
                TimeUnit::Microsecond,
            )
            .expect("a coarser unit"),
        );
    });
}

#[test]
fn a_same_unit_instant_column_shares_its_buffer() {
    // An instant column already at the unit asked for is the answer already,
    // so reading it as unix counts clones two reference-counted buffers and
    // builds nothing; a column at another unit is one fresh buffer and the
    // handle that shares it, however many rows it holds.
    use arrow_array::{TimestampMicrosecondArray, TimestampSecondArray};
    use yggdryl::{TimeUnit, txhash};

    for rows in [16_i64, 4_096] {
        let micros = TimestampMicrosecondArray::from_iter_values(0..rows).with_timezone("UTC");
        free(&format!("reading {rows} same-unit instants"), || {
            black_box(
                txhash::arrow::unix_array(black_box(&micros), TimeUnit::Microsecond)
                    .expect("an instant column")
                    .len(),
            );
        });
        let seconds = TimestampSecondArray::from_iter_values(0..rows);
        costs(&format!("restating {rows} instants"), 2, || {
            black_box(
                txhash::arrow::unix_array(black_box(&seconds), TimeUnit::Microsecond)
                    .expect("an instant column")
                    .len(),
            );
        });
    }
}

/// One value per prebuilt shared field, built through the datatype's own
/// contract so each names exactly the datatype it is pinned under.
///
/// `Variant` keeps a shared field but no value names it - a variant value
/// describes itself - so it is the one prebuilt id with nothing to infer.
fn prebuilt_values() -> Vec<(DataTypeId, Scalar)> {
    let seeds: [(DataTypeId, Scalar); 36] = [
        (DataTypeId::Null, Scalar::Null),
        (DataTypeId::Boolean, Scalar::from(true)),
        (DataTypeId::Int8, Scalar::from(1_i64)),
        (DataTypeId::Int16, Scalar::from(1_i64)),
        (DataTypeId::Int32, Scalar::from(1_i64)),
        (DataTypeId::Int64, Scalar::from(1_i64)),
        (DataTypeId::UInt8, Scalar::from(1_i64)),
        (DataTypeId::UInt16, Scalar::from(1_i64)),
        (DataTypeId::UInt32, Scalar::from(1_i64)),
        (DataTypeId::UInt64, Scalar::from(1_i64)),
        (DataTypeId::Float16, Scalar::from(1.5_f64)),
        (DataTypeId::Float32, Scalar::from(1.5_f64)),
        (DataTypeId::Float64, Scalar::from(1.5_f64)),
        (DataTypeId::Date32, Scalar::date32(19_723)),
        (DataTypeId::Date64, Scalar::date32(19_723)),
        (DataTypeId::Binary, Scalar::from(&b"ABC"[..])),
        (DataTypeId::LargeBinary, Scalar::from(&b"ABC"[..])),
        (DataTypeId::BinaryView, Scalar::from(&b"ABC"[..])),
        (DataTypeId::Utf8String, Scalar::from("AAPL")),
        (DataTypeId::LargeUtf8String, Scalar::from("AAPL")),
        (DataTypeId::Utf8StringView, Scalar::from("AAPL")),
        (DataTypeId::Country, Scalar::from("US")),
        (DataTypeId::Currency, Scalar::from("USD")),
        (DataTypeId::MicCode, Scalar::from("XNAS")),
        (DataTypeId::CfiCode, Scalar::from("ESVUFR")),
        (DataTypeId::IsinCode, Scalar::from("US0378331005")),
        (DataTypeId::CusipCode, Scalar::from("037833100")),
        (DataTypeId::SedolCode, Scalar::from("B0YBKJ7")),
        (DataTypeId::BloombergCode, Scalar::from("AAPL US EQUITY")),
        (DataTypeId::FIGICode, Scalar::from("BBG000BLNQ16")),
        (DataTypeId::Side, Scalar::from("1")),
        (DataTypeId::State, Scalar::from("20NEW")),
        (DataTypeId::TimeInForce, Scalar::from("0")),
        (
            DataTypeId::Uuid,
            Scalar::from("123e4567-e89b-12d3-a456-426614174000"),
        ),
        (DataTypeId::Version, Scalar::from("5.0.2")),
        (DataTypeId::Timezone, Scalar::from("America/New_York")),
    ];
    let mut values: Vec<(DataTypeId, Scalar)> = seeds
        .into_iter()
        .map(|(id, seed)| {
            let dtype = DataType::from_str(id.as_str()).expect("the id names a datatype");
            let value = dtype
                .scalar(seed)
                .expect("the seed is a value of the datatype");
            assert_eq!(value.dtype().expect("a leaf names itself"), dtype, "{id:?}");
            (id, value)
        })
        .collect();
    let url = DataType::url()
        .scalar("https://example.com/a")
        .expect("the text is a URL");
    values.push((DataTypeId::Url, url));
    let urn = DataType::urn()
        .scalar("URN:ISBN:0451450523")
        .expect("the text is a URN");
    values.push((DataTypeId::Urn, urn));
    // A MIME type and a media type are pinned outside the seed loop for the
    // same reason a URL and a URN are: their canonical text is not what was
    // written.
    let mime = DataType::MimeType
        .scalar("application/json")
        .expect("the text is a MIME type");
    values.push((DataTypeId::MimeType, mime));
    let media = DataType::MediaType
        .scalar("application/json; charset=utf-8")
        .expect("the text is a media type");
    values.push((DataTypeId::MediaType, media));
    // Every prebuilt id is either pinned here or one no value ever names:
    // a variant is the one datatype with no value of its own.
    let unnamed = [DataTypeId::Variant];
    let pinned: std::collections::HashSet<DataTypeId> = values.iter().map(|(id, _)| *id).collect();
    for id in DataTypeId::ALL {
        let prebuilt = !id.is_parameterized()
            && DataType::from_str(id.as_str()).is_ok_and(|dtype| dtype.id() == id);
        if prebuilt && !unnamed.contains(&id) {
            assert!(pinned.contains(&id), "{id:?} has a shared field and no pin");
        }
    }
    values
}

#[test]
fn inferring_a_field_scalar_borrows_a_prebuilt_field_and_allocates_nothing() {
    // The typed view borrows the field the crate keeps for the datatype, so
    // typing a leaf value that names itself builds no field and no copy of
    // one - once, and a thousand times, for every prebuilt id.
    for (id, value) in prebuilt_values() {
        free(&format!("inferring a typed {id:?}"), || {
            black_box(FieldScalar::infer(black_box(&value).clone()).expect("the value infers"));
        });
    }
    // The plain string and the plain byte value name the family's default
    // layout, whose field the crate keeps beside the other prebuilt ones.
    for value in [Scalar::from("x"), Scalar::from(vec![1_u8])] {
        free(&format!("inferring a typed {}", value.kind()), || {
            black_box(FieldScalar::infer(black_box(&value).clone()).expect("the value infers"));
        });
    }
    // A parameterized leaf is interned on its first ask and borrowed after.
    for dtype in [
        DataType::decimal128(10, 2).expect("a valid decimal"),
        DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC).expect("a valid instant"),
        DataType::fixed_ascii(4).unwrap(),
    ] {
        free(&format!("looking up the shared field of {dtype}"), || {
            black_box(black_box(&dtype).shared_field().expect("an interned field"));
        });
    }
}

#[test]
fn typing_a_value_a_field_already_holds_allocates_nothing() {
    // The pairing is the field's own value contract, which answers a value
    // already in its declared representation untouched; the same holds for
    // a whole canonical row under its Struct root.
    let (root, row) = payload_row();
    free("typing a canonical row under its root", || {
        black_box(FieldScalar::new(black_box(&root), black_box(&row).clone()).expect("typed"));
    });
    for (index, cell) in row.as_sequence().expect("a row").iter().enumerate() {
        let field = &root.fields()[index];
        free(
            &format!("typing the canonical {} cell", field.name()),
            || {
                black_box(
                    FieldScalar::new(black_box(field), black_box(cell).clone()).expect("typed"),
                );
            },
        );
    }
    let text = Field::new("symbol", DataType::utf8(), false);
    let unchecked = UncheckedFieldScalar::from_str(
        &text,
        "a symbol far longer than any inline string buffer can hold",
    );
    free("borrowing the text an unchecked pairing holds", || {
        black_box(
            black_box(&unchecked)
                .as_str()
                .expect("the held value is text"),
        );
    });
}

/// A canonical row of `width` integer columns under its Struct root.
fn wide_row(width: usize) -> (Field, Scalar) {
    let root = StructType::from_fields(
        (0..width).map(|index| DataType::Int64.required_field(format!("column_{index}"))),
    )
    .map(DataType::from)
    .expect("the row schema is valid")
    .required_field("row");
    let row = root
        .canonicalize_value(Scalar::from_sequence(
            (0..width).map(|index| Scalar::from(i64::try_from(index).expect("the index fits"))),
        ))
        .expect("the row satisfies its schema");
    (root, row)
}

#[test]
fn reading_a_typed_row_costs_one_allocation_and_its_accessors_none() {
    // A typed row is the cells' `Vec` and nothing else: the row is proven by
    // the one canonicalization walk, each cell borrows its child, and a
    // shared value clones a reference. Reading back out borrows.
    for width in [4_usize, 64, 1_024] {
        let (root, row) = wide_row(width);
        costs(&format!("typing a canonical {width}-column row"), 1, || {
            black_box(FieldRecord::new(black_box(&root), black_box(&row).clone()).expect("typed"));
        });
        let record = FieldRecord::new(&root, row.clone()).expect("typed");
        let last = format!("column_{}", width - 1);
        let folded = last.to_ascii_uppercase();
        free(&format!("looking up a cell of {width} by name"), || {
            black_box(
                black_box(&record)
                    .get_by_name(black_box(&last))
                    .expect("a cell"),
            );
        });
        // A name resolves exactly, so a folded one is a miss, and a miss
        // walks the same children without allocating either.
        free(&format!("missing a cell of {width} by name"), || {
            assert!(black_box(&record).get_by_name(black_box(&folded)).is_none());
        });
        free(&format!("looking up a cell of {width} by position"), || {
            black_box(
                black_box(&record)
                    .get_by_index(black_box(width - 1))
                    .expect("a cell"),
            );
        });
        free(&format!("naming the {width} columns"), || {
            black_box(black_box(&record).names().count());
        });
        free(&format!("iterating the {width} cells"), || {
            black_box(
                black_box(&record)
                    .iter()
                    .filter(|cell| cell.is_null())
                    .count(),
            );
        });
    }
}

#[test]
fn a_typed_row_projects_without_general_row_staging() {
    for width in [4, 64] {
        let (field, row) = wide_row(width);
        let record = FieldRecord::new(&field, row.clone()).unwrap();
        let rows = Scalar::from_sequence([row]);
        // Warm Arrow field projection caches before comparing the two doors.
        drop(yggdryl::arrow::batch_from_value(&field, &rows).unwrap());
        drop(record.clone().into_arrow_batch().unwrap());
        let (general_cost, expected) =
            counted(|| yggdryl::arrow::batch_from_value(&field, &rows).unwrap());
        let prepared = record.clone();
        let (typed_cost, actual) = counted(|| prepared.into_arrow_batch().unwrap());
        assert_eq!(actual, expected);
        eprintln!("{width}-column Arrow row: general={general_cost}, typed={typed_cost}");
        assert!(
            typed_cost < general_cost,
            "a proven row must skip general row staging"
        );
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
const FIX_LINE_COSTS: [(usize, usize); 3] = [(4, 31), (16, 35), (64, 38)];

/// A dictionary of `count` `Utf8` fields, tagged from 2000.
///
/// Text rather than integers, because what is measured next door is a
/// *value's width*: a field that only types a number never carries three
/// kilobytes, so a numeric dictionary could not state the case at all.
fn fix_text_registry(count: usize) -> FixRegistry {
    let mut msgtype = DataType::utf8().nullable_field("MsgType");
    msgtype.as_fix_mut().set_tag(35).expect("a static tag");
    let generated = (0..count).map(|index| {
        let mut field = DataType::utf8().nullable_field(format!("Text{index:04}"));
        let tag = i32::try_from(2_000 + index).expect("a small tag");
        field.as_fix_mut().set_tag(tag).expect("a generated tag");
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
const WIDE_VALUE_COSTS: [(usize, (usize, usize)); 3] =
    [(4, (31, 37)), (16, (35, 65)), (64, (38, 164))];

#[test]
fn a_wide_value_costs_the_entries_nothing_and_the_row_one_column() {
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
        field.as_fix_mut().set_tag(tag).expect("a generated tag");
        field
    });
    let item = StructType::from_fields(declared)
        .map(DataType::from)
        .expect("a struct item")
        .required_field("item");
    let mut parties = DataType::list(item).nullable_field("Parties");
    parties
        .as_fix_mut()
        .set_counter(453)
        .expect("a static counter");
    let mut counter = DataType::Int32.nullable_field("NoPartyIDs");
    counter.as_fix_mut().set_tag(453).expect("a static tag");
    let mut msgtype = DataType::utf8().nullable_field("MsgType");
    msgtype.as_fix_mut().set_tag(35).expect("a static tag");
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
/// `Slot.values` buffer exists. A packed row has exactly three scalar slots:
/// `MsgType`, the counter, and fallback `BeginString`.
///
/// Four members and sixteen, because the number that matters is the slope,
/// and the rest of it is the row a wider group builds. The codec reads a
/// row's pairs directly and descends into none of them, so the tree the
/// packed value would have been scanned into is not among these.
/// Two member counts, because the number that matters is the slope and not
/// the constant a message pays whatever it carries.
///
const PACKED_MEMBER_COSTS: [(usize, usize); 2] = [(4, 77), (16, 137)];

#[test]
fn a_packed_occurrence_costs_one_allocation_for_each_key_it_renders() {
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
const FIX_TEXT_LINE_COSTS: [(usize, usize); 3] = [(4, 30), (16, 34), (64, 37)];

#[test]
fn a_message_read_from_a_decoded_line_does_not_pay_for_its_page_again() {
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

/// A line built by hand costs its row header and nothing else: the body is
/// the range it was handed, the options a reference count, and no other
/// reading is resolved until asked. The header comes off the line at
/// construction, so the match - the locations the regex fills, and the list
/// of captures the line keeps - is paid there, once, and every later ask for
/// the captures is free because the slot already holds them. Reading the body
/// and the index adds nothing.
///
/// Reading the content code does, and this is where it moved: the code is no
/// longer the body's bytes alone but the facts the line states as an event,
/// which walks the names it goes by - a name and a value owned per capture,
/// and the map that holds them. A line under no header goes by no name, so it
/// still builds and digests for nothing.
#[test]
fn a_line_built_and_read_allocates_nothing_and_its_captures_once() {
    let options = Arc::new(
        TextOptions::new()
            .try_with_rowheader(r"^\[(?<level>[A-Z]+)\] (?<id>\d+) ")
            .expect("a header"),
    );
    let page = TextBytes::from_bytes("[INFO] 7 body of the line").expect("a page");
    costs("a line built, its body and its index read", 2, || {
        let line = TextLine::from_bytes(0, page.clone(), Arc::clone(&options)).expect("a line");
        black_box(line.body());
        black_box(line.index());
    });
    // The regex keeps a per-thread cache it fills on its first use, which is
    // the expression's cost and not a line's: warmed outside the count.
    black_box(
        TextLine::from_bytes(0, page.clone(), Arc::clone(&options))
            .expect("a line")
            .get_currhashcode(),
    );
    let line = TextLine::from_bytes(0, page.clone(), Arc::clone(&options)).expect("a line");
    let (first, count) = counted(|| black_box(line.captures().len()));
    assert_eq!(count, 2);
    assert_eq!(
        first, 0,
        "the header was taken off at construction and the list is kept"
    );
    free("the captures asked again", || {
        black_box(line.captures().len());
        black_box(line.capture(1));
    });
    // The two captures are the two names this line goes by, and the digest
    // owns them: each is a name and a value, over the map they are held in.
    let (code, _) = counted(|| black_box(line.get_currhashcode()));
    assert_eq!(code, 6, "a name and a value per capture, over their map");
    free("the content code asked again", || {
        black_box(line.get_currhashcode());
    });
    let bare = TextLine::from_bytes(0, page, Arc::new(TextOptions::new())).expect("a line");
    free("a line under no header, read and digested", || {
        black_box(bare.captures().len());
        black_box(bare.get_currhashcode());
    });
    // The tree is the first ask's cost and nothing on the second: a line
    // read for its body and its place never pays for it.
    let paired = TextLine::from_bytes(
        0,
        TextBytes::from_bytes("[INFO] 7 a=1|b=2").expect("a page"),
        Arc::clone(&options),
    )
    .expect("a line");
    let (first_ask, entries) = counted(|| paired.entries().map(TextEntries::len));
    assert_eq!(entries, Some(2));
    assert!(first_ask > 0, "the tree is built on the first ask");
    free("the tree asked again", || {
        black_box(paired.entries().map(TextEntries::len));
    });
}

#[test]
fn first_text_line_from_arrow_does_not_decode_the_rest_of_its_batch() {
    use arrow_array::RecordBatchIterator;
    use yggdryl::text::{from_arrow_reader, into_arrow_batch};

    let options = TextOptions::new();
    let mut first_cost = None;
    for rows in [1, 64, 1024] {
        let lines = (0..rows).map(|index| {
            TextLine::from_bytes(
                index,
                TextBytes::from_bytes("one body").unwrap(),
                std::sync::Arc::new(yggdryl::text::TextOptions::new()),
            )
            .unwrap()
        });
        let batch = into_arrow_batch(lines, &options).unwrap();
        let stream = || {
            Box::new(RecordBatchIterator::new(
                [Ok(batch.clone())],
                batch.schema(),
            )) as yggdryl::arrow::BatchReader
        };
        // Warm shared datatype caches outside the measured row operation.
        black_box(
            from_arrow_reader(stream(), &options)
                .unwrap()
                .next()
                .unwrap()
                .unwrap(),
        );
        let mut reader = from_arrow_reader(stream(), &options).unwrap();
        let (allocations, line) = counted(|| reader.next().unwrap().unwrap());
        assert_eq!(line.body(), "one body");
        assert_eq!(line.index(), 0);
        assert_eq!(
            allocations,
            *first_cost.get_or_insert(allocations),
            "first-row work grew with {rows} source rows"
        );
        assert!(
            allocations <= 8,
            "one decoded body owns bounded text state, got {allocations}"
        );
    }
}

#[test]
fn a_fix_message_read_from_a_line_costs_what_its_pairs_cost() {
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

/// `rows` lines of the shape a bridge writes - [`fix_packed_line`]'s
/// occurrence and a trailing text value - each holding one Latin-1 letter.
///
/// One high byte per line, because the declared read below is measured
/// against this same text: a line the transport decoded must cost exactly
/// what the same line arriving as UTF-8 costs, and an all-ASCII line would
/// not exercise the transcode at all.
fn bridge_lines(rows: usize) -> String {
    let occurrence = String::from_utf8(fix_packed_line(4)).expect("the bridge shape is ASCII");
    let mut text = String::new();
    for index in 0..rows {
        let _ = writeln!(text, "{occurrence}|TEXT=Z\u{fc}rich {index:04}");
    }
    text
}

/// What reading `rows` lines through [`read_text_lines`] allocates.
fn text_lines_cost(source: &Buffer, rows: usize) -> usize {
    let options = TextOptions::new();
    // A buffer builds the location it answers `url` with once, on the first
    // ask; that is the handle's own first read, two allocations, and not
    // the reader's, so it is asked for before the count.
    black_box(yggdryl::IOBase::url(source));
    let (allocations, read) = counted(|| {
        read_text_lines(black_box(source), black_box(&options))
            .expect("a reader")
            .fold(0, |seen, line| {
                line.expect("a line");
                seen + 1
            })
    });
    assert_eq!(read, rows, "every line was read");
    allocations
}

#[test]
fn located_lines_render_and_project_one_shared_crosscode() {
    for rows in [1_usize, 64, 1_024] {
        let source = Buffer::from_bytes(bridge_lines(rows).into_bytes())
            .with_media_type(MediaType::from_str("text/plain").expect("a media type"));
        let expected = yggdryl::IOBase::url(&source)
            .expect("a buffer identity")
            .to_string();
        let held = read_text_lines(&source, &TextOptions::new())
            .expect("a reader")
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("located lines");

        // The first event-column ask renders the URL exactly once for the
        // reader. More rows clone its shared `Str`; they do not allocate one
        // URL String or one scalar string each.
        let (allocations, projected) = counted(|| {
            let mut projected = 0;
            for line in &held {
                match line
                    .event_fact(EventColumn::CrossCode)
                    .expect("a crosscode reading")
                {
                    Some(Scalar::String(code)) => {
                        black_box(code);
                        projected += 1;
                    }
                    _ => panic!("a located line has a string crosscode"),
                }
            }
            projected
        });
        assert_eq!(projected, rows);
        assert_eq!(
            allocations, 2,
            "crosscode projection did not render exactly one shared value for {rows} rows"
        );

        assert!(held.iter().all(|line| line.get_crosscode() == expected));
        let shared = held[0].get_crosscode().as_ptr();
        assert!(
            held.iter()
                .all(|line| line.get_crosscode().as_ptr() == shared),
            "one read renders one shared crosscode"
        );
        free("projecting a warmed located-line crosscode", || {
            for line in &held {
                black_box(
                    line.event_fact(EventColumn::CrossCode)
                        .expect("a crosscode reading"),
                );
            }
        });

        // A line whose source changes owns a new cache and identity; its
        // siblings keep the reader's original shared value.
        let mut changed = held[0].clone();
        let original_uuid = changed.get_curruuid();
        let replacement = Arc::new(
            yggdryl::Uri::from_str("file:///replacement/location.log")
                .expect("a replacement identifier"),
        );
        changed.set_sourceuri(Some(Arc::clone(&replacement)));
        assert_eq!(changed.get_crosscode(), replacement.to_string());
        assert_ne!(changed.get_curruuid(), original_uuid);
        assert_eq!(held[0].get_crosscode(), expected);
        changed.set_sourceuri(None);
        assert_eq!(changed.get_crosscode(), "");
    }
}

/// What `owned_handle`'s copy of a buffer costs, by how many rows it holds.
///
/// A text read over a buffer with no location re-opens it as a copy, staged
/// through the memory filesystem, and that staging grows with the object:
/// twenty-three allocations for anything under one 64 KiB window, three
/// more for the 114 KiB that 1 024 rows are. It is measured beside the read
/// and taken off it, because it is `main`'s and the transport's, not the
/// reader's: the reader's own cost is what is left, and that is linear.
const OWNED_COPY_COSTS: [(usize, usize); 2] = [(16, 23), (1_024, 26)];

/// What the read itself costs past the copy: nine, and nothing a line.
///
/// Seven of the nine are built before a byte is read - the cursor over the
/// owned handle and the transport boxed around it, the options and the
/// location each shared once, and the splitter's window as a vector and as
/// the shared box that seals it - and two on the first pull, where the
/// transport opens: the fetch buffer and the box the coding chain ends in.
///
/// Nothing a line, because a line is not a thing that is built: the window
/// is the page, and a line is the range of it the splitter cut, so the
/// header off its front, the strips off its edges and the byte limit off
/// its tail move two offsets and copy nothing. With the copy, the assertion
/// below counts 37 for 16 rows and the same 14 over the copy for 1 024 -
/// after the two the buffer's first `url` costs, which [`text_lines_cost`]
/// asks for before the counter is armed and which are in neither number.
///
/// Five of the fourteen are the nineteen event columns the plan compiles once
/// per read: the two identity lists, the names' map and the state's own type
/// allocate as the columns are planned, and nothing of them per line.
///
/// A read now shares what it was addressed by rather than where that
/// resolves to, and the count did not move: the location is a narrowing of
/// the identifier rather than a second value beside it, so the read still
/// holds one reference-counted source and a row still clones one handle.
const TEXT_LINES_ONCE: usize = 14;

/// What a reader that keeps its lines pays on top: two per window it had to
/// leave behind.
///
/// A window a line is a range of cannot be written over, so the refill that
/// finds one takes a fresh window - one vector and one shared box - and
/// moves the open tail into it. That is the whole price of retention, and it
/// is per window of the object rather than per line of it: a caller holding
/// a million lines of a megabyte holds sixteen pages, not a million.
const TEXT_LINES_RETAINED_PER_WINDOW: usize = 2;

/// Initializes the shared datatype projection behind the event clocks before
/// measuring a reader. That cache is process-global rather than a cost of one
/// read, and the result must not depend on which allocation test ran first.
fn warm_text_event_schema() {
    black_box(
        TextOptions::new()
            .source_field()
            .expect("the default text event schema"),
    );
}

/// What a declared `windows-1252` read costs over the UTF-8 read of the same
/// lines: the transport, and nothing a line.
///
/// Three for anything under one window - the reader's two 64 KiB buffers and
/// the box around the reader - and one more for 1 024 rows, where the first
/// chunk decodes to more than the 64 KiB its decoded buffer was reserved at
/// and the buffer grows once. Once per reader, whatever the object's length
/// past that: the buffer keeps its capacity across chunks.
const DECLARED_COSTS: [(usize, usize); 2] = [(16, 3), (1_024, 4)];

#[test]
fn reading_text_lines_costs_a_constant_and_nothing_a_line() {
    warm_text_event_schema();
    for (rows, copy) in OWNED_COPY_COSTS {
        let text = bridge_lines(rows);
        let source = Buffer::from_bytes(text.into_bytes())
            .with_media_type(MediaType::from_str("text/plain").expect("a media type"));
        let (staged, _) = counted(|| {
            let mut staged = Buffer::new();
            yggdryl::IOBase::copy_into(black_box(&source), &mut staged).expect("a copy");
            black_box(staged);
        });
        assert_eq!(staged, copy, "the owned copy of {rows} rows");
        assert_eq!(
            text_lines_cost(&source, rows),
            copy + TEXT_LINES_ONCE,
            "reading {rows} UTF-8 rows"
        );
    }
}

#[test]
fn keeping_every_line_costs_its_windows_and_not_its_lines() {
    warm_text_event_schema();
    // The other half of the claim above. A reader that drops each line lets
    // the splitter write its window over again, so the count is flat; one
    // that keeps them cannot, and what it pays is a window at a time.
    for rows in [16_usize, 1_024] {
        let text = bridge_lines(rows);
        let windows = text.len().div_ceil(yggdryl::DEFAULT_STREAM_BATCH_SIZE);
        let source = Buffer::from_bytes(text.into_bytes())
            .with_media_type(MediaType::from_str("text/plain").expect("a media type"));
        // The handle's own first read, as [`text_lines_cost`] takes it.
        black_box(yggdryl::IOBase::url(&source));
        let options = TextOptions::new();
        let (allocations, held) = counted(|| {
            // Sized up front, so the only vector growing here is the
            // splitter's own and the count is the reader's alone.
            let mut held = Vec::with_capacity(rows);
            for line in read_text_lines(black_box(&source), black_box(&options)).expect("a reader")
            {
                held.push(line.expect("a line"));
            }
            held
        });
        assert_eq!(held.len(), rows, "every line was read");
        assert_eq!(
            allocations,
            OWNED_COPY_COSTS
                .iter()
                .find(|(at, _)| *at == rows)
                .map_or(0, |(_, copy)| *copy)
                + TEXT_LINES_ONCE
                + 1
                + TEXT_LINES_RETAINED_PER_WINDOW * windows,
            "keeping {rows} rows across {windows} windows"
        );
        // Every body is still a range of a page, and the pages are the
        // windows: far fewer than the lines that name them.
        let pages = held
            .iter()
            .filter_map(|line| line.body_bytes().page().map(Arc::as_ptr))
            .collect::<std::collections::HashSet<_>>();
        assert!(
            pages.len() <= windows,
            "{rows} rows named {} pages across {windows} windows",
            pages.len()
        );
    }
}

#[test]
fn a_declared_charset_costs_its_transport_and_nothing_a_line() {
    // The claim the charset layer makes for the transport: nothing per line. The
    // same lines, once as the UTF-8 they are and once as windows-1252 under
    // a handle that declares it, differ by the transport alone.
    for (rows, each) in DECLARED_COSTS {
        let text = bridge_lines(rows);
        let utf8 = Buffer::from_bytes(text.clone().into_bytes())
            .with_media_type(MediaType::from_str("text/plain").expect("a media type"));
        let declared = Buffer::from_bytes(
            Charset::Cp1252
                .encode(&text)
                .expect("windows-1252 holds it")
                .into_owned(),
        )
        .with_media_type(
            MediaType::from_str("text/plain;charset=windows-1252").expect("a media type"),
        );
        assert_eq!(
            text_lines_cost(&declared, rows) - text_lines_cost(&utf8, rows),
            each,
            "the transport over {rows} declared rows"
        );
    }
}

/// The charsets that agree with US-ASCII, which is what lets a decode borrow.
const ASCII_COMPATIBLE: [Charset; 8] = [
    Charset::Utf8,
    Charset::Ascii,
    Charset::Latin1,
    Charset::Latin2,
    Charset::Latin9,
    Charset::Cp1252,
    Charset::Cp437,
    Charset::MacRoman,
];

#[test]
fn an_ascii_payload_is_read_in_any_charset_without_allocating() {
    // The claim `yggdryl::charset` makes under its `# Borrowing` heading: a
    // legacy export is mostly ASCII, and the ASCII part must cost a borrow.
    // Several sizes, because one buffer could be short enough to hide a copy.
    for rows in [1_usize, 16, 1_024] {
        let text = "symbol,price\nAAPL,187.23\n".repeat(rows);
        let payload = text.clone().into_bytes();
        for charset in ASCII_COMPATIBLE {
            free(&format!("decoding {rows} ASCII rows as {charset}"), || {
                let decoded = charset
                    .decode(black_box(payload.as_slice()))
                    .expect("US-ASCII under every charset here");
                assert!(matches!(decoded, std::borrow::Cow::Borrowed(_)));
                black_box(decoded);
            });
            free(&format!("encoding {rows} ASCII rows as {charset}"), || {
                let encoded = charset
                    .encode(black_box(text.as_str()))
                    .expect("US-ASCII under every charset here");
                assert!(matches!(encoded, std::borrow::Cow::Borrowed(_)));
                black_box(encoded);
            });
        }
    }
}

#[test]
fn resolving_a_charset_name_allocates_nothing() {
    // Intake runs once per read, but it runs on every read; a name that
    // allocated to resolve would be a cost on the first byte of every file.
    for name in ["utf-8", "UTF-8", "  windows-1252 ", "latin1", "cp437"] {
        free(&format!("resolving {name:?}"), || {
            black_box(Charset::from_str(black_box(name)).expect("a known charset"));
        });
    }
    free("reading a declared charset off a media type", || {
        black_box(Charset::from_media_type(black_box(&MediaType::default())));
    });
}

#[test]
fn a_transcode_pays_for_the_text_it_builds() {
    // The borrow above is only meaningful beside the case that does not
    // borrow: bytes that are not already UTF-8 become a string that is.
    let wire = Charset::Cp1252
        .encode("symbol,désk\nAAPL,€1\n")
        .expect("windows-1252 holds it")
        .into_owned();
    let (allocations, decoded) = counted(|| {
        Charset::Cp1252
            .decode(black_box(wire.as_slice()))
            .expect("windows-1252")
            .into_owned()
    });
    assert!(
        allocations > 0,
        "a transcode reported a borrow of bytes it does not own"
    );
    assert_eq!(decoded, "symbol,désk\nAAPL,€1\n");
}

/// One column of `n` values of `width` bytes, every one of them US-ASCII.
fn ascii_cells(count: usize, width: usize) -> Scalar {
    Scalar::from_sequence((0..count).map(|index| Scalar::from(format!("{index:0width$}"))))
}

#[test]
fn a_string_column_is_built_into_one_buffer_whatever_its_charset() {
    // The write path measures the whole payload with `Charset::encoded_len`
    // before it builds a byte of it, so the cost of a column is the buffers it
    // publishes and nothing per row. Before that it encoded each cell into its
    // own `Vec<u8>` - even for an all-ASCII cell, where the encode borrows -
    // and the count grew with the row count.
    for dtype in [
        DataType::cp1252(),
        DataType::large_cp1252(),
        DataType::utf8(),
    ] {
        let field = dtype.clone().nullable_field("value");
        let mut counts = Vec::new();
        for rows in [16_usize, 1_024, 16_384] {
            let column = ascii_cells(rows, 32);
            let (allocations, _) = counted(|| {
                yggdryl::arrow::array_from_value(black_box(&field), black_box(&column))
                    .expect("a string column")
            });
            counts.push(allocations);
        }
        assert!(
            counts.windows(2).all(|pair| pair[0] == pair[1]),
            "{dtype} built {counts:?} allocations at 16, 1024 and 16384 rows; \
             a column's cost must not grow with its rows"
        );
    }
}

#[test]
fn encoded_variants_build_arrow_columns_without_per_row_allocations() {
    let field = DataType::Variant.nullable_field("value");
    let encoded = Scalar::Variant(variant_object(4).into_variant().unwrap());
    let mut counts = Vec::new();
    for rows in [16_usize, 1_024, 16_384] {
        let column = Scalar::from_sequence((0..rows).map(|_| encoded.clone()));
        let build = || yggdryl::arrow::array_from_value(&field, &column).unwrap();
        drop(build());
        let (allocations, array) = counted(build);
        assert_eq!(array.len(), rows);
        counts.push(allocations);
    }
    eprintln!("encoded_variant_arrow_columns: {counts:?} allocations at 16, 1024, 16384 rows");
    assert!(
        counts.windows(2).all(|pair| pair[0] == pair[1]),
        "encoded Variant columns must allocate their final buffers, not buffers per row: {counts:?}"
    );
}

#[test]
fn an_ascii_payload_in_a_declared_charset_column_transcribes_by_borrowing() {
    // Every charset with a byte to transcribe is still ASCII-compatible, so an
    // all-ASCII payload is already its own answer. `decode`, `decode_lossy`
    // and `encode` all take this borrow at their first line; `transcribe` used
    // to be the one door that did not, and allocated for text it never touched.
    free("transcribing an all-ASCII payload", || {
        black_box(Charset::Cp1252.transcribe(black_box(b"symbol,price")));
    });
    free(
        "transcribing a short all-ASCII payload into compact storage",
        || {
            black_box(Charset::Cp1252.transcribe_smol(black_box(b"AAPL")));
        },
    );
}

#[test]
fn a_short_transcoded_cell_fits_the_inline_buffer() {
    // A payload that transcribes to twenty-three bytes or fewer is the case
    // compact storage exists for, and it reaches the heap only if something
    // built an intermediate first. `0x81` is unassigned in windows-1252 and
    // reads as its ISO 8859-1 scalar, so this is the transcoding path.
    free("transcribing a short cell into compact storage", || {
        black_box(Charset::Cp1252.transcribe_smol(black_box(b"ok\x81 caf\xe9")));
    });
}

#[test]
fn a_long_transcoded_cell_costs_its_buffer_and_its_handle() {
    // Above the inline buffer there is no saving to claim: the text is built
    // once into a sized buffer and copied once into the shared handle, and
    // `String` and `Arc<str>` have different layouts, so no conversion between
    // them is free. Two is the floor, and this pins it as the floor rather
    // than leaving a later change room to quietly reach three.
    let mut wire = b"caf\xe9 ".repeat(12);
    wire.truncate(60);
    costs("transcribing a long cell into compact storage", 2, || {
        black_box(Charset::Cp1252.transcribe_smol(black_box(wire.as_slice())));
    });
}

#[test]
fn default_aliases_allocation_profile_is_idempotent() {
    // Before direct reindexing, the first pass made 313,853,260 allocations
    // by refreshing the catalog for each of 170 fields; the repeat still made
    // 7,867,934. One alias registration now rebuilds the catalog once, so the
    // first cap permits one refresh with 37% fixture headroom. An already
    // aliased registry skips that refresh; its cap leaves room for spelling
    // generation but remains below the cost of cloning the full catalog.
    const FIRST_MAX_ALLOCATIONS: usize = 2_500_000;
    const REPEATED_MAX_ALLOCATIONS: usize = 4_096;
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let folder = yggdryl::local::LocalFolder::new(root).expect("the local seed path");
    let loaded_at = Instant::now();
    let registry = FixRegistry::from_handle(&folder).expect("the committed dictionary loads");
    let load_elapsed = loaded_at.elapsed();

    let first_input = registry.clone();
    let first_at = Instant::now();
    let (first_allocations, registered) = counted(|| {
        first_input
            .with_default_aliases()
            .expect("the committed aliases register")
    });
    let first_elapsed = first_at.elapsed();
    assert!(
        first_allocations <= FIRST_MAX_ALLOCATIONS,
        "default aliases first pass made {first_allocations} allocations; \
         the one catalog refresh budget is {FIRST_MAX_ALLOCATIONS}"
    );
    assert_eq!(
        registered
            .field_by_name("askprice")
            .expect("AskPrice resolves")
            .name(),
        "offerpx",
        "the default aliases retain their canonical owner"
    );

    let repeated_input = registered.clone();
    let repeated_at = Instant::now();
    let (repeated_allocations, repeated) = counted(|| {
        repeated_input
            .with_default_aliases()
            .expect("registering aliases twice succeeds")
    });
    let repeated_elapsed = repeated_at.elapsed();
    assert!(
        repeated_allocations <= REPEATED_MAX_ALLOCATIONS,
        "default aliases repeat made {repeated_allocations} allocations; \
         the no-op budget is {REPEATED_MAX_ALLOCATIONS}"
    );
    assert_eq!(
        repeated
            .field_by_name("askprice")
            .expect("AskPrice still resolves")
            .name(),
        "offerpx"
    );
    eprintln!(
        "default_aliases: load={load_elapsed:?}; first={first_allocations} allocations, \
         {first_elapsed:?}; repeated={repeated_allocations} allocations, {repeated_elapsed:?}"
    );
}

#[test]
fn a_registry_whose_derivations_refuse_compiles_once_and_refuses_every_door() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let folder = yggdryl::local::LocalFolder::new(root).expect("the local seed path");
    let mut registry = FixRegistry::from_handle(&folder).expect("the committed dictionary loads");
    let mut gross = registry.field_by_tag(381).expect("GrossTradeAmt").clone();
    gross
        .as_fix_mut()
        .set_derivation(&"lastqty * nosuchfield".parse().expect("a term"))
        .expect("stored");
    registry.update(gross).expect("the text is a term");
    let registry = Arc::new(registry);
    let codec = FixCodec::new(Arc::clone(&registry));
    let line = b"8=FIX.4.4|35=8|37=A|48=US0378331005|22=4|100=XNAS|150=F|10=0|";
    // A parse enriches, so a derivation the registry cannot bind refuses the
    // read itself. The first ask pays the compile up to the refusal; the
    // registry keeps the refusal as it would keep the compiled list, so
    // every ask after it pays the same and none of them pays a compile.
    let refuse = || {
        codec
            .parse_line(black_box(line))
            .and_then(|mut messages| messages.next().expect("one frame"))
            .expect_err("the derivation refuses")
    };
    let (cold, refused) = counted(refuse);
    assert!(refused.to_string().contains("grosstradeamt"), "{refused}");
    let (warm, _) = counted(refuse);
    let (again, _) = counted(refuse);
    assert!(
        cold - warm > 1_000,
        "the refused compile is a thousand allocations and is paid once: \
         {cold} cold, {warm} warm"
    );
    assert_eq!(warm, again, "the refusal is kept, not recompiled");
}
#[test]
fn instrument_codes_construct_and_classify_without_allocating() {
    use yggdryl::{BloombergCode, CfiCode, CusipCode, FIGICode, IsinCode, SedolCode};
    free("long Bloomberg validation", || {
        assert!(BloombergCode::is_canonical(
            "AAPL US Equity Long Identifier"
        ));
    });
    free("ISIN construction", || {
        std::hint::black_box(IsinCode::new("us0378331005").unwrap());
    });
    free("CUSIP construction", || {
        std::hint::black_box(CusipCode::new("037833100").unwrap());
    });
    free("SEDOL construction", || {
        std::hint::black_box(SedolCode::new("b0swjx3").unwrap());
    });
    let figi = FIGICode::new("BBG000BLNQ16").unwrap();
    free("FIGI construction", || {
        black_box(FIGICode::new("bbg000blnq16").unwrap());
    });
    free("FIGI clone", || {
        black_box(figi.clone());
    });
    free("CFI validation", || {
        assert!(CfiCode::is_classified("ESVUFR"));
    });
    free("CFI merging", || {
        assert_eq!(
            CfiCode::merged("ESXXXX", "ESVUFR").as_deref(),
            Some("ESVUFR")
        );
    });
    free("CFI inference", || {
        assert_eq!(CfiCode::coarse('E', Some('S')).as_deref(), Some("ESXXXX"));
    });
}
