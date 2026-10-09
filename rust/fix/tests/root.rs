//! One test file per file at the crate root, under `tests/root/`.
//!
//! `rust/fix/tests/` mirrors `rust/fix/src/`: a source file has exactly one
//! test file at the matching path, and this target is the harness for the
//! files the crate root itself holds. A test reaches this crate through
//! `yggdryl_fix::`, the market crate through `yggdryl_market::` and the core
//! through `yggdryl::`; where what it pins is not reachable that way, it
//! reaches `yggdryl_fix::internals`, which exists only under the `internals`
//! feature, and the file that reaches it is declared behind that feature
//! here.

#[path = "support/install.rs"]
mod install;
use yggdryl_fix::FixField;

#[path = "support/allocations.rs"]
mod allocations;

/// The instrument's classification, at the seam between its value and FIX.
/// The FIX module's own edge cases, driven with explicit inputs.
/// The threads a codec reads on.
#[global_allocator]
static ALLOCATOR: allocations::CountingAllocator = allocations::CountingAllocator;

/// What a reader warned about while it ran, on this thread alone.
///
/// A CBlock is read best-effort, so what it drops is a warning rather than a
/// return value and the tests that pin a drop have to read the log. `log` is
/// process-global and this suite is threaded, so the records are buffered per
/// thread and every other thread's are ignored - which is what lets a warning
/// be asserted without the isolation a process-global fixture needs.
mod warned {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, Once, PoisonError};

    thread_local! {
        static HELD: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
    }

    /// One watch over every thread: its number, the site it reads and the
    /// subject it names, beside what it caught.
    type Watch = (u64, &'static str, String, Vec<String>);

    /// The watches open.
    static WATCHED: Mutex<Vec<Watch>> = Mutex::new(Vec::new());

    struct Sink;

    impl log::Log for Sink {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.level() <= log::Level::Warn
        }

        fn log(&self, record: &log::Record<'_>) {
            if !self.enabled(record.metadata()) {
                return;
            }
            HELD.with_borrow_mut(|held| {
                if let Some(held) = held.as_mut() {
                    held.push(record.args().to_string());
                }
            });
            let mut watched = WATCHED.lock().unwrap_or_else(PoisonError::into_inner);
            for (_, site, subject, caught) in watched.iter_mut() {
                // The site first, so a record no watch reads builds no text.
                if record.target() == *site {
                    let text = record.args().to_string();
                    if text.contains(subject.as_str()) {
                        caught.push(text);
                    }
                }
            }
        }

        fn flush(&self) {}
    }

    static SINK: Sink = Sink;
    static INSTALLED: Once = Once::new();

    fn install() {
        INSTALLED.call_once(|| {
            // Another logger may already own the process; the buffer is then
            // empty and the assertions say so rather than the install failing.
            drop(log::set_logger(&SINK));
            log::set_max_level(log::LevelFilter::Warn);
        });
    }

    /// Runs `body`, answering every warning `site` - a module path - raised
    /// about `subject` on any thread while it ran.
    ///
    /// A pool's workers warn on their own threads, which [`during_all`]
    /// does not read; a subject no other test names keeps another test's
    /// warnings out.
    pub fn naming<T>(
        site: &'static str,
        subject: &str,
        body: impl FnOnce() -> T,
    ) -> (T, Vec<String>) {
        static WATCHES: AtomicU64 = AtomicU64::new(0);
        install();
        let watch = WATCHES.fetch_add(1, Ordering::Relaxed);
        WATCHED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((watch, site, subject.to_owned(), Vec::new()));
        let answered = body();
        let mut watched = WATCHED.lock().unwrap_or_else(PoisonError::into_inner);
        let at = watched
            .iter()
            .position(|(held, ..)| *held == watch)
            .expect("the watch this call opened");
        (answered, watched.swap_remove(at).3)
    }

    /// Runs `body`, answering everything it warned about beside what it
    /// answered.
    pub fn during_all<T>(body: impl FnOnce() -> T) -> (T, Vec<String>) {
        install();
        HELD.with_borrow_mut(|held| *held = Some(Vec::new()));
        let answered = body();
        let warnings = HELD.with_borrow_mut(Option::take).unwrap_or_default();
        (answered, warnings)
    }

    /// Runs `body`, answering what the thing it read warned about.
    ///
    /// What a registry's construction warns about is one fact, pinned by
    /// `digest::every_registry_registers_the_crates_fields_without_warning`;
    /// a test reading a CBlock or a store asserts that reading's warnings
    /// alone, so the construction's are left out here.
    pub fn during<T>(body: impl FnOnce() -> T) -> (T, Vec<String>) {
        let (answered, warnings) = during_all(body);
        let warnings = warnings
            .into_iter()
            .filter(|warning| !warning.starts_with("registering FIX crate definition"))
            .collect();
        (answered, warnings)
    }
}

