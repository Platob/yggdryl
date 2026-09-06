//! `YGGDRYL_FIX_REGISTRY` names the default, loudly.
//!
//! The environment and the process default are both process-wide, so the
//! layer binary runs this case in its own selected child process.

use std::ffi::OsString;
use std::sync::Arc;

use yggdryl::holder::local::Folder;
use yggdryl::{DataType, FixRegistry};

const LOCATION: &str = "YGGDRYL_FIX_REGISTRY";

#[test]
fn a_malformed_location_errors_and_a_valid_one_settles_the_default() {
    if super::run_isolated(
        "global_env::a_malformed_location_errors_and_a_valid_one_settles_the_default",
        "env",
    ) {
        return;
    }
    let original: Option<OsString> = std::env::var_os(LOCATION);
    let root = Folder::temporary()
        .expect("the temporary directory")
        .path()
        .expect("a platform path")
        .join(format!("yggdryl-fix-global-env-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    // A folder holding a standard shard that does not parse.
    let bad = root.join("bad");
    std::fs::create_dir_all(bad.join("primitive")).expect("a fresh folder");
    std::fs::write(bad.join("primitive").join("0.json"), b"not json").expect("a malformed shard");

    // SAFETY: `set_var` is `unsafe` because another thread reading the
    // environment concurrently is a data race. This binary holds only this
    // test, so it runs on one thread, and the test spawns none.
    unsafe {
        std::env::set_var(LOCATION, &bad);
    }
    let error = FixRegistry::global().expect_err("a malformed shard is never empty");
    assert!(error.to_string().contains("0.json"), "{error}");

    // The default did not settle on the failure, so a valid location now
    // resolves it.
    let good = root.join("good");
    let mut symbol = DataType::Utf8.required_field("Symbol");
    symbol.as_fix_mut().set_tag(55).expect("a valid tag");
    FixRegistry::from_fields([symbol])
        .expect("one field")
        .write_into(&mut Folder::new(&good).expect("a local folder"))
        .expect("the shard written");
    // SAFETY: the same reasoning as above.
    unsafe {
        std::env::set_var(LOCATION, &good);
    }
    let global = FixRegistry::global().expect("the located registry");
    assert_eq!(global.field_by_tag(55).expect("Symbol").name(), "Symbol");

    // Settled: the environment is read exactly once.
    // SAFETY: the same reasoning as above.
    unsafe {
        std::env::set_var(LOCATION, &bad);
    }
    assert!(Arc::ptr_eq(
        FixRegistry::global().expect("the same registry"),
        global
    ));

    // SAFETY: the same reasoning as above.
    unsafe {
        match original {
            Some(value) => std::env::set_var(LOCATION, value),
            None => std::env::remove_var(LOCATION),
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}
