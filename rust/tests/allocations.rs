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

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::fmt;
use std::fmt::Write as _;
use std::hint::black_box;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use std::sync::Arc;

#[path = "support/excel_package.rs"]
mod excel_package;

use smol_str::SmolStr;
use yggdryl::SerieValue as _;
use yggdryl::expression::Term;
use yggdryl::graph::{
    BookEvent, BookIterator, BookRef, CandleIterator, CandleOptions, Element, ElementColumn, Event,
    ExecutionEvent, Market, MarketData, MdUpdateAction, Operation, OrderEvent, QuoteEvent,
    TradeEvent,
};
use yggdryl::holder::Buffer;
use yggdryl::text::{TextBytes, TextEntries, TextLine, TextOptions, read_text_lines};
use yggdryl::xmla::Rowset;
use yggdryl::{
    ArrowCastOptions, ArrowCastPlan, Charset, ChunkedSerie, DataType, DataTypeId, Decimal, Field,
    FieldPath, FieldRecord, FieldScalar, FixCode, FixCodec, FixId, FixMsg, FixRegistry, IdSource,
    IdType, Identifier, Int64, MediaType, MimeType, PythonKind, PythonMetadata, Scalar, Serie,
    Side, State, TimeUnit, Timezone, Value, Variant, Version,
};
use yggdryl::{
    Bytes, INLINE_BYTES, INLINE_CAPACITY, Str, StringType, StructType, UncheckedFieldScalar, Uuid,
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

#[cfg(feature = "internals")]
#[test]
fn excel_ordered_sum_push_and_finish_allocate_nothing_per_value() {
    use yggdryl::internals::excel_formula_aggregate::Accumulator;

    let values = [1.0, -f64::from_bits(0x3fef_ffff_ffff_fffe), f64::EPSILON];
    for rows in [64, 4_096] {
        let (allocations, sum) = counted(|| {
            let mut accumulator = Accumulator::default();
            for index in 0..rows {
                accumulator
                    .push_number(black_box(values[index % values.len()]))
                    .unwrap();
            }
            accumulator.finish_sum().unwrap()
        });
        black_box(sum);
        assert_eq!(allocations, 0, "{rows} ordered numeric values allocated");
    }
}

#[test]
fn expression_calendar_parts_allocate_nothing_per_row() {
    let schema = DataType::from(
        StructType::from_fields([DataType::datetime64(TimeUnit::Second, Timezone::UTC)
            .unwrap()
            .required_field("stamp")])
        .unwrap(),
    )
    .required_field("row");
    let row =
        Scalar::from_sequence([Scalar::datetime64(0, TimeUnit::Second, Timezone::UTC).unwrap()]);
    let bound = "year(stamp)"
        .parse::<Term>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert_eq!(bound.eval(&row).unwrap(), Scalar::from(1970));
    for rows in [64, 4_096] {
        let (allocations, ()) = counted(|| {
            for _ in 0..rows {
                black_box(bound.eval(black_box(&row)).unwrap());
            }
        });
        assert_eq!(
            allocations, 0,
            "calendar extraction allocated over {rows} rows"
        );
    }
}

/// Count the allocations `work` performs, and return them with its answer.
fn counted<T>(work: impl FnOnce() -> T) -> (usize, T) {
    let guard = COUNTING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // The machine's name is read once per process, by whichever surface asks
    // first; read here, outside every count, it is never any one test's cost.
    black_box(yggdryl::HOSTNAME.len());
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
fn version_allocates_only_a_patch_past_the_inline_capacity() {
    free("parsing an inline version", || {
        black_box("5.0.10".parse::<Version>().expect("a static version"));
    });
    for text in [
        "5.0sp250",
        "005.000Sp00250",
        "5.0SP256",
        "65535.65535sP65535",
    ] {
        free("parsing a compact FIX version", || {
            black_box(text.parse::<Version>().expect("a static FIX version"));
        });
    }
    for (text, patch) in [
        ("1.2-rc1", "-rc1"),
        ("1.2SP2_EP240", "SP2_EP240"),
        ("1.2.65536", "65536"),
        ("1.2.00065536", "65536"),
        ("1.2ÃƒÂ§Ã¢â‚¬Â¢Ã…â€™", "ÃƒÂ§Ã¢â‚¬Â¢Ã…â€™"),
    ] {
        let expected = Version::new(1, 2, Some(patch));
        free("parsing a version with a qualified patch", || {
            assert_eq!(
                black_box(text)
                    .parse::<Version>()
                    .expect("a qualified version"),
                expected
            );
        });
    }
    // The patch is one `SmolStr`: held inline up to 23 bytes, and one shared
    // allocation past them, which a clone shares rather than copies.
    for (patch_bytes, each) in [(16, 0), (23, 0), (24, 1), (240, 1), (4096, 1)] {
        let text = format!("1.2-{}", "x".repeat(patch_bytes - 1));
        let expected = text.parse::<Version>().expect("a generated qualifier");
        assert_eq!(expected.patch().map(str::len), Some(patch_bytes));
        costs(
            &format!("parsing a {patch_bytes}-byte version patch"),
            each,
            || {
                assert_eq!(
                    black_box(text.as_str())
                        .parse::<Version>()
                        .expect("a generated qualifier"),
                    expected
                );
            },
        );
        free(
            &format!("cloning a {patch_bytes}-byte version patch"),
            || {
                black_box(expected.clone());
            },
        );
    }

    let left = "5.0.2".parse::<Version>().expect("a static version");
    let right = "5.0.10".parse::<Version>().expect("a static version");
    free("comparing inline versions", || {
        black_box(left.cmp(&right));
    });
    let qualified = "1.2-rc2".parse::<Version>().expect("a qualified version");
    let later = "1.2-rc10".parse::<Version>().expect("a qualified version");
    free("comparing qualified versions", || {
        black_box(qualified.cmp(&later));
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
fn market_following_allocates_nothing_for_an_inherited_ticker() {
    let mut previous = OrderEvent::at(1);
    previous.finalize();
    let next = OrderEvent::at(2);
    let (baseline, _) = counted(|| next.with_previous(&previous).unwrap());

    previous.set_ticker(Some(SmolStr::new("AAPL")), true);
    previous.finalize();
    let next = OrderEvent::at(2);
    let (inherited, next) = counted(|| next.with_previous(&previous).unwrap());
    assert_eq!(next.get_ticker(), Some("AAPL"));
    // A ticker is a `SmolStr`: one of a symbol's length lives inline, so
    // inheriting it copies and allocates nothing where the owned `String`
    // it replaced cost one allocation.
    assert_eq!(inherited, baseline, "an inherited inline ticker is a copy");

    let mut next = OrderEvent::at(2);
    next.set_ticker(Some(SmolStr::new("MSFT")), true);
    let (stated, next) = counted(|| next.with_previous(&previous).unwrap());
    assert_eq!(next.get_ticker(), Some("MSFT"));
    assert_eq!(stated, baseline, "a stated ticker needs no clone");
}

#[test]
fn market_event_identity_refresh_and_finalization_allocate_nothing() {
    let mut event = OrderEvent::at(1_700_000_000_000_000_000);
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
    let mut parties = DataType::serie(item).nullable_field("Parties");
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
                held.get_currunix(),
                held.get_currhashcode(),
                held.get_curruuid(),
                held.get_crosscode(),
            ));
        });
    }
}

#[test]
fn direct_fix_operation_conversion_needs_no_intermediate_allocation() {
    let codec = FixCodec::new(Arc::new(fix_registry(0)));
    for wire in [b"35=D|55=AAPL|".as_slice(), b"35=S|55=AAPL|"] {
        let message = codec.parse_line(wire).unwrap().next().unwrap().unwrap();
        let (allocations, operation) =
            counted(|| yggdryl::FixMsg::into_market_leaf(message).unwrap());
        assert_eq!(operation.get_ticker(), Some("AAPL"));
        assert_eq!(
            allocations, 0,
            "a direct operation moves its existing holder"
        );
    }
}

#[test]
fn a_leaf_moves_into_and_out_of_market_data_without_allocating() {
    let mut event = OrderEvent::at(1);
    event.set_crosscode("ORDER-1".to_owned());
    event.finalize();
    let mut operation = Some(event);
    let (into_value, value) =
        counted(|| MarketData::from(operation.take().expect("one operation")));
    assert_eq!(into_value, 0, "leaf to value allocated");

    let mut value = Some(value);
    let (into_leaf, operation) =
        counted(|| OrderEvent::try_from(value.take().expect("one value")).unwrap());
    assert_eq!(into_leaf, 0, "value to leaf allocated");

    // An event drops its clocks into the element it states, and an element
    // dated again is an event: both moves keep the one holder.
    let mut operation = Some(operation);
    let (into_element, element) =
        counted(|| operation.take().expect("one operation").into_element());
    assert_eq!(into_element, 0, "event to element allocated");

    let mut element = Some(element);
    let (at, operation) = counted(|| element.take().expect("one element").at(2));
    assert_eq!(at, 0, "element to event allocated");
    black_box(operation);
}

fn allocation_trade_parts(executions: usize) -> (OrderEvent, Vec<ExecutionEvent>) {
    let mut root = OrderEvent::at(1);
    root.set_crosscode("ALLOC-TRADE".to_owned());
    root.set_ticker(Some(SmolStr::new("ALLOC")), true);
    root.set_state(State::read("Filled").expect("the shipped filled state"));
    root.finalize();
    let executions = (0..executions)
        .rev()
        .map(|index| {
            let mut event = ExecutionEvent::at(1);
            event.set_crosscode(format!("ALLOC-EXEC-{index:04}"));
            event.set_ticker(Some(SmolStr::new("ALLOC")), true);
            event.set_side(
                Side::read(if index % 2 == 0 { "Buy" } else { "Sell" }).unwrap(),
                true,
            );
            event.set_state(State::read("Filled").expect("the shipped filled state"));
            event.finalize();
            event
        })
        .collect();
    (root, executions)
}

#[test]
fn trade_construction_does_not_allocate_per_execution() {
    let (root, executions) = allocation_trade_parts(1);
    let (shallow_allocations, shallow) = counted(|| TradeEvent::from_parts(&root, executions));
    let shallow = shallow.expect("the shallow trade");

    let (root, executions) = allocation_trade_parts(128);
    let (deep_allocations, deep) = counted(|| TradeEvent::from_parts(&root, executions));
    let deep = deep.expect("the deep trade");
    assert_eq!(shallow.executions().len(), 1);
    assert_eq!(deep.executions().len(), 128);
    assert!(
        deep_allocations <= shallow_allocations + 4,
        "constructing 128 executions allocated {deep_allocations} times but one execution allocated {shallow_allocations} times"
    );
    black_box((shallow, deep));
}

/// `count` books of `ALLOC` inside one minute, each carrying one fill of
/// 5 whose `EXECID` is `exec(index)` - thirty-odd bytes, past `SmolStr`'s
/// inline width, so holding one the fold has not seen allocates.
fn allocation_candle_books(count: usize, exec: impl Fn(usize) -> String) -> Vec<BookEvent> {
    (0..count)
        .map(|index| {
            let unix = i64::try_from(index).expect("a small corpus") + 1;
            let mut fill = ExecutionEvent::at(unix);
            fill.set_crosscode(format!("ALLOC-FILL-{index:04}"));
            fill.set_ticker(Some(SmolStr::new("ALLOC")), true);
            fill.set_side(Side::read("Buy").expect("the shipped buy side"), true);
            fill.set_lastqty(Some(Decimal::from_int(5)), true);
            fill.set_state(State::read("Filled").expect("the shipped filled state"));
            assert!(
                fill.insert_identifier(
                    Identifier::new(IdSource::Fix, IdType::ExecId, &exec(index)).unwrap()
                )
                .unwrap()
            );
            fill.finalize();
            let mut book = BookEvent::new(unix, "ALLOC");
            book.add_operations([MarketData::from(fill)])
                .expect("one execution");
            book
        })
        .collect()
}

/// What folding `books` into minute candles allocates, the one candle it
/// answers included, and that candle's volume.
fn allocation_candle_fold(books: Vec<BookEvent>) -> (usize, Decimal) {
    let options = CandleOptions::from_spelling("1m").expect("a shipped spelling");
    let (allocations, candles) = counted(|| {
        CandleIterator::new(books.into_iter().map(Ok), options)
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("a sorted stream")
    });
    assert_eq!(candles.len(), 1, "one bucket, one cross code");
    (allocations, candles[0].volume)
}

#[test]
fn a_candle_fold_allocates_per_trade_and_never_per_statement() {
    // Every book restates one fill: its name is looked up borrowed, so a
    // hundred and twenty-eight statements cost what one does.
    let restated = |_: usize| "ALLOC-EXECUTION-IDENTIFIER-00000000".to_owned();
    let (one, volume) = allocation_candle_fold(allocation_candle_books(1, restated));
    assert_eq!(volume, Decimal::from_int(5));
    let (many, volume) = allocation_candle_fold(allocation_candle_books(128, restated));
    assert_eq!(volume, Decimal::from_int(5), "one trade, counted once");
    assert_eq!(
        many, one,
        "128 statements of one trade allocated {many} times, one statement {one}"
    );

    // Every book states a fill of its own: each new name is held once, and
    // the maps and slots grow by doubling - a logarithm of the trades.
    let fresh = |index: usize| format!("ALLOC-EXECUTION-IDENTIFIER-{index:08}");
    let (one, _) = allocation_candle_fold(allocation_candle_books(1, fresh));
    let (many, volume) = allocation_candle_fold(allocation_candle_books(128, fresh));
    assert_eq!(volume, Decimal::from_int(640), "128 trades of 5");
    assert!(
        many >= one + 127 && many <= one + 127 + 32,
        "128 trades allocated {many} times where one trade allocated {one}"
    );
}

fn allocation_book_operation(
    code: impl Into<String>,
    unix: i64,
    quantity: i64,
    state: &str,
) -> MarketData {
    let mut event = QuoteEvent::at(unix);
    event.set_crosscode(code.into());
    event.set_ticker(Some(SmolStr::new("ALLOC")), true);
    event.set_side(Side::read("Buy").expect("the shipped buy side"), true);
    event.set_price(Some(Decimal::from_int(100)), true);
    event.set_quantity(Some(Decimal::from_int(quantity)), true);
    event.set_state(State::read(state).expect("a shipped state"));
    event.finalize();
    event.into()
}

fn allocation_book(entries: usize) -> BookEvent {
    let mut book = BookEvent::new(1, "ALLOC");
    book.add_operations(
        (0..entries).map(|index| allocation_book_operation(format!("ALLOC-{index}"), 1, 1, "New")),
    )
    .expect("the initial depth");
    book
}

#[test]
fn one_book_iterator_update_clones_depth_only_for_its_output() {
    let overhead = |entries| {
        let initial = (0..entries)
            .map(|index| allocation_book_operation(format!("ALLOC-{index}"), 1, 1, "New"))
            .collect::<Vec<_>>();
        let update = allocation_book_operation("ALLOC-0", 2, 2, "Replaced");
        let mut books = BookIterator::new(initial.into_iter().chain([update]), 0).unwrap();
        assert_eq!(books.next().unwrap().unwrap().alive().count(), entries);
        let (allocations, book) = counted(|| books.next().unwrap().unwrap());
        assert_eq!(book.alive().count(), entries);
        let (output, _) = counted(|| book.clone());
        allocations
            .checked_sub(output)
            .expect("one owned output book")
    };
    let shallow = overhead(1);
    let deep = overhead(128);
    // Output owns its depth. Transactional work has the same small fixed
    // allowance as direct BookEvent::add_operations, regardless of resting entries.
    assert!(
        deep <= shallow + 4,
        "iterator overhead allocated {deep} times at depth 128 but {shallow} times at depth 1"
    );
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
    assert_eq!(shallow.alive().count(), 1);
    assert_eq!(deep.alive().count(), 128);

    assert!(
        deep_allocations <= shallow_allocations + 4,
        "one update allocated {deep_allocations} times at depth 128 but {shallow_allocations} times at depth 1"
    );
    black_box((shallow, deep));
}

/// A book of `levels` distinct prices on each side - bids from 1000 down,
/// asks from 1001 up - with eight entries resting at every price: past the
/// four a vector grown by pushing holds in its first allocation, so a limit
/// that grew its identities rather than collecting them shows in the count.
fn allocation_book_levels(levels: usize) -> BookEvent {
    let entry = |side: &str, level: usize, slot: usize| {
        let offset = i64::try_from(level).expect("a small corpus");
        let (name, price) = if side == "Buy" {
            ("B", 1_000 - offset)
        } else {
            ("A", 1_001 + offset)
        };
        let mut event = QuoteEvent::at(1);
        event.set_crosscode(format!("{name}-{level}-{slot}"));
        event.set_ticker(Some(SmolStr::new("ALLOC")), true);
        event.set_side(Side::read(side).expect("a shipped side"), true);
        event.set_price(Some(Decimal::from_int(price)), true);
        event.set_quantity(Some(Decimal::from_int(1 + offset)), true);
        event.set_state(State::read("New").expect("the shipped new state"));
        event.finalize();
        MarketData::from(event)
    };
    let mut book = BookEvent::new(1, "ALLOC");
    book.add_operations(["Buy", "Sell"].into_iter().flat_map(|side| {
        (0..levels).flat_map(move |level| (0..8).map(move |slot| entry(side, level, slot)))
    }))
    .expect("the initial depth");
    book
}

/// Every reading of a book's bests and depth is read off the price levels
/// in place: nothing is built, at two depths.
#[test]
fn book_readings_allocate_nothing() {
    for levels in [8, 128] {
        let book = allocation_book_levels(levels);
        assert_eq!(book.limits(Side::Buy).count(), levels);
        free("best_price", || {
            black_box(black_box(&book).best_price(Side::Buy));
        });
        free("best_quantity", || {
            black_box(black_box(&book).best_quantity(Side::Sell));
        });
        free("is_crossed", || {
            black_box(black_box(&book).is_crossed());
        });
        free("is_locked", || {
            black_box(black_box(&book).is_locked());
        });
        free("spread", || {
            black_box(black_box(&book).spread());
        });
        free("bbo_midpoint", || {
            black_box(black_box(&book).bbo_midpoint());
        });
        free("median_quantity", || {
            black_box(black_box(&book).median_quantity());
        });
        free("imbalance", || {
            black_box(black_box(&book).imbalance(levels));
        });
        free("depth", || {
            black_box(black_box(&book).depth(Side::Buy, levels));
        });
    }
}

/// A side's limits cost one vector each - its entries' identities,
/// collected at their exact count - and nothing per entry, and the
/// `bidlimits` cell a book row states for the side one vector more, which
/// holds them, at two depths.
#[test]
fn book_limits_allocate_one_vector_per_limit() {
    for levels in [8, 128] {
        let book = allocation_book_levels(levels);
        let (allocations, count) = counted(|| book.limits(Side::Buy).map(black_box).count());
        assert_eq!(count, levels);
        assert_eq!(
            allocations, levels,
            "{levels} limits of eight entries each allocated {allocations} times"
        );
        let (allocations, cell) = counted(|| book.limits(Side::Buy).collect::<Vec<_>>());
        assert_eq!(cell.len(), levels);
        assert_eq!(
            allocations,
            levels + 1,
            "the bidlimits cell of {levels} limits allocated {allocations} times"
        );
    }
}

/// One order the market-data read pin reads back: every fact a row hands
/// back owning heap - the cross code, the sources, three identifiers (past
/// the map's inline two), the metadata and a book control - each at an
/// inline width, so a clone owns exactly what the row does. The control's
/// action and position are walk-time and read back as none; the box holding
/// its scope is the same one allocation.
fn allocation_market_order(index: usize) -> MarketData {
    let unix = 1_700_000_000_000_000_000 + i64::try_from(index).expect("a small corpus");
    let mut order = OrderEvent::at(unix);
    order.set_crosscode(format!("ALLOC-ORDER-{index:06}"));
    order.set_seqnum(1 + u64::try_from(index).expect("a small corpus"));
    order.set_price(Some(Decimal::from_int(100)), true);
    order.set_quantity(Some(Decimal::from_int(10)), true);
    order.set_side(Side::read("Buy").expect("the shipped buy side"), true);
    order.set_ticker(Some(SmolStr::new("ALLOC")), true);
    order.set_state(State::read("New").expect("the shipped new state"));
    order.set_srcuuids(vec![Uuid::from_v8(7)]);
    order
        .insert_identifier(
            Identifier::new(IdSource::Fix, IdType::OrderId, &format!("ORDER-{index:06}")).unwrap(),
        )
        .expect("an identifier");
    order
        .insert_identifier(
            Identifier::new(
                IdSource::Fix,
                IdType::MdEntryId,
                &format!("ENTRY-{index:06}"),
            )
            .unwrap(),
        )
        .expect("an identifier");
    order
        .insert_identifier(
            Identifier::new(IdSource::Fix, IdType::ClOrdId, &format!("CL-{index:06}")).unwrap(),
        )
        .expect("an identifier");
    order.set_metadata(
        Some(
            [(SmolStr::new("venue"), SmolStr::new("XNAS"))]
                .into_iter()
                .collect(),
        ),
        true,
    );
    order.set_book(Some(BookRef {
        action: Some(MdUpdateAction::New),
        scope: Some(SmolStr::new("Symbol=ALLOC")),
        position: Some(1),
        entry_px: None,
        entry_size: None,
    }));
    order.finalize();
    MarketData::from(order)
}

/// A `marketdata` row read back costs the value it answers and nothing
/// else: the plan, the landing and the narrowed leaves are the stream's and
/// the batch's, paid before its first row, and every later row reads its
/// cells off the leaves, validates its identities and facts against the
/// rebuilt leaf without rebuilding what it compares, and allocates exactly
/// what a clone of the answer does - at two corpus sizes.
#[test]
fn a_market_data_row_read_allocates_only_what_it_hands_back() {
    for rows in [64, 512] {
        let values: Vec<MarketData> = (0..rows).map(allocation_market_order).collect();
        let (handed, ()) = counted(|| {
            for value in &values {
                black_box(value.clone());
            }
        });
        assert_eq!(handed % rows, 0, "every order owns the same heap");
        let batch = MarketData::arrow_reader(values, Some(rows), None)
            .expect("a reader")
            .next()
            .expect("one batch")
            .expect("the batch");
        let source = yggdryl::arrow::batch_reader(batch.schema(), [batch]);
        let mut read = MarketData::from_arrow_reader(source).expect("the canonical schema");
        black_box(read.next().expect("a first row").expect("the first order"));
        let (allocations, count) = counted(|| {
            read.by_ref().fold(0, |count, value| {
                black_box(value.expect("an order"));
                count + 1
            })
        });
        assert_eq!(count, rows - 1);
        assert_eq!(
            allocations,
            handed / rows * (rows - 1),
            "reading {count} rows allocated {allocations} times, but the orders they answer own {} allocations each",
            handed / rows
        );
    }
}

/// An order that also states an ISIN and two FX rates: the `isincode` and
/// `fxrates` columns a row read reads beside every other.
fn allocation_market_order_with_rates(index: usize) -> MarketData {
    let MarketData::OrderEvent(mut order) = allocation_market_order(index) else {
        unreachable!("the allocation order is an order event")
    };
    order
        .insert_securityid(
            Identifier::new(IdSource::Base, IdType::Isin, "US0378331005").expect("an ISIN"),
        )
        .expect("a plain holder");
    order.set_fxrates(
        ["EUR", "JPY"]
            .into_iter()
            .map(|target| {
                (
                    yggdryl::Ccy::new(target).expect("a currency"),
                    Decimal::from_int(2),
                )
            })
            .collect(),
        true,
    );
    order.finalize();
    MarketData::from(order)
}

/// A row stating an ISIN and FX rates reads as the row without them does:
/// the value it answers and nothing else - the `isincode` compared in place
/// against `securityids`, the rates read into the one map the answer
/// holds and checked against it without a copy - at two corpus sizes.
#[test]
fn a_market_data_row_with_rates_reads_only_what_it_hands_back() {
    for rows in [64, 512] {
        let values: Vec<MarketData> = (0..rows).map(allocation_market_order_with_rates).collect();
        let (handed, ()) = counted(|| {
            for value in &values {
                black_box(value.clone());
            }
        });
        assert_eq!(handed % rows, 0, "every order owns the same heap");
        let batch = MarketData::arrow_reader(values, Some(rows), None)
            .expect("a reader")
            .next()
            .expect("one batch")
            .expect("the batch");
        let source = yggdryl::arrow::batch_reader(batch.schema(), [batch]);
        let mut read = MarketData::from_arrow_reader(source).expect("the canonical schema");
        black_box(read.next().expect("a first row").expect("the first order"));
        let (allocations, count) = counted(|| {
            read.by_ref().fold(0, |count, value| {
                black_box(value.expect("an order"));
                count + 1
            })
        });
        assert_eq!(count, rows - 1);
        assert_eq!(
            allocations,
            handed / rows * (rows - 1),
            "reading {count} rows allocated {allocations} times, but the orders they answer own {} allocations each",
            handed / rows
        );
    }
}

/// The key of a ticker's book is the ticker itself, borrowed.
#[test]
fn a_book_crosscode_allocates_nothing_for_a_ticker_input() {
    let order = allocation_market_order(1);
    let (allocations, key) = counted(|| black_box(order.book_crosscode().len()));
    assert_eq!(key, "ALLOC".len());
    assert_eq!(allocations, 0);
}

/// A `marketdata` batch is laid out column by column from the typed
/// leaves: per row it costs the canonical clone its write check compares
/// against and nothing else - no cell is built as a value - so what the
/// columns cost beyond those clones grows with the buffers' doublings and
/// not with the rows, at two corpus sizes.
#[test]
fn a_market_data_batch_write_allocates_per_row_only_its_canonical_check() {
    let overhead = |rows: usize| {
        let values: Vec<MarketData> = (0..rows).map(allocation_market_order).collect();
        let (cloned, ()) = counted(|| {
            for value in &values {
                black_box(value.clone());
            }
        });
        let mut reader = MarketData::arrow_reader(values, Some(rows), None).expect("a reader");
        let (written, batch) = counted(|| reader.next().expect("one batch").expect("the batch"));
        assert_eq!(batch.num_rows(), rows);
        black_box(batch);
        written
            .checked_sub(cloned)
            .expect("a write costs at least its checks")
    };
    let (small, large) = (overhead(64), overhead(512));
    // 448 more rows add under one allocation per four of them: the
    // buffers' doublings, where a cell built per row would add hundreds.
    assert!(
        large < small + (512 - 64) / 4,
        "512 rows cost {large} allocations beyond their checks, against {small} for 64"
    );
}

