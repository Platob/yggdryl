//! FIX integration tests.

#[path = "fix/batch.rs"]
mod batch;
#[path = "fix/capture.rs"]
mod capture;
#[path = "fix/cfb.rs"]
mod cfb;
#[path = "fix/codec.rs"]
mod codec;
#[path = "fix/content_identity.rs"]
mod content_identity;
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
#[path = "fix/global_env.rs"]
mod global_env;
#[path = "fix/global_home.rs"]
mod global_home;
#[path = "fix/global_install.rs"]
mod global_install;
#[path = "fix/identifier_dictionary.rs"]
mod identifier_dictionary;
#[path = "fix/identifiers.rs"]
mod identifiers;
#[path = "fix/latest.rs"]
mod latest;
#[path = "fix/lifecycle.rs"]
mod lifecycle;
#[path = "fix/lifecycle_chains.rs"]
mod lifecycle_chains;
#[path = "fix/lifecycle_grid.rs"]
mod lifecycle_grid;
#[path = "fix/lifecycle_previous.rs"]
mod lifecycle_previous;
#[path = "fix/lifecycle_targets.rs"]
mod lifecycle_targets;
#[path = "fix/lift.rs"]
mod lift;
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

    /// Runs `body`, answering what it warned about beside what it answered.
    pub fn during<T>(body: impl FnOnce() -> T) -> (T, Vec<String>) {
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

fn plugin_fields_registry() -> std::sync::Arc<yggdryl::FixRegistry> {
    static REGISTRY: std::sync::OnceLock<std::sync::Arc<yggdryl::FixRegistry>> =
        std::sync::OnceLock::new();
    std::sync::Arc::clone(REGISTRY.get_or_init(|| {
        std::sync::Arc::new(
            committed_registry()
                .as_ref()
                .clone()
                .with_plugin_fields()
                .expect("the bridge's own fields"),
        )
    }))
}

/// The sole message a fixture carrying one is read into.
///
/// A row yields none, one or many (decision 16), so a fixture that carries
/// exactly one says so here: what the assertions below are about is that one
/// message, and a fixture that grew a second would otherwise be read as its
/// first with nobody noticing.
trait SoleMessage {
    fn sole_line(&self, row: &[u8], enrich: bool) -> yggdryl::Result<yggdryl::FixMsg>;
    fn sole_plugin_line(&self, row: &[u8], enrich: bool) -> yggdryl::Result<yggdryl::FixMsg>;
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

/// The sole message a fixture yields, filled where asked: a stage is a call
/// on the codec, so the flag lives in the test helper alone.
fn sole_message_filled(
    codec: &yggdryl::FixCodec,
    messages: impl Iterator<Item = yggdryl::Result<yggdryl::FixMsg>>,
    enrich: bool,
) -> yggdryl::Result<yggdryl::FixMsg> {
    let message = sole_message(messages)?;
    if enrich {
        return codec.enrich_message(message);
    }
    Ok(message)
}

impl SoleMessage for yggdryl::FixCodec {
    fn sole_line(&self, row: &[u8], enrich: bool) -> yggdryl::Result<yggdryl::FixMsg> {
        sole_message_filled(self, self.parse_line(row)?, enrich)
    }

    fn sole_plugin_line(&self, row: &[u8], enrich: bool) -> yggdryl::Result<yggdryl::FixMsg> {
        sole_message_filled(self, self.parse_plugin_line(row), enrich)
    }
}

/// The tags the Jolokia envelope used to occupy, which name nothing now.
///
/// `MBean`, `Operation`, `Status` and `Error` were what the transport asked
/// and how the asking went, never a fact about the plugin the answer carried,
/// so decision 17 deleted the four of them. They are retired rather than
/// reused - a capture written last year holds `MBean` on 20001 - so what the
/// suites below assert about them is absence: no field in the dictionary, no
/// entry on a message, and no reader handing one back.
const RETIRED_ENVELOPE_TAGS: std::ops::RangeInclusive<i32> = 20_001..=20_004;

/// The four names those tags were defined under.
const RETIRED_ENVELOPE_NAMES: [&str; 4] = ["MBean", "Operation", "Status", "Error"];

/// That a message states nothing of the exchange that carried it.
///
/// By name and by tag both, because neither half is the whole claim: the
/// dictionary defines no field on 20001 to 20004 any more, so an entry a
/// reader still wrote under the key `MBean` would resolve to no tag at all
/// and slip past a tag-only check. The name is the falsifiable half; the tag
/// is the one a capture written before decision 17 would collide on.
fn states_no_envelope(message: &yggdryl::FixMsg) {
    for name in RETIRED_ENVELOPE_NAMES {
        assert!(message.get_by_name(name).is_none(), "{name}");
    }
    for retired in RETIRED_ENVELOPE_TAGS {
        assert!(message.get_by_tag(retired).is_none(), "{retired}");
    }
}

/// The tag carrying the ObjectName a read answered for.
///
/// The one place a configuration message names itself, and where the
/// ObjectName always belonged: the envelope that used to restate it on 20001
/// is gone. It is also the smallest tag ULBridge's dictionary now defines,
/// which is why [`yggdryl::PLUGIN_TAG_MIN`] is a floor rather than an
/// equal.
const SESSIONINTERFACE_TAG: i32 = 20_010;

/// How many message types every registry holds before a test registers one.
///
/// The crate's own, which `FixRegistry::new` seeds beside its fields exactly
/// as it seeds `pluginid`: `pluginconfig` is the one of them (decision 19).
/// Counted rather than spelled `1`, the way [`crated_fields`] counts the fields, so
/// every total below stays true of the next one.
fn crated_messages() -> usize {
    usize::from(yggdryl::fix_plugin_message().is_ok())
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
        .filter(|field| !field.dtype().is_nested())
        .count()
}

/// Initial registry fields, including the standard SendingTime and TransactTime seeds.
fn seeded_fields() -> usize {
    static COUNT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *COUNT.get_or_init(|| yggdryl::FixRegistry::new().len())
}

/// Where the column carrying `tag` sits in a batch: by the tag its field
/// carries, never by its spelling.
fn tag_index(batch: &arrow_array::RecordBatch, tag: i32) -> usize {
    let schema =
        yggdryl::Field::from_arrow_schema("row", &batch.schema()).expect("the batch schema reads");
    yggdryl::fix_column_of(&schema, tag).unwrap_or_else(|| panic!("a column for tag {tag}"))
}