/// Immutable seed fixtures share parsing and compiled plans within this binary.
/// One path, resolved once, as every FIX navigator now takes it.
///
/// A position is written the way the one grammar writes it -
/// `Parties[0].PartyID` - and reaches the same member through a message and
/// through the registry that declares it.
fn path(spelling: &str) -> yggdryl::FieldPath {
    yggdryl::FieldPath::from_str(spelling).unwrap_or_else(|error| panic!("{spelling}: {error}"))
}

fn committed_registry() -> std::sync::Arc<yggdryl_fix::FixRegistry> {
    static REGISTRY: std::sync::OnceLock<std::sync::Arc<yggdryl_fix::FixRegistry>> =
        std::sync::OnceLock::new();
    std::sync::Arc::clone(REGISTRY.get_or_init(|| {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config/fix");
        let folder = yggdryl::local::LocalFolder::new(root).expect("the local seed path");
        std::sync::Arc::new(
            yggdryl_fix::FixRegistry::from_handle(&folder).expect("the committed dictionary loads"),
        )
    }))
}

/// Undated test bytes have one explicit intake clock; replay never consults now.
fn fixed_codec(registry: std::sync::Arc<yggdryl_fix::FixRegistry>) -> yggdryl_fix::FixCodec {
    yggdryl_fix::FixCodec::new(registry)
        // Allocation and warning pins observe this thread. Pool behavior has
        // its own explicit multi-worker fixtures in `parallel`.
        .with_threads(1)
        .try_with_default_sending_time(Some(
            yggdryl::Scalar::datetime64(
                1_704_190_530_000_000_000,
                yggdryl::TimeUnit::Nanosecond,
                yggdryl::Timezone::UTC,
            )
            .unwrap(),
        ))
        .unwrap()
}

/// One line read at a version the row itself states.
///
/// A codec pins no version: the two ranks are what the row states and what
/// the line implies, so a fixture that wants a version of its own says it
/// the way a bridge log says it - in the `beginstring` the transport wrote
/// around the body.
fn dated_line(
    codec: &yggdryl_fix::FixCodec,
    body: &[u8],
    version: &str,
) -> yggdryl::Result<yggdryl_fix::FixMsg> {
    use yggdryl::text::{TextBytes, TextLine};

    let codec = codec.clone().with_capture_names(["beginstring"]);
    let line = TextLine::from_bytes(
        0,
        TextBytes::from_bytes(body)?,
        std::sync::Arc::new(yggdryl::text::TextOptions::new()),
    )?
    .with_captures(vec![Some(TextBytes::from_bytes(version.as_bytes())?)])?;
    sole_message(codec.parse_text_line(&line)?)
}

/// The sole message a fixture carrying one is read into.
///
/// A row yields none, one or many, so a fixture that carries
/// exactly one says so here: what the assertions below are about is that one
/// message, and a fixture that grew a second would otherwise be read as its
/// first with nobody noticing. What the parse splits off that message (an
/// execution, a trade's sided executions, a batch's entries) follows it,
/// each naming it as its source, and is not a second message of the
/// fixture.
trait SoleMessage {
    fn sole_line(&self, row: &[u8]) -> yggdryl::Result<yggdryl_fix::FixMsg>;
}

