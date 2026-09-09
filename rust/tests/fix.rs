//! FIX integration tests.

#[path = "fix/batch.rs"]
mod batch;
#[path = "fix/capture.rs"]
mod capture;
#[path = "fix/cfb.rs"]
mod cfb;
#[path = "fix/codec.rs"]
mod codec;
#[path = "fix/dictionary.rs"]
mod dictionary;
#[path = "fix/digest.rs"]
mod digest;
#[path = "fix/global_env.rs"]
mod global_env;
#[path = "fix/global_home.rs"]
mod global_home;
#[path = "fix/global_install.rs"]
mod global_install;
#[path = "fix/lift.rs"]
mod lift;
#[path = "fix/numeric_branch.rs"]
mod numeric_branch;
#[path = "fix/pipeline.rs"]
mod pipeline;
#[path = "fix/schema.rs"]
mod schema;
#[path = "fix/store.rs"]
mod store;

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
    fn one_record(&self, row: &yggdryl::Scalar, enrich: bool) -> yggdryl::Result<yggdryl::FixMsg>;
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

impl OneMessage for yggdryl::FixCodec {
    fn one_line(&self, row: &[u8], enrich: bool) -> yggdryl::Result<yggdryl::FixMsg> {
        one_message(self.transform_line(row, enrich)?)
    }

    fn one_record(&self, row: &yggdryl::Scalar, enrich: bool) -> yggdryl::Result<yggdryl::FixMsg> {
        one_message(self.transform_record(row, enrich)?)
    }

    fn one_ulconfig_line(&self, row: &[u8], enrich: bool) -> yggdryl::Result<yggdryl::FixMsg> {
        one_message(self.transform_ulconfig_line(row, enrich)?)
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
