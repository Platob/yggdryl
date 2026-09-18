//! FIX integration tests.

#[path = "fix/batch.rs"]
mod batch;
#[path = "fix/capture.rs"]
mod capture;
#[path = "fix/cfb.rs"]
mod cfb;
/// The instrument's classification, at the seam between its value and FIX.
#[path = "fix/classification.rs"]
mod classification;
#[path = "fix/codec.rs"]
mod codec;
#[path = "fix/dataset.rs"]
mod dataset;
#[path = "fix/dictionary.rs"]
mod dictionary;
#[path = "fix/digest.rs"]
mod digest;
#[path = "fix/direction.rs"]
mod direction;
#[path = "fix/enrich.rs"]
mod enrich;
#[path = "fix/equivalence.rs"]
mod equivalence;
#[path = "fix/format.rs"]
mod format;
#[path = "fix/global_env.rs"]
mod global_env;
#[path = "fix/global_home.rs"]
mod global_home;
#[path = "fix/global_install.rs"]
mod global_install;
#[path = "fix/identifiers.rs"]
mod identifiers;
#[path = "fix/latest.rs"]
mod latest;
#[path = "fix/map_groups.rs"]
mod map_groups;
#[path = "fix/merge.rs"]
mod merge;
#[path = "fix/message.rs"]
mod message;
#[path = "fix/pipeline.rs"]
mod pipeline;
#[path = "fix/schema.rs"]
mod schema;
#[path = "fix/store.rs"]
mod store;
#[path = "fix/zero_entries.rs"]
mod zero_entries;

/// What a reader warned about while it ran, on this thread alone.
///
/// A CBlock is read best-effort, so what it drops is a warning rather than a
/// return value and the tests that pin a drop have to read the log. `log` is
/// process-global and this suite is threaded, so the records are buffered per
/// thread and every other thread's are ignored - which is what lets a warning
/// be asserted without the isolation a process-global fixture needs.
mod warned {
    use std::cell::RefCell;
    use std::sync::Once;