fn sole_message(
    mut messages: impl Iterator<Item = yggdryl::Result<yggdryl_fix::FixMsg>>,
) -> yggdryl::Result<yggdryl_fix::FixMsg> {
    use yggdryl::graph::Element;
    let message = messages
        .next()
        .expect("a fixture of one message yields it")?;
    for split in messages {
        assert!(
            split?.get_srcuuids().contains(&message.get_uuid()),
            "a fixture of one message yields exactly one, and what it splits into"
        );
    }
    Ok(message)
}

impl SoleMessage for yggdryl_fix::FixCodec {
    fn sole_line(&self, row: &[u8]) -> yggdryl::Result<yggdryl_fix::FixMsg> {
        sole_message(self.parse_line(row)?)
    }
}

const ISOLATED_FIX_TEST: &str = "YGGDRYL_ISOLATED_FIX_TEST";

/// Run a process-global case in a child containing only that selected test.
fn run_isolated(test_name: &str, marker: &str) -> bool {
    if std::env::var(ISOLATED_FIX_TEST).as_deref() == Ok(marker) {
        return false;
    }
    let output = std::process::Command::new(std::env::current_exe().expect("the FIX test binary"))
        .args(["--exact", test_name, "--nocapture"])
        .env(ISOLATED_FIX_TEST, marker)
        .output()
        .expect("the isolated FIX test must start");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "isolated FIX test {test_name} failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // A child that matched no test exits successfully having run nothing; the
    // name must reach exactly one test, or the body never ran.
    assert!(
        stdout.contains("test result: ok. 1 passed"),
        "isolated FIX test {test_name} ran no test (the name matches none):\n{stdout}"
    );
    true
}

/// Crate-owned scalar definitions inherited by every dictionary: every one
/// but the columns the fixed row derives, which no registry holds.
fn crated_fields() -> usize {
    yggdryl_fix::fix_crate_fields()
        .expect("the crate's own fields")
        .iter()
        .filter(|field| category_of(field) == yggdryl_fix::FixCategory::Fields)
        .filter(|field| {
            !FixField::new(field)
                .tag()
                .ok()
                .flatten()
                .is_some_and(yggdryl_fix::is_derived_tag)
        })
        .count()
}

/// The named components this crate defines beside the dictionary's own.
///
/// A definition is filed by the shape it has, so every crate column shaped as
/// a Struct is a component: `instids` is one, and it answers to its crate tag
/// rather than to a derived one.
fn crated_components() -> usize {
    yggdryl_fix::fix_crate_fields()
        .expect("the crate's own fields")
        .iter()
        .filter(|field| matches!(field.dtype(), yggdryl::DataType::Struct(_)))
        .count()
}

/// The scalar fields a registry holds: what a dictionary's own fields add
/// to, beside the components and groups `len` counts with them.
fn scalars(registry: &yggdryl_fix::FixRegistry) -> usize {
    registry
        .iter()
        .filter(|field| category_of(field) == yggdryl_fix::FixCategory::Fields)
        .count()
}

/// The occurrences a value holds, owned.
///
/// A lookup answers a value rather than a borrow of one, so a caller that
/// walks a group's occurrences owns them: this is that walk, spelled once.
#[track_caller]
fn sequence(value: yggdryl::Scalar) -> Vec<yggdryl::Scalar> {
    value.as_sequence().expect("a sequence").to_vec()
}

/// The item field a serie column repeats.
#[track_caller]
fn item_of(list: &yggdryl::Field) -> yggdryl::Field {
    list.dtype()
        .as_serie_type()
        .expect("a serie column")
        .item()
        .clone()
}