/// A capture expands into its sorted leaves at a cost per message that does
/// not grow with the capture - no row parsed again, no metadata key built
/// per leaf beyond its own, one sort of the leaves - whether the leaves
/// carry their unmapped fields or not, and carrying them costs something.
#[test]
fn fix_market_operations_cost_is_linear_in_the_leaves() {
    let registry = Arc::new(committed_registry().clone());
    // Orders, quotes, a fill and a two-entry snapshot in turn, a second
    // apart, each stating fields no typed column reads. The quote states its
    // side: a two-sided one is its parse's to split (A13) and no leaf here.
    let capture = |codec: &FixCodec, messages: usize| -> Vec<FixMsg> {
        (0..messages)
            .map(|index| {
                let clock = format!("20260921-10:{:02}:{:02}", index / 60, index % 60);
                let line = match index % 4 {
                    0 => format!(
                        "8=FIX.4.4|35=D|52={clock}|11=C{index}|55=AAPL|54=1|44=100|38=5|40=2|21=1|10=0|"
                    ),
                    1 => format!(
                        "8=FIX.4.4|35=S|52={clock}|117=Q{index}|55=AAPL|54=1|132=99|134=7|133=101|135=8|537=1|10=0|"
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
    free("is_unknown", || {
        let _ = black_box(field.as_iceberg().is_unknown());
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
        ("a decimal", Scalar::decimal128(18_723, 2)),
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
        Scalar::decimal256(yggdryl::i256::from_i128(i128::MIN), -3),
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
        Field::new("ccy", DataType::Ccy, false),
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
    // costs exactly the shared handle. Restating the text as a value of
    // another leaf moves the handle, so the characters are never copied.
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
        let restated = large
            .scalar(black_box(&source).clone())
            .expect("the leaf holds it");
        assert!(std::ptr::eq(
            source.as_str(),
            restated.as_str().expect("a string value")
        ));
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
    // A column's rows and a map's entries state their count, so they are
    // written straight into the storage too, never through a vector first.
    let column = Serie::from_scalars(
        Field::new("count", DataType::Int64, false),
        (0..64_i64).map(Scalar::from),
    )
    .expect("a column");
    costs("building a row from a column's rows", 1, || {
        black_box(Scalar::from_sequence(
            black_box(&column).iter().map(std::borrow::Cow::into_owned),
        ));
    });
    // Twelve entries: past sixteen the duplicate-key check builds a set of
    // its own, which is that check's cost and not the storage's.
    let entries: std::collections::BTreeMap<i64, i64> = (0..12_i64).map(|key| (key, key)).collect();
    costs("building a mapping from a map's entries", 1, || {
        black_box(
            Scalar::from_mapping(
                black_box(&entries)
                    .iter()
                    .map(|(key, value)| (Scalar::from(*key), Scalar::from(*value))),
            )
            .expect("unique keys"),
        );
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
        Serie::from_arrow_array(
            Some(&Field::new("count", DataType::Int64, true)),
            Arc::new(counts),
            ArrowCastOptions::new(),
        )
        .expect("an int64 column"),
        Serie::from_arrow_array(
            Some(&Field::new("symbol", DataType::utf8(), true)),
            Arc::new(symbols),
            ArrowCastOptions::new(),
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
        let held = Scalar::Serie(counts.clone());

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

/// The int64 column of [`leaf_columns`] at `rows` rows, held as `cuts`
/// chunks of equal length sliced out of it.
fn chunked_counts(rows: usize, cuts: usize) -> ChunkedSerie {
    let (counts, _) = leaf_columns(rows);
    let size = rows / cuts;
    let chunked = ChunkedSerie::from_series(
        None,
        (0..cuts).map(|chunk| counts.slice(chunk * size, size).expect("a chunk")),
        ArrowCastOptions::new(),
    )
    .expect("chunks under one field");
    assert_eq!((chunked.len(), chunked.num_chunks()), (rows, cuts));
    chunked
}

#[test]
fn reading_through_a_chunked_leaf_allocates_nothing() {
    // A row is found by a binary search over the chunk ends and read out of
    // the chunk that holds it, so a chunked read costs exactly what the
    // same read on the chunk costs - nothing, for a leaf - however many
    // rows and chunks there are.
    for (rows, cuts) in [(4_usize, 2_usize), (16_384, 8)] {
        let chunked = chunked_counts(rows, cuts);
        let size = rows / cuts;
        let middle = rows / 2;
        let absent = 1;
        let expected = Scalar::from(i64::try_from(middle).expect("a row count"));

        let chunk = &chunked.chunks()[middle / size];
        free(&format!("a cell of one of {cuts} chunks"), || {
            assert_eq!(
                black_box(chunk)
                    .scalar(black_box(middle % size))
                    .expect("in range"),
                expected
            );
        });
        free(
            &format!("len and null_count on {rows} rows in {cuts} chunks"),
            || {
                assert_eq!(black_box(&chunked).len(), rows);
                assert!(!black_box(&chunked).is_empty());
                assert_eq!(black_box(&chunked).num_chunks(), cuts);
                assert_eq!(black_box(&chunked).null_count(), (rows + 1) / 3);
                black_box(black_box(&chunked).field());
            },
        );
        free(&format!("is_null on {rows} rows in {cuts} chunks"), || {
            assert!(
                !black_box(&chunked)
                    .is_null(black_box(middle))
                    .expect("in range")
            );
            assert!(
                black_box(&chunked)
                    .is_null(black_box(absent))
                    .expect("in range")
            );
        });
        free(&format!("scalar on {rows} rows in {cuts} chunks"), || {
            assert_eq!(
                black_box(&chunked)
                    .scalar(black_box(middle))
                    .expect("in range"),
                expected
            );
            assert_eq!(
                black_box(&chunked)
                    .scalar(black_box(absent))
                    .expect("in range"),
                Scalar::Null
            );
        });
        free(&format!("get on {rows} rows in {cuts} chunks"), || {
            assert_eq!(
                black_box(&chunked).get(black_box(middle)).as_ref(),
                Some(&expected)
            );
            assert_eq!(black_box(&chunked).get(black_box(rows)), None);
        });
        free(
            &format!("the first row of a walk over {cuts} chunks"),
            || {
                assert_eq!(black_box(&chunked).iter().next(), Some(Scalar::from(0_i64)));
            },
        );
    }
}

#[test]
fn cloning_or_slicing_a_chunked_leaf_costs_its_two_vectors_and_nothing_per_row() {
    // A chunked serie is its chunks and their ends, two vectors beside one
    // shared field: a clone copies the two and bumps a pointer per chunk.
    // A window keeps the chunks it reaches in two new vectors, sliced where
    // it cuts one - the leaf's own one handle - and whole, a pointer bump,
    // where it does not. Every count is the same at four rows and at sixteen
    // thousand; a count that followed the rows would be a copy.
    for (rows, cuts) in [(4_usize, 2_usize), (16_384, 8)] {
        let chunked = chunked_counts(rows, cuts);
        let size = rows / cuts;

        costs(&format!("cloning {rows} rows in {cuts} chunks"), 2, || {
            black_box(black_box(&chunked).clone());
        });
        costs(&format!("slicing a chunk of {size} rows"), 1, || {
            black_box(
                black_box(&chunked.chunks()[0])
                    .slice(1, size - 1)
                    .expect("inside the chunk"),
            );
        });
        costs(
            &format!("slicing inside one of {cuts} chunks of {size} rows"),
            3,
            || {
                black_box(
                    black_box(&chunked)
                        .slice(1, size - 1)
                        .expect("inside the first chunk"),
                );
            },
        );
        costs(
            &format!("a window of one whole chunk of {size} rows"),
            2,
            || {
                black_box(
                    black_box(&chunked)
                        .slice(size, size)
                        .expect("the second chunk"),
                );
            },
        );
        // The chunks a window reaches are counted before they are kept, so
        // the whole window costs the two vectors however many chunks it
        // spans: two at two chunks, and two at eight.
        costs(
            &format!("a window of every one of {cuts} chunks of {size} rows"),
            2,
            || {
                black_box(black_box(&chunked).slice(0, rows).expect("the whole"));
            },
        );
        // One array handle per chunk, beside the vector that holds them.
        costs(
            &format!("the arrays of {cuts} chunks of {size} rows"),
            1 + cuts,
            || {
                black_box(black_box(&chunked).into_arrow_arrays());
            },
        );
    }
}

/// The record column of [`leaf_columns`] at `rows` rows, held as `cuts`
/// chunks of equal length sliced out of it.
fn chunked_records(rows: usize, cuts: usize) -> ChunkedSerie {
    let (counts, symbols) = leaf_columns(rows);
    let root = Field::new(
        "row",
        DataType::from(
            StructType::from_fields([
                counts.field().expect("a column").clone(),
                symbols.field().expect("a column").clone(),
            ])
            .expect("two named children"),
        ),
        false,
    );
    let records = Serie::from_scalars(
        root,
        (0..rows).map(|index| {
            Scalar::from_sequence([
                counts.scalar(index).expect("in range"),
                symbols.scalar(index).expect("in range"),
            ])
        }),
    )
    .expect("a record column");
    let size = rows / cuts;
    ChunkedSerie::from_series(
        None,
        (0..cuts).map(|chunk| records.slice(chunk * size, size).expect("a chunk")),
        ArrowCastOptions::new(),
    )
    .expect("chunks under one field")
}

#[test]
fn a_chunked_child_is_its_two_vectors_whatever_the_rows_or_chunks() {
    // A child is the child of every chunk - a pointer bump each - moved
    // into one vector of chunks sized up front and one of their ends, so it
    // costs two at two chunks and at eight, at ninety-six rows and at
    // sixteen thousand. `children` is that per child, beside the one
    // vector holding them.
    for (rows, cuts) in [(96_usize, 2_usize), (96, 8), (16_384, 2), (16_384, 8)] {
        let chunked = chunked_records(rows, cuts);
        let path = FieldPath::from_str("symbol").expect("a path");
        costs(
            &format!("a child of {rows} rows in {cuts} chunks"),
            2,
            || {
                black_box(black_box(&chunked).child("count").expect("a child"));
            },
        );
        costs(
            &format!("a child at of {rows} rows in {cuts} chunks"),
            2,
            || {
                black_box(black_box(&chunked).child_at(1).expect("a child"));
            },
        );
        costs(
            &format!("a path of {rows} rows in {cuts} chunks"),
            2,
            || {
                black_box(
                    black_box(&chunked)
                        .get_child_by_path(&path)
                        .expect("a child"),
                );
            },
        );
        costs(
            &format!("the children of {rows} rows in {cuts} chunks"),
            5,
            || {
                black_box(black_box(&chunked).children());
            },
        );
    }
}

#[test]
fn a_chunked_cast_compiles_one_plan_and_applies_it_per_chunk() {
    // A cast compiles its plan once and applies it to every chunk, so eight
    // chunks cost six applications more than two - never six compilations.
    // The same holds for chunks cast in by `from_series`, whose one plan
    // serves the run of chunks under one source field. An identity plan
    // hands a chunked serie under its own field back as the two vectors of
    // a clone, and a held chunked column crosses into a stream without a
    // row read: the same count at ninety-six rows as at sixteen thousand.
    let wide = Field::new("count", DataType::Float64, true);
    let options = ArrowCastOptions::new();
    let mut streamed = Vec::new();
    for rows in [96_usize, 16_384] {
        let two = chunked_counts(rows, 2);
        let eight = chunked_counts(rows, 8);
        let plan = ArrowCastPlan::compile(two.field(), &wide, options).expect("int64 widens");
        let (apply, _) = counted(|| plan.apply(&eight.chunks()[0]).expect("a chunk casts"));
        let (casting_two, _) = counted(|| two.cast(&wide, options).expect("two chunks cast"));
        let (casting_eight, _) = counted(|| eight.cast(&wide, options).expect("eight chunks cast"));
        assert_eq!(
            casting_eight - casting_two,
            6 * apply,
            "a cast of {rows} rows compiled per chunk"
        );

        let (from_two, _) = counted(|| {
            ChunkedSerie::from_series(Some(&wide), two.chunks().to_vec(), options)
                .expect("two chunks cast in")
        });
        let (from_eight, _) = counted(|| {
            ChunkedSerie::from_series(Some(&wide), eight.chunks().to_vec(), options)
                .expect("eight chunks cast in")
        });
        assert_eq!(
            from_eight - from_two,
            6 * apply,
            "chunks of {rows} rows cast in compiled per chunk"
        );

        let identity =
            ArrowCastPlan::compile(two.field(), two.field(), options).expect("an identity");
        costs(&format!("an identity plan over {rows} rows"), 2, || {
            black_box(identity.apply_chunked(black_box(&eight)).expect("itself"));
        });

        let records = chunked_records(rows, 8);
        let (stream, _) =
            counted(|| yggdryl::SerieReader::from_chunked(records.clone()).expect("a stream"));
        streamed.push(stream);
    }
    assert_eq!(
        streamed[0], streamed[1],
        "a held chunked column read a row on its way into a stream"
    );
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
        Serie::from_arrow_array(Some(&field()), Arc::new(array), ArrowCastOptions::new())
            .expect("an int64 column")
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

/// The record columns of the two batches every cast-budget case reads, the
/// field they lay out as, and the root they answer to.
///
/// The batches differ only in their values, so anything that varies between
/// casting one and casting the other is per-batch work rather than schema work.
fn cast_corpus() -> (Field, [Serie; 2], Field) {
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
            DataType::utf8().nullable_field("venue"),
        ])
        .map(DataType::from)
        .expect("the root fields are valid"),
        false,
    );
    let source = Field::from_arrow_schema("row", &schema).expect("the batch schema imports");
    let landed = |batch: RecordBatch| {
        Serie::from_arrow_batch(None, &batch, ArrowCastOptions::new())
            .expect("the batch lands as its own record column")
    };
    (source, [landed(batch(0)), landed(batch(2))], root)
}

#[test]
fn a_compiled_cast_costs_the_same_for_every_batch_it_answers() {
    use yggdryl::ArrowCastPlan;

    let (source, batches, root) = cast_corpus();
    let plan = ArrowCastPlan::compile(&source, &root, ArrowCastOptions::new())
        .expect("the cast is plannable");

    // The budget belongs to a batch, not to the stream: applying the plan a
    // thousand times costs a thousand times one batch, because nothing about
    // the previous batch is retained. Reading it as a total would hide exactly
    // the leak this pins - a plan that grew with every batch it saw.
    let mut index = 0;
    let (once, repeated) = counted_once_and_repeated(|| {
        let batch = &batches[index % batches.len()];
        index += 1;
        black_box(
            plan.apply(black_box(batch))
                .expect("the batch fits the plan"),
        );
    });
    assert_eq!(
        repeated,
        once * 1_000,
        "applying one compiled plan cost {once} for one batch and {repeated} for a thousand"
    );
}

#[test]
fn planning_once_is_what_a_reused_plan_saves_per_batch() {
    use yggdryl::ArrowCastPlan;

    let (source, batches, root) = cast_corpus();
    let plan = ArrowCastPlan::compile(&source, &root, ArrowCastOptions::new())
        .expect("the cast is plannable");
    let batch = &batches[0];

    // Warm both paths so neither is charged for a first-call cache fill.
    let _ = plan.apply(batch);
    let _ = batch.cast(&root, ArrowCastOptions::new());

    // `Serie::cast` is the door that compiles one plan per call and applies
    // it, so the two differ by exactly the compilation.
    let (applied, reused) = counted(|| {
        black_box(
            plan.apply(black_box(batch))
                .expect("the batch fits the plan"),
        )
    });
    let (planned, recompiled) = counted(|| {
        black_box(
            black_box(batch)
                .cast(&root, ArrowCastOptions::new())
                .expect("the batch fits the root"),
        )
    });

    // The same batch, the same answer, and the difference is the plan: a
    // reader that compiles per batch pays that difference on every one.
    assert_eq!(reused, recompiled, "both paths answer the same rows");
    assert!(
        planned > applied,
        "compiling per batch cost {planned} and reusing one plan cost {applied}"
    );
}

/// A stored batch keyed `0..rows` and an incoming batch updating every one of
/// its keys, so each incoming batch folds the same rows into the same place.
#[cfg(feature = "internals")]
fn merge_corpus(rows: usize) -> (Field, arrow_array::RecordBatch, arrow_array::RecordBatch) {
    use arrow_array::{ArrayRef, Int64Array, RecordBatch, StringArray};

    let field = StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])
    .map(DataType::from)
    .expect("the merge fields are valid")
    .required_field("row");
    let schema = field
        .clone()
        .into_arrow_schema()
        .expect("the merge root projects");
    let ids: ArrayRef = Arc::new(Int64Array::from_iter_values(
        0..i64::try_from(rows).expect("a small corpus"),
    ));
    let batch = |symbol: &str| {
        RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::clone(&ids),
                Arc::new(StringArray::from(vec![symbol; rows])),
            ],
        )
        .expect("the merge batch matches its schema")
    };
    (field, batch("OLD"), batch("NEW"))
}

#[cfg(feature = "internals")]
#[test]
fn a_merge_casts_every_incoming_batch_alike() {
    use yggdryl::internals::media_merge::merged;

    for rows in [2, 64] {
        let (field, stored, incoming) = merge_corpus(rows);
        let key = yggdryl::Selector::from_columns(["id"]);
        let cost = |batches: usize| {
            let stored = yggdryl::arrow::batch_reader(stored.schema(), [stored.clone()]);
            let incoming =
                yggdryl::arrow::batch_reader(incoming.schema(), vec![incoming.clone(); batches]);
            counted(|| {
                for batch in merged(stored, incoming, &field, &key, true).expect("the merge plans")
                {
                    black_box(batch.expect("the merged batch"));
                }
            })
            .0
        };
        cost(1);

        // Every incoming batch is cast into the field, but the cast is planned
        // by the first alone: N batches cost the plan once and N times one
        // batch.
        let (none, one, two, four) = (cost(0), cost(1), cost(2), cost(4));
        let each = two - one;
        assert_eq!(
            four - one,
            3 * each,
            "{rows} rows: every batch after the first cost {each}, but four cost {four} and one {one}"
        );
        assert!(
            one - none > each,
            "{rows} rows: the first batch cost {} and every later one {each}, so the plan was paid per batch",
            one - none
        );
    }
}

/// A batch of `rows` rows: an id, and a sorted text-keyed map holding three
/// identifiers in every row, the `ISIN` one of them.
fn map_key_corpus(rows: usize) -> arrow_array::RecordBatch {
    let entries = StructType::from_fields([
        DataType::utf8().required_field("key"),
        DataType::utf8().required_field("value"),
    ])
    .map(DataType::from)
    .expect("the entry fields are valid")
    .required_field("entries");
    let root = StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::map(entries, true)
            .expect("a text-keyed map")
            .nullable_field("ids"),
    ])
    .map(DataType::from)
    .expect("the root fields are valid")
    .required_field("row");
    let values: Vec<Scalar> = (0..rows)
        .map(|row| {
            let isin = format!("US{row:010}");
            Scalar::from_sequence([
                Scalar::from(i64::try_from(row).expect("a small corpus")),
                Scalar::from_mapping([
                    (Scalar::from("CUSIP"), Scalar::from(&isin[2..11])),
                    (Scalar::from("ISIN"), Scalar::from(isin.as_str())),
                    (Scalar::from("SEDOL"), Scalar::from("B0YBKJ7")),
                ])
                .expect("a map of three entries"),
            ])
        })
        .collect();
    Serie::from_scalars(root, values)
        .expect("the corpus lays out")
        .into_arrow_batch()
        .expect("a record column is a batch")
}

#[test]
fn a_map_key_lift_allocates_only_what_it_hands_back() {
    let selector: yggdryl::Selector = "id, ids['ISIN'] as isin".parse().expect("a lift");
    let mut each_at = Vec::new();
    for rows in [2, 64] {
        let batch = map_key_corpus(rows);
        let cost = |batches: usize| {
            let reader = yggdryl::arrow::batch_reader(batch.schema(), vec![batch.clone(); batches]);
            counted(|| {
                for projected in selector.apply_arrow_reader(reader).expect("the lift binds") {
                    black_box(projected.expect("the lift answers"));
                }
            })
            .0
        };
        cost(1);
        let (one, two, four) = (cost(1), cost(2), cost(4));
        let each = two - one;
        assert_eq!(
            four - one,
            3 * each,
            "{rows} rows: every batch after the first cost {each}, but four cost {four} and one {one}"
        );
        each_at.push(each);
    }
    // The key is found in place and the values taken once: a batch costs
    // the same count of allocations whatever it holds, the taken output
    // included.
    assert_eq!(
        each_at[0], each_at[1],
        "a batch of 2 rows cost {} and one of 64 cost {}: the lift allocated per row",
        each_at[0], each_at[1]
    );
    eprintln!("map_key_lift: per batch {}", each_at[0]);
}

/// A root of an id beside a serie, and a batch of `rows` rows under it
/// whose series hold two elements each, laid out as one contiguous run.
fn unnest_corpus(rows: usize) -> (Field, arrow_array::RecordBatch) {
    let root = StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::serie(DataType::Int64.nullable_field("item")).nullable_field("xs"),
    ])
    .map(DataType::from)
    .expect("the root fields are valid")
    .required_field("row");
    let values: Vec<Scalar> = (0..rows)
        .map(|row| {
            let row = i64::try_from(row).expect("a small corpus");
            Scalar::from_sequence([
                Scalar::from(row),
                Scalar::from_sequence([Scalar::from(2 * row), Scalar::from(2 * row + 1)]),
            ])
        })
        .collect();
    let batch = Serie::from_scalars(root.clone(), values)
        .expect("the corpus lays out")
        .into_arrow_batch()
        .expect("a record column is a batch");
    (root, batch)
}

/// An unnest over contiguous runs builds the parents - the take every other
/// column is read by - and shares the elements as they lie: a batch costs
/// the same count of allocations whatever it holds, the taken `id` and the
/// batch itself included.
#[test]
fn a_contiguous_unnest_allocates_only_its_parents_and_what_it_takes() {
    let selector: yggdryl::Selector = "id, unnest(xs) as x".parse().expect("an unnest");
    let mut each_at = Vec::new();
    for rows in [2, 64] {
        let (root, batch) = unnest_corpus(rows);
        let bound = selector.bind(&root).expect("the unnest binds");
        let (once, repeated) = counted_once_and_repeated(|| {
            let out = bound.apply_arrow_batch(&batch).expect("the unnest answers");
            assert_eq!(out.num_rows(), 2 * rows);
            black_box(out);
        });
        assert_eq!(
            repeated,
            once * 1_000,
            "{rows} rows: one batch cost {once} and a thousand cost {repeated}"
        );
        each_at.push(once);
    }
    eprintln!("contiguous_unnest: per batch {each_at:?}");
    assert_eq!(
        each_at, [UNNEST_BATCH_ALLOCATIONS; 2],
        "a batch of 2 rows cost {} and one of 64 cost {}",
        each_at[0], each_at[1]
    );
}

/// What one batch of a contiguous unnest allocates at any row count: the
/// parents, the `id` taken by them, and the batch holding both beside the
/// shared elements. Taking the elements instead of sharing them costs four
/// more. The output schema shares the root's cached projection list rather
/// than projecting each child again per batch, which is four fewer than the
/// sixteen that re-projection cost.
const UNNEST_BATCH_ALLOCATIONS: usize = 12;

/// One `marketdata` batch: two orders, a trade of two executions and a book.
fn view_corpus() -> arrow_array::RecordBatch {
    let (root, executions) = allocation_trade_parts(2);
    let trade = TradeEvent::from_parts(&root, executions).expect("the trade");
    let values = (0..2).map(allocation_market_order).chain([
        MarketData::from(trade),
        MarketData::from(allocation_book(1)),
    ]);
    let mut rows = MarketData::arrow_reader(values, None, None).expect("the row field");
    let batch = rows
        .next()
        .expect("one batch")
        .expect("the leaves are canonical");
    assert!(rows.next().is_none());
    batch
}

#[test]
fn a_view_plan_is_compiled_once_per_stream() {
    use yggdryl::graph::MarketView;

    let batch = view_corpus();
    let isin: FieldPath = "securityids['base:isin'].value as isin"
        .parse()
        .expect("a lift");
    for (view, lifts) in [
        (MarketView::Orders, vec![isin]),
        (MarketView::Trades, Vec::new()),
        (MarketView::Books, Vec::new()),
    ] {
        // An endless stream of the one batch: the view binds when it is
        // applied, and every batch it is then pulled for costs one batch.
        let source = {
            let batch = batch.clone();
            std::iter::repeat_with(move || Ok(batch.clone()))
        };
        let reader: yggdryl::arrow::BatchReader = Box::new(arrow_array::RecordBatchIterator::new(
            source,
            batch.schema(),
        ));
        let mut viewed =
            MarketData::apply_view(&view, &lifts, reader).expect("the view binds once");
        let (once, repeated) = counted_once_and_repeated(|| {
            black_box(
                viewed
                    .next()
                    .expect("an endless stream")
                    .expect("the view answers"),
            );
        });
        assert!(
            once > 0,
            "{view}: a batch that allocates nothing moved nothing"
        );
        assert_eq!(
            repeated,
            once * 1_000,
            "{view}: one batch cost {once} and a thousand cost {repeated}"
        );
        eprintln!("view_{view}: per batch {once}");
    }
}

/// The cost of each chunk one resumable write session is pushed.
///
/// Every chunk is `batches` row-less batches: a batch with no rows is shaped
/// and joins no cadence, so a chunk costs its pull and its shaping alone.
fn session_chunk_costs(
    options: &yggdryl::media::RecordOptions,
    layouts: &[&arrow_schema::SchemaRef],
    batches: usize,
) -> Vec<usize> {
    use arrow_array::RecordBatch;

    let mut handle = Buffer::new().with_media_type(MediaType::new(MimeType::ARROW_STREAM));
    let mut session = yggdryl::ArrowWriteSession::overwrite(options).expect("a session");
    let chunks: Vec<_> = layouts
        .iter()
        .map(|layout| {
            yggdryl::arrow::batch_reader(
                Arc::clone(layout),
                vec![RecordBatch::new_empty(Arc::clone(layout)); batches],
            )
        })
        .collect();
    chunks
        .into_iter()
        .map(|chunk| {
            counted(|| {
                assert!(
                    session
                        .push(&mut handle, chunk)
                        .expect("the chunk is shaped")
                );
            })
            .0
        })
        .collect()
}

#[test]
fn a_write_session_compiles_its_shaping_once() {
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
    use yggdryl::media::IORecordOptions as _;

    let root = StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])
    .map(DataType::from)
    .expect("the session fields are valid")
    .required_field("row");
    let options = yggdryl::media::RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)
        .expect("Arrow IPC options")
        .with_field(root.clone())
        .with_commit_row_size(1);
    let declared = root.into_arrow_schema().expect("the session root projects");
    // The same columns under another layout: a key admitting nulls.
    let relaxed = Arc::new(Schema::new(vec![
        ArrowField::new("id", ArrowDataType::Int64, true),
        ArrowField::new("symbol", ArrowDataType::Utf8, true),
    ]));
    let layouts = [
        &declared, &declared, &declared, &relaxed, &relaxed, &relaxed,
    ];

    for batches in [1, 4] {
        session_chunk_costs(&options, &layouts, batches);
        let costs = session_chunk_costs(&options, &layouts, batches);

        // A layout's first chunk compiles its shaping, and every later chunk
        // of it costs the same and strictly less.
        assert_eq!(costs[1], costs[2], "{batches} batches a chunk: {costs:?}");
        assert!(costs[1] < costs[0], "{batches} batches a chunk: {costs:?}");
        assert_eq!(costs[4], costs[5], "{batches} batches a chunk: {costs:?}");
        assert!(
            costs[4] < costs[3],
            "{batches} batches a chunk: a later chunk of a layout compiled again: {costs:?}"
        );
    }
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
    let seeds: [(DataTypeId, Scalar); 53] = [
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
        (
            DataTypeId::Decimal,
            Scalar::Decimal("82.5".parse().unwrap()),
        ),
        (
            DataTypeId::BigDecimal,
            Scalar::BigDecimal(yggdryl::BigDecimal::from_int(3)),
        ),
        (DataTypeId::Float32, Scalar::from(1.5_f64)),
        (DataTypeId::Float64, Scalar::from(1.5_f64)),
        (DataTypeId::Date32, Scalar::date32(19_723)),
        (DataTypeId::Date64, Scalar::date32(19_723)),
        (DataTypeId::Binary, Scalar::from(&b"ABC"[..])),
        (DataTypeId::LargeBinary, Scalar::from(&b"ABC"[..])),
        (DataTypeId::BinaryView, Scalar::from(&b"ABC"[..])),
        (DataTypeId::LargeBinaryView, Scalar::from(&b"ABC"[..])),
        (DataTypeId::Utf8String, Scalar::from("AAPL")),
        (DataTypeId::LargeUtf8String, Scalar::from("AAPL")),
        (DataTypeId::Utf8StringView, Scalar::from("AAPL")),
        (DataTypeId::LargeUtf8StringView, Scalar::from("AAPL")),
        (DataTypeId::AsciiString, Scalar::from("AAPL")),
        (DataTypeId::LargeAsciiString, Scalar::from("AAPL")),
        (DataTypeId::AsciiStringView, Scalar::from("AAPL")),
        (DataTypeId::LargeAsciiStringView, Scalar::from("AAPL")),
        (DataTypeId::Cp1252String, Scalar::from("AAPL")),
        (DataTypeId::LargeCp1252String, Scalar::from("AAPL")),
        (DataTypeId::Cp1252StringView, Scalar::from("AAPL")),
        (DataTypeId::LargeCp1252StringView, Scalar::from("AAPL")),
        (DataTypeId::Country, Scalar::from("US")),
        (DataTypeId::Ccy, Scalar::from("USD")),
        (DataTypeId::Mic, Scalar::from("XNAS")),
        (DataTypeId::Cfi, Scalar::from("ESVUFR")),
        (DataTypeId::Isin, Scalar::from("US0378331005")),
        (DataTypeId::Cusip, Scalar::from("037833100")),
        (DataTypeId::Sedol, Scalar::from("B0YBKJ7")),
        (DataTypeId::Bbg, Scalar::from("AAPL US EQUITY")),
        (DataTypeId::Ric, Scalar::from("AAPL.OQ")),
        (DataTypeId::Forex, Scalar::from("EURUSD")),
        (DataTypeId::Figi, Scalar::from("BBG000BLNQ16")),
        (DataTypeId::Side, Scalar::from("1")),
        (DataTypeId::State, Scalar::from("NEW")),
        (DataTypeId::MarketDataKind, Scalar::from("ORDR")),
        (DataTypeId::TimeInForce, Scalar::from("DAY")),
        (DataTypeId::MarketDataType, Scalar::from("ORDLIMIT")),
        (DataTypeId::Unit, Scalar::from("Shares")),
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

#[cfg(feature = "internals")]
#[test]
fn canonical_rows_lay_out_without_a_second_proof() {
    // A bounded string's layout is not its datatype's contract, so a landing
    // that proves it reads every row - and a row longer than a cell holds
    // inline is a value built. Rows that already went through the field's
    // contract are laid out and landed proven: the cost is the buffers,
    // whatever the row count.
    let field = Arc::new(Field::new(
        "note",
        DataType::sized_utf8(64).expect("a bounded string"),
        false,
    ));
    let mut counts = Vec::new();
    for rows in [1_024_usize, 16_384] {
        let canonical: Vec<Scalar> = (0..rows)
            .map(|index| {
                field
                    .scalar(Scalar::from(format!(
                        "a note longer than any inline cell {index:08}"
                    )))
                    .expect("the field's contract")
            })
            .collect();
        let borrowed: Vec<&Scalar> = canonical.iter().collect();
        let lay_out =
            || yggdryl::internals::serie_arrow::from_canonical_rows(Arc::clone(&field), &borrowed);
        drop(lay_out().expect("canonical rows lay out"));
        let (allocations, column) = counted(lay_out);
        assert_eq!(column.expect("canonical rows lay out").len(), rows);
        counts.push(allocations);
    }
    assert_eq!(
        counts[0], counts[1],
        "laying out canonical rows cost {counts:?} at 1024 and 16384 rows: a row read again"
    );
}

#[test]
fn an_arrow_column_is_one_value_without_a_row() {
    // The column is the value: wrapping it reads no row and copies no
    // buffer, so a string column of any length costs the same.
    let field = DataType::utf8().nullable_field("symbol");
    let mut counts = Vec::new();
    for rows in [4_usize, 1_024, 16_384] {
        let array: arrow_array::ArrayRef = Arc::new(arrow_array::StringArray::from(
            (0..rows)
                .map(|index| format!("a symbol long enough to live off the stack {index:08}"))
                .collect::<Vec<_>>(),
        ));
        let wrap = || {
            Scalar::from(
                Serie::from_arrow_array(
                    Some(&field),
                    Arc::clone(&array),
                    ArrowCastOptions::default(),
                )
                .expect("the column lands"),
            )
        };
        drop(wrap());
        let (allocations, value) = counted(wrap);
        assert_eq!(value.as_serie().map(Serie::len), Some(rows));
        counts.push(allocations);
    }
    assert!(
        counts.windows(2).all(|pair| pair[0] == pair[1]),
        "an Arrow column became a value for {counts:?} allocations at 4, 1024 and 16384 rows"
    );
}

#[test]
fn proving_exact_arrow_maps_allocates_per_column_not_per_row() {
    // Long keys make a materialized Scalar visible to the allocator. Two
    // entries need no sorting scratch: each map is proved over its buffers,
    // and an exact landing shares those buffers at either corpus size.
    use arrow_array::{ArrayRef, Int64Array, MapArray, StringArray, StructArray};
    use arrow_buffer::OffsetBuffer;
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Fields};

    let fields: Fields = vec![
        Arc::new(ArrowField::new("key", ArrowDataType::Utf8, false)),
        Arc::new(ArrowField::new("value", ArrowDataType::Int64, true)),
    ]
    .into();
    let entries = Field::new(
        "entries",
        DataType::from(
            StructType::from_fields([
                DataType::utf8().required_field("key"),
                DataType::Int64.nullable_field("value"),
            ])
            .expect("two map children"),
        ),
        false,
    );
    for sorted in [false, true] {
        let field = Field::new(
            "item",
            DataType::map(entries.clone(), sorted).expect("a map"),
            false,
        );
        for declared in [false, true] {
            let mut counts = Vec::new();
            for rows in [1_024_usize, 16_384] {
                let keys = if sorted {
                    [
                        "a map key longer than inline storage",
                        "b map key longer than inline storage",
                    ]
                } else {
                    [
                        "b map key longer than inline storage",
                        "a map key longer than inline storage",
                    ]
                };
                let keys = StringArray::from_iter_values((0..rows).flat_map(|_| keys));
                let payload = keys.value_data().as_ptr();
                let records = StructArray::new(
                    fields.clone(),
                    vec![
                        Arc::new(keys),
                        Arc::new(Int64Array::from_iter_values(
                            (0..rows).flat_map(|_| [1_i64, 2]),
                        )),
                    ],
                    None,
                );
                let array: ArrayRef = Arc::new(MapArray::new(
                    Arc::new(ArrowField::new(
                        "entries",
                        ArrowDataType::Struct(fields.clone()),
                        false,
                    )),
                    OffsetBuffer::new(
                        (0..=rows)
                            .map(|row| i32::try_from(row * 2).expect("the corpus fits"))
                            .collect::<Vec<_>>()
                            .into(),
                    ),
                    records,
                    None,
                    sorted,
                ));
                let land = || {
                    Serie::from_arrow_array(
                        declared.then_some(&field),
                        Arc::clone(&array),
                        ArrowCastOptions::default(),
                    )
                    .expect("the exact map lands")
                };
                drop(land());
                let (allocations, column) = counted(land);
                assert_eq!(column.len(), rows);
                assert_eq!(
                    column
                        .as_map()
                        .expect("a map")
                        .keys()
                        .as_utf8()
                        .expect("text keys")
                        .payload()
                        .as_ptr(),
                    payload,
                    "an exact map shares its key payload"
                );
                counts.push(allocations);
            }
            assert_eq!(
                counts[0], counts[1],
                "exact map proof cost {counts:?} at 1024 and 16384 rows \
                 (sorted={sorted}, declared={declared})"
            );
        }
    }
}