    thread_local! {
        static HELD: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
    }

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
        }

        fn flush(&self) {}
    }

    static SINK: Sink = Sink;
    static INSTALLED: Once = Once::new();

    /// Runs `body`, answering everything it warned about beside what it
    /// answered.
    pub fn during_all<T>(body: impl FnOnce() -> T) -> (T, Vec<String>) {
        INSTALLED.call_once(|| {
            // Another logger may already own the process; the buffer is then
            // empty and the assertions say so rather than the install failing.
            drop(log::set_logger(&SINK));
            log::set_max_level(log::LevelFilter::Warn);
        });
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

fn committed_registry() -> std::sync::Arc<yggdryl::FixRegistry> {
    static REGISTRY: std::sync::OnceLock<std::sync::Arc<yggdryl::FixRegistry>> =
        std::sync::OnceLock::new();
    std::sync::Arc::clone(REGISTRY.get_or_init(|| {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
        let folder = yggdryl::holder::local::Folder::new(root).expect("the local seed path");
        std::sync::Arc::new(
            yggdryl::FixRegistry::from_handle(&folder).expect("the committed dictionary loads"),
        )
    }))
}

/// Undated test bytes have one explicit intake clock; replay never consults now.
fn fixed_codec(registry: std::sync::Arc<yggdryl::FixRegistry>) -> yggdryl::FixCodec {
    yggdryl::FixCodec::new(registry)
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
    codec: &yggdryl::FixCodec,
    body: &[u8],
    version: &str,
) -> yggdryl::Result<yggdryl::FixMsg> {
    use yggdryl::media::text::{TextBytes, TextLine};

    let codec = codec.clone().with_capture_names(["beginstring"]);
    let line = TextLine::from_bytes(0, TextBytes::from_bytes(body)?)?
        .with_captures(vec![Some(TextBytes::from_bytes(version.as_bytes())?)])?;
    sole_message(codec.parse_text_line(&line)?)
}

/// The sole message a fixture carrying one is read into.
///
/// A row yields none, one or many, so a fixture that carries
/// exactly one says so here: what the assertions below are about is that one
/// message, and a fixture that grew a second would otherwise be read as its
/// first with nobody noticing.
trait SoleMessage {
    fn sole_line(&self, row: &[u8]) -> yggdryl::Result<yggdryl::FixMsg>;
}

fn sole_message(
    mut messages: impl Iterator<Item = yggdryl::Result<yggdryl::FixMsg>>,
) -> yggdryl::Result<yggdryl::FixMsg> {
    let message = messages
        .next()
        .expect("a fixture of one message yields it")?;
    assert!(
        messages.next().is_none(),
        "a fixture of one message yields exactly one"
    );
    Ok(message)
}

impl SoleMessage for yggdryl::FixCodec {
    fn sole_line(&self, row: &[u8]) -> yggdryl::Result<yggdryl::FixMsg> {
        sole_message(self.parse_line(row)?)
    }
}

const ISOLATED_FIX_TEST: &str = "YGGDRYL_ISOLATED_FIX_TEST";

/// Run a process-global case in a child containing only that selected test.
fn run_isolated(test_name: &str, marker: &str) -> bool {
    if std::env::var(ISOLATED_FIX_TEST).as_deref() == Ok(marker) {
        return false;
    }
    let status = std::process::Command::new(std::env::current_exe().expect("the FIX test binary"))
        .args(["--exact", test_name, "--nocapture"])
        .env(ISOLATED_FIX_TEST, marker)
        .status()
        .expect("the isolated FIX test must start");
    assert!(status.success(), "isolated FIX test {test_name} failed");
    true
}

/// Crate-owned scalar definitions inherited by every dictionary.
fn crated_fields() -> usize {
    yggdryl::fix_crate_fields()
        .expect("the crate's own fields")
        .iter()
        .filter(|field| category_of(field) == yggdryl::FixCategory::Fields)
        .count()
}

/// The named components this crate defines beside the dictionary's own.
///
/// A definition is filed by the shape it has, so every crate column shaped as
/// a Struct is a component: `instids` is one, and it answers to its crate tag
/// rather than to a derived one.
fn crated_components() -> usize {
    yggdryl::fix_crate_fields()
        .expect("the crate's own fields")
        .iter()
        .filter(|field| matches!(field.dtype(), yggdryl::DataType::Structure(_)))
        .count()
}

/// The scalar fields a registry holds: what a dictionary's own fields add
/// to, beside the components and groups `len` counts with them.
fn scalars(registry: &yggdryl::FixRegistry) -> usize {
    registry
        .iter()
        .filter(|field| category_of(field) == yggdryl::FixCategory::Fields)
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

/// Which category a registry field is filed under: a definition is filed by
/// the shape it has - a Struct is a component, a List or a Map a group - and
/// everything else is a wire field.
///
/// One nested shape is a field rather than a definition: a list of non-null
/// scalars under one of this crate's own tags is one column under one name -
/// `parentuuids` is that - because a group's occurrence is a Struct of
/// members a wire states one tag at a time.
fn category_of(field: &yggdryl::Field) -> yggdryl::FixCategory {
    match field.dtype() {
        yggdryl::DataType::Structure(_) => yggdryl::FixCategory::Components,
        yggdryl::DataType::Sequence(yggdryl::types::SequenceType::List(item))
        | yggdryl::DataType::Sequence(yggdryl::types::SequenceType::LargeList(item))
            if !item.is_nullable() && !item.dtype().is_nested() =>
        {
            yggdryl::FixCategory::Fields
        }
        dtype if dtype.is_nested() => yggdryl::FixCategory::Groups,
        _ => yggdryl::FixCategory::Fields,
    }
}

/// The registry's fields of one category, in the one iteration order.
fn definitions(
    registry: &yggdryl::FixRegistry,
    category: yggdryl::FixCategory,
) -> impl Iterator<Item = &yggdryl::Field> {
    registry
        .iter()
        .filter(move |field| category_of(field) == category)
}

/// The message components a registry holds: a component naming a wire code.
fn msgtypes(registry: &yggdryl::FixRegistry) -> impl Iterator<Item = &yggdryl::Field> {
    registry
        .iter()
        .filter(|field| field.as_fix().msgtype().is_some())
}

/// Initial registry scalar fields: the crate's own and the standard
/// SendingTime and TransactTime seeds.
fn seeded_fields() -> usize {
    static COUNT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *COUNT.get_or_init(|| scalars(&yggdryl::FixRegistry::new()))
}

/// Where the column carrying `tag` sits in a batch: by the tag its field
/// carries, never by its spelling.
fn tag_index(batch: &arrow_array::RecordBatch, tag: i32) -> usize {
    let schema =
        yggdryl::Field::from_arrow_schema("row", &batch.schema()).expect("the batch schema reads");
    yggdryl::fix_column_of(&schema, tag).unwrap_or_else(|| panic!("a column for tag {tag}"))
}

/// The format target a suite reads a whole capture under.
///
/// The shape a consumer reads: what a parse lands in is the
/// [stable row](yggdryl::fix_schema), and a
/// [format](yggdryl::FixCodec::format_arrow_reader) into that same row keeps
/// every column the capture landed in.
fn format_target(registry: &yggdryl::FixRegistry) -> yggdryl::Field {
    yggdryl::fix_schema(registry, "fix").expect("the fixed row")
}

/// One exact number, spelled the way a wire spells it.
///
/// Every FIX quantity, price, price offset and amount is
/// `decimal128(38, 18)`, so a pin states the number in text and never as a
/// float: `41.25` is a value a `f64` cannot hold and a decimal can.
fn decimal(text: &str) -> yggdryl::Scalar {
    yggdryl::Scalar::from(yggdryl::Decimal::parse(text).expect("an exact number"))
}
