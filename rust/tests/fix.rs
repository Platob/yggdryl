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
#[path = "fix/pipeline.rs"]
mod pipeline;
#[path = "fix/schema.rs"]
mod schema;
#[path = "fix/store.rs"]
mod store;

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

/// The crate's own field names, in the order every registry iterates them:
/// last, because their tags are above every tag a test claims.
fn crate_names() -> Vec<&'static str> {
    yggdryl::fix_crate_fields()
        .expect("the crate's own fields")
        .iter()
        .map(yggdryl::Field::name)
        .collect()
}

/// Where the column carrying `tag` sits in a batch: by the tag its field
/// carries, never by its spelling.
fn tag_index(batch: &arrow_array::RecordBatch, tag: i32) -> usize {
    let schema = yggdryl::Field::from_arrow_schema("row", &batch.schema())
        .expect("the batch schema reads");
    yggdryl::fix_column_of(&schema, tag).unwrap_or_else(|| panic!("a column for tag {tag}"))
}