#[test]
fn reading_a_map_cell_costs_its_mapping_alone() {
    // Two inline keys and two integers per cell: the one allocation a cell
    // read pays is the entries' shared storage, written in place off the
    // column's two children rather than through a vector copied into it.
    use arrow_array::{ArrayRef, Int64Array, MapArray, StringArray, StructArray};
    use arrow_buffer::OffsetBuffer;
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Fields};

    let fields: Fields = vec![
        Arc::new(ArrowField::new("key", ArrowDataType::Utf8, false)),
        Arc::new(ArrowField::new("value", ArrowDataType::Int64, true)),
    ]
    .into();
    let rows = 64_usize;
    let records = StructArray::new(
        fields.clone(),
        vec![
            Arc::new(StringArray::from_iter_values(
                (0..rows).flat_map(|_| ["a", "b"]),
            )),
            Arc::new(Int64Array::from_iter_values(
                (0..rows).flat_map(|_| [1_i64, 2]),
            )),
        ],
        None,
    );
    let array: ArrayRef = Arc::new(MapArray::new(
        Arc::new(ArrowField::new(
            "entries",
            ArrowDataType::Struct(fields),
            false,
        )),
        OffsetBuffer::new(
            (0..=rows)
                .map(|row| i32::try_from(row * 2).expect("the corpus fits"))
                .collect::<Vec<_>>()
                .into(),
        ),
        records,
        None,
        false,
    ));
    let column =
        Serie::from_arrow_array(None, array, ArrowCastOptions::default()).expect("the map lands");
    costs("reading a two-entry map cell", 1, || {
        black_box(black_box(&column).scalar(rows - 1).expect("a cell"));
    });
}

#[test]
fn proving_hidden_list_spans_allocates_per_column_not_per_row() {
    use arrow_array::{ArrayRef, ListArray, StringArray};
    use arrow_buffer::{NullBuffer, OffsetBuffer};
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};

    let mut counts = Vec::new();
    for rows in [1_024_usize, 16_384] {
        let values = StringArray::from_iter_values(std::iter::repeat_n(
            "a physical item longer than inline storage",
            rows,
        ));
        let payload = values.value_data().as_ptr();
        let offsets = OffsetBuffer::new(
            (0..=rows)
                .map(|row| i32::try_from(row).expect("the corpus fits"))
                .collect::<Vec<_>>()
                .into(),
        );
        let offset_buffer = offsets.as_ptr();
        let array: ArrayRef = Arc::new(ListArray::new(
            Arc::new(ArrowField::new("item", ArrowDataType::Utf8, false)),
            offsets,
            Arc::new(values),
            Some(NullBuffer::from_iter((0..rows).map(|row| row % 2 == 0))),
        ));
        let land = || {
            Serie::from_arrow_array(None, Arc::clone(&array), ArrowCastOptions::default())
                .expect("hidden physical spans remain borrowed")
        };
        drop(land());
        let (allocations, column) = counted(land);
        let lists = column.as_serie().expect("the list column");
        assert_eq!(lists.items().len(), rows);
        assert_eq!(lists.offsets().as_ptr(), offset_buffer);
        assert_eq!(lists.items().as_utf8().unwrap().payload().as_ptr(), payload);
        counts.push(allocations);
    }
    assert_eq!(counts[0], counts[1], "hidden-span proof cost {counts:?}");
}

#[test]
fn proving_a_decimal_or_date64_intake_builds_nothing() {
    // A decimal's precision and a date64's whole days are narrower than the
    // storage, so the landing reads each row once - through the column's
    // own reading, into a value that needs no allocation.
    let decimal = DataType::decimal(10, 2)
        .expect("a decimal")
        .nullable_field("price");
    let date = DataType::Date64.nullable_field("day");
    for (field, build) in [
        (
            &decimal,
            (|rows: usize| {
                // The decimal's own layout, so the landing proves it and casts
                // nothing.
                Arc::new(
                    arrow_array::Decimal64Array::from_iter_values(
                        (0..rows).map(|index| i64::try_from(index).expect("fits") * 100 + 25),
                    )
                    .with_precision_and_scale(10, 2)
                    .expect("a decimal column"),
                ) as arrow_array::ArrayRef
            }) as fn(usize) -> arrow_array::ArrayRef,
        ),
        (
            &date,
            (|rows: usize| {
                Arc::new(arrow_array::Date64Array::from_iter_values(
                    (0..rows).map(|index| i64::try_from(index).expect("fits") * 86_400_000),
                )) as arrow_array::ArrayRef
            }) as fn(usize) -> arrow_array::ArrayRef,
        ),
    ] {
        let mut counts = Vec::new();
        for rows in [1_024_usize, 16_384] {
            let array = build(rows);
            let land = || {
                Serie::from_arrow_array(
                    Some(field),
                    Arc::clone(&array),
                    ArrowCastOptions::default(),
                )
                .expect("the column lands")
            };
            drop(land());
            let (allocations, column) = counted(land);
            assert_eq!(column.len(), rows);
            counts.push(allocations);
        }
        assert_eq!(
            counts[0],
            counts[1],
            "proving a {} intake cost {counts:?} at 1024 and 16384 rows",
            field.dtype()
        );
    }
}

/// One column per leaf a cell read must build nothing for.
fn typed_leaf_columns() -> Vec<Serie> {
    let zone = Timezone::from_str("Europe/Paris").expect("a zone");
    let at = DataType::DateTime64 {
        unit: TimeUnit::Microsecond,
        timezone: zone,
    };
    [
        (DataType::Int64, Scalar::from(42_i64)),
        (DataType::Boolean, Scalar::from(true)),
        (DataType::Float64, Scalar::from(1.5_f64)),
        (
            DataType::decimal(10, 2).expect("a decimal"),
            Scalar::decimal128(1_025, 2),
        ),
        (DataType::Date64, Scalar::date64(86_400_000)),
        (
            at,
            Scalar::datetime64(1_700_000_000_000_000, TimeUnit::Microsecond, zone)
                .expect("an instant"),
        ),
        (
            DataType::Duration32(TimeUnit::Second),
            Scalar::duration32(90, TimeUnit::Second).expect("a duration"),
        ),
        (DataType::utf8(), Scalar::from("AAPL")),
    ]
    .into_iter()
    .map(|(dtype, value)| {
        Serie::from_scalars(dtype.nullable_field("cell"), [value, Scalar::Null])
            .expect("a leaf column")
    })
    .collect()
}

#[test]
fn a_leaf_cell_read_allocates_nothing() {
    // A leaf reads its own typed buffer through the reading its field
    // resolved where it landed: one buffer read and one constructor, no
    // downcast and no value that owns memory.
    for column in typed_leaf_columns() {
        let dtype = column.field().expect("a column").dtype().clone();
        free(&format!("reading a {dtype} cell"), || {
            black_box(black_box(&column).scalar(0).expect("a cell"));
        });
        free(&format!("reading an absent {dtype} cell"), || {
            black_box(black_box(&column).scalar(1).expect("a cell"));
        });
    }
}

#[test]
fn a_record_row_costs_its_run_and_nothing_per_cell() {
    // A record row is one run of its cells: the run's storage is the one
    // allocation, and every cell under it is read without one.
    let columns = typed_leaf_columns();
    let root = StructType::from_fields(columns.iter().enumerate().map(|(index, column)| {
        column
            .field()
            .expect("a column")
            .clone()
            .with_name(format!("cell_{index}"))
    }))
    .map(DataType::from)
    .expect("a record")
    .required_field("row");
    let rows = [0_usize, 1].map(|row| {
        Scalar::from_sequence(
            columns
                .iter()
                .map(|column| column.scalar(row).expect("a cell")),
        )
    });
    let records = Serie::from_scalars(root, rows).expect("a record column");
    costs("reading a record row", 1, || {
        black_box(black_box(&records).scalar(0).expect("a row"));
    });
}

