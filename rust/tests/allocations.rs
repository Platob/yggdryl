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

use std::sync::Arc;

use yggdryl::holder::Buffer;
use yggdryl::media::text::{TextBytes, TextLine, TextOptions, read_text_lines};
use yggdryl::types::{
    Bytes, INLINE_BYTES, INLINE_CAPACITY, Str, StringLayout, StringParameters,
    UncheckedFieldScalar, Uuid,
};
use yggdryl::{
    Charset, DataType, DataTypeId, Field, FieldPath, FieldRecord, FieldScalar, FixCode, FixCodec,
    FixId, FixLineageEntry, FixMsg, FixPedigree, FixRegistry, IOMedia, MediaType, MimeType,
    PythonKind, PythonMetadata, Scalar, TimeUnit, Timezone, Url, Version,
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
    use yggdryl::hashing::txhash::TxHash;
    use yggdryl::{Digest, DigestAlgorithm};

    let values: Vec<_> = [DigestAlgorithm::Xxh64, DigestAlgorithm::Xxh3]
        .into_iter()
        .flat_map(|algorithm| {
            [
                (i64::MIN, TimeUnit::Nanosecond),
                (-1, TimeUnit::Nanosecond),
                (0, TimeUnit::Nanosecond),
                (15, TimeUnit::Nanosecond),
                (16, TimeUnit::Nanosecond),
                (65_535, TimeUnit::Nanosecond),
                (65_536, TimeUnit::Nanosecond),
                (i64::MAX, TimeUnit::Nanosecond),
                (-1_700_000_000, TimeUnit::Second),
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
            &format!("projecting {count} TxHash values to UUIDv8"),
            || {
                for index in 0..count {
                    black_box(
                        black_box(values[index % values.len()])
                            .into_uuid()
                            .expect("an in-range nanosecond instant and a 64-bit digest"),
                    );
                }
            },
        );
    }
}

/// A field carrying HTTP headers plus `extra` unrelated metadata keys.
///
/// The extra keys sort after every `http:` one, so they are what a read walks
/// past rather than something it stops at.
fn http_field(extra: usize) -> Field {
    let mut field = Field::from_parts(
        "payload",
        DataType::binary(),
        false,
        [
            ("http:content-type", "application/json"),
            ("http:content-encoding", "gzip, br, zstd"),
            ("http:content-length", "4096"),
            ("http:etag", "\"revision-1\""),
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
    use yggdryl::media::iceberg::Transform;

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

/// A FIX registry of `extra` generated fields around two fully keyed fields,
/// one standard and one a venue's member, plus one repeating group.
///
/// The generated fields are what a probe walks past in the maps; the keyed
/// ones are what every hit lands on. The group keeps a nested shape in the
/// same index corpus.
fn fix_registry(extra: usize) -> FixRegistry {
    let item = DataType::from_fields([DataType::utf8().nullable_field("PartyID")])
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
        .set_aliases(["Ticker", "SecuritySymbolIdentifier"])
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
        .set_aliases(["TradeIdentifier"])
        .expect("a static alias");
    let generated = (0..extra).map(|index| {
        let mut field = DataType::Int64.nullable_field(format!("Generated{index:04}"));
        let tag = i32::try_from(1_000 + index).expect("a small tag");
        field.as_fix_mut().set_tag(tag).expect("a generated tag");
        field
            .as_fix_mut()
            .set_aliases([format!("GeneratedAlias{index:04}")])
            .expect("a generated alias");
        field
    });
    let mut registry = FixRegistry::from_fields(
        [symbol, msgtype, trade, counter]
            .into_iter()
            .chain(generated),
    )
    .expect("the generated dictionary has no conflict");
    registry
        .insert_definition(yggdryl::FixCategory::Groups, parties)
        .expect("the group definition");
    registry
}

#[test]
fn a_fix_registry_lookup_allocates_nothing() {
    // A wide dictionary: a hit must cost the same however much it walks past.
    let registry = fix_registry(512);
    let vendor = FixId::of(5_001, "TradeID").expect("a vendor identifier");
    // Another name on the same tag is another identity: an exact miss.
    let foreign = FixId::of(5_001, "OtherTradeID").expect("a foreign identifier");

    free("get_field_by_tag scalar hit", || {
        let _ = black_box(registry.get_field_by_tag(55));
    });
    free("get_field_by_tag counter hit", || {
        let _ = black_box(registry.get_field_by_tag(453));
    });
    free("get_field_by_name counter hit", || {
        let _ = black_box(registry.get_field_by_name("nopartyids"));
    });
    free("get_definition group hit", || {
        let _ = black_box(registry.get_definition(yggdryl::FixCategory::Groups, "PARTIES"));
    });
    free("get_field_by_tag alternate hit", || {
        let _ = black_box(registry.get_field_by_tag(65));
    });
    free("get_field_by_tag miss", || {
        let _ = black_box(registry.get_field_by_tag(7));
    });
    // An identifier is the hash key itself, tag and folded name, so vendor
    // probes cost what standard ones do.
    free("get_field_by_id vendor hit", || {
        let _ = black_box(registry.get_field_by_id(vendor));
    });
    free("get_field_by_id vendor miss", || {
        let _ = black_box(registry.get_field_by_id(foreign));
    });
    // The name index is probed with the caller's text folded as it is
    // hashed, so a differently cased query builds no folded copy.
    free("get_field_by_name differently cased hit", || {
        let _ = black_box(registry.get_field_by_name("sYmBoL"));
    });
    free("get_field_by_name alias hit", || {
        let _ = black_box(registry.get_field_by_name("TICKER"));
    });
    free("get_field_by_name long alias hit", || {
        let _ = black_box(registry.get_field_by_name("securitysymbolidentifier"));
    });
    // A member field lives in the one namespace: its name answers without
    // any dialect being named.
    free("get_field_by_name vendor hit", || {
        let _ = black_box(registry.get_field_by_name("tradeid"));
    });
    free("get_field_by_name miss", || {
        let _ = black_box(registry.get_field_by_name("absent"));
    });
    free("get_field generic", || {
        let _ = black_box(registry.get_field("ticker"));
        let _ = black_box(registry.get_field(65));
        let _ = black_box(registry.get_field(vendor));
    });
    // Resolved once, outside the closure, because that is where a path is
    // read: what the lookup itself costs is nothing.
    let absent_member = FieldPath::from_str("Symbol.absent").expect("a path");
    free("get_field_by_path member", || {
        let _ = black_box(registry.get_field_by_path(&absent_member));
    });
    free("contains", || {
        let _ = black_box(registry.contains("Symbol"));
    });
    free("infer_bytes_protocol FIXML", || {
        let _ = black_box(MimeType::infer_bytes(black_box(b"35=D|Symbol=AAPL|")));
    });
    free("infer_text_protocol UL", || {
        let _ = black_box(MimeType::infer_text(black_box("MsgType=D Symbol=AAPL")));
    });
    free("infer_bytes_msgtype FIX", || {
        let _ = black_box(FixCodec::infer_msgtype_bytes(black_box(
            b"8=FIX.4.4|35=D|55=AAPL|",
        )));
    });
    free("infer_text_msgtype UL", || {
        let _ = black_box(FixCodec::infer_msgtype_text(black_box(
            "MsgType=D Symbol=AAPL",
        )));
    });
    // A bridge configuration is read the same way a frame is: the namespace,
    // the ObjectName's type and the answer keys are all found in the caller's
    // bytes, so classifying a document costs no allocation either. It
    // classifies as `application/json` now (decision 17) - what makes one a
    // configuration is a shape the codec probes at the offset this scan
    // already found, so nothing is looked for twice and the cost is the same.
    const PLUGIN: &[u8] = br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=X,plugin-type=FIX,type=Plugin","type":"read"},"value":{"Name":"X"},"status":200}"#;
    free("infer_bytes_protocol PLUGIN", || {
        let _ = black_box(MimeType::infer_bytes(black_box(PLUGIN)));
    });
    free("infer_bytes_msgtype PLUGIN", || {
        let _ = black_box(FixCodec::infer_msgtype_bytes(black_box(PLUGIN)));
    });
    let reading = registry.msgdirection();
    free("read_bytes_direction PLUGIN", || {
        let _ = black_box(reading.read_bytes(black_box(PLUGIN)));
    });
    // The rules are compiled once with the reading; applying them to the
    // prose in front of a payload costs nothing per line, whether one code
    // matches, two do, or none (decision 15).
    for line in [
        b"sending >> 8=FIX.4.4|35=D|10=0|".as_slice(),
        b"2026-08-14 03:03:13.314 [23] [Jolokia] (DEBUG) Response: 8=FIX.4.4|35=0|10=0|",
        b"sending and receiving 8=FIX.4.4|35=D|10=0|",
        b"no verb printed by this plugin 8=FIX.4.4|35=D|10=0|",
    ] {
        free("read_bytes_direction prose", || {
            let _ = black_box(reading.read_bytes(black_box(line)));
        });
    }
    // And a table the dictionary states costs the same as the defaults.
    let mut ruled = yggdryl::FixRegistry::new();
    let mut field = yggdryl::DataType::utf8().nullable_field("MsgDirection");
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
        free("read_bytes_direction stated table", || {
            let _ = black_box(ruled.read_bytes(black_box(line)));
        });
    }
    free("iter", || {
        let _ = black_box(registry.iter().count());
    });
}

#[test]
fn parsed_plugin_wildcards_iterate_without_allocating_results() {
    for size in [1, 32, 256] {
        let values = Scalar::from_record((0..size).map(|index| {
            let name = format!("Configuration{index:04}");
            let mbean = format!(
                "com.ullink.ulbridge.sessioninterfaces.plugins:name={name},type=ConfigurationPlugin"
            );
            let attributes = Scalar::from_record([("Name", Scalar::from(name))]).unwrap();
            (mbean, attributes)
        }))
        .unwrap();
        let document = Scalar::from_record([("value", values)]).unwrap();

        // Reading a document cannot fail: one that names no plugin answers
        // none, and answering none is what it answers (decision 17), so there
        // is no validation pass in front of the walk and nothing to unwrap.
        let (first_allocations, first) = counted(|| {
            yggdryl::Plugin::from_json_scalar(black_box(&document))
                .next()
                .expect("the wildcard has configurations")
        });
        assert_eq!(first.name(), Some("Configuration0000"));
        // The shared stable hash owns one XXH3 secret buffer; feeding the
        // selected configuration allocates nothing proportional to its siblings.
        costs("hashing one selected plugin", 1, || {
            black_box(first.stable_hash());
        });
        assert_eq!(
            first_allocations, 0,
            "first result for {size} configurations"
        );

        let (drain_allocations, read) = counted(|| {
            yggdryl::Plugin::from_json_scalar(black_box(&document))
                .inspect(|configuration| {
                    black_box(configuration.name());
                })
                .count()
        });
        assert_eq!(read, size);
        assert_eq!(drain_allocations, 0, "draining {size} configurations");
    }
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
    // would break (decision 16).
    // Settle the shared schema/registry plan before counting the repeated path.
    // Cold plan construction belongs to the boundary, not to each frame.
    black_box(read(1));
    let each = 32;
    for frames in [4, 8, 16] {
        assert_eq!(read(frames), each * frames, "{frames} frames");
    }
}

#[test]
fn registry_message_singletons_and_scoped_groups_are_borrowed() {
    let mut registry = fix_registry(512);
    let mut message = DataType::from_fields([
        registry.field_by_tag(453).unwrap().clone(),
        registry
            .definition(yggdryl::FixCategory::Groups, "Parties")
            .unwrap()
            .clone(),
    ])
    .unwrap()
    .required_field("newordersingle");
    message.as_fix_mut().set_msgtype("D").unwrap();
    registry
        .insert_definition(yggdryl::FixCategory::Components, message)
        .unwrap();
    let held = registry.msgtype("D").unwrap();
    let counter = 453;
    assert_eq!(held.get_group_by_tag(counter).unwrap().name(), "Parties");
    free("registry message singleton", || {
        black_box(registry.get_msgtype("D"));
        black_box(registry.msgtypes().next());
    });
    free("borrowed message fields and scoped group", || {
        black_box(held.as_field());
        black_box(held.name());
        black_box(held.as_str());
        black_box(held.get_group_by_tag(counter));
    });
}

#[test]
fn fix_hash_state_allocation_is_constant_across_catalog_sizes() {
    for size in [1, 32, 512] {
        let mut registry = fix_registry(size);
        let mut definition = DataType::from_fields(registry.iter().cloned())
            .unwrap()
            .required_field("HashFixture");
        definition.as_fix_mut().set_msgtype("H").unwrap();
        registry
            .create_definition(yggdryl::FixCategory::Components, definition)
            .unwrap();
        let held = registry.msgtype("H").unwrap();
        // Each call constructs one shared XXH3 state. The native structural
        // feed adds no allocations as the fields and message schema grow.
        costs("registry stable hash", 1, || {
            black_box(registry.stable_hash());
        });
        costs("message definition stable hash", 1, || {
            black_box(held.stable_hash());
        });
        let message = FixCodec::new(std::sync::Arc::new(registry))
            .parse_fix_line(b"35=H|55=AAPL|")
            .unwrap();
        costs("message value stable hash", 1, || {
            black_box(message.stable_hash());
        });
    }
}

#[test]
fn fix_identifier_declarations_and_compiled_selection_allocate_nothing() {
    for size in [4, 16, 64] {
        let fields: Vec<_> = (0..size)
            .map(|index| {
                let mut field = DataType::utf8().nullable_field(format!("Identifier{index}"));
                field.as_fix_mut().set_tag(10_000 + index).unwrap();
                field
            })
            .collect();
        let mut definition = DataType::from_fields(fields.clone())
            .unwrap()
            .required_field("identifierfixture");
        definition.as_fix_mut().set_msgtype("UIDS").unwrap();
        definition
            .as_fix_mut()
            .set_identifiers(fields.iter().map(Field::name))
            .unwrap();
        let mut registry = FixRegistry::from_fields(fields).unwrap();
        registry
            .create_definition(yggdryl::FixCategory::Components, definition.clone())
            .unwrap();
        let registry = Arc::new(registry);
        let message = FixMsg::with_registry(
            Arc::clone(&registry),
            definition,
            Scalar::from_sequence(
                (0..size)
                    .map(|index| Scalar::from(format!("VALUE-{index}")))
                    .collect::<Vec<_>>(),
            ),
        )
        .unwrap();
        let compiled = registry.msgtype("UIDS").unwrap();
        assert_eq!(
            compiled.identifier_values(&message).count(),
            usize::try_from(size).unwrap()
        );
        free("borrowed FIX identifier declaration", || {
            for name in compiled.as_field().as_fix().identifiers() {
                black_box(name);
            }
        });
        free("compiled FIX identifier selection", || {
            for (field, value) in compiled.identifier_values(&message) {
                black_box((field, value));
            }
        });
    }
}

#[test]
fn fix_field_code_metadata_and_category_cursors_allocate_nothing() {
    for size in [1, 32, 512] {
        let mut registry = FixRegistry::new();
        for index in 0..size {
            let mut field = DataType::utf8().nullable_field(format!("Code{index}"));
            // Registry tags are positive; zero is only unresolved arrival provenance.
            field.as_fix_mut().set_tag(index + 1).unwrap();
            field
                .as_fix_mut()
                .set_codes(&[FixCode::new("Buy", "1")])
                .unwrap();
            registry.insert(field).unwrap();
        }
        registry
            .insert_definition(
                yggdryl::FixCategory::Components,
                DataType::from_fields([])
                    .unwrap()
                    .required_field("component"),
            )
            .unwrap();
        let field = registry.field(size).unwrap();
        free("field code metadata lookup", || {
            assert_eq!(black_box(field.as_fix().code_name("1")), Some("Buy"));
        });
        free("category cursor and iteration setup", || {
            assert!(
                black_box(registry.definition_at(yggdryl::FixCategory::Components, 0)).is_some()
            );
            assert!(
                black_box(
                    registry
                        .definitions(yggdryl::FixCategory::Components)
                        .next()
                )
                .is_some()
            );
            assert!(black_box(registry.definition_at(yggdryl::FixCategory::Fields, 0)).is_some());
        });
    }
}

#[test]
fn a_fix_lineage_read_allocates_nothing() {
    // Every spelling a lineage answers is a slice of the field's own stored
    // document, so a version filter costs the walk and nothing else. Only
    // `dtype_at` allocates, because building a `DataType` is what it answers.
    let mut field = DataType::utf8().nullable_field("LastQty");
    field.as_fix_mut().set_tag(32).expect("a static tag");
    let entries = [
        FixLineageEntry::new(FixPedigree::new(
            "2.7".parse::<Version>().expect("a version"),
            None,
        ))
        .with_name("LastShares")
        .with_dtype("int"),
        FixLineageEntry::new(FixPedigree::new(
            "4.2".parse::<Version>().expect("a version"),
            Some(204),
        ))
        .with_name("LastShares")
        .with_dtype("Qty")
        .with_doc("Quantity of shares bought or sold on this fill."),
        FixLineageEntry::new(FixPedigree::new(
            "4.3".parse::<Version>().expect("a version"),
            None,
        ))
        .with_name("LastQty")
        .with_dtype("utf8"),
    ];
    field
        .as_fix_mut()
        .set_lineage(&entries)
        .expect("a lineage agreeing with its field");

    let view = field.as_fix();
    let newest = "5.0.2".parse::<Version>().expect("a version");
    let old = "4.2".parse::<Version>().expect("a version");

    free("lineage walk", || {
        let _ = black_box(view.lineage().count());
    });
    free("since", || {
        let _ = black_box(view.since());
    });
    free("until", || {
        let _ = black_box(view.until());
    });
    free("defined_at", || {
        let _ = black_box(view.defined_at(old));
    });
    free("name_at old", || {
        let _ = black_box(view.name_at(old));
    });
    free("name_at newest", || {
        let _ = black_box(view.name_at(newest));
    });
    // A document the scan refuses costs no allocation either: the byte
    // position is carried by the borrowed cursor, not by a rendered copy.
    let mut edited = DataType::utf8().nullable_field("LastShares");
    edited
        .set_metadata([("fix:lineage", r#"{"entries":[{"name":"x","since":"2.7"}]}"#)])
        .expect("a hand-edited document");
    let refused = edited.as_fix();
    free("name_at refused", || {
        let _ = black_box(refused.name_at(old));
    });
}

#[test]
fn a_fix_code_lookup_allocates_nothing() {
    // A 300-code set: a lookup must cost the codes it walks past and no
    // allocation, whichever tier answers it.
    let codes: Vec<FixCode> = (0..300)
        .map(|index| {
            FixCode::new(format!("Member{index:04}"), format!("{index:04}"))
                .with_description(format!("Member number {index} (M{index:04})"))
        })
        .collect();
    let mut field = DataType::utf8().nullable_field("Vocabulary");
    field.as_fix_mut().set_tag(9995).expect("a static tag");
    field
        .as_fix_mut()
        .set_codes(&codes)
        .expect("a valid code set");
    let view = field.as_fix();

    free("codes walk", || {
        let _ = black_box(view.codes().count());
    });
    // Tier 1 stops at the match; the last code is the worst case.
    free("code first", || {
        let _ = black_box(view.code(black_box("0000")));
    });
    free("code last", || {
        let _ = black_box(view.code(black_box("0299")));
    });
    free("code miss", || {
        let _ = black_box(view.code(black_box("absent")));
    });
    // Tier 2 runs the whole set, because ambiguity must answer nothing.
    free("code_by_name folded", || {
        let _ = black_box(view.code_by_name(black_box("member_0299")));
    });
    free("code_value tier one", || {
        let _ = black_box(view.code_value(black_box("0150")));
    });
    free("code_value tier two", || {
        let _ = black_box(view.code_value(black_box("MEMBER 0150")));
    });
    // Tier 3 reads a description it never decodes, so it allocates nothing
    // either.
    free("code_value tier three", || {
        let _ = black_box(view.code_value(black_box("m0150")));
    });
    free("code_value_at", || {
        let _ = black_box(view.code_value_at(black_box(Version::MAX), black_box("0150")));
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
    let root = DataType::from_fields([symbol, trade, DataType::utf8().nullable_field("9999")])
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
        let _ = black_box(msg.get_by_name("tradeid"));
    });
    free("get_by_tag vendor", || {
        let _ = black_box(msg.get_by_tag(5_001));
    });
    free("get_by_tag known", || {
        let _ = black_box(msg.get_by_tag(55));
    });
    free("get_by_id vendor", || {
        let _ = black_box(msg.get_by_id(vendor));
    });
    free("get_by_id foreign", || {
        let _ = black_box(msg.get_by_id(foreign));
    });
    // An unknown tag is rendered on the stack and looked up by that name.
    free("get_by_tag unknown retained", || {
        let _ = black_box(msg.get_by_tag(9999));
    });
    free("get_by_tag unknown absent", || {
        let _ = black_box(msg.get_by_tag(1234));
    });
    free("get_by_name", || {
        let _ = black_box(msg.get_by_name("ticker"));
    });
    let absent_member = FieldPath::from_str("Symbol.absent").expect("a path");
    free("get_by_path", || {
        let _ = black_box(msg.get_by_path(&absent_member));
    });
}

#[test]
fn settled_fix_identity_getters_borrow_without_allocating_at_every_row_width() {
    for width in [0, 64, 1_024] {
        let field = DataType::from_fields(
            (0..width).map(|index| DataType::Int64.required_field(format!("datum{index}"))),
        )
        .unwrap()
        .required_field("row");
        let value = Scalar::from_sequence((0..width).map(Scalar::from));
        let message = FixMsg::with_registry(Arc::new(FixRegistry::new()), field, value).unwrap();
        free("four settled identity borrows", || {
            let held = black_box(&message);
            black_box((
                held.updatedat(),
                held.createdat(),
                held.msghash(),
                held.msgphash(),
            ));
        });
        free("four settled identity tag borrows", || {
            let held = black_box(&message);
            for tag in [
                yggdryl::UPDATEDAT_TAG_NAME.0,
                yggdryl::CREATEDAT_TAG_NAME.0,
                yggdryl::MSGHASH_TAG_NAME.0,
                yggdryl::MSGPHASH_TAG_NAME.0,
            ] {
                black_box(held.get_by_tag(black_box(tag)));
            }
        });
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
        [("http:content-type", "application/json")],
    )
    .expect("the static content type is valid");
    free("media_type without codings", || {
        let _ = black_box(base_only.as_http().media_type());
    });
}

#[cfg(feature = "iceberg")]
#[test]
fn an_iceberg_read_costs_only_a_key_the_inline_buffer_cannot_hold() {
    let field = iceberg_field(256);

    // A lookup key is assembled into a `SmolStr`, which holds 23 bytes inline.
    // `iceberg:schema-id` and `iceberg:spec-id` fit, so those reads are free.
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

    // `iceberg:partition-source-id` is 27 bytes and does not, so the assembled
    // key goes to the heap. This is the boundary, pinned: it is a property of
    // how long the name is, not of the value being parsed.
    costs("partition_source_id", 2, || {
        let _ = black_box(field.as_iceberg().partition_source_id());
    });

    // The identifier list costs that same long key plus the vector it returns,
    // which grows by doubling rather than once per identifier.
    costs("identifier_field_ids", 3, || {
        let _ = black_box(field.as_iceberg().identifier_field_ids());
    });
    let mut wider = iceberg_field(0);
    wider
        .as_iceberg_mut()
        .set_identifier_field_ids(&[1, 2, 3, 4, 5, 6, 7, 8, 9])
        .expect("static identifier columns");
    costs("identifier_field_ids over nine", 5, || {
        let _ = black_box(wider.as_iceberg().identifier_field_ids());
    });
}

#[test]
fn a_python_read_costs_only_the_declaration_it_hands_back() {
    let field = python_field("trading.book", 256);

    // Every `python:` key is shorter than `SmolStr`'s 23-byte inline buffer -
    // `python:qualname` is the longest at 15 - so no assembled lookup key ever
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
    let wide = Scalar::from_record(
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
        let mut sink = yggdryl::hashing::xxhash::Xxh3::new();
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
    let root = DataType::from_fields([
        Field::new("symbol", DataType::utf8(), false),
        Field::new("payload", DataType::binary(), false),
        Field::new("ccy", DataType::Currency, false),
        Field::new("venue", DataType::ascii(), false),
    ])
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
    let large = StringParameters::utf8(StringLayout::LargeString);
    free(
        "restating a shared string value under another layout",
        || {
            let restated = black_box(&source)
                .clone()
                .try_with_parameters(large)
                .expect("the layout holds it");
            assert!(std::ptr::eq(source.as_str(), restated.as_str()));
            black_box(restated);
        },
    );
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

/// The two batches every cast-budget case reads, and the root they answer to.
///
/// The batches differ only in their values, so anything that varies between
/// casting one and casting the other is per-batch work rather than schema work.
#[cfg(feature = "arrow")]
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
        DataType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
            DataType::utf8().required_field("venue"),
        ])
        .expect("the root fields are valid"),
        false,
    );
    (Arc::clone(&schema), [batch(0), batch(2)], root)
}

#[test]
#[cfg(feature = "arrow")]
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
#[cfg(feature = "arrow")]
fn planning_once_is_what_a_reused_plan_saves_per_batch() {
    use yggdryl::{ArrowCast, ArrowCastOptions, ArrowCastPlan};

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
    use yggdryl::hashing::txhash::{self, TxHash};
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
    use yggdryl::hashing::txhash;
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
    use yggdryl::{TimeUnit, hashing::txhash};

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
    let seeds: [(DataTypeId, Scalar); 34] = [
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
        (DataTypeId::String, Scalar::from("AAPL")),
        (DataTypeId::LargeString, Scalar::from("AAPL")),
        (DataTypeId::StringView, Scalar::from("AAPL")),
        (DataTypeId::Country, Scalar::from("US")),
        (DataTypeId::Currency, Scalar::from("USD")),
        (DataTypeId::Mic, Scalar::from("XNAS")),
        (DataTypeId::Cfi, Scalar::from("ESVUFR")),
        (DataTypeId::Isin, Scalar::from("US0378331005")),
        (DataTypeId::Cusip, Scalar::from("037833100")),
        (DataTypeId::Sedol, Scalar::from("B0YBKJ7")),
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
    let url = DataType::Url
        .scalar("https://example.com/a")
        .expect("the text is a URL");
    values.push((DataTypeId::Url, url));
    // A MIME type and a media type are pinned outside the seed loop for the
    // same reason a URL is: their canonical text is not what was written.
    let mime = DataType::MimeType
        .scalar("application/json")
        .expect("the text is a MIME type");
    values.push((DataTypeId::MimeType, mime));
    let media = DataType::MediaType
        .scalar("application/json; charset=utf-8")
        .expect("the text is a media type");
    values.push((DataTypeId::MediaType, media));
    // Every prebuilt id is either pinned here or the one that nothing names.
    let pinned: std::collections::HashSet<DataTypeId> = values.iter().map(|(id, _)| *id).collect();
    for id in DataTypeId::ALL {
        let prebuilt = !id.is_parameterized()
            && DataType::from_str(id.as_str()).is_ok_and(|dtype| dtype.id() == id);
        if prebuilt && id != DataTypeId::Variant {
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
    let root = DataType::from_fields(
        (0..width).map(|index| DataType::Int64.required_field(format!("column_{index}"))),
    )
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

/// A framed body of `pairs` pairs, every one of them a field the dictionary
/// holds.
///
/// The keys are the generated tags [`fix_registry`] writes, so each pair
/// resolves to its own child and no two repeat: what grows between the sizes
/// below is the width of the message and nothing else.
fn fix_pairs_line(pairs: usize) -> Vec<u8> {
    let mut line = b"35=D".to_vec();
    for index in 0..pairs.saturating_sub(1) {
        line.extend_from_slice(format!("|{}={index}", 1_000 + index).as_bytes());
    }
    line.push(b'|');
    line
}

/// What reading a message off a line costs, by how many pairs the line
/// carries: four, sixteen and sixty-four.
///
/// Three widths distinguish a fixed boundary cost from one paid per pair.
/// Settling the mandatory replay bundle adds eight containers: three for the
/// initial column plan (columns, named-content order, shared plan), two staged
/// field/value vectors, two settled shared slices, and the final tag index.
/// The replaced timestamp Slot no longer allocates its one-value vector, so
/// the net increase is seven. Above sixteen final columns, the Struct's
/// duplicate-name check adds one more allocation: these rows have 13, 25 and
/// 73 columns after settlement. Thus the former counts gain 7, 8 and 8, not
/// another allocation per pair. The steady-state final plan is cache-reused;
/// initial buffers reserve their exact bounds and already-correct tags are
/// not rewritten. Native clock and identity values add no per-cell buffer.
///
/// Arrival keys and values remain ranges of one shared page. These short
/// strings fit inline even in the typed row; the separate wide-value test
/// pins the one allocation each long row value needs and none for its entry.
///
/// A frame that marked no key judges none - which every wire frame is - so
/// the reading takes each key as it stands. That drops two things the judged
/// reading paid for: the vector of the row's whole width built only to hand
/// each key straight back, and the growth of the pairs vector, which a
/// `filter_map` gave no width to reserve from and now reserves exactly once.
/// The first is one per message at every width; the second is the doublings
/// a width needs, which is why sixty-four pairs save more than four do.
///
/// A caller who decoded the line already owns that page, and
/// [`FIX_TEXT_LINE_COSTS`] is the same three widths through the door that
/// takes it: two fewer at each, which is the page and nothing else.
const FIX_LINE_COSTS: [(usize, usize); 3] = [(4, 32), (16, 46), (64, 94)];

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
/// The wide reading costs exactly one allocation more *per pair*, and that
/// one is the row's own: a typed `Utf8` column holds the value it was given,
/// and a column is what a row is for. The entry beside it adds nothing at
/// all, because a key and a value are ranges of the page the line was read
/// into and a range is two offsets whatever it spans. Under the shape this
/// replaced the entry copied the value too, so the slope was two per pair
/// rather than one - and it copied every one of those kilobytes besides,
/// which no count sees and every capture pays.
///
/// Three pair counts and two widths, because one of each could tell neither a
/// per-message cost from a per-pair one nor a cost that scales with a value
/// from one that does not. The narrow column of this table is
/// [`FIX_LINE_COSTS`] at the same widths, and moves with it.
const WIDE_VALUE_COSTS: [(usize, (usize, usize)); 3] =
    [(4, (32, 35)), (16, (46, 61)), (64, (94, 157))];

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
        // numbers a reader has to subtract: one column per wide value, and
        // nothing for the entry that names the same bytes.
        assert_eq!(
            wide - narrow,
            pairs - 1,
            "a {pairs}-pair line paid more than one allocation per wide value"
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
    let item = DataType::from_fields(declared)
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
    registry
        .insert_definition(yggdryl::FixCategory::Groups, parties)
        .expect("the group definition");
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
/// The path decision 5 is about, and the one the plain frame above never
/// reaches. A bridge writes a whole occurrence into one value and the reader
/// unpacks it into `NOPARTYIDS[0].MEMBER0000` and its siblings, which are
/// keys no range of the line names - so they are the one thing on this path
/// that has to be built rather than pointed at.
///
/// Each is exactly one allocation: the path, held as the bytes it is. That is
/// the whole reason a rendered key is not a `TextBytes` - wrapping one in a
/// counted page of its own is three, the rendered vector, a copy of it and
/// the page, and the page is then only ever borrowed back as a slice. Two
/// more per member, measured, on the very path decision 5 names the cost of.
///
/// Four members and sixteen, because the number that matters is the slope,
/// and the rest of it is the row a wider group builds. The codec reads a
/// row's pairs directly and descends into none of them, so the tree the
/// packed value would have been scanned into is not among these.
/// The settled bundle adds the same net seven containers as [`FIX_LINE_COSTS`]
/// at both widths: neither root crosses the Struct duplicate-check threshold.
/// The 34-allocation growth for twelve more members is unchanged.
const PACKED_MEMBER_COSTS: [(usize, usize); 2] = [(4, 66), (16, 100)];

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
/// Two allocations fewer per message than [`FIX_LINE_COSTS`] at every width,
/// and the two are the page. A caller holding a [`TextLine`] already owns the
/// bytes as a range of a page it read them into, so the codec is handed that
/// page instead of making a second one - which is what the byte door must do,
/// because a bare slice is not a page and a message keeps ranges of one.
///
/// Two and not one because a page is an `Arc<Vec<u8>>`: the vector's own
/// buffer, and the shared box around it that lets every key and value name a
/// range of the same bytes without copying them.
///
/// The saving is per message and not per pair, which is exactly right: a page
/// is one page however many pairs the line carries, so the slope is unchanged
/// and only the constant moves. Three widths again, so that the claim is the
/// constant and not a number that happens to be smaller.
const FIX_TEXT_LINE_COSTS: [(usize, usize); 3] = [(4, 30), (16, 44), (64, 92)];

#[test]
fn a_message_read_from_a_decoded_line_does_not_pay_for_its_page_again() {
    let codec = FixCodec::new(Arc::new(fix_registry(64)));
    for ((pairs, each), (widest, bytes)) in FIX_TEXT_LINE_COSTS.iter().zip(FIX_LINE_COSTS) {
        assert_eq!(*pairs, widest, "the two pins measure the same widths");
        assert_eq!(each + 2, bytes, "the page is the whole of the difference");
        let held = fix_pairs_line(*pairs);
        // The page is made outside the counted closure because that is what a
        // caller reading text actually has: the decode already happened, and
        // what is measured here is what reading a message from it adds.
        let line = TextLine::from_bytes(0, TextBytes::from_bytes(&held).expect("a page")).unwrap();
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
fn first_text_line_from_arrow_does_not_decode_the_rest_of_its_batch() {
    use arrow_array::RecordBatchIterator;
    use yggdryl::media::text::{from_arrow_reader, into_arrow_batch};

    let options = TextOptions::new();
    let mut first_cost = None;
    for rows in [1, 64, 1024] {
        let lines = (0..rows).map(|index| {
            TextLine::from_bytes(index, TextBytes::from_bytes("one body").unwrap()).unwrap()
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
/// below counts 32 for 16 rows and the same 9 over the copy for 1 024 -
/// after the two the buffer's first `url` costs, which [`text_lines_cost`]
/// asks for before the counter is armed and which are in neither number.
const TEXT_LINES_ONCE: usize = 9;

/// What a reader that keeps its lines pays on top: two per window it had to
/// leave behind.
///
/// A window a line is a range of cannot be written over, so the refill that
/// finds one takes a fresh window - one vector and one shared box - and
/// moves the open tail into it. That is the whole price of retention, and it
/// is per window of the object rather than per line of it: a caller holding
/// a million lines of a megabyte holds sixteen pages, not a million.
const TEXT_LINES_RETAINED_PER_WINDOW: usize = 2;

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
    // The claim decision 12 makes for the transport: nothing per line. The
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
        DataType::from_str("string(windows-1252)").expect("a charset string"),
        DataType::from_str("large_string(windows-1252)").expect("a charset string"),
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
fn enriching_costs_one_working_row_per_message_and_nothing_per_shape() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let folder = yggdryl::holder::local::Folder::new(root).expect("the local seed path");
    let registry =
        Arc::new(FixRegistry::from_handle(&folder).expect("the committed dictionary loads"));
    let codec = FixCodec::new(Arc::clone(&registry));
    // A fill naming an unsourced ISIN, a CFI and a market: nine derivations
    // land on it (the source, the ISIN column and its country, the security
    // type and the product, the market, the status and the state, and the
    // day order).
    let line = b"8=FIX.4.4|35=8|37=A|48=US0378331005|461=ESVTFR|207=XNAS|150=F|151=0|14=100|10=0|";
    let message = codec
        .parse_line(line)
        .expect("a row")
        .next()
        .expect("one frame")
        .expect("a message");
    // The first message a registry enriches pays the registry's compile:
    // every derivation parsed, the working schema built and every term
    // bound against it, once, for every message and every shape after
    // (decision 38). Thousands of allocations, and the reason it is paid
    // per registry rather than per message or per shape.
    let (cold, enriched) = counted(|| codec.enrich_message(message.clone()).expect("enriches"));
    let (warm, _) = counted(|| codec.enrich_message(message.clone()).expect("enriches"));
    assert!(
        cold > 10 * warm,
        "the compile is paid by the first message: {cold} cold, {warm} warm"
    );
    // What a message costs after that, exactly: the clone (2, the row's
    // value list and the arrival record); restatement's rebuild of the row
    // at the dictionary's newest version and the working row of the 62
    // columns the derivations read or fill (44 together, what a settled
    // message pays below less its clone); and the derivations landing
    // (109): their evaluations, the landed list sized once, the writes
    // staged and the one rebuild that lands them through `set_each`, and
    // the `altids` Map with the rebuild that lands it. Nothing in it is a
    // parse, a bind or a shape: the terms were bound at compile and are
    // read borrowed from the registry.
    //
    // Nine more than before the crate grew `recordedat`, `expiredat` and
    // the two lane currencies: the wider working row and the evaluations
    // and writes those four add.
    assert_eq!(warm, 155, "a warm same-shaped message");
    let (thousand, _) = counted(|| {
        for _ in 0..1_000 {
            black_box(codec.enrich_message(message.clone()).expect("enriches"));
        }
    });
    assert_eq!(
        thousand,
        1_000 * warm,
        "a thousand same-shaped messages bind nothing and grow nothing"
    );
    // A stated value is never overwritten, so a second pass over the
    // enriched message derives nothing: the clone (2), restatement's rebuild
    // and the working row (44); the nine stated targets skip their terms,
    // and the landed list is never sized.
    black_box(
        codec
            .enrich_message(enriched.clone())
            .expect("a second pass"),
    );
    let (settled, _) = counted(|| {
        codec
            .enrich_message(enriched.clone())
            .expect("a second pass")
    });
    // Three more than before the crate grew its four new columns: the
    // working row is eight columns wider and gathering it costs that much,
    // while the derivations themselves still land nothing on a message that
    // already states everything.
    assert_eq!(
        settled, 49,
        "a settled message pays restatement, the working row and nothing per derivation"
    );
    // For scale: one clone, and one `set` on the same message - the clone
    // and one rebuild (16).
    let (clone, _) = counted(|| black_box(message.clone()));
    assert_eq!(clone, 2, "the value list and the arrival record");
    let (set_once, _) = counted(|| {
        let mut held = message.clone();
        held.set(59, Scalar::from("0")).expect("a write");
        black_box(held)
    });
    assert_eq!(set_once, 18, "one clone and one write");
    // A report stating two fields derives nothing but its `altids` Map:
    // every one of the 32 terms evaluates over columns mostly null, which
    // the evaluator pays in its own temporaries - a settled such report
    // costs 71, the clone, restatement, the working row and those
    // evaluations, 25 over the settled report above whose nine stated
    // targets skip their terms - and the Map and the rebuild that lands it
    // are the 19 on top of that.
    let identified = codec
        .parse_line(b"8=FIX.4.4|35=8|37=A|59=0|10=0|")
        .expect("a row")
        .next()
        .expect("one frame")
        .expect("a message");
    let (_, mapped) = counted(|| codec.enrich_message(identified.clone()).expect("enriches"));
    let (mapped_warm, _) = counted(|| codec.enrich_message(identified.clone()).expect("enriches"));
    black_box(codec.enrich_message(mapped.clone()).expect("a second pass"));
    let (mapped_settled, _) =
        counted(|| codec.enrich_message(mapped.clone()).expect("a second pass"));
    assert_eq!(mapped_settled, 74, "a settled two-field report");
    // The altids Map and the rebuild that lands it, and now also the four
    // columns the crate grew: a report that states a currency and a clock
    // lands `recordedat`, `expiredat` and both lane currencies on the warm
    // pass, and finds them stated on the settled one.
    assert_eq!(
        mapped_warm - mapped_settled,
        79,
        "the altids Map and the rebuild that lands it"
    );

    // The corpus: every shape a bridge writes - 95 messages of 54 distinct
    // root shapes, read under the bridge's own registry - enriched three
    // times through one codec. The first pass pays that registry's compile
    // and nothing else over the second; the second and the third cost the
    // same allocation for allocation, because nothing is bound or kept per
    // shape: each message pays its clone, its working row, its sweeps and
    // its rebuilds, whatever shape came before it (decision 38).
    let corpus = std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fix/ulbridge.log"),
    )
    .expect("the corpus");
    let plugged = Arc::new(
        FixRegistry::from_handle(&folder)
            .expect("loads")
            .with_plugin_fields()
            .expect("the bridge's own fields"),
    );
    let bridge = FixCodec::new(Arc::clone(&plugged));
    let messages: Vec<FixMsg> = bridge
        .parse_lines(&ulbridge_bodies(corpus))
        .filter_map(Result::ok)
        .collect();
    assert_eq!(messages.len(), 95, "the corpus");
    let pass = || {
        counted(|| {
            bridge
                .enrich_messages(messages.clone())
                .map(|message| message.expect("enriched").entries().len())
                .sum::<usize>()
        })
        .0
    };
    let (first, second, third) = (pass(), pass(), pass());
    assert_eq!(
        second, third,
        "a pass over every shape costs the same every time"
    );
    // Four more crate terms to compile, each with its sources bound against
    // the working schema, so the one-off compile is that much larger; it is
    // still paid exactly once, which is what the equality above pins.
    assert_eq!(
        first - second,
        13_760,
        "the first pass pays the bridge registry's compile and nothing else"
    );
    // 238 per message on average, the 451 allocations of cloning the 95
    // messages included: the clone, the working row, the sweeps and the
    // rebuilds of each, and the `Remembered` plugin memory of the stream.
    // Fourteen more per message than before the crate grew `recordedat`,
    // `expiredat` and the two lane currencies - a wider working row and
    // four more terms swept over it.
    assert_eq!(
        second, 22_642,
        "95 messages of 54 shapes, each its own working row"
    );
    let (clones, _) = counted(|| black_box(messages.clone()));
    assert_eq!(clones, 451, "what cloning the corpus costs of that");
}

#[test]
fn a_registry_whose_derivations_refuse_compiles_once_and_refuses_every_door() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let folder = yggdryl::holder::local::Folder::new(root).expect("the local seed path");
    let mut registry = FixRegistry::from_handle(&folder).expect("the committed dictionary loads");
    let mut gross = registry.field_by_tag(381).expect("GrossTradeAmt").clone();
    gross
        .as_fix_mut()
        .set_derivation(&"lastqty * nosuchfield".parse().expect("a term"))
        .expect("stored");
    registry.update(gross).expect("the text is a term");
    let registry = Arc::new(registry);
    let codec = FixCodec::new(Arc::clone(&registry));
    let schema = yggdryl::fix_schema(&registry, "fix").expect("the fixed schema");
    let message = codec
        .parse_line(b"8=FIX.4.4|35=8|37=A|48=US0378331005|22=4|100=XNAS|150=F|10=0|")
        .expect("a row")
        .next()
        .expect("one frame")
        .expect("a message");
    // The first ask pays the compile up to the refusal; the registry keeps
    // the refusal as it would keep the compiled list, so the second ask
    // pays the clone (2), restatement's rebuild of the parsed message (21)
    // and the refusal's two strings - both handles to text the registry
    // holds, so nothing - and nothing of a compile (decision 38).
    let (cold, refused) = counted(|| codec.enrich_message(message.clone()).expect_err("refused"));
    assert!(refused.to_string().contains("grosstradeamt"), "{refused}");
    let (warm, _) = counted(|| codec.enrich_message(message.clone()).expect_err("refused"));
    let (again, _) = counted(|| codec.enrich_message(message.clone()).expect_err("refused"));
    assert!(
        cold > 10 * warm,
        "the refused compile is paid once: {cold} cold, {warm} warm"
    );
    assert_eq!(warm, again, "the refusal is kept, not recompiled");
    assert_eq!(
        warm, 23,
        "a refused enrichment: the clone, restatement and the refusal"
    );
    // The row door refuses the same way and compiles nothing either: the
    // first fill pays the clone (2), the fixed schema's column plan (4,
    // resolved once per thread and kept), the row's values up to the first
    // crate column (1, one fewer now that the crate's own columns lead the
    // row and the refusing column is reached sooner) and the refusal; the
    // next fill finds the plan kept and pays 7.
    let (row, refused) = counted(|| message.clone().into_row(&schema).expect_err("refused"));
    assert!(refused.to_string().contains("grosstradeamt"), "{refused}");
    let (row_again, _) = counted(|| message.clone().into_row(&schema).expect_err("refused"));
    assert_eq!(row - row_again, 4, "the fixed schema's column plan, once");
    assert_eq!(row_again, 7, "a refused row fill reads the kept refusal");
}

/// The bodies the text reader hands the codec for the bridge's capture,
/// framed under the bridge's own row header exactly as the pipeline
/// benchmark reads them.
fn ulbridge_bodies(corpus: Vec<u8>) -> Vec<Vec<u8>> {
    use arrow_array::cast::AsArray;

    let source = Buffer::from_bytes(corpus).with_media_type(
        Url::from_str("file:///ulbridge.log")
            .expect("a URL")
            .media_type(),
    );
    let mut options = TextOptions::new()
        .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)
        .expect("the row header compiles")
        .with_timezone(Timezone::UTC);
    options.start_rownum = Some(1);
    options.parse_mimetype = true;
    let options: yggdryl::media::RecordOptions = options.into();
    let mut held = Vec::new();
    for batch in source.read_arrow_reader(&options).expect("a reader") {
        let batch = batch.expect("a batch");
        let at = batch.schema().index_of("body").expect("the body column");
        let column = batch.column(at).as_string::<i32>();
        for row in 0..batch.num_rows() {
            held.push(column.value(row).as_bytes().to_vec());
        }
    }
    held
}