/// A canonical row with the serie cell at `at` restated as the column of its
/// rows under `item`, every other cell kept: the cell a row read out of
/// Arrow holds where the crate's own build holds a run.
#[track_caller]
fn with_column_at(row: &yggdryl::Scalar, at: usize, item: &yggdryl::Field) -> yggdryl::Scalar {
    let cells = row.as_sequence().expect("a canonical row");
    let rows = cells[at]
        .sequence_rows()
        .expect("a serie cell")
        .into_owned();
    let column = yggdryl::Scalar::from(
        yggdryl::Serie::from_scalars(item.clone(), rows).expect("the rows fit their item"),
    );
    assert_eq!(column.as_sequence(), None, "the restated cell is a column");
    yggdryl::Scalar::from_sequence(cells.iter().enumerate().map(|(index, cell)| {
        if index == at {
            column.clone()
        } else {
            cell.clone()
        }
    }))
}

/// A parsed message as the root and row [`yggdryl_fix::FixMsg::with_registry`]
/// rebuilds it from, the header facts `tags` name stated ahead of the
/// content: the content row carries none of them, because a typed fact
/// leaves the row for the holder that owns it.
#[track_caller]
fn restatable(
    registry: &yggdryl_fix::FixRegistry,
    message: &yggdryl_fix::FixMsg,
    tags: &[i32],
) -> (yggdryl::Field, yggdryl::Scalar) {
    let content = message.as_value().as_sequence().expect("a canonical row");
    let root = yggdryl::StructType::from_fields(
        tags.iter()
            .map(|tag| registry.field_by_tag(*tag).expect("a header field").clone())
            .chain(message.as_field().fields().iter().cloned()),
    )
    .map(yggdryl::DataType::from)
    .expect("a root")
    .required_field(message.as_field().name());
    let row = yggdryl::Scalar::from_sequence(
        tags.iter()
            .map(|tag| message.by_tag(*tag).expect("a header fact"))
            .chain(content.iter().cloned()),
    );
    (root, row)
}

/// Whether the message holds the group at `name` as a column: the fixture
/// proving canonicalization kept the column it was given.
#[track_caller]
fn holds_column(message: &yggdryl_fix::FixMsg, name: &str) -> bool {
    let at = message
        .as_field()
        .index_of(name)
        .expect("the group's column");
    let cell = &message.as_value().as_sequence().expect("a canonical row")[at];
    cell.as_serie().is_some() && cell.as_sequence().is_none()
}

/// Which category a registry field is filed under: a definition is filed by
/// the shape it has - a Struct is a component, a Serie or a Map a group - and
/// everything else is a wire field.
///
/// One nested shape is a field rather than a definition: a serie of non-null
/// scalars under one of this crate's own tags is one column under one name -
/// `srcuuids` is that - because a group's occurrence is a
/// Struct of members a wire states one tag at a time.
fn category_of(field: &yggdryl::Field) -> yggdryl_fix::FixCategory {
    match field.dtype() {
        yggdryl::DataType::Struct(_) => yggdryl_fix::FixCategory::Components,
        yggdryl::DataType::Serie(item) | yggdryl::DataType::LargeSerie(item)
            if !item.is_nullable() && !item.dtype().is_nested() =>
        {
            yggdryl_fix::FixCategory::Fields
        }
        dtype if dtype.is_nested() => yggdryl_fix::FixCategory::Groups,
        _ => yggdryl_fix::FixCategory::Fields,
    }
}

/// The registry's fields of one category, in the one iteration order.
fn definitions(
    registry: &yggdryl_fix::FixRegistry,
    category: yggdryl_fix::FixCategory,
) -> impl Iterator<Item = &yggdryl::Field> {
    registry
        .iter()
        .filter(move |field| category_of(field) == category)
}

/// The message components a registry holds: a component naming a wire code.
fn msgtypes(registry: &yggdryl_fix::FixRegistry) -> impl Iterator<Item = &yggdryl::Field> {
    registry
        .iter()
        .filter(|field| FixField::new(field).msgtype().is_some())
}