#[test]
fn digesting_a_column_allocates_nothing_per_row() {
    // Each cell is fed from the leaf it lands in, so a column's digests cost
    // their output and nothing per row, whatever the leaf.
    for (dtype, cell) in [
        (
            DataType::Int64,
            (|index: usize| Scalar::from(i64::try_from(index).expect("fits")))
                as fn(usize) -> Scalar,
        ),
        (DataType::utf8(), |index| {
            Scalar::from(format!("{index:032}"))
        }),
        (DataType::decimal(18, 4).expect("a decimal"), |index| {
            Scalar::decimal128(i128::try_from(index).expect("fits"), 4)
        }),
        (DataType::Country, |index| {
            Scalar::from(if index % 2 == 0 { "FR" } else { "US" })
        }),
    ] {
        let field = dtype.clone().nullable_field("value");
        let mut counts = Vec::new();
        for rows in [1_024_usize, 16_384] {
            let array = Serie::from_scalars(field.clone(), (0..rows).map(cell))
                .expect("a column")
                .require_arrow_array()
                .expect("its buffers");
            let digest = || {
                yggdryl::xxhash::arrow::column_digests(
                    Arc::clone(&array),
                    &field,
                    yggdryl::DigestAlgorithm::Xxh3,
                )
                .expect("digests")
            };
            drop(digest());
            let (allocations, digests) = counted(digest);
            assert_eq!(digests.len(), rows);
            counts.push(allocations);
        }
        assert_eq!(
            counts[0], counts[1],
            "digesting a {dtype} column cost {counts:?} at 1024 and 16384 rows"
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
const FIX_LINE_COSTS: [(usize, usize); 3] = [(4, 29), (16, 30), (64, 32)];

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
///
/// It last moved with [`FIX_LINE_COSTS`], by the same one in both columns.
const WIDE_VALUE_COSTS: [(usize, (usize, usize)); 3] =
    [(4, (29, 35)), (16, (30, 60)), (64, (31, 157))];

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
    let mut parties = DataType::serie(item).nullable_field("Parties");
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
const PACKED_MEMBER_COSTS: [(usize, usize); 2] = [(4, 55), (16, 75)];

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
/// The arrival record's one shared allocation and the name table sized once
/// are in all three, as they are in [`FIX_LINE_COSTS`].
///
/// It moved with [`FIX_LINE_COSTS`], by the same four: 31 to 27 at four
/// pairs, 32 to 28 at sixteen and 33 to 29 at sixty-four; then by the
/// same one: 28, 29 and 30; and last by its displayed quantity at
/// sixty-four alone: 31.
const FIX_TEXT_LINE_COSTS: [(usize, usize); 3] = [(4, 28), (16, 29), (64, 31)];

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
    // The named captures are the line's own reading and no event fact: the
    // digest feeds the state, the place, the predecessor and the body, all
    // of them held already, so the code costs nothing. It cost six - a name
    // and a value per capture, over their map - while the names an element
    // went by were an event fact the digest fed.
    let (code, _) = counted(|| black_box(line.get_currhashcode()));
    assert_eq!(code, 0, "the code reads what the line already holds");
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

    // The first pull lands the batch - each column the plan locates, once -
    // and reads its first row; no other row is read, so the pull costs the
    // same whatever the batch holds. Every row after it reads through the
    // landed leaves and owns only its bounded text state.
    let options = TextOptions::new();
    let mut first_cost = None;
    for rows in [2, 64, 1024] {
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
        let (allocations, line) = counted(|| reader.next().unwrap().unwrap());
        assert_eq!(line.index(), 1);
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
                let fact = line.element_fact(ElementColumn::CrossCode);
                match fact.as_ref().and_then(Scalar::as_string) {
                    Some(code) => {
                        black_box(code);
                        projected += 1;
                    }
                    None => panic!("a located line has a string crosscode"),
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
                black_box(line.element_fact(ElementColumn::CrossCode));
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

/// Public atomic copy and private owned intake costs at the same corpus sizes.
///
/// Main's part-built staging URL and shared host remove five allocations
/// from the public atomic copy (23/26 to 18/21). Private intake still moves
/// its first stream chunk directly: 2 allocations below 64 KiB, 4 for the
/// 114 KiB corpus, without the public copy's transaction.
const HANDLE_COSTS: [(usize, usize, usize); 2] = [(16, 18, 2), (1_024, 21, 4)];

/// What the read itself costs past owned intake: ten, and nothing a line.
///
/// Seven of the original nine are built before a byte is read - the cursor over the
/// owned handle and the transport boxed around it, the options and the
/// location each shared once, and the splitter's window as a vector and as
/// the shared box that seals it - and two on the first pull, where the
/// transport opens: the fetch buffer and the box the coding chain ends in.
///
/// Nothing a line, because a line is not a thing that is built: the window
/// is the page, and a line is the range of it the splitter cut, so the
/// header off its front, the strips off its edges and the byte limit off
/// its tail move two offsets and copy nothing. With private intake, the assertion
/// below counts 12 for 16 rows and the same 10 over intake for 1 024 -
/// after the two the buffer's first `url` costs, which [`text_lines_cost`]
/// asks for before the counter is armed and which are in neither number.
///
/// One of the ten is the fifteen event columns the plan compiles once per
/// read: the identity list and the state's own type allocate as the
/// columns are planned, and nothing of them per line. The count was
/// thirteen while the event had an `identifiers` column: the names' map
/// cost three as it was planned, and left with the column.
///
/// A read now shares what it was addressed by rather than where that
/// resolves to, and the count did not move: the location is a narrowing of
/// the identifier rather than a second value beside it, so the read still
/// holds one reference-counted source and a row still clones one handle.
const TEXT_LINES_ONCE: usize = 10;

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
    for (rows, copy, owned) in HANDLE_COSTS {
        let text = bridge_lines(rows);
        let source = Buffer::from_bytes(text.into_bytes())
            .with_media_type(MediaType::from_str("text/plain").expect("a media type"));
        let (staged, _) = counted(|| {
            let mut staged = Buffer::new();
            yggdryl::IOBase::copy_into(black_box(&source), &mut staged).expect("a copy");
            black_box(staged);
        });
        assert_eq!(staged, copy, "the public atomic copy of {rows} rows");
        assert_eq!(
            text_lines_cost(&source, rows),
            owned + TEXT_LINES_ONCE,
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
    for (rows, _, owned) in HANDLE_COSTS {
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
            owned + TEXT_LINES_ONCE + 1 + TEXT_LINES_RETAINED_PER_WINDOW * windows,
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
        .encode("symbol,dÃƒÆ’Ã‚Â©sk\nAAPL,ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬1\n")
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
    assert_eq!(decoded, "symbol,dÃƒÆ’Ã‚Â©sk\nAAPL,ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬1\n");
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
                Serie::from_scalars(
                    black_box(&field).clone(),
                    black_box(&column)
                        .sequence_rows()
                        .expect("rows")
                        .into_owned(),
                )
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
fn cp1252_write_preflight_adds_no_per_row_allocation() {
    // A non-ASCII encoding preflight must inspect the repertoire without
    // encoding into a temporary Vec for each row. Both spellings occupy
    // four wire bytes and fit a Scalar inline, so the only allocations are
    // the same replacement buffers, whatever the text or corpus size.
    for dtype in [
        DataType::cp1252(),
        DataType::large_cp1252(),
        DataType::cp1252_view(),
        DataType::large_cp1252_view(),
        DataType::fixed_cp1252(4).expect("a fixed width"),
        DataType::sized_cp1252(4).expect("a bounded width"),
    ] {
        let mut counts = Vec::new();
        for rows in [1_024_usize, 16_384] {
            for text in ["cafe", "caf\u{00e9}"] {
                let field = dtype.clone().required_field("note");
                let canonical = field
                    .scalar(Scalar::from(text))
                    .expect("an encodable value");
                let incoming = vec![canonical; rows];
                let mut column = Serie::with_capacity(field, rows).expect("reserved buffers");
                let (allocations, ()) = counted(|| {
                    column
                        .splice(0..0, incoming)
                        .expect("encodable rows append");
                });
                assert_eq!(column.len(), rows);
                assert_eq!(
                    column.scalar(rows - 1).expect("the last row").as_str(),
                    Some(text)
                );
                counts.push(allocations);
            }
        }
        assert!(
            counts.windows(2).all(|pair| pair[0] == pair[1]),
            "{dtype} writes cost {counts:?} for ASCII/non-ASCII at 1024/16384 rows; \
             repertoire preflight must not allocate per row"
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
        let build = || {
            Serie::from_scalars(field.clone(), column.sequence_rows().unwrap().into_owned())
                .unwrap()
                .require_arrow_array()
                .unwrap()
        };
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

/// The bridge's own capture, the corpus `rust/tests/fix/ulbridge.rs` reads
/// whole and the `fix/ulbridge` benchmark times.
const ULBRIDGE: &[u8] = include_bytes!("fix/ulbridge.log");

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
/// split off an execution of side `UNKN` where it noted an anomaly: the
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
/// buffer: 1589.
///
/// [`projecting_a_root_projects_every_level_below_it_into_its_own_cache`]: ../root/field.rs
const FIX_PIPELINE_COSTS: [(&str, usize, StageCosts); 3] = [
    (
        "bridge_pipe",
        1,
        StageCosts {
            parse: 595,
            into_row: 152,
            landing: 1589,
            batch: 224,
            digest: 24,
            lifecycle: 7,
        },
    ),
    (
        "frame_pipe",
        72,
        StageCosts {
            parse: 241,
            into_row: 109,
            landing: 1571,
            batch: 224,
            digest: 24,
            lifecycle: 7,
        },
    ),
    (
        "frame_packed",
        111,
        StageCosts {
            parse: 1153,
            into_row: 279,
            landing: 1608,
            batch: 224,
            digest: 16,
            lifecycle: 7,
        },
    ),
];

#[test]
fn a_real_line_costs_the_same_at_every_stage_every_time() {
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
    let schema = yggdryl::fix_schema(&registry, "fix").expect("the fixed schema");
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
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
        let folder = yggdryl::local::LocalFolder::new(root).expect("the local seed path");
        FixRegistry::from_handle(&folder).expect("the committed dictionary loads")
    })
}

#[test]
fn a_word_alias_lookup_allocates_nothing() {
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
#[test]
fn instrument_codes_construct_and_classify_without_allocating() {
    use yggdryl::{Bbg, Cfi, Cusip, Figi, Isin, Ric, Sedol};
    free("long Bloomberg validation", || {
        assert!(Bbg::is_canonical("AAPL US Equity Long Identifier"));
    });
    free("RIC construction", || {
        black_box(Ric::new(black_box("0005.HK")).unwrap());
    });
    let ric = Ric::new("VOD.L").unwrap();
    free("RIC validation and exchange code", || {
        assert!(Ric::is_canonical(black_box("0#.FTSE")));
        assert_eq!(black_box(&ric).exchange_code(), Some("L"));
    });
    free("ISIN construction", || {
        std::hint::black_box(Isin::new("us0378331005").unwrap());
    });
    free("CUSIP construction", || {
        std::hint::black_box(Cusip::new("037833100").unwrap());
    });
    free("SEDOL construction", || {
        std::hint::black_box(Sedol::new("b0swjx3").unwrap());
    });
    let figi = Figi::new("BBG000BLNQ16").unwrap();
    free("FIGI construction", || {
        black_box(Figi::new("bbg000blnq16").unwrap());
    });
    free("FIGI clone", || {
        black_box(figi.clone());
    });
    free("CFI validation", || {
        assert!(Cfi::is_classified("ESVUFR"));
    });
    free("CFI merging", || {
        assert_eq!(Cfi::merged("ESXXXX", "ESVUFR").as_deref(), Some("ESVUFR"));
    });
    free("CFI inference", || {
        assert_eq!(Cfi::coarse('E', Some('S')).as_deref(), Some("ESXXXX"));
    });
}

#[test]
fn a_forex_pair_and_an_fx_symbol_read_without_allocating() {
    use yggdryl::{Forex, FxSymbol, FxTenor};
    // Two legs on the stack, two binary searches, and a seven-byte pair
    // inline in its string: no spelling costs a heap block.
    free("Forex construction from another spelling", || {
        black_box(Forex::new(black_box(" eur-usd ")).unwrap());
    });
    free("Forex canonical check", || {
        assert!(Forex::is_canonical(black_box("EUR/USD")));
    });
    let metal = Forex::new("XAU/USD").unwrap();
    free("Forex legs", || {
        assert_eq!(black_box(&metal).base().as_str(), "XAU");
        assert_eq!(black_box(&metal).quote().as_str(), "USD");
        assert!(black_box(&metal).is_metal());
    });
    // The detector, over every branch it takes: a settlement type is at
    // most four bytes, inline too.
    for (symbol, tenor) in [
        ("EURUSD", Some(FxTenor::Unstated)),
        ("EUR-USD 1M", Some(FxTenor::Forward)),
        ("EUR/USD 123D", Some(FxTenor::Forward)),
        ("GBPUSD SPOT", Some(FxTenor::Spot)),
        ("EURUSD SN", Some(FxTenor::Forward)),
        ("EURGBP=R", Some(FxTenor::Spot)),
        ("EUR/USD Curncy", Some(FxTenor::Unstated)),
        ("EUR=", None),
        ("EUR/USD XYZ", None),
        ("AAPL", None),
    ] {
        free(symbol, || {
            assert_eq!(
                black_box(FxSymbol::from_symbol(black_box(symbol))).map(|read| read.tenor),
                tenor
            );
        });
    }
}

#[test]
fn identifier_reads_and_inline_inserts_allocate_nothing() {
    use yggdryl::Identifiers;
    // A source, a type and a value, 24 bytes each: 72, where the retired
    // `parent` and `orig` were two more 24-byte strings at 120.
    assert_eq!(std::mem::size_of::<Identifier>(), 72);
    assert_eq!(std::mem::size_of::<Identifiers>(), IDENTIFIERS_SIZE);
    let mut ids = Identifiers::new();
    ids.insert(Identifier::new(IdSource::Base, IdType::Account, "ACC-1").unwrap());
    let venue: IdSource = "venue".parse().unwrap();
    ids.insert(Identifier::new(venue.clone(), IdType::UserId, "U-1").unwrap());
    free("an Identifiers read of a held type", || {
        assert_eq!(ids.get(black_box(&IdType::Account)), Some("ACC-1"));
    });
    free("an Identifiers read of a held type and source", || {
        assert_eq!(
            ids.get_from(black_box(&venue), black_box(&IdType::UserId)),
            Some("U-1")
        );
    });
    free("an Identifiers read of an absent type", || {
        assert_eq!(ids.get(black_box(&IdType::DeskId)), None);
    });
    // A word is folded on the stack: a member is static text and any other
    // word within SmolStr's inline width is held inline, so reading a
    // spelling into its type costs nothing either.
    free("an IdType read of a member through an alias", || {
        assert_eq!(
            black_box("ISIN_Number").parse::<IdType>().unwrap(),
            IdType::Isin
        );
    });
    free("an IdType read of a short word no member names", || {
        assert!(
            !black_box("House Code")
                .parse::<IdType>()
                .unwrap()
                .is_known()
        );
    });
    free("an IdSource read of a member", || {
        assert_eq!(
            black_box("PROPRIETARY").parse::<IdSource>().unwrap(),
            IdSource::Proprietary
        );
    });
    // A codified type and source are static text and a 23-byte value is the
    // widest SmolStr holds inline: two identifiers cost the set its one
    // backing and nothing each.
    costs("two identifiers into a new set", 1, || {
        let mut ids = Identifiers::new();
        assert!(ids.insert(Identifier::new(IdSource::Fix, IdType::OrderId, "Z").unwrap()));
        assert!(
            ids.insert(
                Identifier::new(
                    black_box(IdSource::Bic),
                    black_box(IdType::ExecutingTrader),
                    black_box("ABCDEFGHIJKLMNOPQRSTUVW")
                )
                .unwrap()
            )
        );
        assert_eq!(ids.len(), 2);
        black_box(&ids);
    });
}

/// The size of [`yggdryl::Identifiers`]: one vector, its pointer, length and
/// capacity.
const IDENTIFIERS_SIZE: usize = 24;

#[test]
fn security_identifier_construction_is_inline_for_every_checked_code() {
    for (kind, code) in [
        (IdType::Isin, "US0378331005"),
        (IdType::Cusip, "037833100"),
        (IdType::Sedol, "0263494"),
        (IdType::Figi, "BBG000B9XRY4"),
        (IdType::Wkn, "716460"),
        (IdType::Valor, "3886335"),
        (IdType::Bloomberg, "AAPL US Equity"),
    ] {
        free(&format!("constructing {kind}:{code}"), || {
            let id = Identifier::new(
                black_box(IdSource::Base),
                black_box(kind.clone()),
                black_box(code),
            )
            .unwrap();
            assert_eq!(id.kind(), &kind);
            black_box(id.value());
        });
    }
    let widest = "B".repeat(32);
    costs("constructing a 32-byte Bloomberg identifier", 1, || {
        black_box(
            Identifier::new(
                IdSource::Base,
                IdType::Bloomberg,
                black_box(widest.as_str()),
            )
            .unwrap(),
        );
    });
    assert!(
        Identifier::new(IdSource::Base, IdType::Bloomberg, &"B".repeat(33)).is_err(),
        "33 bytes are refused"
    );
    let heap = Identifier::new(IdSource::Base, IdType::Bloomberg, &widest).unwrap();
    free("cloning a heap Bloomberg identifier", || {
        black_box(heap.clone());
    });

    let ids: yggdryl::Identifiers = [
        Identifier::new(IdSource::Base, IdType::Isin, "US0378331005").unwrap(),
        heap.clone(),
    ]
    .into_iter()
    .collect();
    free("an Identifiers read through every folded spelling", || {
        let read = |spelling: &str| spelling.parse::<IdType>().unwrap();
        assert_eq!(ids.get(&read(black_box("isin"))), Some("US0378331005"));
        assert_eq!(
            ids.get(&read(black_box("Bloomberg"))),
            Some(widest.as_str())
        );
        assert_eq!(ids.get(&read(black_box("sedol"))), None);
    });
}

/// A rowset row is written cell by cell off the column leaves - a text cell
/// lends its bytes where they lie, a number, a boolean or a null is spelled
/// straight into the sink - so the per-row path allocates nothing, and what a
/// document costs beyond its sink is the same at every corpus size.
#[test]
fn a_rowset_write_allocates_nothing_per_row() {
    let field = DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
            DataType::Float64.required_field("price"),
            DataType::Boolean.required_field("live"),
        ])
        .expect("a valid root"),
    )
    .required_field("row");
    let rowset = Rowset::new(field.clone()).expect("a rowset over a record field");
    let cost = |rows: usize| {
        let batch = Serie::from_scalars(
            field.clone(),
            (0..rows).map(|index| {
                Scalar::from_sequence([
                    Scalar::from(index as i64),
                    if index % 5 == 0 {
                        Scalar::Null
                    } else {
                        Scalar::from(format!("SYM{index:04}"))
                    },
                    Scalar::from(index as f64 * 0.25),
                    Scalar::from(index % 2 == 0),
                ])
            }),
        )
        .expect("rows under the field");
        // Reserved past what the rows take, so the sink never grows and the
        // count is the row path's alone.
        let mut document = Vec::with_capacity(rows * 160);
        let (allocations, ()) = counted(|| {
            rowset
                .write_rows(&mut document, black_box(&batch))
                .expect("the rows are written");
        });
        assert!(
            document.len() < rows * 160,
            "{rows} rows filled the reserved sink"
        );
        assert!(document.ends_with(b"</row>"));
        allocations
    };
    let (small, large) = (cost(64), cost(4_096));
    assert_eq!(
        small, large,
        "64 rows cost {small} allocations, 4096 cost {large}; a rowset row must be written off its leaves"
    );
}

/// A record batch of `columns` required int64 columns and `rows` rows.
fn int_batch(columns: usize, rows: usize) -> arrow_array::RecordBatch {
    let fields: Vec<arrow_schema::Field> = (0..columns)
        .map(|index| {
            arrow_schema::Field::new(format!("c{index:03}"), arrow_schema::DataType::Int64, false)
        })
        .collect();
    let arrays: Vec<arrow_array::ArrayRef> = (0..columns)
        .map(|column| {
            Arc::new(arrow_array::Int64Array::from(
                (0..rows)
                    .map(|row| i64::try_from(row * column).expect("small"))
                    .collect::<Vec<_>>(),
            )) as arrow_array::ArrayRef
        })
        .collect();
    arrow_array::RecordBatch::try_new(Arc::new(arrow_schema::Schema::new(fields)), arrays)
        .expect("a batch")
}

#[test]
fn a_held_reader_costs_its_root_and_an_exact_batch_lands_for_less_than_an_imported_one() {
    // A held column becomes a stream for the root and the record vector,
    // whatever its width: no plan is compiled for records that are yielded
    // as they stand. A batch that already lays out as its root lands under
    // it as it stands too, for less than the same batch under no root -
    // which imports the root first - where it once compiled a plan and paid
    // twice the landing.
    let mut held = Vec::new();
    for columns in [2_usize, 64] {
        let batch = int_batch(columns, 16);
        let root = Field::from_arrow_schema("row", batch.schema().as_ref()).expect("the root");
        let records =
            Serie::from_arrow_batch(None, &batch, ArrowCastOptions::new()).expect("lands");
        let (ctor, _) =
            counted(|| yggdryl::SerieReader::from_serie(records.clone()).expect("a stream"));
        held.push(ctor);
        let (with_root, _) = counted(|| {
            Serie::from_arrow_batch(Some(&root), &batch, ArrowCastOptions::new()).expect("exact")
        });
        let (without_root, _) = counted(|| {
            Serie::from_arrow_batch(None, &batch, ArrowCastOptions::new()).expect("imported")
        });
        assert!(
            with_root <= without_root,
            "an exact {columns}-column batch under its root cost {with_root} against {without_root} imported"
        );
    }
    // The root's clone, the preflight walk's pending vector, the record
    // vector and the schema's box - its field list is the root projection's
    // own, shared - and, past sixteen children, the name set the record's
    // validation builds and grows.
    assert_eq!(
        held,
        [7, 9],
        "a held stream cost {held:?} at 2 and 64 columns: a plan compiled per column"
    );
}

#[test]
fn a_reader_lands_each_batch_under_boxes_resolved_once() {
    // Every batch of a stream lands under the same root, so the box each
    // child column holds its field in is resolved once with the plan and
    // borrowed per batch: a batch costs its leaves and its record, and
    // eight batches cost eight times one.
    let batch = int_batch(64, 16);
    let root = Field::from_arrow_schema("row", batch.schema().as_ref()).expect("the root");
    let drain = |batches: usize| {
        let reader = yggdryl::arrow::batch_reader(batch.schema(), vec![batch.clone(); batches]);
        let stream =
            yggdryl::SerieReader::from_arrow_reader(Some(&root), reader, ArrowCastOptions::new())
                .expect("one plan");
        let (allocations, drained) = counted(move || {
            stream
                .map(|record| record.expect("lands").len())
                .sum::<usize>()
        });
        assert_eq!(drained, 16 * batches);
        allocations
    };
    let (one, two, eight) = (drain(1), drain(2), drain(8));
    let per_batch = two - one;
    assert_eq!(
        eight - one,
        7 * per_batch,
        "eight batches did not cost eight times one"
    );
    // One box per leaf column, and five for the batch itself: the struct
    // array of its columns and that array's box, the plan's output vector,
    // the children vector, and the record's own box. A box per child field
    // per batch would be sixty-four more.
    assert_eq!(per_batch, 64 + 5, "a batch of 64 leaves cost {per_batch}");
}

/// `rows` rows of `dtype`, every third one absent, through the field's own
/// contract.
fn leaf_rows(field: &Field, rows: usize) -> Vec<Scalar> {
    (0..rows)
        .map(|index| {
            if index % 3 == 1 {
                return Scalar::Null;
            }
            let count = i64::try_from(index).expect("a row count");
            let value = match field.dtype() {
                DataType::Boolean => Scalar::from(index % 2 == 0),
                DataType::Float64 => Scalar::from(count as f64),
                DataType::DateTime64 { .. } => {
                    Scalar::datetime64(count * 1_000, TimeUnit::Nanosecond, Timezone::UTC)
                        .expect("an instant")
                }
                _ => Scalar::from(count),
            };
            field.scalar(value).expect("the field's contract")
        })
        .collect()
}

#[test]
fn laying_out_a_column_costs_its_buffers_whatever_the_row_count() {
    // The rows are written straight into the buffers a column publishes,
    // sized once from the row count - no vector of options grows between
    // the rows and the array, and a serie's items are reserved once - so a
    // column of sixteen thousand rows costs what one of sixteen costs.
    let leaves = [
        Field::new("count", DataType::Int64, true),
        Field::new(
            "price",
            DataType::decimal128(18, 4).expect("a decimal"),
            true,
        ),
        Field::new(
            "at",
            DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC).expect("an instant"),
            true,
        ),
        Field::new("live", DataType::Boolean, true),
        Field::new("weight", DataType::Float64, true),
    ];
    for leaf in &leaves {
        let field = Arc::new(leaf.clone());
        let mut counts = Vec::new();
        for rows in [16_usize, 1_024, 16_384] {
            let rows = leaf_rows(leaf, rows);
            let lay_out = || Serie::from_scalars(Arc::clone(&field), rows.clone()).expect("lands");
            drop(lay_out());
            let (allocations, _) = counted(lay_out);
            counts.push(allocations);
        }
        assert!(
            counts.windows(2).all(|pair| pair[0] == pair[1]),
            "a {} column cost {counts:?} at 16, 1024 and 16384 rows: grown per row",
            leaf.name()
        );
    }
    // A record of those leaves, and a serie of counts: the same rule at
    // every level.
    let root = Arc::new(
        DataType::from(StructType::from_fields(leaves.clone()).expect("five named children"))
            .required_field("row"),
    );
    let counts_field = Field::new("counts", DataType::serie(leaves[0].clone()), true);
    let mut record_counts = Vec::new();
    let mut serie_counts = Vec::new();
    for rows in [16_usize, 1_024, 16_384] {
        let columns: Vec<Vec<Scalar>> = leaves.iter().map(|leaf| leaf_rows(leaf, rows)).collect();
        let records: Vec<Scalar> = (0..rows)
            .map(|row| Scalar::from_sequence(columns.iter().map(|column| column[row].clone())))
            .collect();
        let lay_out = || Serie::from_scalars(Arc::clone(&root), records.clone()).expect("lands");
        drop(lay_out());
        let (allocations, _) = counted(lay_out);
        record_counts.push(allocations);

        let lists: Vec<Scalar> = (0..rows)
            .map(|row| {
                if row % 5 == 0 {
                    Scalar::Null
                } else {
                    Scalar::from_sequence(columns[0][..row % 4].iter().cloned())
                }
            })
            .collect();
        let field = Arc::new(counts_field.clone());
        let lay_out = || Serie::from_scalars(Arc::clone(&field), lists.clone()).expect("lands");
        drop(lay_out());
        let (allocations, _) = counted(lay_out);
        serie_counts.push(allocations);
    }
    assert!(
        record_counts.windows(2).all(|pair| pair[0] == pair[1]),
        "a record column cost {record_counts:?} at 16, 1024 and 16384 rows: grown per row"
    );
    assert!(
        serie_counts.windows(2).all(|pair| pair[0] == pair[1]),
        "a serie column cost {serie_counts:?} at 16, 1024 and 16384 rows: grown per row"
    );
}

#[test]
fn a_record_crosses_into_a_batch_for_one_box_per_leaf() {
    // Every child's projection lives in its own cache, filled once when the
    // root was projected and shared by every clone of the record, so a
    // batch is the leaves' own arrays boxed, and nothing projected again:
    // a hundred more columns are a hundred more boxes.
    let mut counts = Vec::new();
    for columns in [100_usize, 200] {
        let batch = int_batch(columns, 4);
        let records =
            Serie::from_arrow_batch(None, &batch, ArrowCastOptions::new()).expect("lands");
        drop(records.into_arrow_batch().expect("a batch"));
        let (allocations, _) = counted(|| records.into_arrow_batch().expect("a batch"));
        counts.push(allocations);
    }
    assert_eq!(
        counts[1] - counts[0],
        100,
        "a record of 100 and 200 columns crossed into batches for {counts:?}: projected per column"
    );
}

/// Resolving omitted options costs one clone of the wrapper's stored settings,
/// independent of the number of rows. Explicit and omitted paths share all
/// parsing, selection and publication work, so no absolute codec cost is pinned.
#[test]
fn iomedia_omitted_options_allocate_only_one_owned_options_clone() {
    use yggdryl::ipc::{Ipc, IpcOptions};
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{IOBase, IOMedia, IOMode, SerieReader};

    let mut overhead = Vec::new();
    for rows in [64, 4_096] {
        let batch = int_batch(2, rows);
        let records = Serie::from_arrow_batch(None, &batch, ArrowCastOptions::new()).unwrap();
        let stream = || SerieReader::from_serie(records.clone()).unwrap();
        let buffer = || Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
        let mut raw = buffer();
        raw.write_arrow(stream(), IOMode::Overwrite, None).unwrap();
        let selected = IpcOptions::new().with_select("c001").unwrap();
        let source = Ipc::new(raw).with_options(selected.clone());
        let options = source.record_options().unwrap();
        let read = |options| {
            let reader = source.read_arrow(options).unwrap();
            assert_eq!(reader.field().field_len(), 1);
            assert_eq!(reader.field().fields()[0].name(), "c001");
            reader.map(|column| column.unwrap().len()).sum::<usize>()
        };
        // Warm both routes before measuring, including any projection caches.
        assert_eq!(read(Some(&options)), rows);
        assert_eq!(read(None), rows);
        let (cloned, _) = counted(|| source.record_options().unwrap());
        let (explicit_read, count) = counted(|| read(Some(&options)));
        assert_eq!(count, rows);
        let (omitted_read, count) = counted(|| read(None));
        assert_eq!(count, rows);
        assert_eq!(omitted_read, explicit_read + cloned, "read: {rows} rows");

        for options in [Some(&options), None] {
            let mut warm = Ipc::new(buffer()).with_options(selected.clone());
            warm.write_arrow(stream(), IOMode::Overwrite, options)
                .unwrap();
        }
        let mut writes = Vec::new();
        let mut bytes = Vec::new();
        for options in [Some(&options), None] {
            // Fresh buffers have equal capacities; stream construction and
            // wrapper setup are outside the measured publication boundary.
            let mut target = Ipc::new(buffer()).with_options(selected.clone());
            let reader = stream();
            let (allocations, ()) = counted(|| {
                target
                    .write_arrow(reader, IOMode::Overwrite, options)
                    .unwrap();
            });
            writes.push(allocations);
            bytes.push(target.read_all_bytes().unwrap());
            let plain = RecordOptions::Ipc(IpcOptions::new());
            let stored = target.read_arrow(Some(&plain)).unwrap();
            assert_eq!(stored.field().field_len(), 1);
            assert_eq!(stored.field().fields()[0].name(), "c001");
            assert_eq!(
                stored.map(|column| column.unwrap().len()).sum::<usize>(),
                rows
            );
        }
        assert_eq!(writes[1], writes[0] + cloned, "write: {rows} rows");
        assert_eq!(bytes[0], bytes[1]);
        overhead.push((omitted_read - explicit_read, writes[1] - writes[0], cloned));
    }
    assert_eq!(
        overhead[0], overhead[1],
        "resolving options never scales with rows"
    );
}

/// `rows` CSV records of `cells` cells each, comma-separated, quoted every
/// fourth cell so the quoted path runs too.
fn csv_records(rows: usize, cells: usize) -> Buffer {
    let mut text = String::new();
    for cell in 0..cells {
        let _ = write!(text, "{}c{cell}", if cell == 0 { "" } else { "," });
    }
    text.push('\n');
    for row in 0..rows {
        for cell in 0..cells {
            if cell > 0 {
                text.push(',');
            }
            // Fixed-width numbers, so every record is the same length and
            // the record buffer never has to grow past the first.
            if cell % 4 == 3 {
                let _ = write!(text, "\"v {row:05},{cell:03}\"");
            } else {
                let _ = write!(text, "{:08}", row * cells + cell);
            }
        }
        text.push('\n');
    }
    Buffer::from_bytes(text.into_bytes()).with_media_type(MediaType::from_file_name("rows.csv"))
}

/// What counting `rows` records of `cells` cells through `row_size` costs.
fn csv_count_cost(source: &Buffer, rows: usize) -> usize {
    let options = yggdryl::csv::CsvOptions::new();
    let media = yggdryl::csv::Csv::new(source.clone()).with_options(options);
    let (allocations, counted_rows) =
        counted(|| yggdryl::IOMedia::row_size(black_box(&media)).expect("a count"));
    assert_eq!(counted_rows as usize, rows, "every record was counted");
    allocations
}

/// What counting a CSV costs, by how many cells a record holds: twelve for
/// eight cells, eighteen for sixty-four, and the same at sixteen records as
/// at a thousand.
///
/// Seven for a one-cell record, measured: five for the transport and the
/// cutter - the stream over the handle and its box, the fetch buffer and
/// the box the coding chain ends in, the cutter's window - and one each for
/// the record buffer and the cell index on the first record. Every count
/// past that is those two vectors doubling up to the first record's width,
/// five and two more at eight cells and eight and five at sixty-four, and
/// nothing after it: a record is cut into the buffer the one before it was
/// cut into, and a cell is a range of it.
const CSV_COUNT_COSTS: [(usize, usize); 2] = [(8, 12), (64, 18)];

#[test]
fn tokenizing_csv_records_costs_a_constant_and_nothing_a_record() {
    for (cells, expected) in CSV_COUNT_COSTS {
        let narrow = csv_count_cost(&csv_records(16, cells), 16);
        let wide = csv_count_cost(&csv_records(1_024, cells), 1_024);
        assert_eq!(
            narrow, wide,
            "{cells} cells: 16 records cost {narrow} allocations and 1024 cost {wide}"
        );
        assert_eq!(
            narrow, expected,
            "{cells} cells: the count's constant moved"
        );
    }
}

/// The trades table every Excel allocation case lays out: an integer, a
/// nullable text and a float, the three storages a cell takes.
fn excel_field() -> Field {
    DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
            DataType::Float64.required_field("price"),
        ])
        .expect("a valid root"),
    )
    .required_field("row")
}

/// `count` trades rows under [`excel_field`].
fn excel_rows(count: usize) -> Serie {
    Serie::from_scalars(
        excel_field(),
        (0..count).map(|index| {
            Scalar::from_sequence([
                Scalar::from(index as i64),
                Scalar::from(format!("SYM{index:04}")),
                Scalar::from(index as f64 * 0.25),
            ])
        }),
    )
    .expect("rows under the field")
}

/// Landing an imported rectangle of null records retains one physical row
/// per record. Its work is independent of the number of null columns.
#[test]
fn excel_land_null_record_allocations_follow_rows_not_columns() {
    use yggdryl::{
        RecordHeader,
        excel::{CellRange, CellRef, Edit, Landing, Sheet, Workbook},
    };

    let cost = |rows: usize, width: usize| {
        let field = DataType::from(
            StructType::from_fields(
                (0..width).map(|column| DataType::Float64.nullable_field(format!("c{column}"))),
            )
            .unwrap(),
        )
        .required_field("record");
        let records = Serie::from_scalars(
            field,
            (0..rows).map(|_| Scalar::from_sequence(std::iter::repeat_n(Scalar::Null, width))),
        )
        .unwrap();
        let cells = Sheet::from_serie("Imported", &records, RecordHeader::None).unwrap();
        assert!(cells.dimension().is_none());
        let mut workbook = Workbook::new();
        workbook.add_sheet("Data").unwrap();
        let (allocations, applied) = counted(|| {
            workbook
                .apply(Edit::Land {
                    destination: Landing::At {
                        sheet: "Data".into(),
                        anchor: CellRef::new(0, 0),
                    },
                    cells: Box::new(cells),
                })
                .unwrap()
        });
        assert_eq!(
            applied.touched,
            [(
                "Data".into(),
                CellRange::new(
                    CellRef::new(0, 0),
                    CellRef::new(rows as u32 - 1, width as u32 - 1)
                )
            )]
        );
        allocations
    };
    let (short_one, short_wide) = (cost(64, 1), cost(64, 16));
    let (long_one, long_wide) = (cost(512, 1), cost(512, 16));
    assert_eq!(short_one, short_wide, "a null column allocated during Land");
    assert_eq!(long_one, long_wide, "a null column allocated during Land");
    // Only B-tree nodes for physical record rows grow with height; the
    // number of node allocations is below the number of new rows.
    assert!(
        long_one >= short_one && long_one - short_one < 512 - 64,
        "448 more null rows cost {long_one} versus {short_one} allocations"
    );
}

/// A reference and a range are parsed per cell of a part, and a cell is
/// looked up per row of a read, so none of them may allocate: a reference
/// is two integers, a range two references, and a lookup a walk of the
/// sheet's maps that hands back a borrow. The one thing a cell read builds
/// is the text of a number, which has no bytes until it is rendered.
#[test]
fn excel_cell_reads_allocate_nothing() {
    use yggdryl::{
        RecordHeader,
        excel::{CellRange, CellRef, Sheet},
    };

    let sheet =
        Sheet::from_serie("Sheet1", &excel_rows(64), RecordHeader::Source).expect("a sheet");
    let number: CellRef = "C3".parse().expect("a reference");
    let text: CellRef = "B3".parse().expect("a reference");
    let range: CellRange = "A1:C64".parse().expect("a range");
    free("a cell reference parse", || {
        let _ = black_box("$C$3".parse::<CellRef>());
    });
    free("a cell range parse", || {
        let _ = black_box("A1:C64".parse::<CellRange>());
    });
    free("a range test", || {
        let _ = black_box(range.contains(number));
    });
    free("a cell lookup", || {
        let _ = black_box(sheet.cell(number));
    });
    free("a number cell's value", || {
        let _ = black_box(sheet.scalar(number));
    });
    free("a text cell's value", || {
        let _ = black_box(sheet.scalar(text));
    });
    free("a row lookup", || {
        let _ = black_box(sheet.row(2).map(yggdryl::excel::Row::len));
    });
    free("the cells of a range", || {
        let _ = black_box(sheet.cells_in(range).count());
    });
    free("a cell's style", || {
        let _ = black_box(sheet.cell(number).map(yggdryl::excel::Cell::style));
    });
    free("the cell count", || {
        let _ = black_box(sheet.cell_count());
    });
    free("the dimension", || {
        let _ = black_box(sheet.dimension());
    });
    costs("the text of a number cell", 1, || {
        let _ = black_box(sheet.cell(number).map(yggdryl::excel::Cell::text));
    });
}

/// A cell is at most 80 bytes: its 48-byte value, its 8-byte reference, one
/// pointer to a shared formula, a 2-byte style and one byte each for its
/// kind, its format and its error - 69 bytes, rounded up to the value's
/// 16-byte alignment. A row is its index beside one vector of cells.
#[test]
fn excel_cell_is_at_most_80_bytes() {
    use yggdryl::excel::{Cell, Row};

    assert_eq!(std::mem::size_of::<Scalar>(), 48);
    assert_eq!(std::mem::size_of::<Cell>(), 80);
    assert_eq!(std::mem::size_of::<Row>(), 32);
}

/// What a sheet costs per row, at two corpus sizes so a per-cell cost would
/// show as a slope: laying rows out into cells is the row's own vector of
/// cells, reading them back is the run each row becomes and the record it
/// is laid out under, and rendering the part builds nothing per row - the
/// numbers are written from their digits and the text is interned as it is.
///
/// Re-pinned when a row became one exact-capacity vector of cells rather
/// than a B-tree node per eleven of them: parsing a row costs that vector
/// and nothing else, the parser's own buffer handed back for the next row.
#[test]
fn excel_sheet_costs_per_row_and_nothing_per_cell() {
    use yggdryl::{
        RecordHeader,
        excel::{Sheet, Workbook},
    };

    let field = excel_field();
    let cost = |count: usize| {
        let rows = excel_rows(count);
        let (laid_out, sheet) =
            counted(|| Sheet::from_serie("Sheet1", &rows, RecordHeader::Source).expect("a sheet"));
        let (read_back, serie) = counted(|| {
            sheet
                .clone()
                .into_serie(Some(&field), RecordHeader::Source, Default::default())
                .expect("the rows lay out")
        });
        assert_eq!(serie.len(), count);
        let mut workbook = Workbook::new();
        workbook.insert_sheet(sheet).expect("inserted");
        let (rendered, bytes) = counted(|| workbook.into_bytes().expect("the package"));
        let (opened, reopened) =
            counted(|| Workbook::from_bytes(bytes).expect("the package opens"));
        let (parsed, rows) = counted(|| reopened.sheet("Sheet1").expect("the sheet").len());
        assert_eq!(rows, count + 1);
        (laid_out, read_back, rendered, opened, parsed)
    };
    let (small, large) = (cost(64), cost(640));
    let more = 640 - 64;
    // The row's one vector, and a node of the rows map every few rows.
    assert!(
        large.0 - small.0 < more * 5 / 4,
        "laying out 576 more rows cost {} allocations, against {} for 64 rows",
        large.0 - small.0,
        small.0
    );
    // Under four per row: the cells gathered, the run, the record's row.
    assert!(
        large.1 - small.1 < more * 4,
        "reading back 576 more rows cost {} allocations",
        large.1 - small.1
    );
    // Rendering the package costs its parts, not its rows: the part is
    // written into one buffer and every cell from what it already holds.
    assert!(
        large.2 < small.2 + 64,
        "rendering 576 more rows cost {} allocations more",
        large.2 - small.2
    );
    // Opening reads the package documents and no sheet.
    assert_eq!(small.3, large.3, "opening a workbook cost a row");
    // The row's one vector of the exact length, and a node of the rows map
    // every few rows; the cells the parser read go back to it.
    assert!(
        large.4 - small.4 < more * 5 / 4,
        "parsing 576 more rows cost {} allocations",
        large.4 - small.4
    );
}

/// `bytes` with the text of member `part` edited, every other member copied
/// as stored.
fn excel_repacked(bytes: Vec<u8>, part: &str, edit: impl FnOnce(String) -> String) -> Vec<u8> {
    use yggdryl::holder::{Buffer, Holder};
    use yggdryl::zip::ZipArchive;

    let source = std::sync::Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(bytes))));
    let target = ZipArchive::new(Holder::buffer(Buffer::new()));
    let mut edit = Some(edit);
    for entry in source.entries().expect("the members") {
        if entry.name() == part {
            let text = String::from_utf8(source.read_member(part).expect("the part"))
                .expect("the part is text");
            let edit = edit.take().expect("one member of that name");
            target
                .write_member(part, edit(text).as_bytes())
                .expect("written");
        } else {
            target
                .copy_member_from(&source, entry.name())
                .expect("copied");
        }
    }
    target.flush().expect("flushed");
    match target.into_handle().expect("the handle") {
        Holder::Buffer(buffer) => buffer.into_bytes(),
        other => yggdryl::IOBase::read_all_bytes(&other).expect("the bytes"),
    }
}

