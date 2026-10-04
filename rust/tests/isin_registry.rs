//! The harness for the files `rust/src/isin_registry/` holds, one test
//! module per source file, beside the counting filesystem the store's cost
//! pins are taken over. `rust/src/isin_registry.rs` itself is pinned by
//! `rust/tests/root/isin_registry.rs`.

use std::path::PathBuf;

#[path = "support/counting_filesystem.rs"]
mod counting_filesystem;

#[path = "isin_registry/env.rs"]
mod env;
#[path = "isin_registry/store.rs"]
mod store;

/// A scratch folder under the temporary directory, cleared, that no test
/// shares with another.
fn scratch(label: &str) -> PathBuf {
    let path = yggdryl::local::LocalFolder::temporary()
        .expect("a temporary directory")
        .path()
        .expect("a path")
        .join(format!(
            "yggdryl-isin-registry-{label}-{}",
            std::process::id()
        ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("the scratch folder");
    path
}

const ISOLATED_TEST: &str = "YGGDRYL_ISOLATED_ISIN_REGISTRY_TEST";

/// Runs a process-global case in a child containing only that selected
/// test, whose environment names scratch folders alone: `HOME` and
/// `USERPROFILE` a scratch home, `YGGDRYL_ISIN_REGISTRY_URI` the folder
/// `location` under it, and no FIX registry location. Answers whether this
/// is the parent, which ran the child; the child answers `false` and runs
/// the body.
fn run_isolated(test_name: &str, marker: &str, location: Option<&str>) -> bool {
    if std::env::var(ISOLATED_TEST).as_deref() == Ok(marker) {
        return false;
    }
    let home = scratch(marker);
    let mut child = std::process::Command::new(std::env::current_exe().expect("the test binary"));
    child
        .args(["--exact", test_name, "--nocapture"])
        .env(ISOLATED_TEST, marker)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("YGGDRYL_FIX_REGISTRY");
    match location {
        Some(location) => {
            child.env("YGGDRYL_ISIN_REGISTRY_URI", home.join(location));
        }
        None => {
            child.env_remove("YGGDRYL_ISIN_REGISTRY_URI");
        }
    }
    let status = child.status().expect("the isolated test must start");
    assert!(status.success(), "isolated test {test_name} failed");
    let _ = std::fs::remove_dir_all(&home);
    true
}
