//! `~/.config/fix` is the production default when nothing names another.
//!
//! `HOME`, `USERPROFILE`, `YGGDRYL_FIX_REGISTRY` and the process default are
//! all process-wide, so this binary holds exactly one test and therefore runs
//! on one thread. The home it points at is a fresh directory of its own,
//! never the developer's.

use std::ffi::OsString;

use yggdryl::local::Folder;
use yggdryl::{DataType, FixRegistry};

const LOCATION: &str = "YGGDRYL_FIX_REGISTRY";

#[test]
fn the_configuration_directory_seeds_the_default() {
    let original: [(&str, Option<OsString>); 3] = [
        ("HOME", std::env::var_os("HOME")),
        ("USERPROFILE", std::env::var_os("USERPROFILE")),
        (LOCATION, std::env::var_os(LOCATION)),
    ];
    let home = Folder::temporary()
        .expect("the temporary directory")
        .path()
        .expect("a platform path")
        .join(format!("yggdryl-fix-global-home-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);

    let mut symbol = DataType::Utf8.required_field("Symbol");
    symbol.as_fix_mut().set_tag(55).expect("a valid tag");
    FixRegistry::from_fields([symbol])
        .expect("one field")
        .write_into(&mut Folder::new(home.join(".config").join("fix")).expect("a local folder"))
        .expect("the shard written");
    assert!(home.join(".config/fix/records/0.json").is_file());

    // SAFETY: `set_var` is `unsafe` because another thread reading the
    // environment concurrently is a data race. This binary holds only this
    // test, so it runs on one thread, and the test spawns none.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);
        std::env::remove_var(LOCATION);
    }
    let global = FixRegistry::global().expect("the configured registry");
    assert_eq!(global.field_by_tag(55).expect("Symbol").name(), "Symbol");
    assert_eq!(global.len(), 1);

    // SAFETY: the same reasoning as above.
    unsafe {
        for (name, value) in original {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
    let _ = std::fs::remove_dir_all(&home);
}