/// A sheet parsed ten times as wide costs what the narrow one does: a row
/// is one vector however many cells it holds, a number cell's value is its
/// digits parsed in place, and the parser's buffer is reused row to row.
///
/// What only a few cells state (`cm`, `vm`, `ph`) is an entry beside the
/// cells, so a cell stating none has none: the same sheet with `ph` on
/// every cell costs the entries' nodes on top, which is what the flat cost
/// of the plain one proves it does not pay.
#[test]
fn excel_sheet_parse_costs_nothing_per_cell() {
    use yggdryl::excel::{CellRef, Sheet, Workbook};

    let cost = |width: u32, phonetic: bool| {
        let mut sheet = Sheet::new("Sheet1").expect("a sheet");
        for row in 0..256 {
            for column in 0..width {
                sheet
                    .set_cell(CellRef::new(row, column), f64::from(row * width + column))
                    .expect("a number cell");
            }
        }
        let mut workbook = Workbook::new();
        workbook.insert_sheet(sheet).expect("inserted");
        let mut bytes = workbook.into_bytes().expect("the package");
        if phonetic {
            bytes = excel_repacked(bytes, "xl/worksheets/sheet1.xml", |part| {
                part.replace("<c r=", "<c ph=\"1\" r=")
            });
        }
        let reopened = Workbook::from_bytes(bytes).expect("opens");
        let (parsed, cells) = counted(|| reopened.sheet("Sheet1").expect("the sheet").cell_count());
        assert_eq!(cells, 256 * width as usize);
        parsed
    };
    let (narrow, wide) = (cost(4, false), cost(40, false));
    assert!(
        wide < narrow + 16,
        "parsing 256 rows of 40 cells cost {wide} allocations, against {narrow} for 4 cells"
    );
    // A B-tree leaf holds eleven entries, so 10,240 of them are at least
    // 931 nodes.
    let stating = cost(40, true);
    assert!(
        stating >= wide + 256 * 40 / 11,
        "parsing 10,240 cells stating `ph` cost {stating} allocations, against {wide} for none"
    );
}

/// Text written into the part as an inline string - openpyxl writes every
/// string so - costs a text cell what a number cell costs: a plain `<is>`
/// is read through one buffer the parse reuses, and only a rich one's runs
/// are kept beside its cell.
#[test]
fn excel_inline_string_parse_costs_nothing_per_cell() {
    use yggdryl::excel::{CellRef, Sheet, Workbook};

    let cost = |width: u32| {
        let mut sheet = Sheet::new("Sheet1").expect("a sheet");
        for row in 0..256 {
            for column in 0..width {
                sheet
                    .set_cell(CellRef::new(row, column), f64::from(row * width + column))
                    .expect("a number cell");
            }
        }
        let mut workbook = Workbook::new();
        workbook.insert_sheet(sheet).expect("inserted");
        let bytes = excel_repacked(
            workbook.into_bytes().expect("the package"),
            "xl/worksheets/sheet1.xml",
            |part| {
                part.replace("\"><v>", "\" t=\"inlineStr\"><is><t>")
                    .replace("</v></c>", "</t></is></c>")
            },
        );
        let reopened = Workbook::from_bytes(bytes).expect("opens");
        let (parsed, cells) = counted(|| reopened.sheet("Sheet1").expect("the sheet").cell_count());
        assert_eq!(cells, 256 * width as usize);
        assert_eq!(
            reopened
                .sheet("Sheet1")
                .expect("the sheet")
                .scalar(CellRef::new(1, 0)),
            yggdryl::Scalar::from(width.to_string())
        );
        parsed
    };
    let (narrow, wide) = (cost(4), cost(40));
    assert!(
        wide < narrow + 16,
        "parsing 256 rows of 40 inline strings cost {wide} allocations, against {narrow} for 4"
    );
}

/// A save that appends a string extends the shared strings as the package
/// stores them - every item it held rewritten from the stored bytes - and
/// that rewrite holds nothing per item: a table sixty-four times as long
/// costs what the short one does, bar the output's growth.
#[test]
fn excel_shared_strings_extended_cost_nothing_per_item() {
    use yggdryl::excel::{CellRef, Workbook};

    let cost = |items: u32| {
        let mut workbook = Workbook::new();
        let table = workbook.add_sheet("Table").expect("a sheet");
        for row in 0..items {
            table
                .set_cell(CellRef::new(row, 0), format!("item {row}"))
                .expect("a text cell");
        }
        workbook.add_sheet("Edited").expect("a sheet");
        let reopened = Workbook::from_bytes(workbook.into_bytes().expect("the package"));
        let mut reopened = reopened.expect("opens");
        reopened.parse_all().expect("parsed");
        reopened
            .sheet_mut("Edited")
            .expect("the sheet")
            .set_cell(CellRef::new(0, 0), "new")
            .expect("a text cell");
        let (saved, bytes) = counted(|| reopened.into_bytes().expect("the package"));
        let written = Workbook::from_bytes(bytes).expect("opens");
        assert_eq!(
            written
                .sheet("Edited")
                .expect("the sheet")
                .scalar(CellRef::new(0, 0)),
            yggdryl::Scalar::from("new")
        );
        saved
    };
    let (short, long) = (cost(64), cost(4_096));
    assert!(
        long < short + 16,
        "extending a table of 4,096 strings cost {long} allocations, against {short} for 64"
    );
}

/// A shared formula group is one shape: every dependent holds the
/// master's `Arc`, so a column of `N` dependents costs what `N` number
/// cells cost plus the one shape - at two corpus sizes - where a column of
/// `N` formulas stated one by one lexes each, and interning keeps one.
#[test]
fn excel_shared_formula_parse_allocates_per_shape() {
    use yggdryl::excel::{CellRef, Sheet, Workbook};

    // A two-column sheet: `A` the row number, `B` twice it, as numbers, as
    // one shared group anchored in `B1`, or as a formula stated per cell.
    let cost = |rows: u32, formulas: Option<bool>| {
        let mut sheet = Sheet::new("Sheet1").expect("a sheet");
        for row in 0..rows {
            sheet
                .set_cell(CellRef::new(row, 0), f64::from(row + 1))
                .expect("a number cell");
            sheet
                .set_cell(CellRef::new(row, 1), f64::from(2 * (row + 1)))
                .expect("a number cell");
        }
        let mut workbook = Workbook::new();
        workbook.insert_sheet(sheet).expect("inserted");
        let mut bytes = workbook.into_bytes().expect("the package");
        if let Some(shared) = formulas {
            bytes = excel_repacked(bytes, "xl/worksheets/sheet1.xml", |part| {
                let mut pieces = part.split("<c r=\"B");
                let mut edited = pieces.next().expect("the head").to_owned();
                for (at, piece) in pieces.enumerate() {
                    let row = at + 1;
                    let (reference, rest) = piece.split_once('>').expect("a cell");
                    let formula = match (shared, row) {
                        (true, 1) => {
                            format!("<f t=\"shared\" ref=\"B1:B{rows}\" si=\"0\">A1*2</f>")
                        }
                        (true, _) => "<f t=\"shared\" si=\"0\"/>".to_owned(),
                        (false, _) => format!("<f>A{row}*2</f>"),
                    };
                    edited.push_str(&format!("<c r=\"B{reference}>{formula}{rest}"));
                }
                edited
            });
        }
        let reopened = Workbook::from_bytes(bytes).expect("opens");
        let (parsed, cells) = counted(|| reopened.sheet("Sheet1").expect("the sheet").cell_count());
        assert_eq!(cells, 2 * rows as usize);
        if formulas.is_some() {
            let sheet = reopened.sheet("Sheet1").expect("the sheet");
            let last = sheet.cell(CellRef::new(rows - 1, 1)).expect("B");
            assert_eq!(
                last.formula()
                    .expect("a formula")
                    .at(last.reference())
                    .to_string(),
                format!("A{rows}*2")
            );
        }
        parsed
    };
    let shared = |rows: u32| cost(rows, Some(true)) - cost(rows, None);
    let (small, large) = (shared(64), shared(1_024));
    assert!(
        large <= small + 4,
        "a shared group of 1,024 cells cost {large} allocations beyond its numbers, \
         against {small} for 64"
    );
    // Stated one by one, each formula is lexed before the interner finds
    // the shape it already holds.
    let stated = cost(1_024, Some(false)) - cost(1_024, None);
    assert!(
        stated > large + 1_024,
        "1,024 formulas stated one by one cost {stated} allocations, against {large} shared"
    );
}

/// What a sheet holds beside its cells: nothing while it is empty, one box
/// of column counts - [`MAX_COLUMNS`](yggdryl::excel::MAX_COLUMNS) of them,
/// 64 KiB - taken with its first cell and let go with its last, and per
/// cell nothing but what its row's vector grows by.
#[test]
fn excel_sheet_holds_one_box_of_column_counts_and_its_rows() {
    use yggdryl::excel::{CellRef, Sheet};

    let (built, sheet) = counted(|| Sheet::new("Held").expect("a sheet"));
    assert_eq!(built, 0, "an empty sheet allocated");
    let mut sheet = sheet;
    let (first, _) = counted(|| sheet.set_cell(CellRef::new(0, 0), 1.0).expect("a cell"));
    // The box of column counts, the rows map's node, the row's vector.
    assert_eq!(first, 3, "the first cell");
    let (same_row, ()) = counted(|| {
        for column in 1..4 {
            sheet
                .set_cell(CellRef::new(0, column), 1.0)
                .expect("a cell");
        }
    });
    assert_eq!(
        same_row, 0,
        "three more cells the row's vector holds room for"
    );
    let (grown, _) = counted(|| sheet.set_cell(CellRef::new(0, 4), 1.0).expect("a cell"));
    assert_eq!(grown, 1, "a fifth cell grows the row's vector");
    let (next_row, _) = counted(|| sheet.set_cell(CellRef::new(1, 0), 1.0).expect("a cell"));
    assert_eq!(next_row, 1, "a cell in a new row is that row's vector");
    let (replaced, _) = counted(|| sheet.set_cell(CellRef::new(1, 0), 2.0).expect("a cell"));
    assert_eq!(replaced, 0, "replacing a cell");

    // Emptied, the sheet lets the box go; its next cell takes it again.
    let references: Vec<CellRef> = sheet.cells().map(yggdryl::excel::Cell::reference).collect();
    for reference in references {
        sheet.remove_cell(reference);
    }
    assert_eq!(sheet.cell_count(), 0);
    // The box and the row's vector: an emptied map keeps its root node.
    let (again, _) = counted(|| sheet.set_cell(CellRef::new(5, 5), 1.0).expect("a cell"));
    assert_eq!(again, 2, "the first cell of an emptied sheet");
}

/// The record doors: a write renders every row into the part it streams
/// and allocates nothing per row, and a read costs each row its cells and
/// the run it becomes - never a value copied, a reference built or an
/// attribute read into a string of its own.
#[test]
fn excel_record_doors_cost_per_row_and_nothing_per_cell() {
    use yggdryl::IOMedia;
    use yggdryl::media::IORecordOptions;

    let field = excel_field();
    let cost = |count: usize| {
        let batch = excel_rows(count).into_arrow_batch().expect("a batch");
        let mut handle = Buffer::new().with_media_type(MimeType::XLSX.into());
        let options = handle
            .record_options()
            .expect("Excel options")
            .with_field(field.clone());
        let (written, ()) = counted(|| {
            handle
                .overwrite_arrow_batch(batch.clone(), &options)
                .expect("the rows write")
        });
        let (read, rows) = counted(|| {
            handle
                .read_arrow_reader(&options)
                .expect("a reader")
                .map(|batch| batch.expect("a batch").num_rows())
                .sum::<usize>()
        });
        assert_eq!(rows, count);
        (written, read)
    };
    let (small, large) = (cost(64), cost(640));
    let more = 640 - 64;
    assert!(
        large.0 < small.0 + 64,
        "writing 576 more rows cost {} allocations more",
        large.0 - small.0
    );
    assert!(
        large.1 - small.1 < more * 3,
        "reading 576 more rows cost {} allocations",
        large.1 - small.1
    );
}

/// Rendering a value under a parsed number format allocates the text it
/// answers and nothing else: the number's fifteen-digit form and the text
/// are built on the stack, so a text that fits inline allocates nothing, a
/// longer one its one buffer, and General its one list of narrower
/// spellings.
#[test]
fn excel_format_render_allocates_only_its_text() {
    use yggdryl::excel::{DateSystem, FormatCode};

    let system = DateSystem::Year1900;
    for (code, value) in [
        ("#,##0.00;[Red](#,##0.00)", Scalar::from(-1_234_567.891)),
        ("0.00%", Scalar::from(0.1234)),
        ("0.00E+00", Scalar::from(6.022e23)),
        ("# ??/??", Scalar::from(std::f64::consts::PI)),
        ("_(\"$\"* #,##0.00_)", Scalar::from(1234.5)),
        ("m/d/yyyy", Scalar::date32(19_724)),
        ("[h]:mm:ss.000", Scalar::from(1.500_01)),
        ("dddd, mmmm d, yyyy", Scalar::from(45_292.0)),
        ("0;-0;0;\"<\"@\">\"", Scalar::from("text")),
        ("General", Scalar::from(7.0)),
    ] {
        let format = FormatCode::from_code(code).expect("a format code");
        free(&format!("rendering under {code}"), || {
            let _ = black_box(format.render(&value, system));
        });
    }
    let long = FormatCode::from_code("\"Total amount due: \"#,##0.00").expect("a format code");
    costs("a text past the inline bound", 1, || {
        let _ = black_box(long.render(&Scalar::from(1234.5), system));
    });
    let general = FormatCode::general();
    for value in [1234.5678, 12_345_678_901.0] {
        costs("General's narrower spellings", 1, || {
            let _ = black_box(general.render(&Scalar::from(value), system));
        });
    }
}

/// A cell's number format is read once, when its style is: displaying a
/// cell reads the code the styles hold, never parses it again, and so
/// allocates only its text - nothing for one that fits inline, whichever
/// of a thousand cells in three styles (currency, percentage, date) it is.
#[test]
fn excel_display_text_reads_a_format_parsed_once_per_style() {
    use yggdryl::excel::{CellRef, Workbook};

    let mut workbook = Workbook::new();
    workbook.add_sheet("Sheet1").expect("a sheet");
    for row in 0..1_000_u32 {
        let text = match row % 3 {
            0 => format!("${row}.5"),
            1 => format!("{}%", row % 100),
            _ => "1/2/2024".to_owned(),
        };
        workbook
            .set_entry("Sheet1", CellRef::new(row, 0), &text)
            .expect("an entry");
    }
    // The default style and the three the entries suggested.
    assert_eq!(workbook.style_sheet().expect("styles").len(), 4);
    let mut row = 0_u32;
    free("displaying a styled cell", || {
        row = (row + 1) % 1_000;
        let _ = black_box(workbook.display_text("Sheet1", CellRef::new(row, 0)));
    });
}

/// A patch over a range derives each distinct style it meets once: ten
/// thousand cells in three styles intern exactly three, and the cells cost
/// the plan of what changes - one entry per cell, in one vector grown as it
/// fills - and nothing each.
#[test]
fn excel_set_style_interns_one_style_per_distinct_source() {
    use yggdryl::excel::{CellRange, CellRef, StylePatch, Workbook};

    let patched = |rows: u32| {
        let mut workbook = Workbook::new();
        let sheet = workbook.add_sheet("Sheet1").expect("a sheet");
        for row in 0..rows {
            for column in 0..100 {
                sheet
                    .set_cell(CellRef::new(row, column), f64::from(row + column))
                    .expect("a cell");
            }
        }
        let tenth = rows / 10;
        for (first, code) in [(0, "0%"), (tenth, "m/d/yyyy")] {
            let patch = StylePatch {
                number_format: Some(code.into()),
                ..StylePatch::default()
            };
            let range = CellRange::new(CellRef::new(first, 0), CellRef::new(first + tenth - 1, 99));
            workbook
                .set_style("Sheet1", &[range], &patch)
                .expect("a patch");
        }
        let before = workbook.style_sheet().expect("styles").len();
        let bold = StylePatch {
            bold: Some(true),
            ..StylePatch::default()
        };
        let all = CellRange::new(CellRef::new(0, 0), CellRef::new(rows - 1, 99));
        let (allocations, ()) = counted(|| {
            workbook
                .set_style("Sheet1", &[all], &bold)
                .expect("a patch");
        });
        assert_eq!(workbook.style_sheet().expect("styles").len(), before + 3);
        allocations
    };
    let small = patched(25);
    let large = patched(100);
    eprintln!("excel_set_style: 2,500 cells {small}, 10,000 cells {large}");
    // Four times the cells: the same three styles, and a plan grown by
    // doubling - two more reallocations - but nothing per cell.
    assert!(
        large <= small + 4,
        "2,500 cells cost {small}, 10,000 cost {large}"
    );
}

/// Rows opened above a sheet of formulas move every cell and rewrite every
/// reference, and the rewrites are memoized per shape and per class of
/// host the rows treat alike: a column of `N` cells sharing one shape costs
/// what a column of `N` numbers costs - the rows map rebuilt, the cells
/// moved in their own vectors - plus the one rewritten shape, at two corpus
/// sizes.
#[test]
fn excel_structural_edit_allocates_per_shape_and_nothing_per_formula() {
    use yggdryl::excel::{Cell as ExcelCell, CellRef, DateSystem, Formula, Workbook};

    // `B` is `A2*2` in every row, `C` names the absolute `$E$1` above
    // every row: opening rows above row 1 leaves `B`'s shape as it is and
    // gives `C` one new shape, `A2+$E$3`, held by every cell.
    let cost = |rows: u32, formulas: bool| {
        let mut workbook = Workbook::new();
        let data = workbook.add_sheet("Data").expect("a sheet");
        let shapes = [
            Formula::from_file("A1*2", CellRef::new(0, 1)),
            Formula::from_file("A1+$E$1", CellRef::new(0, 2)),
        ];
        for row in 0..rows {
            data.set_cell(CellRef::new(row, 0), f64::from(row))
                .expect("a number");
            for (column, shape) in (1..).zip(&shapes) {
                let at = CellRef::new(row, column);
                let cell = ExcelCell::from_scalar(at, f64::from(row).into(), DateSystem::Year1900)
                    .expect("a cell");
                data.insert_cell(if formulas {
                    cell.with_formula(shape.clone())
                } else {
                    cell
                })
                .expect("a cell");
            }
        }
        let (allocations, ()) = counted(|| {
            workbook.insert_rows("Data", 0, 2).expect("the rows open");
        });
        if formulas {
            let sheet = workbook.sheet("Data").expect("the sheet");
            let last = sheet.cell(CellRef::new(rows + 1, 2)).expect("moved");
            assert_eq!(
                last.formula()
                    .expect("a formula")
                    .at(last.reference())
                    .to_string(),
                format!("A{}+$E$3", rows + 2)
            );
        }
        allocations
    };
    let formulas = |rows: u32| cost(rows, true).saturating_sub(cost(rows, false));
    let (small, large) = (formulas(1_024), formulas(16_384));
    eprintln!(
        "excel_structural_edit: numbers {} / {}, formulas beyond them {small} / {large}",
        cost(1_024, false),
        cost(16_384, false)
    );
    assert!(
        large <= small + 4,
        "16,384 rows of formulas cost {large} allocations beyond their numbers, against \
         {small} for 1,024"
    );
}

/// A paste walks the cells its two ranges hold, never the grid between
/// them: pasting the whole grid of a sparse sheet costs what pasting the
/// one column holding its cells costs, and each cell more costs a bounded
/// few allocations, at two corpus sizes. A walk of the grid's 17 billion
/// coordinates would not finish.
#[test]
fn excel_paste_allocates_per_cell_held_and_nothing_per_cell_of_the_grid() {
    let cost = |cells, range| excel_sparse_paste_cost(cells, range, false);
    let (column, grid) = (cost(256, "A:A"), cost(256, "A1:XFD1048576"));
    assert!(
        grid <= column + 8,
        "the whole grid cost {grid} allocations, the column holding the same cells {column}"
    );
    let (small, large) = (cost(16, "A1:XFD1048576"), grid);
    assert!(
        large - small < (256 - 16) * 8,
        "240 more cells cost {} allocations more",
        large - small
    );
}

/// Numeric sparse cells: copying and cutting have separate pins, sharing
/// only the setup and postconditions. Formula relocation is not inferred
/// from a copy's costs or semantics.
fn excel_sparse_paste_cost(cells: u32, range: &str, cut: bool) -> usize {
    use yggdryl::excel::{CellRef, Paste, Workbook};

    let mut workbook = Workbook::new();
    let from = workbook.add_sheet("From").expect("a sheet");
    for row in 0..cells {
        from.set_cell(CellRef::new(row * 97, 0), f64::from(row))
            .expect("a number");
    }
    workbook.add_sheet("To").expect("a sheet");
    let block = range.parse().expect("a range");
    let (allocations, landed) =
        counted(|| workbook.paste(("From", block), ("To", CellRef::new(0, 0)), Paste::All, cut));
    assert_eq!(landed.expect("the paste lands"), block);
    assert_eq!(
        workbook.sheet("To").expect("the sheet").len(),
        cells as usize
    );
    assert_eq!(
        workbook.sheet("From").expect("the source").len(),
        if cut { 0 } else { cells as usize },
    );
    assert_eq!(
        workbook
            .sheet("To")
            .unwrap()
            .scalar(CellRef::new((cells - 1) * 97, 0)),
        f64::from(cells - 1).into(),
    );
    allocations
}

/// A cut retains inverses and moves cells, but the empty grid between its
/// stored cells must cost nothing at either corpus size.
#[test]
fn excel_cut_allocates_nothing_per_empty_cell_of_the_grid() {
    for cells in [16, 256] {
        let column = excel_sparse_paste_cost(cells, "A:A", true);
        let grid = excel_sparse_paste_cost(cells, "A1:XFD1048576", true);
        eprintln!("excel_cut: {cells} cells, column {column}, grid {grid}");
        assert_eq!(grid, column, "cutting the empty columns cost allocations");
    }
}

/// Refusing intersecting whole tables precedes cell slices and inverses:
/// the table metadata is identical at both sizes, so populated cells in
/// the rejected cut add no allocations.
#[test]
fn excel_table_cut_collision_allocates_nothing_per_source_cell() {
    use excel_package::{
        NS, R_NS, content_types, package, root_relationships, workbook, workbook_relationships,
        worksheet,
    };
    use yggdryl::excel::Paste;

    let cost = |rows: u32| {
        let table = |id, name, range| {
            format!(
                "<table xmlns=\"{NS}\" id=\"{id}\" name=\"{name}\" displayName=\"{name}\" ref=\"{range}\" totalsRowShown=\"0\"><autoFilter ref=\"{range}\"/><tableColumns count=\"2\"><tableColumn id=\"1\" name=\"Item\"/><tableColumn id=\"2\" name=\"Cost\"/></tableColumns></table>"
            )
        };
        let data: String = (1..=rows).map(|row| {
            let headers = if row == 2 {
                "<c r=\"E2\" t=\"inlineStr\"><is><t>Item</t></is></c><c r=\"F2\" t=\"inlineStr\"><is><t>Cost</t></is></c><c r=\"N2\" t=\"inlineStr\"><is><t>Item</t></is></c><c r=\"O2\" t=\"inlineStr\"><is><t>Cost</t></is></c>"
            } else { "" };
            format!("<row r=\"{row}\"><c r=\"A{row}\"><v>{row}</v></c>{headers}</row>")
        }).collect();
        let sheet = worksheet(&data).replace("</worksheet>", "<tableParts count=\"2\"><tablePart r:id=\"rIdCosts\"/><tablePart r:id=\"rIdTaken\"/></tableParts></worksheet>");
        let relationships = format!(
            "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdCosts\" Type=\"{R_NS}/table\" Target=\"../tables/table1.xml\"/><Relationship Id=\"rIdTaken\" Type=\"{R_NS}/table\" Target=\"../tables/table2.xml\"/></Relationships>"
        );
        let declarations: String = (1..=2).map(|id| format!(
            "<Override PartName=\"/xl/tables/table{id}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml\"/>"
        )).collect();
        let types =
            content_types(1, false, false).replace("</Types>", &format!("{declarations}</Types>"));
        let bytes = package(&[
            ("[Content_Types].xml", &types),
            ("_rels/.rels", &root_relationships()),
            ("xl/workbook.xml", &workbook(&["Data"], false)),
            (
                "xl/_rels/workbook.xml.rels",
                &workbook_relationships(1, false, false),
            ),
            ("xl/worksheets/sheet1.xml", &sheet),
            ("xl/worksheets/_rels/sheet1.xml.rels", &relationships),
            ("xl/tables/table1.xml", &table(1, "Costs", "E2:F6")),
            ("xl/tables/table2.xml", &table(2, "Taken", "N2:O6")),
        ]);
        let mut opened = yggdryl::excel::Workbook::from_bytes(bytes).unwrap();
        opened.parse_all().unwrap();
        let revision = opened.sheet("Data").unwrap().revision();
        let source = "A1:F1048576".parse().unwrap();
        let target = "J1".parse().unwrap();
        let (allocations, result) =
            counted(|| opened.paste(("Data", source), ("Data", target), Paste::All, true));
        match result.unwrap_err() {
            yggdryl::Error::InvalidRecord { path, reason } => {
                assert_eq!(path.as_str(), "Data!N2:O6");
                assert_eq!(
                    reason.as_str(),
                    "expected the moved table Costs to avoid other tables, got Taken at N2:O6"
                );
            }
            error => panic!("expected the table collision, got {error}"),
        }
        assert!(!opened.is_dirty());
        assert_eq!(opened.sheet("Data").unwrap().revision(), revision);
        assert_eq!(
            opened
                .sheet("Data")
                .unwrap()
                .scalar(format!("A{rows}").parse().unwrap()),
            f64::from(rows).into()
        );
        allocations
    };
    // Registration's empty MarkupContext owns one process-wide lazy Arc.
    // Warm that allocation before comparing fresh workbooks, so this pin
    // also runs alone rather than relying on another Excel test's order.
    black_box(cost(1_024));
    let (small, large) = (cost(1_024), cost(16_384));
    eprintln!("excel_table_cut_collision: 1,024 cells {small}, 16,384 cells {large}");
    assert_eq!(large, small, "a refused cut allocated for its source cells");
}

/// Retaining an edit's cells allocates their slice, independent of style
/// identity. An appended style adds one descriptor allocation, even when
/// thousands of cells use it; styles read from a package add none.
#[test]
fn excel_retained_style_descriptors_allocate_per_distinct_style() {
    use yggdryl::excel::{CellRange, CellRef, Edit, StylePatch, Workbook};

    let cost = |cells: u32, kind: &str| {
        let mut book = Workbook::new();
        let sheet = book.add_sheet("Sheet1").unwrap();
        for row in 0..cells {
            sheet
                .set_cell(CellRef::new(row, 0), f64::from(row))
                .unwrap();
        }
        let range = CellRange::new(CellRef::new(0, 0), CellRef::new(cells - 1, 0));
        if kind != "default" {
            book.set_style(
                "Sheet1",
                &[range],
                &StylePatch {
                    bold: Some(true),
                    ..StylePatch::default()
                },
            )
            .unwrap();
        }
        if kind == "original" {
            book = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
            book.parse_all().unwrap();
        }
        book.style_sheet().unwrap();
        // Replacement keeps every cell in all three cases; clearing would
        // remove default blanks but retain styled blanks, changing the work.
        let edit = Edit::SetEntries {
            sheet: "Sheet1".into(),
            entries: (0..cells)
                .map(|row| (CellRef::new(row, 0), "8".into()))
                .collect(),
        };
        let (allocations, applied) = counted(|| book.apply(edit).unwrap());
        assert!(applied.inverse.is_some());
        assert_eq!(book.sheet("Sheet1").unwrap().cell_count(), cells as usize);
        allocations
    };
    // Warm datatype and style caches outside every measured operation.
    for kind in ["default", "original", "appended"] {
        cost(1, kind);
    }
    for cells in [64, 4_096] {
        let default = cost(cells, "default");
        let original = cost(cells, "original");
        let appended = cost(cells, "appended");
        eprintln!(
            "excel_retained_style_descriptors: {cells} cells, default={default}, original={original}, appended={appended}"
        );
        assert_eq!(
            original, default,
            "original styles retain no descriptors for {cells} cells"
        );
        assert_eq!(
            appended,
            default + 1,
            "one shared appended style retains one vector for {cells} cells"
        );
    }
}

