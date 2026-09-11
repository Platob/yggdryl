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

use yggdryl::media::text::{TextBytes, TextLine};
use yggdryl::types::{MsgDirection, UncheckedFieldScalar};
use yggdryl::{
    DataType, DataTypeId, Field, FieldRecord, FieldScalar, FixBranch, FixCode, FixCodec, FixId,
    FixLineageEntry, FixMsg, FixPedigree, FixRegistry, MediaType, MimeType, PythonKind,
    PythonMetadata, Scalar, TimeUnit, Timezone, Version,
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

/// A field carrying HTTP headers plus `extra` unrelated metadata keys.
///
/// The extra keys sort after every `http:` one, so they are what a read walks
/// past rather than something it stops at.
fn http_field(extra: usize) -> Field {
    let mut field = Field::from_parts(
        "payload",
        DataType::Binary,
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

/// The venue branch the branched cases probe, at exactly the length
/// `FixBranch::MAX_LENGTH` allows.
///
/// A branch at the bound must still be inline, because every identifier
/// probe clones one. If the bound ever exceeds `smol_str`'s inline capacity,
/// the cases below start allocating and fail here rather than silently
/// costing the documented hot path a heap round trip.
fn venue() -> FixBranch {
    FixBranch::from_str(&"v".repeat(FixBranch::MAX_LENGTH)).expect("a branch at the bound")
}

/// A FIX registry of `extra` generated fields around two fully keyed fields,
/// one in the standard branch and one in a venue's, plus one repeating group.
///
/// The generated fields are what a probe walks past in the maps; the keyed
/// ones are what every hit lands on. The group keeps a nested shape in the
/// same index corpus.
fn fix_registry(extra: usize) -> FixRegistry {
    let item = DataType::from_fields([DataType::Utf8.nullable_field("PartyID")])
        .expect("a struct item")
        .required_field("item");
    let mut parties = DataType::list(item).nullable_field("Parties");
    parties
        .as_fix_mut()
        .set_counter(453)
        .expect("a static counter");
    let mut counter = DataType::Int32.nullable_field("NoPartyIDs");
    counter.as_fix_mut().set_tag(453).expect("a static tag");
    let mut symbol = DataType::Utf8.nullable_field("Symbol");
    symbol.as_fix_mut().set_tag(55).expect("a static tag");
    symbol
        .as_fix_mut()
        .set_tags(&[65])
        .expect("a static alternate tag");
    symbol
        .as_fix_mut()
        .set_aliases(["Ticker", "SecuritySymbolIdentifier"])
        .expect("static aliases");
    let mut msgtype = DataType::Utf8.nullable_field("MsgType");
    msgtype.as_fix_mut().set_tag(35).expect("a static tag");
    let mut trade = DataType::Utf8.nullable_field("TradeID");
    trade
        .as_fix_mut()
        .set_id(&venue(), 5_001)
        .expect("a static identifier");
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
    let standard = FixBranch::STANDARD;
    let venue = venue();
    let vendor = FixId::from_parts(&venue, 5_001).expect("a vendor identifier");

    free("get_field_by_tag scalar hit", || {
        let _ = black_box(registry.get_field_by_tag(55));
    });
    free("get_field_by_tag counter hit", || {
        let _ = black_box(registry.get_field_by_tag(453));
    });
    free("get_field_by_name counter hit", || {
        let _ = black_box(registry.get_field_by_name("nopartyids", Some(&standard)));
    });
    free("get_definition group hit", || {
        let _ = black_box(registry.get_definition(
            yggdryl::FixCategory::Groups,
            "PARTIES",
            Some(&standard),
        ));
    });
    free("get_field_by_tag alternate hit", || {
        let _ = black_box(registry.get_field_by_tag(65));
    });
    free("get_field_by_tag miss", || {
        let _ = black_box(registry.get_field_by_tag(7));
    });
    // An identifier is the hash key itself, both halves, so vendor probes
    // cost what standard ones do.
    free("get_field_by_id vendor hit", || {
        let _ = black_box(registry.get_field_by_id(vendor));
    });
    free("get_field_by_id vendor miss", || {
        let _ = black_box(registry.get_field_by_id(FixId::standard(5_001)));
    });
    // The name index is probed with the caller's text folded as it is
    // hashed, so a differently cased query builds no folded copy.
    free("get_field_by_name differently cased hit", || {
        let _ = black_box(registry.get_field_by_name("sYmBoL", Some(&standard)));
    });
    free("get_field_by_name alias hit", || {
        let _ = black_box(registry.get_field_by_name("TICKER", Some(&standard)));
    });
    free("get_field_by_name long alias hit", || {
        let _ = black_box(registry.get_field_by_name("securitysymbolidentifier", Some(&standard)));
    });
    free("get_field_by_name vendor hit", || {
        let _ = black_box(registry.get_field_by_name("tradeid", Some(&venue)));
    });
    free("get_field_by_name inferred vendor hit", || {
        let _ = black_box(registry.get_field_by_name("tradeid", None));
    });
    free("get_field_by_name inferred standard hit", || {
        let _ = black_box(registry.get_field_by_name("ticker", None));
    });
    free("get_field_by_name miss", || {
        let _ = black_box(registry.get_field_by_name("absent", Some(&standard)));
    });
    free("get_field_by_name inferred miss", || {
        let _ = black_box(registry.get_field_by_name("absent", None));
    });
    free("get_field generic", || {
        let _ = black_box(registry.get_field("ticker"));
        let _ = black_box(registry.get_field(65));
        let _ = black_box(registry.get_field(vendor));
    });
    free("get_field_by_path member", || {
        let _ = black_box(registry.get_field_by_path("Symbol.absent", Some(&standard)));
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
    // bytes, so classifying a document costs no allocation either.
    const ULCONFIG: &[u8] = br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=X,plugin-type=FIX,type=Plugin","type":"read"},"value":{"Name":"X"},"status":200}"#;
    free("infer_bytes_protocol ULCONFIG", || {
        let _ = black_box(MimeType::infer_bytes(black_box(ULCONFIG)));
    });
    free("infer_bytes_msgtype ULCONFIG", || {
        let _ = black_box(FixCodec::infer_msgtype_bytes(black_box(ULCONFIG)));
    });
    free("infer_bytes_direction ULCONFIG", || {
        let _ = black_box(MsgDirection::infer_bytes(black_box(ULCONFIG)));
    });
    free("iter", || {
        let _ = black_box(registry.iter().count());
    });
}

#[test]
fn parsed_ulconfig_wildcards_iterate_without_allocating_results() {
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

        let (first_allocations, first) = counted(|| {
            yggdryl::UlPlugin::from_json_scalar(black_box(&document))
                .expect("validated wildcard response")
                .next()
                .expect("the wildcard has configurations")
        });
        assert_eq!(first.name(), Some("Configuration0000"));
        // The shared stable hash owns one XXH3 secret buffer; feeding the
        // selected configuration allocates nothing proportional to its siblings.
        costs("hashing one selected UL configuration", 1, || {
            black_box(first.stable_hash());
        });
        assert_eq!(
            first_allocations, 0,
            "first result for {size} configurations"
        );

        let (drain_allocations, read) = counted(|| {
            yggdryl::UlPlugin::from_json_scalar(black_box(&document))
                .expect("validated wildcard response")
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
fn registry_message_singletons_and_scoped_groups_are_borrowed() {
    let mut registry = fix_registry(512);
    let mut message = DataType::from_fields([
        registry.field_by_tag(453).unwrap().clone(),
        registry
            .definition(yggdryl::FixCategory::Groups, "Parties", None)
            .unwrap()
            .clone(),
    ])
    .unwrap()
    .required_field("newordersingle");
    message.as_fix_mut().set_msgtype("D").unwrap();
    registry
        .insert_definition(yggdryl::FixCategory::Messages, message)
        .unwrap();
    let held = registry.msgtype("D", None).unwrap();
    let counter = FixId::standard(453);
    assert_eq!(
        held.get_group_by_counter(counter).unwrap().name(),
        "Parties"
    );
    free("registry message singleton", || {
        black_box(registry.get_msgtype("D", None));
        black_box(registry.msgtypes().next());
    });
    free("borrowed message fields and scoped group", || {
        black_box(held.as_field());
        black_box(held.name());
        black_box(held.as_str());
        black_box(held.get_group_by_counter(counter));
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
            .create_definition(yggdryl::FixCategory::Messages, definition)
            .unwrap();
        let held = registry.msgtype("H", None).unwrap();
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
fn fix_field_code_metadata_and_category_cursors_allocate_nothing() {
    for size in [1, 32, 512] {
        let mut registry = FixRegistry::new();
        for index in 0..size {
            let mut field = DataType::Utf8.nullable_field(format!("Code{index}"));
            field.as_fix_mut().set_tag(index).unwrap();
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
        let field = registry.field(size - 1).unwrap();
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
    let mut field = DataType::Utf8.nullable_field("LastQty");
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
    let mut edited = DataType::Utf8.nullable_field("LastShares");
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
    let mut field = DataType::Utf8.nullable_field("Vocabulary");
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
    let venue = venue();
    let vendor = FixId::from_parts(&venue, 5_001).expect("a vendor identifier");
    let mut symbol = DataType::Utf8.nullable_field("Symbol");
    symbol.as_fix_mut().set_tag(55).expect("a static tag");
    let mut trade = DataType::Utf8.nullable_field("TradeID");
    trade
        .as_fix_mut()
        .set_id(&venue, 5_001)
        .expect("a static identifier");
    let mut root = DataType::from_fields([symbol, trade, DataType::Utf8.nullable_field("9999")])
        .expect("three children")
        .required_field("row");
    root.as_fix_mut()
        .set_branch(&venue)
        .expect("a venue message");
    let value = Scalar::from_sequence([
        Scalar::from("AAPL"),
        Scalar::from("T-1"),
        Scalar::from("custom"),
    ]);
    let msg = FixMsg::with_registry(registry, root, value).expect("a valid message");

    // The two-step tier: the message's own branch, then the standard one.
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
        let _ = black_box(msg.get_by_id(FixId::standard(5_001)));
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
    free("get_by_path", || {
        let _ = black_box(msg.get_by_path("Symbol.absent"));
    });
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
        DataType::Binary,
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
        Scalar::d256(yggdryl::I256::from_i128(i128::MIN), -3),
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
        Field::new("symbol", DataType::Utf8, false),
        Field::new("payload", DataType::Binary, false),
        Field::new("ccy", DataType::Currency, false),
        Field::new("venue", DataType::Ascii, false),
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
    for dtype in [DataType::LargeBinary, DataType::BinaryView] {
        let field = Field::new("payload", dtype, false);
        let rewritten = field.scalar(source.clone()).expect("the payload is bytes");
        assert_ne!(rewritten.id(), source.id());
        assert_eq!(address(&rewritten), from, "a rewrite copied the payload");
    }

    let text = Scalar::from("a symbol far longer than any inline string buffer can hold");
    let characters = |value: &Scalar| value.as_str().expect("the text is there").as_ptr();
    let from = characters(&text);
    for dtype in [DataType::LargeUtf8, DataType::Utf8View] {
        let field = Field::new("symbol", dtype, false);
        let rewritten = field.scalar(text.clone()).expect("the value is text");
        assert_ne!(rewritten.id(), text.id());
        assert_eq!(characters(&rewritten), from, "a rewrite copied the text");
    }
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
            DataType::Utf8.nullable_field("symbol"),
            DataType::Utf8.required_field("venue"),
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
    costs("projecting the whole value as a byte scalar", 1, || {
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
    let seeds: [(DataTypeId, Scalar); 33] = [
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
        (DataTypeId::Utf8, Scalar::from("AAPL")),
        (DataTypeId::LargeUtf8, Scalar::from("AAPL")),
        (DataTypeId::Utf8View, Scalar::from("AAPL")),
        (DataTypeId::Ascii, Scalar::from("AAPL")),
        (DataTypeId::Country, Scalar::from("US")),
        (DataTypeId::Currency, Scalar::from("USD")),
        (DataTypeId::Mic, Scalar::from("XNAS")),
        (DataTypeId::Cfi, Scalar::from("ESVUFR")),
        (DataTypeId::Isin, Scalar::from("US0378331005")),
        (DataTypeId::Side, Scalar::from("1")),
        (DataTypeId::State, Scalar::from("20NEW")),
        (DataTypeId::TimeInForce, Scalar::from("0")),
        (DataTypeId::MsgDirection, Scalar::from(MsgDirection::SENT)),
        (
            DataTypeId::Uuid,
            Scalar::from("123e4567-e89b-12d3-a456-426614174000"),
        ),
        (DataTypeId::Version, Scalar::from("5.0.2")),
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
    // A parameterized leaf is interned on its first ask and borrowed after.
    for dtype in [
        DataType::decimal128(10, 2).expect("a valid decimal"),
        DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC).expect("a valid instant"),
        DataType::FixedAscii(4),
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
    let text = Field::new("symbol", DataType::Utf8, false);
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
/// Three widths rather than one, because the interesting number is not the
/// total but how it grows: fifteen allocations for twelve more pairs and fifty
/// for forty-eight more, which is the same growth these three widths measured
/// before the entries became ranges of the line's own page - 22, 37 and 87.
/// Every one of the five that were added is per *message*, and the per-pair
/// cost did not move at all.
///
/// That is worth stating plainly, because the change was expected to save two
/// allocations a pair and did not. It could not: this line's keys are four
/// bytes and its values one or two, so both halves fitted `SmolStr`'s inline
/// buffer and the copies the entry stopped making were never allocations at
/// these widths. Where they were is a value too wide for that buffer, and
/// that is pinned next door, in
/// [`a_wide_value_costs_a_message_what_a_narrow_one_does`]: the same pair
/// counts with three-kilobyte values cost exactly what two-byte values cost,
/// which an owned copy could not have managed.
///
/// The five are the page the line is copied into - two allocations, the
/// vector and the shared box around it - and the lists the two-stage read
/// holds, what the line said and what the build folds in. The page is what a
/// message owning its bytes costs when it is handed a borrowed slice, which
/// is all this door can be handed.
///
/// A caller who decoded the line already owns that page, and
/// [`FIX_TEXT_LINE_COSTS`] is the same three widths through the door that
/// takes it: two fewer at each, which is the page and nothing else.
const FIX_LINE_COSTS: [(usize, usize); 3] = [(4, 27), (16, 42), (64, 92)];

/// A dictionary of `count` `Utf8` fields, tagged from 2000.
///
/// Text rather than integers, because what is measured next door is a
/// *value's width*: a field that only types a number never carries three
/// kilobytes, so a numeric dictionary could not state the case at all.
fn fix_text_registry(count: usize) -> FixRegistry {
    let mut msgtype = DataType::Utf8.nullable_field("MsgType");
    msgtype.as_fix_mut().set_tag(35).expect("a static tag");
    let generated = (0..count).map(|index| {
        let mut field = DataType::Utf8.nullable_field(format!("Text{index:04}"));
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
/// Two pair counts and two widths, because one of each could tell neither a
/// per-message cost from a per-pair one nor a cost that scales with a value
/// from one that does not.
const WIDE_VALUE_COSTS: [(usize, (usize, usize)); 2] = [(4, (27, 30)), (16, (42, 57))];

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
        let mut field = DataType::Utf8.nullable_field(format!("Member{index:04}"));
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
    let mut msgtype = DataType::Utf8.nullable_field("MsgType");
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
/// and the rest of it is the row a wider group builds.
const PACKED_MEMBER_COSTS: [(usize, usize); 2] = [(4, 61), (16, 95)];

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
const FIX_TEXT_LINE_COSTS: [(usize, usize); 3] = [(4, 25), (16, 40), (64, 90)];

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
        let line = TextLine::new(0, TextBytes::from_bytes(&held).expect("a page"));
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