/// Initial registry scalar fields: the crate's own and the standard
/// SendingTime and TransactTime seeds.
fn seeded_fields() -> usize {
    static COUNT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *COUNT.get_or_init(|| scalars(&yggdryl_fix::FixRegistry::new()))
}

/// Where the column carrying `tag` sits in a batch: by the tag its field
/// carries, never by its spelling.
fn tag_index(batch: &arrow_array::RecordBatch, tag: i32) -> usize {
    let schema =
        yggdryl::Field::from_arrow_schema("row", &batch.schema()).expect("the batch schema reads");
    yggdryl_fix::fix_column_of(&schema, tag).unwrap_or_else(|| panic!("a column for tag {tag}"))
}

/// The format target a suite reads a whole capture under.
///
/// The shape a consumer reads: what a parse lands in is the
/// [stable row](yggdryl_fix::fix_schema), and a
/// [format](yggdryl_fix::FixCodec::format_arrow_reader) into that same row keeps
/// every column the capture landed in.
fn format_target(registry: &yggdryl_fix::FixRegistry) -> yggdryl::Field {
    yggdryl_fix::fix_schema(registry, "fix").expect("the fixed row")
}

/// One exact number, spelled the way a wire spells it.
///
/// Every FIX quantity, price, price offset and amount is
/// `decimal128(38, 18)`, so a pin states the number in text and never as a
/// float: `41.25` is a value a `f64` cannot hold and a decimal can.
fn decimal(text: &str) -> yggdryl::Scalar {
    yggdryl::Scalar::from(yggdryl::Decimal::parse(text).expect("an exact number"))
}

#[path = "root/alias_rule.rs"]
mod alias_rule;
#[path = "root/aliases.rs"]
mod aliases;
#[path = "root/anomaly.rs"]
mod anomaly;
#[path = "root/batch.rs"]
mod batch;
#[path = "root/build.rs"]
mod build;
#[cfg(feature = "internals")]
#[path = "root/catalog.rs"]
mod catalog;
#[path = "root/cfb.rs"]
mod cfb;
#[path = "root/cfi.rs"]
mod cfi;
#[path = "root/codec.rs"]
mod codec;
#[path = "root/codes.rs"]
mod codes;
#[path = "root/component.rs"]
mod component;
#[path = "root/crated.rs"]
mod crated;
#[path = "root/digest.rs"]
mod digest;
#[path = "root/direction.rs"]
mod direction;
#[path = "root/document.rs"]
mod document;
#[path = "root/enrich.rs"]
mod enrich;
#[path = "root/entry.rs"]
mod entry;
#[path = "root/field.rs"]
mod field;
#[path = "root/forex.rs"]
mod forex;
#[path = "root/global.rs"]
mod global;
#[cfg(feature = "internals")]
#[path = "root/group_plan.rs"]
mod group_plan;
#[path = "root/identity.rs"]
mod identity;
#[path = "root/idmap.rs"]
mod idmap;
#[path = "root/latest.rs"]
mod latest;
#[path = "root/lib.rs"]
mod lib;
#[path = "root/market.rs"]
mod market;
#[cfg(feature = "internals")]
#[path = "root/memo.rs"]
mod memo;
#[path = "root/messages.rs"]
mod messages;
#[path = "root/msg.rs"]
mod msg;
#[path = "root/msgtype.rs"]
mod msgtype;
#[path = "root/registry.rs"]
mod registry;
#[cfg(feature = "internals")]
#[path = "root/retired.rs"]
mod retired;
#[path = "root/schema.rs"]
mod schema;
#[path = "root/securityids.rs"]
mod securityids;
#[path = "root/source.rs"]
mod source;
#[path = "root/state.rs"]
mod state;
#[path = "root/store.rs"]
mod store;
#[path = "root/ulbridge.rs"]
mod ulbridge;