/// An already parsed worksheet with only original style IDs retains its
/// root bytes and metadata once. Removing it must not walk raw cell tags
/// merely to discover that none can require a retained style descriptor.
#[test]
fn excel_remove_original_styles_allocates_nothing_per_cell() {
    use excel_package::{
        content_types, package_coded, root_relationships, styles, workbook, workbook_relationships,
        worksheet,
    };
    use yggdryl::excel::{CellRef, Edit, StyleId, Workbook};

    let cost = |cells: u32| {
        let mut row = String::from("<row r=\"1\">");
        for column in 0..cells {
            let at = CellRef::new(0, column);
            row.push_str(&format!("<c r=\"{at}\" s=\"1\"><v>7</v></c>"));
        }
        row.push_str("</row>");
        // Identity coding and a fixed package graph isolate the retained
        // payload's one allocation from decoder and XML traversal costs.
        let bytes = package_coded(
            &[
                ("[Content_Types].xml", &content_types(2, false, true)),
                ("_rels/.rels", &root_relationships()),
                ("xl/workbook.xml", &workbook(&["Original", "Keep"], false)),
                (
                    "xl/_rels/workbook.xml.rels",
                    &workbook_relationships(2, false, true),
                ),
                ("xl/worksheets/sheet1.xml", &worksheet(&row)),
                ("xl/worksheets/sheet2.xml", &worksheet("")),
                ("xl/styles.xml", &styles(&[(164, "0.000")], &[0, 164])),
            ],
            yggdryl::Codec::Identity,
        );
        let mut book = Workbook::from_bytes(bytes).unwrap();
        book.parse_all().unwrap();
        book.style_sheet().unwrap();
        assert!(!book.is_dirty());
        let last = CellRef::new(0, cells - 1);
        assert_eq!(book.sheet("Original").unwrap().cell_count(), cells as usize);
        assert_eq!(
            book.sheet("Original").unwrap().cell(last).unwrap().style(),
            StyleId::new(1),
        );
        let edit = Edit::RemoveSheet {
            name: "Original".into(),
        };
        let (allocations, applied) = counted(|| book.apply(edit).unwrap());
        assert_eq!(book.sheet_names(), vec!["Keep"]);
        let inverse = applied.inverse.expect("removal retains its original sheet");
        // Exercise the returned retention outside the measured region so
        // a cheap removal that loses original styles cannot satisfy the pin.
        book.apply(inverse).unwrap();
        assert_eq!(book.sheet("Original").unwrap().cell_count(), cells as usize);
        assert_eq!(
            book.cell_style("Original", last).unwrap().number_format,
            "0.000",
        );
        allocations
    };
    cost(1);
    let (small, large) = (cost(64), cost(4_096));
    eprintln!("excel_remove_original_styles: 64 cells {small}, 4,096 cells {large}");
    assert_eq!(
        large, small,
        "original-only style retention must not allocate while revisiting raw cell tags",
    );
}

/// A refusal that never appends a style must not clone a style table held
/// by an in-flight snapshot merely to roll it back to the same checkpoint.
#[test]
fn excel_failed_guard_without_styles_does_not_clone_the_shared_style_table() {
    use excel_package::{
        content_types, package_coded, root_relationships, styles, workbook, workbook_relationships,
        worksheet,
    };
    use yggdryl::excel::{Edit, Workbook};

    let cost = |formats: u32| {
        // Distinct custom-format map entries make a whole-table clone cost
        // visible to the counting allocator, even when every cell uses xf0.
        let codes: Vec<_> = (1..formats)
            .map(|id| (163 + id, format!("0.000&quot;tag{id}&quot;")))
            .collect();
        let borrowed: Vec<_> = codes
            .iter()
            .map(|(id, code)| (*id, code.as_str()))
            .collect();
        let xfs: Vec<_> = (0..formats)
            .map(|id| if id == 0 { 0 } else { 163 + id })
            .collect();
        let bytes = package_coded(
            &[
                ("[Content_Types].xml", &content_types(1, false, true)),
                ("_rels/.rels", &root_relationships()),
                ("xl/workbook.xml", &workbook(&["Sheet1"], false)),
                (
                    "xl/_rels/workbook.xml.rels",
                    &workbook_relationships(1, false, true),
                ),
                (
                    "xl/worksheets/sheet1.xml",
                    &worksheet("<row r=\"1\"><c r=\"A1\"><v>7</v></c></row>"),
                ),
                ("xl/styles.xml", &styles(&borrowed, &xfs)),
            ],
            yggdryl::Codec::Identity,
        );
        let mut book = Workbook::from_bytes(bytes).unwrap();
        book.parse_all().unwrap();
        assert_eq!(book.style_sheet().unwrap().len(), formats as usize);
        let pending = book.into_package().unwrap();
        let revision = book.sheet("Sheet1").unwrap().revision();
        let edit = Edit::RowHeight {
            sheet: "Sheet1".into(),
            start: 0,
            count: 1,
            height: Some(-1.0),
        };
        let (allocations, result) = counted(|| book.apply(edit));
        assert!(matches!(
            result.unwrap_err(),
            yggdryl::Error::InvalidRecord { .. }
        ));
        assert_eq!(book.style_sheet().unwrap().len(), formats as usize);
        assert_eq!(book.sheet("Sheet1").unwrap().revision(), revision);
        assert!(!book.is_dirty());
        black_box(&pending);
        allocations
    };
    cost(1);
    let (small, large) = (cost(64), cost(4_096));
    eprintln!("excel_failed_guard_without_styles: 64 XFs {small}, 4,096 XFs {large}");
    assert_eq!(
        large, small,
        "a refusal with no style append cloned the shared style table"
    );
}

/// A failed entry restores its cells and newly appended formats without
/// cloning the surrounding worksheet. Percent and date entry cover both
/// a new style ID and the cell's temporal-kind transition.
#[test]
fn excel_failed_guard_with_styles_costs_only_the_attempted_cells() {
    use yggdryl::excel::{CellRef, Workbook};

    let cost = |cells: u32, entry: &str| {
        let mut book = Workbook::new();
        let sheet = book.add_sheet("Sheet1").unwrap();
        // Keep an untouched sibling in each row: rollback need not remove
        // and reinsert BTree rows, while a whole-sheet clone scales by rows.
        for row in 0..cells / 2 {
            for column in 0..2 {
                sheet.set_cell(CellRef::new(row, column), 7.0).unwrap();
            }
        }
        let package = book.into_package().unwrap();
        book.rebase(package).unwrap();
        let pending = book.into_package().unwrap();
        let count = book.style_sheet().unwrap().len();
        let first = CellRef::new(0, 0);
        let second = CellRef::new(1, 0);
        let last = CellRef::new(cells / 2 - 1, 1);
        let style = book.cell_style("Sheet1", first).unwrap();
        let format = book.sheet("Sheet1").unwrap().cell(first).unwrap().format();
        let revision = book.sheet("Sheet1").unwrap().revision();
        let tsv = format!("{entry}\n=SUM(A1");
        let (allocations, result) = counted(|| book.paste_text("Sheet1", first, &tsv));
        assert!(matches!(result.unwrap_err(), yggdryl::Error::Parse { .. }));
        assert_eq!(book.style_sheet().unwrap().len(), count);
        let sheet = book.sheet("Sheet1").unwrap();
        assert_eq!(sheet.revision(), revision);
        assert_eq!(sheet.cell_count(), cells as usize);
        for at in [first, second, last] {
            assert_eq!(sheet.scalar(at), Scalar::from(7.0));
        }
        assert_eq!(sheet.cell(first).unwrap().format(), format);
        assert_eq!(book.cell_style("Sheet1", first).unwrap(), style);
        assert!(!book.is_dirty());
        black_box(&pending);
        allocations
    };
    for entry in ["12%", "1/1/2024"] {
        cost(4, entry);
        let (small, large) = (cost(64, entry), cost(4_096, entry));
        eprintln!(
            "excel_failed_guard_with_styles {entry:?}: 64 resident cells {small}, 4,096 cells {large}"
        );
        assert_eq!(
            large, small,
            "rollback copied cells outside its two-cell payload"
        );
    }
}

/// A fixed two-child failure owns only its touched cells and metadata.
/// More original XFs, other sheets or resident cells must not enlarge it.
#[test]
fn excel_failed_batch_allocates_only_for_touched_state() {
    use excel_package::{
        content_types, package_coded, root_relationships, styles, workbook, workbook_relationships,
        worksheet,
    };
    use yggdryl::excel::{Edit, Workbook};

    let cost = |formats: u32, sheets: usize, cells: u32| {
        let names: Vec<_> = (0..sheets).map(|id| format!("Sheet{id}")).collect();
        let borrowed_names: Vec<_> = names.iter().map(String::as_str).collect();
        let codes: Vec<_> = (1..formats)
            .map(|id| (163 + id, format!("0.000&quot;tag{id}&quot;")))
            .collect();
        let borrowed_codes: Vec<_> = codes
            .iter()
            .map(|(id, code)| (*id, code.as_str()))
            .collect();
        let xfs: Vec<_> = (0..formats)
            .map(|id| if id == 0 { 0 } else { 163 + id })
            .collect();
        let rows: String = (1..=cells / 2).map(|row| format!(
            "<row r=\"{row}\"><c r=\"A{row}\"><v>7</v></c><c r=\"B{row}\"><v>7</v></c></row>"
        )).collect();
        let mut parts = vec![
            (
                "[Content_Types].xml".to_owned(),
                content_types(sheets, false, true),
            ),
            ("_rels/.rels".to_owned(), root_relationships()),
            (
                "xl/workbook.xml".to_owned(),
                workbook(&borrowed_names, false),
            ),
            (
                "xl/_rels/workbook.xml.rels".to_owned(),
                workbook_relationships(sheets, false, true),
            ),
            ("xl/styles.xml".to_owned(), styles(&borrowed_codes, &xfs)),
        ];
        for id in 1..=sheets {
            parts.push((
                format!("xl/worksheets/sheet{id}.xml"),
                worksheet(if id == 1 {
                    &rows
                } else {
                    "<row r=\"1\"><c r=\"A1\"><v>7</v></c><c r=\"B1\"><v>7</v></c></row>"
                }),
            ));
        }
        let borrowed: Vec<_> = parts
            .iter()
            .map(|(name, text)| (name.as_str(), text.as_str()))
            .collect();
        let mut book =
            Workbook::from_bytes(package_coded(&borrowed, yggdryl::Codec::Identity)).unwrap();
        book.parse_all().unwrap();
        assert_eq!(book.style_sheet().unwrap().len(), formats as usize);
        let revisions: Vec<_> = names
            .iter()
            .map(|name| book.sheet(name).unwrap().revision())
            .collect();
        let edit = Edit::Batch(vec![
            Edit::SetEntries {
                sheet: "Sheet0".into(),
                entries: vec![("A1".parse().unwrap(), "8".into())],
            },
            // An existing target avoids measuring construction of a missing-
            // sheet error listing every unrelated tab in the workbook.
            Edit::RowHeight {
                sheet: "Sheet0".into(),
                start: 0,
                count: 1,
                height: Some(-1.0),
            },
        ]);
        let (allocations, result) = counted(|| book.apply(edit));
        assert!(matches!(
            result.unwrap_err(),
            yggdryl::Error::InvalidRecord { .. }
        ));
        assert_eq!(
            book.sheet("Sheet0").unwrap().scalar("A1".parse().unwrap()),
            Scalar::from(7.0)
        );
        assert_eq!(book.sheet("Sheet0").unwrap().cell_count(), cells as usize);
        assert_eq!(
            names
                .iter()
                .map(|name| book.sheet(name).unwrap().revision())
                .collect::<Vec<_>>(),
            revisions
        );
        assert_eq!(book.style_sheet().unwrap().len(), formats as usize);
        assert!(!book.is_dirty());
        allocations
    };
    cost(1, 2, 4);
    let baseline = cost(64, 2, 64);
    for (axis, formats, sheets, cells) in [
        ("original styles", 4_096, 2, 64),
        ("unrelated sheets", 64, 128, 64),
        ("resident cells", 64, 2, 4_096),
    ] {
        let larger = cost(formats, sheets, cells);
        eprintln!("excel_failed_batch {axis}: baseline {baseline}, larger {larger}");
        assert_eq!(
            larger, baseline,
            "Batch rollback allocated for {axis} outside its touched state"
        );
    }
}

/// A note-only cut allocates for its metadata, never unrelated parsed cells.
#[test]
fn excel_note_cut_costs_notes_and_not_unrelated_cells() {
    use yggdryl::excel::{Paste, Workbook};
    let cost = |notes, cells| {
        let mut workbook =
            Workbook::from_bytes(excel_package::note_cost_package(notes, cells)).unwrap();
        workbook.parse_all().unwrap();
        let (allocations, result) = counted(|| {
            workbook.paste(
                ("Data", "B2".parse().unwrap()),
                ("Data", "F6".parse().unwrap()),
                Paste::All,
                true,
            )
        });
        assert_eq!(result.unwrap().to_string(), "F6");
        assert_eq!(workbook.sheet("Data").unwrap().len(), cells as usize);
        excel_package::assert_cost_note_moved(&workbook, notes);
        allocations
    };
    // Warm process-global lazy state outside either measured corpus.
    cost(1, 1);
    for notes in [1, 16] {
        let (small, large) = (cost(notes, 64), cost(notes, 4_096));
        eprintln!("excel_note_cut: {notes} notes, 64 cells {small}, 4096 cells {large}");
        assert_eq!(
            large, small,
            "unrelated cells changed note cut allocation cost"
        );
    }
}

#[test]
fn excel_cross_sheet_carried_cut_allocates_for_registrations_not_unrelated_cells() {
    use yggdryl::excel::{Paste, Workbook};
    let cost = |kind: &str, registrations: u32, cells: u32| {
        let bytes = excel_package::carried_cost_package(kind, registrations, cells);
        let mut workbook = Workbook::from_bytes(bytes).unwrap();
        workbook.parse_all().unwrap();
        let source = format!("B2:B{}", registrations + 1);
        let (allocations, moved) = counted(|| {
            workbook.paste(
                ("Data", source.parse().unwrap()),
                ("Other", "J10".parse().unwrap()),
                Paste::All,
                true,
            )
        });
        assert!(moved.is_ok(), "{kind}: {moved:?}");
        assert_eq!(workbook.sheet("Data").unwrap().cell_count(), cells as usize);
        allocations
    };
    // Warm process-global lazy state before comparing either corpus.
    for kind in ["cf", "dv", "x14cf", "x14dv", "x14spark", "hyperlink"] {
        cost(kind, 1, 1);
    }
    for kind in ["cf", "dv", "x14cf", "x14dv", "x14spark", "hyperlink"] {
        for registrations in [1, 16] {
            let small = cost(kind, registrations, 64);
            let large = cost(kind, registrations, 4_096);
            eprintln!(
                "excel_carried_cut: {kind}, {registrations} registrations, 64 cells {small}, 4096 cells {large}"
            );
            assert_eq!(
                large, small,
                "unrelated cells changed {kind} carried transfer allocations"
            );
        }
    }
}

/// Thread resolution costs its roots/replies, never unrelated resident cells.
#[test]
fn excel_threaded_cut_costs_threads_and_not_unrelated_cells() {
    use yggdryl::excel::{Paste, Workbook};
    let cost = |threads, cells| {
        let mut workbook =
            Workbook::from_bytes(excel_package::threaded_cost_package(threads, cells)).unwrap();
        workbook.parse_all().unwrap();
        let (allocations, result) = counted(|| {
            workbook.paste(
                ("Data", "B2".parse().unwrap()),
                ("Data", "F6".parse().unwrap()),
                Paste::All,
                true,
            )
        });
        assert_eq!(result.unwrap().to_string(), "F6");
        assert_eq!(workbook.sheet("Data").unwrap().len(), cells as usize);
        excel_package::assert_cost_thread_moved(&workbook, threads);
        allocations
    };
    cost(1, 1);
    for threads in [1, 16] {
        let (small, large) = (cost(threads, 64), cost(threads, 4_096));
        eprintln!(
            "excel_threaded_cut: {threads} roots and replies, 64 cells {small}, 4096 cells {large}"
        );
        assert_eq!(
            large, small,
            "unrelated cells changed threaded cut allocation cost"
        );
    }
}

/// Only context intake owns a long local-name key. Per-token lookup folds
/// borrowed bytes; rewritten formula tokens share the original name buffer.
#[test]
fn excel_scoped_name_cut_allocates_for_one_resolved_key_not_each_lookup() {
    use yggdryl::excel::{CellRange, CellRef, Paste, Workbook};

    fn cost(cells: u32, name: &str, occurrences: usize, unrelated: usize) -> usize {
        let bytes =
            excel_package::scoped_name_cost_package(cells, name, occurrences, unrelated, false);
        let mut workbook = Workbook::from_bytes(bytes).unwrap();
        workbook.parse_all().unwrap();
        let block = CellRange::new(CellRef::new(2, 2), CellRef::new(cells + 1, 2));
        let target = CellRef::new(7, 7);
        let (allocations, moved) =
            counted(|| workbook.paste(("Data", block), ("Other", target), Paste::All, true));
        assert_eq!(
            moved.unwrap(),
            CellRange::new(target, CellRef::new(cells + 6, 7))
        );
        assert_eq!(workbook.sheet("Data").unwrap().len(), 0);
        assert_eq!(workbook.sheet("Other").unwrap().len(), cells as usize);
        let expected = vec![format!("Data!{}", name.to_ascii_lowercase()); occurrences].join("+");
        for at in [target, CellRef::new(cells + 6, 7)] {
            let cell = workbook.sheet("Other").unwrap().cell(at).unwrap();
            assert_eq!(cell.formula().unwrap().at(at).to_string(), expected);
        }
        allocations
    }

    const SHORT: &str = "ShOrTRaTe";
    const LONG: &str = "LoNgMiXeDReferenceNameBeyondInlineCapacity";
    for cells in [64, 4_096] {
        for occurrences in [1, 32] {
            let short = cost(cells, SHORT, occurrences, 0);
            let long = cost(cells, LONG, occurrences, 0);
            eprintln!(
                "excel_scoped_name_cut: {cells} cells/{occurrences} names, short={short}, long={long}"
            );
            // One long context key, never a lowercase allocation per
            // occurrence; rewritten tokens share their original buffer.
            assert_eq!(
                long,
                short + 1,
                "long-name lookup allocated per formula token"
            );
        }
    }
    let baseline = cost(64, LONG, 4, 0);
    for unrelated in [64, 4_096] {
        let with_registry = cost(64, LONG, 4, unrelated);
        eprintln!(
            "excel_scoped_name_cut: {unrelated} globals and other-sheet locals, {with_registry}"
        );
        assert_eq!(
            with_registry, baseline,
            "unrelated scopes entered the cut context"
        );
    }
}

#[test]
fn iomedia_result_field_moves_identity_without_allocating_or_rebuilding_children() {
    use yggdryl::ipc::IpcOptions;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{IOMedia, StructType};

    let source = Buffer::from_bytes(b"unread header".to_vec());
    for columns in [64, 4_096] {
        let mut field = StructType::from_fields(
            (0..columns).map(|index| DataType::Int64.required_field(format!("c{index:04}"))),
        )
        .map(DataType::from)
        .unwrap()
        .required_field("records");
        field
            .set_comment("metadata remains on the identity result")
            .unwrap();
        let identity = IpcOptions::new();
        let (once, repeated) = counted_each(
            || field.clone(),
            |owned| identity.result_field(owned).unwrap(),
        );
        assert_eq!(
            (once, repeated),
            (0, 0),
            "identity resolver: {columns} columns"
        );

        let declared = RecordOptions::Ipc(identity.with_field(field.clone()));
        free("declared identity result field", || {
            let actual = source.read_arrow_field(&declared).unwrap();
            assert_eq!(actual, field);
        });
    }
}

#[test]
fn excel_named_table_write_allocations_do_not_follow_unrelated_rows() {
    use arrow_array::{Float64Array, RecordBatch, StringArray};
    use yggdryl::excel::{ExcelOptions, overwrite_arrow_reader};

    let root = DataType::from(
        StructType::from_fields([
            DataType::Float64.required_field("id"),
            DataType::utf8().required_field("name"),
        ])
        .unwrap(),
    )
    .required_field("row");
    let schema = root.into_arrow_schema().unwrap();
    let input = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Float64Array::from(vec![9.0, 10.0])),
            Arc::new(StringArray::from(vec!["nine", "ten"])),
        ],
    )
    .unwrap();
    let options = ExcelOptions::new().with_table("Names");
    let cost = |unrelated: u32| {
        let bytes = excel_package::named_table_cost_package(2, unrelated);
        let mut handle = Buffer::from_bytes(bytes).with_media_type(MimeType::XLSX.into());
        let (allocations, ()) = counted(|| {
            overwrite_arrow_reader(
                &mut handle,
                yggdryl::arrow::batch_reader(schema.clone(), [input.clone()]),
                &options,
            )
            .unwrap();
        });
        allocations
    };
    let _ = cost(64); // Warm global field projections outside the comparison.
    let (small, large) = (cost(64), cost(4096));
    eprintln!("named_table_overlay: unrelated64={small} unrelated4096={large}");
    // Raw neighboring cells bypass one attribute Vec and one owned r for
    // each of four numeric cells. Disabling this path measured 836; disabling
    // exact edited-document capacity too recovered the old 837. Full-suite
    // and focused runs differ by one fixed allocation (828/829), whose owner
    // is not established. Pin the guaranteed saving and zero unrelated-row
    // slope, without claiming the additional capacity saving on every run.
    assert_eq!(small, large, "unselected rows must add no XML allocations");
    assert!(
        small <= 829,
        "raw neighboring cells must retain the saving: {small}"
    );
}

#[test]
fn excel_named_table_resize_allocations_do_not_follow_unrelated_rows() {
    use arrow_array::{Float64Array, RecordBatch, StringArray};
    use yggdryl::excel::{ExcelOptions, overwrite_arrow_reader};

    let root = DataType::from(
        StructType::from_fields([
            DataType::Float64.required_field("id"),
            DataType::utf8().required_field("name"),
        ])
        .unwrap(),
    )
    .required_field("row");
    let schema = root.into_arrow_schema().unwrap();
    let options = ExcelOptions::new().with_table("Names");
    for (mode, ids, labels) in [
        ("grow", vec![9.0, 10.0, 11.0], vec!["nine", "ten", "eleven"]),
        ("shrink", vec![9.0], vec!["nine"]),
    ] {
        let input = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Float64Array::from(ids)),
                Arc::new(StringArray::from(labels)),
            ],
        )
        .unwrap();
        let cost = |unrelated: u32| {
            let bytes = excel_package::named_table_cost_package(2, unrelated);
            let mut handle = Buffer::from_bytes(bytes).with_media_type(MimeType::XLSX.into());
            counted(|| {
                overwrite_arrow_reader(
                    &mut handle,
                    yggdryl::arrow::batch_reader(schema.clone(), [input.clone()]),
                    &options,
                )
                .unwrap();
            })
            .0
        };
        let _ = cost(64);
        let (small, large) = (cost(64), cost(4096));
        eprintln!("named_table_{mode}: unrelated64={small} unrelated4096={large}");
        assert_eq!(
            small, large,
            "{mode} must retain the raw-subtree fast path for unrelated rows"
        );
    }
}

#[test]
fn excel_named_totals_resize_allocations_do_not_follow_unrelated_rows_or_cells() {
    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::excel::{ExcelOptions, overwrite_arrow_reader};

    let root = DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("year"),
            DataType::Int64.required_field("qty"),
        ])
        .unwrap(),
    )
    .required_field("row");
    let schema = root.into_arrow_schema().unwrap();
    let input = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![2030, 2031, 2032])),
            Arc::new(Int64Array::from(vec![6, 8, 10])),
        ],
    )
    .unwrap();
    let options = ExcelOptions::new().with_table("Quantities");
    let cost = |rows, cells, local_namespace, old_row_cells| {
        let bytes =
            excel_package::named_totals_cost_package(rows, cells, local_namespace, old_row_cells);
        let mut handle = Buffer::from_bytes(bytes).with_media_type(MimeType::XLSX.into());
        counted(|| {
            overwrite_arrow_reader(
                &mut handle,
                yggdryl::arrow::batch_reader(schema.clone(), [input.clone()]),
                &options,
            )
            .unwrap();
        })
        .0
    };
    let _ = cost(64, 1, false, 0);
    let (small, large) = (cost(64, 1, false, 0), cost(4096, 1, false, 0));
    eprintln!("named_totals_overlay: unrelated64={small} unrelated4096={large}");
    let (scope_one, scope_sixteen) = (cost(64, 1, true, 0), cost(64, 16, true, 0));
    eprintln!(
        "named_totals_local_namespace: unrelated_cells1={scope_one} unrelated_cells16={scope_sixteen}"
    );
    let (one, sixteen) = (cost(64, 1, false, 1), cost(64, 1, false, 16));
    eprintln!("named_totals_old_row: unrelated_cells1={one} unrelated_cells16={sixteen}");
    // The larger ZIP carries deflate restart extras (two small allocations),
    // while a size-dependent buffer avoids one geometric reallocation.
    // Neither event is in the selected table body. The public boundary may
    // therefore cost one extra archive allocation, but not one per row.
    assert!(
        large <= small + 1,
        "table-body resize must not allocate per unrelated row: {small} -> {large}"
    );
    // More unrelated namespace cells begin with larger ZIP member/output
    // buffers, avoiding one geometric reallocation; they add no row work.
    assert!(
        scope_sixteen <= scope_one,
        "table-body resize must not allocate per unrelated cell: {scope_one} -> {scope_sixteen}"
    );
    assert_eq!(
        one, sixteen,
        "unrelated cells in the selected totals row must stay borrowed and raw"
    );
}

#[test]
fn excel_named_table_write_selected_row_plan_allocations_are_pinned() {
    use arrow_array::{Float64Array, RecordBatch, StringArray};
    use yggdryl::excel::{ExcelOptions, overwrite_arrow_reader};

    let root = DataType::from(
        StructType::from_fields([
            DataType::Float64.required_field("id"),
            DataType::utf8().required_field("name"),
        ])
        .unwrap(),
    )
    .required_field("row");
    let schema = root.into_arrow_schema().unwrap();
    let options = ExcelOptions::new().with_table("Names");
    let cost = |selected: u32| {
        let bytes = excel_package::named_table_cost_package(selected, 64);
        let mut handle = Buffer::from_bytes(bytes).with_media_type(MimeType::XLSX.into());
        let input = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Float64Array::from_iter_values(
                    (1..=selected).map(f64::from),
                )),
                Arc::new(StringArray::from_iter_values(
                    (1..=selected).map(|_| "written"),
                )),
            ],
        )
        .unwrap();
        let (allocations, ()) = counted(|| {
            overwrite_arrow_reader(
                &mut handle,
                yggdryl::arrow::batch_reader(schema.clone(), [input.clone()]),
                &options,
            )
            .unwrap();
        });
        allocations
    };
    let _ = cost(2);
    let (short, long) = (cost(2), cost(256));
    eprintln!("named_table_overlay_selected: rows2={short} rows256={long}");
    // Selected cells retain an eager splice plan, not constant-space output.
    // The neighboring cells own 8/13 attribute allocations: four numeric
    // cells, then one more numeric cell and one inline string at 256 rows.
    // Disabling raw-copy measured 836/6688; disabling exact output capacity
    // too recovered 837/6689. Full and focused runs differ by one unlocated
    // allocation (828/6675 versus 829/6676), with identical growth.
    // These ceilings preserve at least 8/13 savings and the improved slope;
    // the separate benchmark catches repeated scanning of earlier splices.
    assert!(
        short <= 829 && long <= 6676,
        "selected row plan exceeds its allocation bound: {short}/{long}"
    );
    assert!(
        long <= short + 5847,
        "selected-row allocation growth changed: {short} -> {long}"
    );
}

