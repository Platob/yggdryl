//! The harness for the files `rust/market/src/instrument/` holds, one test
//! module per source file, beside the counting filesystem the store's cost
//! pins are taken over. `rust/market/src/instrument.rs` itself is pinned by
//! `rust/market/tests/root/instrument.rs`.

#[path = "support/install.rs"]
mod install;
use std::path::PathBuf;

#[path = "../../tests/support/counting_filesystem.rs"]
mod counting_filesystem;

#[path = "instrument/env.rs"]
mod env;
#[path = "instrument/seed.rs"]
mod seed;
#[path = "instrument/store.rs"]
mod store;

/// A scratch folder under the temporary directory, cleared, that no test
/// shares with another.
fn scratch(label: &str) -> PathBuf {
    let path = yggdryl::local::LocalFolder::temporary()
        .expect("a temporary directory")
        .path()
        .expect("a path")
        .join(format!(
            "yggdryl-instruments-{label}-{}",
            std::process::id()
        ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("the scratch folder");
    path
}

const ISOLATED_TEST: &str = "YGGDRYL_ISOLATED_INSTRUMENT_TEST";

/// Runs a process-global case in a child containing only that selected
/// test, whose environment names scratch folders alone: `HOME` and
/// `USERPROFILE` a scratch home, `YGGDRYL_INSTRUMENTS_URI` the folder
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
            child.env("YGGDRYL_INSTRUMENTS_URI", home.join(location));
        }
        None => {
            child.env_remove("YGGDRYL_INSTRUMENTS_URI");
        }
    }
    let output = child.output().expect("the isolated test must start");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "isolated test {test_name} failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // A child that matched no test exits successfully having run nothing; the
    // name must reach exactly one test, or the body never ran.
    assert!(
        stdout.contains("test result: ok. 1 passed"),
        "isolated test {test_name} ran no test (the name matches none):\n{stdout}"
    );
    let _ = std::fs::remove_dir_all(&home);
    true
}
