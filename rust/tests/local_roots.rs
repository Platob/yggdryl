//! The environment-driven half of the well-known local roots.
//!
//! `HOME` and `USERPROFILE` are process-wide, so this binary holds exactly one
//! test and therefore runs on one thread; the resolution rule itself is
//! covered with explicit inputs in the crate's unit tests.

use std::ffi::OsString;

use yggdryl::IOBase;
use yggdryl::holder::local::Folder;

#[test]
fn home_and_config_follow_the_environment_and_create_nothing() {
    let original: (Option<OsString>, Option<OsString>) =
        (std::env::var_os("HOME"), std::env::var_os("USERPROFILE"));

    // A fresh directory of this test's own, never the developer's real home.
    let home = Folder::temporary()
        .expect("the temporary directory")
        .path()
        .expect("a platform path")
        .join(format!("yggdryl-local-roots-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("a fresh home");

    // SAFETY: `set_var` is `unsafe` because another thread reading the
    // environment concurrently is a data race. This binary holds only this
    // test, so it runs on one thread, and the test spawns none.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);
    }

    let resolved = Folder::home().expect("a home from the environment");
    assert_eq!(resolved.path().expect("a platform path"), home);

    let config = Folder::config().expect("a configuration directory");
    assert_eq!(
        config.path().expect("a platform path"),
        home.join(".config")
    );

    // Neither call created anything: the handle is the whole effect.
    assert!(!config.exists());
    assert!(!home.join(".config").exists());
    assert_eq!(
        Folder::new(&home)
            .expect("a local folder")
            .ls(false, true)
            .count(),
        0
    );

    // SAFETY: the same reasoning as above.
    unsafe {
        std::env::remove_var("HOME");
        std::env::remove_var("USERPROFILE");
    }

    let error = Folder::home().expect_err("no home without either variable");
    assert!(error.is_absent());
    let message = error.to_string();
    assert!(message.contains("HOME"), "{message}");
    assert!(message.contains("USERPROFILE"), "{message}");
    assert!(
        Folder::config()
            .expect_err("no config without a home")
            .is_absent()
    );

    // SAFETY: the same reasoning as above.
    unsafe {
        match original.0 {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match original.1 {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }
    }
    let _ = std::fs::remove_dir_all(&home);
}