#[test]
fn excel_named_table_metadata_observation_skips_unrelated_cells() {
    use yggdryl::IOMedia;
    use yggdryl::excel::ExcelOptions;
    use yggdryl::media::{IORecordOptions, RecordOptions};

    let cost = |table_rows: u32, unrelated_rows: u32| {
        let bytes = excel_package::named_table_cost_package(table_rows, unrelated_rows);
        let handle = Buffer::from_bytes(bytes).with_media_type(MimeType::XLSX.into());
        let field = DataType::from(
            StructType::from_fields([
                DataType::Float64.required_field("id"),
                DataType::utf8().required_field("name"),
            ])
            .unwrap(),
        )
        .required_field("row");
        let options =
            RecordOptions::from(ExcelOptions::new().with_table("Names")).with_field(field);
        // Prime process-wide datatype projections outside the measured read.
        drop(
            handle
                .read_arrow_reader(&options)
                .expect("a warm named reader"),
        );
        let (allocations, ()) = counted(|| {
            drop(handle.read_arrow_reader(&options).expect("a named reader"));
        });
        let rows: usize = handle
            .read_arrow_reader(&options)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum();
        assert_eq!(rows, table_rows as usize);
        allocations
    };
    for table_rows in [2, 32] {
        let small = cost(table_rows, 16);
        let large = cost(table_rows, 256);
        eprintln!(
            "named_table_metadata_allocations: table_rows={table_rows} unrelated16={small} unrelated256={large}"
        );
        // Package metadata and the selected reader allocate once; the shared
        // parser skips sheetData during membership observation, constructing
        // no discarded rows or cell strings at either corpus size.
        // Namespace validation adds 16 fixed allocations: two consumed
        // worksheet resolvers (6 each) and the unpolled output resolver (4).
        // Private package intake now moves its first chunk: 2 instead of 23
        // allocations for these compressed packages, removing 21 fixed costs.
        assert_eq!((small, large), (428, 428));
    }
}

#[test]
fn excel_named_table_inferred_field_and_full_read_skip_unrelated_rows() {
    use yggdryl::IOMedia;
    use yggdryl::excel::ExcelOptions;
    use yggdryl::media::RecordOptions;

    let cost = |table_rows: u32, unrelated_rows: u32| {
        let bytes = excel_package::named_table_cost_package(table_rows, unrelated_rows);
        let handle = Buffer::from_bytes(bytes).with_media_type(MimeType::XLSX.into());
        let options = RecordOptions::from(ExcelOptions::new().with_table("Names"));
        // Exclude process-wide projection initialization from the comparison.
        drop(
            handle
                .read_arrow_field(&options)
                .expect("warm inferred field"),
        );
        let (field_allocations, field) = counted(|| {
            handle
                .read_arrow_field(&options)
                .expect("inferred named field")
        });
        assert_eq!(
            field
                .fields()
                .iter()
                .map(|child| child.name())
                .collect::<Vec<_>>(),
            ["id", "name"]
        );
        assert_eq!(field.fields()[0].dtype(), &DataType::Float64);
        assert_eq!(field.fields()[1].dtype(), &DataType::utf8());

        let warm_rows: usize = handle
            .read_arrow_reader(&options)
            .expect("warm inferred reader")
            .map(|batch| batch.expect("warm batch").num_rows())
            .sum();
        assert_eq!(warm_rows, table_rows as usize);
        let (read_allocations, rows) = counted(|| {
            handle
                .read_arrow_reader(&options)
                .expect("inferred named reader")
                .map(|batch| batch.expect("named batch").num_rows())
                .sum::<usize>()
        });
        assert_eq!(rows, table_rows as usize);
        (field_allocations, read_allocations)
    };

    for table_rows in [2, 32] {
        let small = cost(table_rows, 16);
        let large = cost(table_rows, 256);
        eprintln!(
            "named_table_inferred_allocations: table_rows={table_rows} unrelated16={small:?} unrelated256={large:?}"
        );
        // A selected table has fixed output here. Both schema inference and
        // full read should stop once the selected body has been tallied.
        // Growth with selected rows is the existing row/value construction;
        // the worksheet tail contributes nothing after the table's body end.
        // Namespace validation adds 22 for field-only / 24 for full read:
        // two observers + inference consume 3 resolvers (6 each), while
        // the output resolver costs 4 unpolled or 6 after reading its root.
        // Private package intake now moves its first chunk: 2 instead of 23
        // allocations for these compressed packages, removing 21 fixed costs.
        // HeaderProbe replaces dtype/present/nullable Vecs and one seen Vec
        // per body row with one shared BTreeMap node for these two columns:
        // 3 + rows - 1 allocations removed (4 at 2 rows, 34 at 32 rows).
        let expected = if table_rows == 2 {
            (425, 457)
        } else {
            (457, 552)
        };
        assert_eq!((small, large), (expected, expected));
    }
}

// A no-body table must derive its two nullable Null columns from the table
// metadata even if the worksheet has thousands of unrelated cells.
#[test]
fn excel_named_table_inferred_no_body_skips_unrelated_rows() {
    use yggdryl::IOMedia;
    use yggdryl::excel::ExcelOptions;
    use yggdryl::media::RecordOptions;

    let cost = |unrelated_rows: u32| {
        let mut parts = excel_package::named_table_parts();
        // Keep Quantities' header and totals row, but no body row.
        let table = &mut parts
            .iter_mut()
            .find(|(part, _)| *part == "xl/tables/table2.xml")
            .unwrap()
            .1;
        *table = table.replace("ref=\"D1:E4\"", "ref=\"D1:E2\"");
        let worksheet = &mut parts
            .iter_mut()
            .find(|(part, _)| *part == "xl/worksheets/sheet1.xml")
            .unwrap()
            .1;
        let mut unrelated = String::new();
        for extra in 0..unrelated_rows {
            let row = 5 + extra;
            unrelated.push_str(&format!("<row r=\"{row}\"><c r=\"G{row}\" t=\"inlineStr\"><is><t>unrelated-cell-value-{row:08}</t></is></c></row>"));
        }
        *worksheet = worksheet.replace("</sheetData>", &format!("{unrelated}</sheetData>"));
        let handle = Buffer::from_bytes(excel_package::named_table_package(&parts))
            .with_media_type(MimeType::XLSX.into());
        let options = RecordOptions::from(ExcelOptions::new().with_table("Quantities"));
        drop(
            handle
                .read_arrow_field(&options)
                .expect("warm no-body field"),
        );
        let (allocations, field) = counted(|| {
            handle
                .read_arrow_field(&options)
                .expect("inferred no-body field")
        });
        assert_eq!(
            field
                .fields()
                .iter()
                .map(|child| child.name())
                .collect::<Vec<_>>(),
            ["year", "qty"]
        );
        assert!(
            field
                .fields()
                .iter()
                .all(|child| child.dtype() == &DataType::Null && child.is_nullable())
        );
        assert_eq!(
            handle
                .read_arrow_reader(&options)
                .unwrap()
                .map(|batch| batch.unwrap().num_rows())
                .sum::<usize>(),
            0
        );
        allocations
    };
    let small = cost(16);
    let large = cost(256);
    eprintln!("named_table_no_body_inferred_allocations: unrelated16={small} unrelated256={large}");
    // Only package/table metadata is resolved; no body row is polled.
    // Namespace validation adds 20: two consumed observers (6 each), plus
    // unpolled inference and output resolvers (4 each), independent of rows.
    // Private package intake now moves its first chunk: 2 instead of 23
    // allocations for these compressed packages, removing 21 fixed costs.
    // HeaderProbe needs no column node when the body is empty. Removing
    // the old dtype/present/nullable Vecs therefore removes exactly three.
    assert_eq!((small, large), (411, 411));
}

#[test]
fn excel_regions_allocations_follow_result_count_not_worksheet_height() {
    use yggdryl::excel::{CellRange, CellRef, ExcelRegionKind};

    let cost = |rows: u32, count: u32, restarts: bool| {
        let handle = Buffer::from_bytes(excel_package::regions_cost_package(rows, count, restarts))
            .with_media_type(MimeType::XLSX.into());
        drop(yggdryl::excel::regions(&handle, None).unwrap());
        let (allocations, regions) = counted(|| yggdryl::excel::regions(&handle, None).unwrap());
        assert_eq!(regions.len(), count as usize);
        let height = rows / count;
        for (index, region) in regions.iter().enumerate() {
            let first = index as u32 * (height + 1);
            assert_eq!(region.sheet.as_str(), "Sheet1");
            assert_eq!(region.kind, ExcelRegionKind::Suggested);
            assert_eq!(
                region.range,
                CellRange::new(CellRef::new(first, 0), CellRef::new(first + height - 1, 0))
            );
        }
        allocations
    };
    let short_one = cost(64, 1, false);
    let tall_one = cost(4_096, 1, false);
    let short_many = cost(64, 32, false);
    let tall_many = cost(4_096, 32, false);
    eprintln!(
        "excel_regions_allocations: regions1 short={short_one} tall={tall_one}; regions32 short={short_many} tall={tall_many}"
    );
    // Numeric cells remain inline, parser rows are recycled, and every live
    // component is compacted at the row boundary. The allocation count is
    // height-independent at each fixed output size. This does not measure
    // peak retained bytes; the bounded frontier owns that separate invariant.
    assert_eq!(
        tall_one, short_one,
        "one connected component over 64/4096 rows"
    );
    assert_eq!(
        tall_many, short_many,
        "32 returned regions over 64/4096 rows"
    );
    assert!(
        short_many >= short_one,
        "more returned rectangles do not erase fixed source work"
    );
    eprintln!(
        "excel_regions_result_overhead: 31 additional regions allocate {} times",
        short_many - short_one
    );
    // Namespace validation adds 12 fixed allocations: the membership and
    // occupancy parsers each resolve two root declarations for 6 allocations.
    // Private package intake now moves its first chunk: 2 instead of 23
    // allocations for these compressed packages, removing 21 fixed costs.
    // Found now retains 24-byte contact vectors: its 96-byte layout lets
    // collection reuse capacity as 64-byte ExcelRegion values without the
    // old single-result shrink allocation (72-byte Found). Unstable sort
    // avoids stable-sort heap scratch for 32 results; output order is total.
    assert_eq!((short_one, short_many), (147, 150));
    // Crossing the ZIP writer's 64 KiB restart stride adds one Vec<u64>
    // and one Arc<[u64]> in package intake. That is transport metadata,
    // not row work: the default writer control deliberately pins its +2.
    assert_eq!(cost(64, 1, true), short_one);
    assert_eq!(cost(4_096, 1, true), tall_one + 2);
    assert_eq!(cost(64, 32, true), short_many);
    assert_eq!(cost(4_096, 32, true), tall_many + 2);
}

#[test]
fn excel_selection_intake_allocates_only_a_retained_long_name() {
    use yggdryl::excel::ExcelSelection;

    let worksheet = yggdryl::from_json_scalar(r#"{"sheet":"Data","range":"B2:C4"}"#).unwrap();
    let (allocations, selected) = counted(|| ExcelSelection::from_scalar(&worksheet).unwrap());
    assert_eq!(allocations, 0);
    assert_eq!(
        selected,
        ExcelSelection::Worksheet {
            sheet: Some("Data".into()),
            range: Some("B2:C4".parse().unwrap()),
        }
    );
    for length in [64, 4_096] {
        let name = "x".repeat(length);
        let input = yggdryl::from_json_scalar(format!(r#"{{"table":"{name}"}}"#)).unwrap();
        let (allocations, selected) = counted(|| ExcelSelection::from_scalar(&input).unwrap());
        // The external name lands once in owned selection storage. No
        // parse tree or intermediate property map is built at this boundary.
        assert_eq!(allocations, 1);
        assert_eq!(selected, ExcelSelection::Table { name: name.into() });
    }
}

/// The first stream chunk becomes the private result buffer. Below one batch,
/// the source object and that payload are the only allocations; there is no
/// staging filesystem or second full-payload publication. Larger values add
/// bounded transport chunks and geometric growth of this same result Vec.
#[cfg(feature = "internals")]
#[test]
fn owned_handle_fallback_keeps_one_payload_allocation_below_a_batch() {
    use yggdryl::IOBase;
    use yggdryl::holder::Holder;
    use yggdryl::internals::iobase_hierarchy::owned_handle;

    let media = MediaType::from_str("application/octet-stream").unwrap();
    for (length, expected) in [(64, 2), (4096, 2), (2 * 64 * 1024 + 17, 6)] {
        let source = Buffer::from_bytes(vec![37; length]).with_media_type(media.clone());
        drop(owned_handle(&source).unwrap());
        let (allocations, owned) = counted(|| owned_handle(black_box(&source)).unwrap());
        assert_eq!(allocations, expected, "{length} bytes");
        assert_eq!(owned.media_type(), &media);
        let Holder::Buffer(buffer) = owned else {
            panic!("an owned buffer")
        };
        assert_eq!(buffer.as_slice(), source.as_slice());
    }
}

fn rows_two_allocation_package(extra: usize) -> Vec<u8> {
    let mut rows = String::from(concat!(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Sales</t></is></c></row>",
        "<row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>Units</t></is></c>",
        "<c r=\"B2\" t=\"inlineStr\"><is><t>Price</t></is></c></row>",
        "<row r=\"3\"><c r=\"A3\"><v>2</v></c><c r=\"B3\"><v>3</v></c></row>"
    ));
    for row in 4..4 + extra {
        rows.push_str(&format!(
            "<row r=\"{row}\"><c r=\"Z{row}\"><v>8</v></c></row>"
        ));
    }
    rows.push_str("<row r=\"1000\"><c r=\"Z1000\"><v>9</v></c></row>");
    let worksheet = excel_package::worksheet(&rows).replace(
        "</worksheet>",
        "<mergeCells count=\"1\"><mergeCell ref=\"A1:B1\"/></mergeCells></worksheet>",
    );
    let types = excel_package::content_types(1, false, false);
    let root = excel_package::root_relationships();
    let rels = excel_package::workbook_relationships(1, false, false);
    let workbook = excel_package::workbook(&["Sheet1"], false);
    excel_package::package(&[
        ("[Content_Types].xml", &types),
        ("_rels/.rels", &root),
        ("xl/workbook.xml", &workbook),
        ("xl/_rels/workbook.xml.rels", &rels),
        ("xl/worksheets/sheet1.xml", &worksheet),
    ])
}

#[test]
fn excel_rows_two_metadata_observation_has_no_unrelated_cell_allocation_slope() {
    use yggdryl::IOMedia;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{RecordHeader, excel::ExcelOptions};

    let nested = DataType::from(
        StructType::from_fields([
            DataType::Float64.required_field("Units"),
            DataType::Float64.required_field("Price"),
        ])
        .unwrap(),
    )
    .required_field("Sales");
    let field = DataType::from(StructType::from_fields([nested]).unwrap()).required_field("row");
    let packages = [
        rows_two_allocation_package(16),
        rows_two_allocation_package(256),
    ];
    for explicit in [true, false] {
        let allocations: Vec<_> = packages
            .iter()
            .map(|bytes| {
                let handle =
                    Buffer::from_bytes(bytes.clone()).with_media_type(MimeType::XLSX.into());
                let options = if explicit {
                    ExcelOptions::new().with_range("A1:B3".parse().unwrap())
                } else {
                    ExcelOptions::new()
                };
                let options = RecordOptions::from(options.with_header(RecordHeader::Rows(2)))
                    .with_field(field.clone());
                drop(
                    handle
                        .read_arrow_reader(&options)
                        .expect("warm Rows(2) plan"),
                );
                let (count, reader) =
                    counted(|| handle.read_arrow_reader(&options).expect("Rows(2) plan"));
                drop(reader);
                count
            })
            .collect();
        eprintln!(
            "Rows header metadata, explicit={explicit}, 16/256 unrelated cells: {allocations:?}"
        );
        assert_eq!(
            allocations[0], allocations[1],
            "metadata observer allocated per unrelated cell: explicit={explicit} {allocations:?}"
        );
    }
}

/// A known-full table must refuse before copying its unrelated format registry.
/// All XFs and both edited cells are identical across the two metadata corpora.
#[test]
fn excel_insert_style_capacity_refusal_does_not_clone_unrelated_format_codes() {
    use excel_package::{
        NS, content_types, package, root_relationships, table_member_map, workbook,
        workbook_relationships, worksheet,
    };
    use yggdryl::excel::{CellRef, MAX_CELL_FORMATS, Workbook};

    fn prepared(codes: usize) -> Workbook {
        let mut formats = String::new();
        for index in 0..codes {
            formats.push_str(&format!(
                "<numFmt numFmtId=\"{}\" formatCode=\"0.00&quot;{}&quot;\"/>",
                164 + index,
                index
            ));
        }
        let mut xfs = String::new();
        for index in 0..MAX_CELL_FORMATS {
            let styled = usize::from(index == 1);
            xfs.push_str(&format!("<xf numFmtId=\"0\" fontId=\"{styled}\" fillId=\"0\" borderId=\"{styled}\" xfId=\"0\"/>"));
        }
        let styles = format!(
            "<styleSheet xmlns=\"{NS}\"><numFmts count=\"{codes}\">{formats}</numFmts><fonts count=\"2\"><font/><font><b/></font></fonts><fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills><borders count=\"2\"><border/><border><left style=\"thin\"/></border></borders><cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs><cellXfs count=\"{MAX_CELL_FORMATS}\">{xfs}</cellXfs></styleSheet>"
        );
        let bytes = package(&[
            ("[Content_Types].xml", &content_types(1, false, true)),
            ("_rels/.rels", &root_relationships()),
            ("xl/workbook.xml", &workbook(&["Data"], false)),
            (
                "xl/_rels/workbook.xml.rels",
                &workbook_relationships(1, false, true),
            ),
            ("xl/styles.xml", &styles),
            (
                "xl/worksheets/sheet1.xml",
                &worksheet("<row r=\"2\"><c r=\"B2\" s=\"1\"><v>1</v></c></row>"),
            ),
        ]);
        let book = Workbook::from_bytes(bytes).unwrap();
        book.parse_all().unwrap();
        assert_eq!(book.style_sheet().unwrap().len(), MAX_CELL_FORMATS);
        book
    }
    let mut counts = Vec::new();
    for codes in [64, 4096] {
        let mut book = prepared(codes);
        let before = table_member_map(&book);
        let revision = book.sheet("Data").unwrap().revision();
        let dirty = book.is_dirty();
        // Warm only immutable parsed metadata and the reference index.
        assert!(
            matches!(book.insert_rows("Data", 2, 1), Err(yggdryl::Error::InvalidRecord { ref path, .. }) if path.as_str() == "$.styles")
        );
        let (allocations, error) = counted(|| book.insert_rows("Data", 2, 1).unwrap_err());
        assert!(
            matches!(error, yggdryl::Error::InvalidRecord { ref path, .. } if path.as_str() == "$.styles"),
            "{error}"
        );
        assert_eq!(book.sheet("Data").unwrap().revision(), revision);
        assert_eq!(book.is_dirty(), dirty);
        assert_eq!(book.style_sheet().unwrap().len(), MAX_CELL_FORMATS);
        assert_eq!(
            book.sheet("Data").unwrap().scalar(CellRef::new(1, 1)),
            Scalar::from(1.0)
        );
        assert_eq!(table_member_map(&book), before);
        counts.push(allocations);
    }
    eprintln!("full-style insertion refusal, 64/4096 format codes: {counts:?}");
    assert_eq!(
        counts[0], counts[1],
        "known-full style refusal cloned unrelated format metadata"
    );
}

#[test]
fn excel_rows_writer_allocates_for_batches_not_each_record() {
    use yggdryl::IOMedia;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{RecordHeader, excel::ExcelOptions};

    let sales = DataType::from(
        StructType::from_fields([
            DataType::Float64.required_field("Units"),
            DataType::Float64.required_field("Price"),
        ])
        .unwrap(),
    )
    .required_field("Sales");
    let field = DataType::from(StructType::from_fields([sales]).unwrap()).required_field("row");
    let cost = |count: usize| {
        let rows = (0..count).map(|index| {
            Scalar::from_sequence([Scalar::from_sequence([
                Scalar::from(index as f64),
                Scalar::from(1.0),
            ])])
        });
        let batch = Serie::from_scalars(field.clone(), rows)
            .unwrap()
            .into_arrow_batch()
            .unwrap();
        let mut handle = Buffer::new().with_media_type(MimeType::XLSX.into());
        let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Rows(2)))
            .with_field(field.clone());
        let (allocations, ()) = counted(|| {
            handle.overwrite_arrow_batch(batch, &options).unwrap();
        });
        allocations
    };
    let _ = cost(64); // Warm shared code before counting either corpus.
    let (small, large) = (cost(64), cost(640));
    eprintln!("Rows stream writes, 64/640records: {small}/{large}");
    // Complete XLSX writes include compression and staging. Pin their
    // measured upper budget and prohibit growth for ten times as many
    // records; an exact total would also reject harmless lower counts.
    assert!(
        small <= 457 && large <= small,
        "Rows package writes exceeded their fixed budget: {small}/{large}"
    );
}

#[test]
fn excel_rows_model_write_ignores_unrelated_held_cells() {
    use yggdryl::{RecordHeader, excel::Sheet};

    let group = DataType::from(
        StructType::from_fields([
            DataType::Float64.required_field("Units"),
            DataType::utf8().required_field("Person"),
        ])
        .unwrap(),
    )
    .required_field("Sales");
    let field = DataType::from(StructType::from_fields([group]).unwrap()).required_field("row");
    let serie = Serie::from_scalars(
        field,
        [Scalar::from_sequence([Scalar::from_sequence([
            Scalar::from(1.0),
            Scalar::from("Ann"),
        ])])],
    )
    .unwrap();
    let cost = |unrelated: u32| {
        let mut sheet = Sheet::new("Sheet1").unwrap();
        for row in 10..10 + unrelated {
            sheet.set_cell((row, 25).into(), row as f64).unwrap();
        }
        let (allocations, ()) = counted(|| {
            sheet
                .write_serie((0, 0).into(), &serie, RecordHeader::Rows(2))
                .unwrap();
        });
        allocations
    };
    let (small, large) = (cost(16), cost(256));
    eprintln!("Rows model write, 16/256 unrelatedcells: {small}/{large}");
    assert!(
        large <= small + 8,
        "Rows writing one selected record with 16 vs 256 unrelated cells allocated {small} -> {large} times"
    );
}

/// Worksheet-inline registrations need no per-resident-cell ownership storage.
#[test]
fn excel_carried_scope_cut_allocates_for_registrations_not_resident_cells() {
    use yggdryl::excel::{CellRange, Paste, Workbook};
    let cost = |kind, registrations: u32, cells: u32| {
        let bytes = excel_package::carried_cost_package(kind, registrations, cells);
        let mut workbook = Workbook::from_bytes(bytes).unwrap();
        workbook.parse_all().unwrap();
        let source: CellRange = format!("B2:B{}", registrations + 1).parse().unwrap();
        let target = "J10".parse().unwrap();
        let (allocations, result) =
            counted(|| workbook.paste(("Data", source), ("Other", target), Paste::All, true));
        result.unwrap();
        assert_eq!(workbook.sheet("Data").unwrap().cell_count(), cells as usize);
        excel_package::assert_cost_carried_scope_moved(&workbook, kind, registrations);
        allocations
    };
    for kind in ["protected", "ignored", "watch"] {
        cost(kind, 1, 1);
        for registrations in [1, 16] {
            let small = cost(kind, registrations, 64);
            let large = cost(kind, registrations, 4_096);
            eprintln!(
                "excel_carried_scope_cut: {kind}, {registrations} registrations, 64 cells {small}, 4096 cells {large}"
            );
            assert_eq!(
                large, small,
                "unrelated resident cells changed {kind} transfer cost"
            );
        }
    }
}

#[test]
fn excel_infer_allocation_slope_is_measured_against_explicit_none() {
    use yggdryl::IOMedia;
    use yggdryl::media::RecordOptions;
    use yggdryl::{RecordHeader, excel::ExcelOptions};

    let cost = |rows: u32, explicit: bool, infer: bool| {
        let bytes = excel_package::regions_cost_package(rows, 1, false);
        let handle = Buffer::from_bytes(bytes).with_media_type(MimeType::XLSX.into());
        let mut options = ExcelOptions::new().with_header(if infer {
            RecordHeader::Infer
        } else {
            RecordHeader::None
        });
        if explicit {
            options = options.with_range(format!("A1:A{rows}").parse().unwrap());
        }
        let options = RecordOptions::from(options);
        // Warm projection caches outside the counting interval.
        drop(handle.read_arrow_field(&options).expect("warm schema"));
        let (schema_allocations, schema) = counted(|| {
            handle
                .read_arrow_field(&options)
                .expect("inferred numeric field")
        });
        assert_eq!(schema.fields().len(), 1);
        assert_eq!(schema.fields()[0].dtype(), &DataType::Float64);
        let warm: usize = handle
            .read_arrow_reader(&options)
            .expect("warm reader")
            .map(|batch| batch.unwrap().num_rows())
            .sum();
        assert_eq!(warm, rows as usize);
        let (read_allocations, returned) = counted(|| {
            handle
                .read_arrow_reader(&options)
                .expect("numeric reader")
                .map(|batch| batch.unwrap().num_rows())
                .sum::<usize>()
        });
        assert_eq!(returned, rows as usize);
        (schema_allocations, read_allocations)
    };

    for explicit in [false, true] {
        let small_none = cost(64, explicit, false);
        let large_none = cost(4_096, explicit, false);
        let small_infer = cost(64, explicit, true);
        let large_infer = cost(4_096, explicit, true);
        eprintln!(
            "excel_infer_allocations explicit={explicit} none64={small_none:?} none4096={large_none:?} infer64={small_infer:?} infer4096={large_infer:?}"
        );
        // Infer adds fixed metadata/probe setup over None. Implicit selection
        // also discovers occupied regions; an explicit range skips that scan.
        // Decoding uses the common parser, with no Infer-only per-row cost.
        let fixed = if explicit { 31 } else { 95 };
        assert_eq!(small_infer.0, small_none.0 + fixed);
        assert_eq!(large_infer.0, large_none.0 + fixed);
        assert_eq!(small_infer.1, small_none.1 + fixed);
        assert_eq!(large_infer.1, large_none.1 + fixed);
    }
}

#[test]
fn excel_infer_tall_merged_header_allocations_do_not_follow_merge_height() {
    use yggdryl::IOMedia;
    use yggdryl::media::RecordOptions;
    use yggdryl::{
        RecordHeader,
        excel::{ExcelOptions, Workbook},
    };

    fn package(height: u32) -> Vec<u8> {
        let rows = format!(
            concat!(
                "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Sales</t></is></c>",
                "<c r=\"C1\" t=\"inlineStr\"><is><t>Owner</t></is></c></row>",
                "<row r=\"{leaf}\"><c r=\"A{leaf}\" t=\"inlineStr\"><is><t>Units</t></is></c>",
                "<c r=\"B{leaf}\" t=\"inlineStr\"><is><t>Price</t></is></c></row>",
                "<row r=\"{height}\"><c r=\"A{height}\"><v>1</v></c>",
                "<c r=\"B{height}\"><v>2</v></c>",
                "<c r=\"C{height}\"><v>3</v></c></row>"
            ),
            height = height,
            leaf = height - 1
        );
        let mut sheet = excel_package::worksheet(&rows);
        let merges = format!(
            "<mergeCells count=\"2\"><mergeCell ref=\"A1:B{}\"/><mergeCell ref=\"C1:C{}\"/></mergeCells></worksheet>",
            height - 2,
            height - 1
        );
        sheet = sheet.replace("</worksheet>", &merges);
        let types = excel_package::content_types(1, false, false);
        let root = excel_package::root_relationships();
        let book = excel_package::workbook(&["Sheet1"], false);
        let relations = excel_package::workbook_relationships(1, false, false);
        excel_package::package(&[
            ("[Content_Types].xml", &types),
            ("_rels/.rels", &root),
            ("xl/workbook.xml", &book),
            ("xl/_rels/workbook.xml.rels", &relations),
            ("xl/worksheets/sheet1.xml", &sheet),
        ])
    }

    let cost = |height| {
        let bytes = package(height);
        let handle = Buffer::from_bytes(bytes.clone()).with_media_type(MimeType::XLSX.into());
        let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Infer));
        drop(handle.read_arrow_field(&options).expect("warm tall schema"));
        let (field_allocations, field) = counted(|| {
            handle
                .read_arrow_field(&options)
                .expect("tall merged schema")
        });
        assert_eq!(field.fields().len(), 2);
        assert_eq!(field.fields()[0].fields().len(), 2);
        let read_rows = || {
            handle
                .read_arrow_reader(&options)
                .expect("tall reader")
                .map(|batch| batch.expect("batch").num_rows())
                .sum::<usize>()
        };
        assert_eq!(read_rows(), 1);
        let (read_allocations, returned) = counted(read_rows);
        assert_eq!(returned, 1);
        let held = Workbook::from_bytes(bytes)
            .expect("tall workbook")
            .sheet("Sheet1")
            .expect("tall sheet")
            .clone();
        drop(
            held.clone()
                .into_serie(None, RecordHeader::Infer, Default::default())
                .expect("warm held header"),
        );
        let (held_allocations, result) = counted(|| {
            held.clone()
                .into_serie(None, RecordHeader::Infer, Default::default())
                .expect("held header")
        });
        assert_eq!(result.len(), 1);
        (field_allocations, read_allocations, held_allocations)
    };
    let (short, tall) = (cost(64), cost(4_096));
    eprintln!("Excel Infer tall merged header, height64/4096: {short:?}/{tall:?}");
    // Two merge intervals and three occupied rows are identical. The parser
    // observes merge endpoints and Frontier contacts, never every covered row.
    assert_eq!(short, tall, "tall merged header allocated by covered row");
}

