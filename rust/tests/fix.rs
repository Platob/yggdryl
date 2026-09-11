//! FIX integration tests.

#[path = "fix/batch.rs"]
mod batch;
#[path = "fix/capture.rs"]
mod capture;
#[path = "fix/cfb.rs"]
mod cfb;
#[path = "fix/codec.rs"]
mod codec;
#[path = "fix/dataset.rs"]
mod dataset;
#[path = "fix/dictionary.rs"]
mod dictionary;
#[path = "fix/digest.rs"]
mod digest;
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
#[path = "fix/latest.rs"]
mod latest;
#[path = "fix/lifecycle.rs"]
mod lifecycle;
#[path = "fix/lift.rs"]
mod lift;
#[path = "fix/merge.rs"]
mod merge;
#[path = "fix/message.rs"]
mod message;
#[path = "fix/numeric_branch.rs"]
mod numeric_branch;
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

fn ulbridge_registry() -> std::sync::Arc<yggdryl::FixRegistry> {
    static REGISTRY: std::sync::OnceLock<std::sync::Arc<yggdryl::FixRegistry>> =
        std::sync::OnceLock::new();
    std::sync::Arc::clone(REGISTRY.get_or_init(|| {
        std::sync::Arc::new(
            committed_registry()
                .as_ref()
                .clone()
                .with_ulbridge_fields()
                .expect("the bridge's own fields"),
        )
    }))
}

trait OneMessage {
    fn one_line(&self, row: &[u8], enrich: bool) -> yggdryl::Result<yggdryl::FixMsg>;
    fn one_ulconfig_line(&self, row: &[u8], enrich: bool) -> yggdryl::Result<yggdryl::FixMsg>;
}

fn one_message(
    mut messages: impl Iterator<Item = yggdryl::Result<yggdryl::FixMsg>>,
) -> yggdryl::Result<yggdryl::FixMsg> {
    let message = messages
        .next()
        .expect("a singleton fixture yields one message")?;
    assert!(
        messages.next().is_none(),
        "a singleton fixture yields exactly one message"
    );
    Ok(message)
}

/// The one message a singleton fixture yields, filled where asked: a stage
/// is a call on the codec, so the flag lives in the test helper alone.
fn one_message_filled(
    codec: &yggdryl::FixCodec,
    messages: impl Iterator<Item = yggdryl::Result<yggdryl::FixMsg>>,
    enrich: bool,
) -> yggdryl::Result<yggdryl::FixMsg> {
    let message = one_message(messages)?;
    if enrich {
        return codec.enrich_message(message);
    }
    Ok(message)
}

impl OneMessage for yggdryl::FixCodec {
    fn one_line(&self, row: &[u8], enrich: bool) -> yggdryl::Result<yggdryl::FixMsg> {
        one_message_filled(self, self.parse_line(row)?, enrich)
    }

    fn one_ulconfig_line(&self, row: &[u8], enrich: bool) -> yggdryl::Result<yggdryl::FixMsg> {
        one_message_filled(self, self.parse_ulconfig_line(row)?, enrich)
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

/// How many fields every registry holds before a test inserts one: the
/// crate's own, which `FixRegistry::new` seeds and no store writes.
fn crated() -> usize {
    yggdryl::fix_crate_fields()
        .expect("the crate's own fields")
        .len()
}

/// Where the column carrying `tag` sits in a batch: by the tag its field
/// carries, never by its spelling.
fn tag_index(batch: &arrow_array::RecordBatch, tag: i32) -> usize {
    let schema =
        yggdryl::Field::from_arrow_schema("row", &batch.schema()).expect("the batch schema reads");
    yggdryl::fix_column_of(&schema, tag).unwrap_or_else(|| panic!("a column for tag {tag}"))
}