#[test]
fn excel_worksheet_filter_cut_costs_criteria_not_unrelated_cells() {
    use yggdryl::excel::{CellRef, Paste, Workbook};
    let cost = |columns, cells, mode| {
        let bytes = excel_package::worksheet_filter_cost_package(columns, cells);
        let mut book = Workbook::from_bytes(bytes).unwrap();
        book.parse_all().unwrap();
        let (block, destination) = excel_package::worksheet_filter_cut_case(mode, columns);
        let (allocations, result) = counted(|| {
            book.paste(
                ("Data", block),
                (destination, CellRef::new(9, 9)),
                Paste::All,
                true,
            )
        });
        result.unwrap();
        assert_eq!(book.sheet("Data").unwrap().cell_count(), cells as usize);
        excel_package::assert_cost_worksheet_filter_cut(&book, columns, mode);
        allocations
    };
    for mode in ["same", "full", "body", "partial-interior", "partial-header"] {
        cost(if mode == "partial-header" { 2 } else { 1 }, 1, mode);
        let widths: &[u32] = if mode == "partial-header" {
            &[2, 16]
        } else {
            &[1, 16]
        };
        for &columns in widths {
            let small = cost(columns, 64, mode);
            let large = cost(columns, 4_096, mode);
            eprintln!(
                "excel_worksheet_filter_cut: {mode}, {columns} criteria, 64 cells {small}, 4096 cells {large}"
            );
            assert_eq!(small, large, "resident cells changed filter {mode} cost");
        }
    }
}

#[test]
fn excel_partial_carried_cut_allocations_do_not_follow_range_area() {
    use yggdryl::excel::{Paste, Workbook};
    let cost = |kind, last_row, registrations| {
        let bytes = excel_package::partial_carried_cost_package(kind, last_row, registrations);
        let mut workbook = Workbook::from_bytes(bytes).unwrap();
        workbook.parse_all().unwrap();
        let (source, target) = excel_package::partial_carried_cost_edit(kind, registrations);
        let (allocations, result) =
            counted(|| workbook.paste(("Data", source), ("Other", target), Paste::All, true));
        result.unwrap();
        excel_package::assert_partial_carried_cost_split(&workbook, kind, registrations, last_row);
        allocations
    };
    for kind in ["protected", "ignored", "hyperlink"] {
        // Match the existing carried-cut pins: initialize process-wide
        // parser/metadata state on a separate workbook before measuring size.
        let warm = cost(kind, 64, 1);
        for registrations in [1, 16] {
            let small = cost(kind, 64, registrations);
            let large = cost(kind, 4096, registrations);
            let reverse_large = cost(kind, 4096, registrations);
            let reverse_small = cost(kind, 64, registrations);
            eprintln!(
                "excel_partial_carried_cut: {kind}, warm={warm}, registrations={registrations}, rows64={small}/{reverse_small}, rows4096={large}/{reverse_large}"
            );
            assert_eq!(
                small, reverse_small,
                "row order changed the small-corpus cost"
            );
            assert_eq!(
                large, reverse_large,
                "row order changed the large-corpus cost"
            );
            // Geometry emits four strips per registration, never one entry per
            // covered cell. XML parsing/serialization and result count remain.
            assert_eq!(
                small, large,
                "covered cell count changed sparse partition cost"
            );
        }
    }
}

#[test]
fn excel_carried_formula_rules_do_not_reparse_their_shared_parent_per_rule() {
    use yggdryl::excel::{Paste, Workbook};
    let cost = |rules, together| {
        let bytes = excel_package::carried_formula_rules_cost_package(rules, together);
        let mut workbook = Workbook::from_bytes(bytes).unwrap();
        workbook.parse_all().unwrap();
        let source = "A1".parse().unwrap();
        let target = "J10".parse().unwrap();
        let (allocations, result) =
            counted(|| workbook.paste(("Data", source), ("Other", target), Paste::All, true));
        result.unwrap();
        excel_package::assert_carried_formula_rules_followed(&workbook, rules);
        allocations
    };
    cost(1, true);
    cost(1, false);
    for (rules, single_parent_limit) in [(32, 16_847), (128, 67_279)] {
        let together = cost(rules, true);
        let separate = cost(rules, false);
        eprintln!(
            "excel_carried_formula_rules: rules={rules}, one_parent={together}, separate_parents={separate}"
        );
        // Measured before introducing the shared parent template. Removing
        // repeated sibling parsing must not add work to single-rule parents.
        assert!(
            separate <= single_parent_limit,
            "the shared-template optimization penalized single-rule parents"
        );
        // Both outputs retain the same rules and formula text. Sharing the
        // source scope cannot justify recapturing all sibling payloads once
        // for each output: the separate-parent corpus bounds that work.
        assert!(
            together <= separate,
            "grouping equivalent CF rules introduced repeated sibling parsing"
        );
    }
}

#[test]
fn a_spelling_read_by_its_words_reads_again_without_allocating() {
    // The first read cuts the spelling into words and keeps the member it
    // named; every later read of that spelling is one lookup.
    for spelling in ["order fill", "part-filled", "partial fill order"] {
        free(&format!("reading {spelling:?} again"), || {
            black_box(yggdryl::State::from_spelling(black_box(spelling)));
        });
    }
}

/// The AST belongs to one shared Shape. The first computed query pays one
/// shape-wide parse; another 4,096 Formula handles add no per-handle parse.
#[test]
fn excel_formula_compiles_once_per_shared_shape() {
    use std::hint::black_box;
    use yggdryl::excel::{CellRef, Formula};

    let cost = |copies: usize| {
        let host: CellRef = "C3".parse().unwrap();
        let formula = Formula::from_file("SUM(A1:A3)+2", host);
        let formulas = vec![formula; copies];
        let (first, computed) = counted(|| {
            formulas
                .iter()
                .filter(|formula| black_box(formula.is_computed()))
                .count()
        });
        assert_eq!(computed, copies);
        let (warm, again) = counted(|| {
            formulas
                .iter()
                .filter(|formula| black_box(formula.is_computed()))
                .count()
        });
        assert_eq!(again, copies);
        assert_eq!(warm, 0, "warm shared-shape queries allocate");
        first
    };
    let (small, large) = (cost(16), cost(4_096));
    assert!(small > 0, "first computed query did not build an arena");
    assert_eq!(
        large, small,
        "lazy AST allocations grew with Formula handles: {small} at 16, {large} at 4,096"
    );
}

#[test]
fn scalar_float_arithmetic_allocates_nothing() {
    let left = Scalar::from(1.0_f64);
    let right = Scalar::from(-(1.0 - 2.0_f64.powi(-52)));
    free("scalar float add", || {
        black_box(left.checked_add(&right).unwrap());
    });
}

#[cfg(feature = "internals")]
#[test]
fn excel_formula_number_comparison_and_finite_guard_allocate_nothing() {
    use yggdryl::internals::excel_formula_number::{add, equal, finite};

    let inputs = [
        (0.1 + 0.2, 0.3),
        (1.0, 1.0 + 2.0_f64.powi(-49)),
        (-0.1 - 0.2, -0.3),
        (f64::MAX, f64::from_bits(f64::MAX.to_bits() - 1)),
    ];
    for count in [64, 4096] {
        let (allocations, matches) = counted(|| {
            let mut matches = 0;
            for at in 0..count {
                let (left, right) = black_box(inputs[at % inputs.len()]);
                matches += usize::from(equal(left, right).unwrap());
                black_box(finite(left).unwrap());
                black_box(add(left, -right, true).unwrap().unwrap());
            }
            matches
        });
        assert_eq!(matches, count);
        // Digits' existing decimal formatting uses StackText; neither its
        // finite comparison nor the raw arithmetic guard materializes text.
        assert_eq!(allocations, 0, "{count} numeric inputs");
    }
}

#[test]
fn excel_numeric_entry_allocates_no_decimal_buffer() {
    use yggdryl::excel::{CellRef, DateSystem, Entry};

    for text in ["12345678901234567", "1,234,567.890123456789", "12.34%"] {
        let Entry::Value { .. } = Entry::from_text(text, CellRef::new(0, 0), DateSystem::Year1900)
            .expect("a numeric entry")
        else {
            panic!("{text:?} must be numeric");
        };
        for rows in [64, 4_096] {
            let (allocations, ()) = counted(|| {
                for _ in 0..rows {
                    black_box(
                        Entry::from_text(black_box(text), CellRef::new(0, 0), DateSystem::Year1900)
                            .expect("a numeric entry"),
                    );
                }
            });
            assert_eq!(allocations, 0, "{text:?} over {rows} numeric entries");
        }
    }
}

#[test]
fn expression_abs_uses_no_per_row_allocation() {
    use yggdryl::expression::Term;

    let schema = StructType::from_fields([DataType::Float64.required_field("x")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    let row = Scalar::from_sequence([Scalar::from(-1.25_f64)]);
    let bound = "abs(x)".parse::<Term>().unwrap().bind(&schema).unwrap();
    free("expression abs", || {
        black_box(bound.eval(black_box(&row)).unwrap());
    });
}

/// Calculation retains its evaluator node-value and explicit-stack buffers,
/// removing the former two allocations per pass. Warm graph/schedule scratch
/// must add none; the bound is checked at both formula-cell corpus sizes.
#[test]
fn excel_calculate_all_warm_noop_allocations_have_no_per_formula_slope() {
    use yggdryl::excel::{Cell, CellRef, DateSystem, Formula, Workbook};

    let cost = |rows: u32, expression: &str| {
        let mut book = Workbook::new();
        let formula = Formula::from_file(expression, CellRef::new(0, 0));
        let sheet = book.add_sheet("Data").unwrap();
        for row in 0..rows {
            let at = CellRef::new(row, 0);
            sheet
                .insert_cell(
                    Cell::from_scalar(at, Scalar::from(0.0), DateSystem::Year1900)
                        .unwrap()
                        .with_formula(formula.clone()),
                )
                .unwrap();
        }
        assert_eq!(book.calculate_all().unwrap().evaluated, u64::from(rows));
        let revision = book.sheet("Data").unwrap().revision();
        let (allocations, report) = counted(|| book.calculate_all().unwrap());
        assert_eq!(report.evaluated, u64::from(rows));
        assert_eq!(report.uncomputed, 0);
        assert_eq!(book.sheet("Data").unwrap().revision(), revision);
        allocations
    };
    for expression in [
        "1+2",
        "ABS(-7.25)",
        "SUM(1,2,3)",
        "ROUND(2.15,1)",
        "SQRT(2)",
        "MOD(-7,3)",
    ] {
        let (small, large) = (cost(64, expression), cost(4_096, expression));
        assert_eq!(
            (small, large),
            (0, 0),
            "warm {expression} allocated per formula cell: 64={small}, 4096={large}"
        );
    }
}

#[cfg(feature = "internals")]
#[test]
fn excel_dependency_queries_allocate_nothing() {
    use yggdryl::excel::{CellRange, CellRef, MAX_COLUMNS, Workbook};
    use yggdryl::internals::excel_formula_graph::Index;

    let mut workbook = Workbook::new();
    workbook.add_sheet("Data").unwrap();
    let sheet = workbook.sheet_key("Data").unwrap();
    for registrations in [64, 4096] {
        let mut index = Index::default();
        index
            .insert(sheet, CellRange::all(), registrations * 3)
            .unwrap();
        for id in 0..registrations {
            let row = id as u32 * 128;
            let point = CellRef::new(row, 1);
            index
                .insert(sheet, CellRange::new(point, point), id * 3)
                .unwrap();
            index
                .insert(
                    sheet,
                    CellRange::new(CellRef::new(row, 0), CellRef::new(row + 31, 7)),
                    id * 3 + 1,
                )
                .unwrap();
        }
        assert_eq!(index.dependents(sheet, CellRef::new(0, 1)).count(), 3);
        let (allocations, (matches, checksum)) = counted(|| {
            let mut matches = 0;
            let mut checksum = 0;
            for id in 0..registrations {
                let row = id as u32 * 128;
                for cell in [CellRef::new(row, 1), CellRef::new(row + 1, MAX_COLUMNS - 1)] {
                    // Query creation and full traversal are both measured;
                    // output is consumed directly rather than collected.
                    for dependent in index.dependents(sheet, black_box(cell)) {
                        matches += 1;
                        checksum += black_box(dependent);
                    }
                }
            }
            (matches, checksum)
        });
        assert_eq!(matches, registrations * 4);
        assert_eq!(
            checksum,
            9 * registrations * registrations - 2 * registrations
        );
        assert_eq!(
            allocations, 0,
            "queries across {registrations} point/area pairs"
        );
    }
}

#[cfg(feature = "internals")]
#[test]
fn excel_dependency_unchanged_schedule_allocates_nothing() {
    use yggdryl::excel::{CellRef, Workbook};
    use yggdryl::internals::excel_formula_graph::Scheduler;

    let mut workbook = Workbook::new();
    workbook.add_sheet("Data").unwrap();
    let sheet = workbook.sheet_key("Data").unwrap();
    for nodes in [64, 4096] {
        let mut graph = Scheduler::default();
        for row in 0..nodes {
            graph
                .set(sheet, CellRef::new(row as u32, 0), &[], false)
                .unwrap();
        }
        let full = graph.prepare(true).unwrap();
        assert_eq!(full.ordered().len(), nodes);
        graph.acknowledge(full, &vec![true; nodes]).unwrap();
        let (allocations, scheduled) = counted(|| {
            (0..64)
                .map(|_| {
                    let pass = black_box(graph.prepare(false).unwrap());
                    pass.ordered().len() + pass.circular().len() + pass.blocked().len()
                })
                .sum::<usize>()
        });
        assert_eq!(scheduled, 0);
        assert_eq!(
            allocations, 0,
            "unchanged pass over {nodes} nonvolatile nodes"
        );
        // The force-all operation remains a different contract even when
        // every cached result is already settled.
        assert_eq!(graph.prepare(true).unwrap().ordered().len(), nodes);
    }
}

#[test]
fn expression_sqrt_bound_rows_allocate_nothing() {
    use yggdryl::expression::Term;
    let schema = StructType::from_fields([DataType::Float32.required_field("x")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    let bound = "sqrt(x)".parse::<Term>().unwrap().bind(&schema).unwrap();
    let row = Scalar::from_sequence([Scalar::from(9.0_f32)]);
    assert_eq!(bound.eval(&row).unwrap(), Scalar::from(3.0_f64));
    for size in [64, 4_096] {
        let (allocations, ()) = counted(|| {
            for _ in 0..size {
                black_box(bound.eval(black_box(&row)).unwrap());
            }
        });
        assert_eq!(allocations, 0, "{size} bound rows");
    }
}

#[cfg(feature = "internals")]
#[test]
fn excel_reference_resolver_reads_and_name_lookups_allocate_nothing() {
    use yggdryl::excel::{CellRef, Formula, MAX_ROWS, Workbook};
    use yggdryl::internals::excel_workbook::{References, Resolved};
    for rows in [64, 4096] {
        for names in [64, 4096] {
            let book =
                Workbook::from_bytes(excel_package::reference_resolver_cost_package(rows, names))
                    .unwrap();
            let resolver = References::new(&book).unwrap();
            let origin = CellRef::new(0, 0);
            let point = Formula::from_file("A1", origin);
            let area = Formula::from_file("$A:$A", origin);
            let named = Formula::from_file("rESOLVERnAMEwITHmOREtHANiNLINEsTORAGE_0000", origin);
            // Shared lazy arenas are setup work; all lookup/read work follows.
            for formula in [&point, &area, &named] {
                black_box(resolver.resolve("Data", formula, origin).unwrap());
            }
            let Resolved::Name(binding) = resolver.resolve("Data", &named, origin).unwrap() else {
                panic!()
            };
            black_box(resolver.resolve_name_reference(&binding).unwrap());
            let (allocations, (stored, checksum, named_rows)) = counted(|| {
                let mut stored = 0;
                let mut checksum = 0_u64;
                for row in 0..rows {
                    let Resolved::Range(view) = resolver
                        .resolve("Data", &point, black_box(CellRef::new(row, 0)))
                        .unwrap()
                    else {
                        panic!()
                    };
                    for (_, cell) in view.cells() {
                        stored += 1;
                        checksum += u64::from(black_box(cell.reference().row()));
                    }
                }
                let Resolved::Range(view) = resolver.resolve("Data", &area, origin).unwrap() else {
                    panic!()
                };
                assert_eq!(view.logical_len(), u128::from(MAX_ROWS));
                stored += view.cells().count();
                let mut named_rows = 0;
                for _ in 0..64 {
                    let Resolved::Name(binding) = resolver.resolve("Data", &named, origin).unwrap()
                    else {
                        panic!()
                    };
                    let Resolved::Range(view) = resolver.resolve_name_reference(&binding).unwrap()
                    else {
                        panic!()
                    };
                    named_rows += view.cells().count();
                }
                (stored, checksum, named_rows)
            });
            assert_eq!(stored, 2 * rows as usize);
            assert_eq!(checksum, u64::from(rows) * u64::from(rows - 1) / 2);
            assert_eq!(named_rows, 64 * rows as usize);
            assert_eq!(allocations, 0, "{rows} stored rows, {names} names");
        }
    }
}

#[test]
fn decimal_float64_scalar_cast_allocates_nothing_per_row() {
    use yggdryl::{BigDecimal, Decimal, Decimal32, Decimal64, i256};

    let values = [
        Scalar::Decimal32(Decimal32::new(225, 2)),
        Scalar::Decimal64(Decimal64::new(225, 2)),
        Scalar::decimal128(225, 2),
        Scalar::decimal256(i256::from_i128(225), 2),
        Scalar::Decimal("2.25".parse::<Decimal>().unwrap()),
        Scalar::BigDecimal("2.25".parse::<BigDecimal>().unwrap()),
        Scalar::decimal256(
            "1234567890123456789012345678901234567890"
                .parse::<i256>()
                .unwrap(),
            2,
        ),
    ];
    let target = DataType::Float64;
    for value in &values {
        assert!(
            target
                .cast_scalar(value)
                .unwrap()
                .as_f64()
                .unwrap()
                .is_finite()
        );
        for count in [64, 4_096] {
            let (allocations, ()) = counted(|| {
                for _ in 0..count {
                    black_box(target.cast_scalar(black_box(value)).unwrap());
                }
            });
            assert_eq!(allocations, 0, "{count} decimal casts for {:?}", value.id());
        }
    }
}

#[cfg(feature = "internals")]
#[test]
fn excel_sheet_changes_inactive_edits_allocate_nothing() {
    use yggdryl::excel::{CellRef, Sheet};
    use yggdryl::internals::excel_sheet::changes_active;

    for rows in [64, 4_096] {
        let mut sheet = Sheet::new("Data").unwrap();
        for row in 0..rows {
            sheet.set_cell(CellRef::new(row, 0), 0.0).unwrap();
        }
        let (allocations, ()) = counted(|| {
            for row in 0..rows {
                sheet
                    .set_cell(black_box(CellRef::new(row, 0)), f64::from(row))
                    .unwrap();
                black_box(sheet.cell_mut(CellRef::new(row, 0)).unwrap().value());
            }
        });
        assert_eq!(sheet.cell_count(), rows as usize);
        assert_eq!(
            sheet.scalar(CellRef::new(rows - 1, 0)),
            Scalar::from(f64::from(rows - 1))
        );
        assert!(!changes_active(&sheet));
        assert_eq!(allocations, 0, "inactive journal, {rows} existing cells");
    }
}

#[cfg(feature = "internals")]
#[test]
fn excel_sheet_changes_repeated_points_and_rollback_reuse_capacity() {
    use yggdryl::excel::{CellRef, Sheet};
    use yggdryl::internals::excel_sheet::{
        acknowledge_changes, change_mark, pending_changes, restore_change_mark, track_changes,
    };

    for edits in [64, 4_096] {
        let mut sheet = Sheet::new("Data").unwrap();
        let at = CellRef::new(0, 0);
        sheet.set_cell(at, 1.0).unwrap();
        let generation = track_changes(&mut sheet);
        acknowledge_changes(&mut sheet);
        // Warm one point allocation. Repeated mutable borrows may affect its
        // formula, but do not need a second map or another point allocation.
        sheet.set_cell(at, 2.0).unwrap();
        let (allocations, ()) = counted(|| {
            for value in 0..edits {
                sheet.set_cell(at, f64::from(value)).unwrap();
                black_box(sheet.cell_mut(at).unwrap().value());
            }
        });
        assert_eq!(
            pending_changes(&sheet),
            Some((generation, false, vec![(at, true)]))
        );
        assert_eq!(allocations, 0, "{edits} rewrites of one pending point");

        sheet.set_cell(at, 1.0).unwrap();
        acknowledge_changes(&mut sheet);
        let (allocations, ()) = counted(|| {
            for value in 0..edits {
                let mark = change_mark(&sheet);
                sheet.set_cell(at, f64::from(value)).unwrap();
                // Payload rollback precedes the bounded bookkeeping reset.
                sheet.set_cell(at, 1.0).unwrap();
                restore_change_mark(&mut sheet, mark).unwrap();
            }
        });
        assert_eq!(sheet.scalar(at), Scalar::from(1.0));
        assert_eq!(pending_changes(&sheet), Some((generation, false, vec![])));
        assert_eq!(
            allocations, 0,
            "{edits} attempts reuse acknowledged capacity"
        );
    }
}

#[cfg(feature = "internals")]
#[test]
fn excel_direct_numeric_text_coercion_has_no_per_operand_allocation() {
    use yggdryl::excel::DateSystem;
    use yggdryl::internals::excel_formula_value::number_text;

    let inputs: [(&str, f64); 3] = [
        ("123456789012345", 123456789012345.0),
        ("1,234.50", 1234.5),
        ("12.5%", 0.125),
    ];
    for count in [64, 4096] {
        let (allocations, matches) = counted(|| {
            let mut matches = 0;
            for index in 0..count {
                let (input, expected) = black_box(inputs[index % inputs.len()]);
                let value = number_text(input, DateSystem::Year1900).unwrap().unwrap();
                matches += usize::from(value.to_bits() == expected.to_bits());
            }
            matches
        });
        assert_eq!(matches, count);
        assert_eq!(allocations, 0, "{count} direct numeric text operands");
    }
}

#[cfg(feature = "internals")]
#[test]
fn excel_dependency_full_schedule_reuses_warmed_buffers() {
    use yggdryl::excel::{CellRef, Workbook};
    use yggdryl::internals::excel_formula_graph::{Scheduler, Workspace};
    let mut book = Workbook::new();
    book.add_sheet("Data").unwrap();
    let sheet = book.sheet_key("Data").unwrap();
    for nodes in [64, 4_096] {
        let mut graph = Scheduler::default();
        for row in 0..nodes {
            graph
                .set(sheet, CellRef::new(row as u32, 0), &[], false)
                .unwrap();
        }
        let mut workspace = Workspace::default();
        let computed = vec![true; nodes];
        graph.prepare_reusing(true, &mut workspace).unwrap();
        graph
            .acknowledge_reusing(&mut workspace, &computed)
            .unwrap();
        let (allocations, visits) = counted(|| {
            let mut visits = 0;
            for _ in 0..4 {
                graph.prepare_reusing(true, &mut workspace).unwrap();
                visits += black_box(workspace.ordered().len());
                graph
                    .acknowledge_reusing(&mut workspace, &computed)
                    .unwrap();
            }
            visits
        });
        assert_eq!(visits, nodes * 4);
        assert_eq!(
            allocations, 0,
            "four warmed full schedules across {nodes} independent nodes"
        );
    }
}

#[test]
fn expression_substring_short_result_has_no_per_row_scratch() {
    use yggdryl::expression::Term;
    let schema = StructType::from_fields([DataType::utf8().required_field("s")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    let bound = "substring(s, 2, 2)"
        .parse::<Term>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    let row = Scalar::from_sequence([Scalar::from("a".repeat(256))]);
    assert_eq!(bound.eval(&row).unwrap(), Scalar::from("aa"));
    for size in [64, 4_096] {
        let (allocations, ()) = counted(|| {
            for _ in 0..size {
                black_box(bound.eval(black_box(&row)).unwrap());
            }
        });
        assert_eq!(allocations, 0, "{size} bound Unicode substring rows");
    }
}

#[test]
fn expression_trim_short_result_avoids_an_intermediate_string_per_row() {
    let schema = StructType::from_fields([DataType::utf8().required_field("s")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    let bound = "trim(s)".parse::<Term>().unwrap().bind(&schema).unwrap();
    let text = format!("{}abc{}", " ".repeat(64), " ".repeat(64));
    let row = Scalar::from_sequence([Scalar::from(text.as_str())]);
    assert_eq!(bound.eval(&row).unwrap(), Scalar::from("abc"));
    for size in [64, 4_096] {
        let (allocations, ()) = counted(|| {
            for _ in 0..size {
                black_box(bound.eval(black_box(&row)).unwrap());
            }
        });
        assert_eq!(allocations, 0, "{size} bound trim rows");
    }
}

#[cfg(feature = "internals")]
#[test]
fn excel_formula_evaluation_policy_visits_children_without_allocation() {
    use yggdryl::excel::{CellRef, Formula};
    use yggdryl::internals::excel_formula::strict_root_child_count;

    let formula = Formula::from_entry("SUM(A1,B1,C1)", CellRef::new(0, 3)).unwrap();
    assert_eq!(strict_root_child_count(&formula), Some(3));
    for count in [64, 4096] {
        let (allocations, visited) = counted(|| {
            let mut visited = 0;
            for _ in 0..count {
                visited += black_box(strict_root_child_count(black_box(&formula)).unwrap());
            }
            visited
        });
        assert_eq!(visited, count * 3);
        assert_eq!(allocations, 0, "{count} policy visits");
    }
}

/// The reference boundary skips known nonnumeric cells before rendering typed
/// text. Native strings share Str storage; graph/evaluator buffers stay owned.
#[test]
fn excel_calculation_sum_reference_text_has_zero_warm_allocations() {
    use yggdryl::excel::{Cell, CellRef, DateSystem, Formula, Workbook};
    let long = "not a numeric value ".repeat(512);
    let rendered = Scalar::from_sequence([Scalar::from(long.clone()), Scalar::from(17_i64)]);
    for (kind, value) in [
        ("native string", Scalar::from(long)),
        ("rendered typed text", rendered),
    ] {
        for rows in [64_u32, 4096] {
            let mut book = Workbook::new();
            let sheet = book.add_sheet("Data").unwrap();
            for row in 0..rows {
                sheet.set_cell(CellRef::new(row, 0), value.clone()).unwrap();
            }
            let at = CellRef::new(0, 1);
            sheet
                .insert_cell(
                    Cell::from_scalar(at, Scalar::from(-1.0), DateSystem::Year1900)
                        .unwrap()
                        .with_formula(Formula::from_file(&format!("SUM(A1:A{rows},2)"), at)),
                )
                .unwrap();
            let first = book.calculate_all().unwrap();
            assert_eq!((first.evaluated, first.uncomputed), (1, 0));
            assert_eq!(book.sheet("Data").unwrap().scalar(at), Scalar::from(2.0));
            let revision = book.sheet("Data").unwrap().revision();
            let (cost, report) = counted(|| book.calculate_all().unwrap());
            assert_eq!((report.evaluated, report.uncomputed), (1, 0));
            assert_eq!(book.sheet("Data").unwrap().revision(), revision);
            assert_eq!(
                cost, 0,
                "{kind}, {rows} rows must not render or allocate per referenced cell"
            );
        }
    }
}
