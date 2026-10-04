//! `rust/src/isin_registry/env.rs`: the process default, resolved once from
//! `YGGDRYL_ISIN_REGISTRY_URI`, the home directory or nothing, in the
//! documented order, and shared with the codec the environment names.

use std::sync::Arc;

use yggdryl::{FixCodec, FixRegistry, IOBase, IdType, Isin, IsinEntry, IsinRegistry, Url};

const HOLCIM: &str = "CH0012214059";

fn row(code: &str) -> IsinEntry {
    IsinEntry::new(Isin::new(HOLCIM).unwrap())
        .try_with_code(IdType::Common, code)
        .unwrap()
}

/// `from_env` resolves once and reads the environment, so it runs in a
/// child whose environment names a scratch store: the registry it answers
/// is bound there, empty on a first run, committed on request, the same
/// `Arc` on every call, what `FixCodec::from_env` shares and what no later
/// install replaces.
#[test]
fn the_process_default_binds_what_the_environment_names_and_a_codec_shares_it() {
    if crate::run_isolated(
        "env::the_process_default_binds_what_the_environment_names_and_a_codec_shares_it",
        "from-env",
        Some("instruments/"),
    ) {
        return;
    }
    let location = std::env::var("YGGDRYL_ISIN_REGISTRY_URI").expect("the child's location");
    let registry = IsinRegistry::from_env().expect("the default resolves");
    {
        let mut held = registry.lock().expect("the registry");
        assert!(held.is_empty(), "an empty first run");
        assert!(!held.is_dirty());
        let bound = held
            .holder()
            .expect("bound to the location")
            .url()
            .expect("a URL")
            .to_string();
        assert_eq!(bound, Url::from_location(&location).unwrap().to_string());
        held.merge(row("C-1")).unwrap();
        held.commit().expect("committed");
    }
    assert!(
        std::path::Path::new(&location)
            .join("part-0.arrows")
            .is_file()
    );
    assert!(Arc::ptr_eq(
        IsinRegistry::from_env().expect("resolved once"),
        registry
    ));
    assert!(
        IsinRegistry::install_env(IsinRegistry::new())
            .unwrap_err()
            .to_string()
            .contains("already resolved")
    );
    // The codec the environment names shares it; one built by hand attaches
    // none.
    let codec = FixCodec::from_env().expect("a codec");
    assert!(Arc::ptr_eq(
        codec.isin_registry().expect("shared"),
        registry
    ));
    assert!(
        FixCodec::new(Arc::new(FixRegistry::new()))
            .isin_registry()
            .is_none()
    );
}

/// With nothing in the environment, the default is the home's own folder,
/// laid out by the first commit; an installed registry wins before anything
/// resolves one.
#[test]
fn the_process_default_is_the_homes_folder_and_an_install_wins_before_it_resolves() {
    if crate::run_isolated(
        "env::the_process_default_is_the_homes_folder_and_an_install_wins_before_it_resolves",
        "install-env",
        None,
    ) {
        return;
    }
    let mut installed = IsinRegistry::new();
    installed.merge(row("C-INSTALLED")).unwrap();
    IsinRegistry::install_env(installed).expect("installed first");
    let registry = IsinRegistry::from_env().expect("the installed one");
    let held = registry.lock().expect("the registry");
    assert_eq!(
        held.get(HOLCIM).unwrap().get(&IdType::Common),
        Some("C-INSTALLED")
    );
    assert!(held.holder().is_none(), "installed, bound to nothing");
    drop(held);
    assert!(IsinRegistry::install_env(IsinRegistry::new()).is_err());
    let home = std::env::var("HOME").expect("the child's home");
    assert!(
        !std::path::Path::new(&home).join(".config").exists(),
        "nothing resolved the home's folder"
    );
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::isin_registry_env::autoload;
    use yggdryl::local::LocalFolder;
    use yggdryl::{IOBase, IdType, Url};

    use super::row;

    /// The pure step under `from_env`, driven by explicit inputs: nothing
    /// and no home an unbound empty registry; a home the folder under its
    /// configuration directory, an empty first run laid out by the first
    /// commit and read back by the next resolution; an explicit location
    /// beating it, spelled as a path, a URL, a leaf or a `~` path; an empty
    /// location unset; a `~` path with no home, a scheme this build has no
    /// backend for and a store that cannot be read refused, never the empty
    /// registry.
    #[test]
    fn the_default_resolves_in_the_documented_order_from_explicit_inputs() {
        let root = crate::scratch("autoload");
        let home = LocalFolder::new(root.join("home")).unwrap();

        let unbound = autoload(None, None).unwrap();
        assert!(unbound.is_empty() && unbound.holder().is_none());

        let mut configured = autoload(None, Some(home.clone())).unwrap();
        assert!(configured.is_empty() && !configured.is_dirty());
        let bound = configured
            .holder()
            .expect("bound")
            .url()
            .expect("a URL")
            .to_string();
        assert!(bound.ends_with("/home/.config/yggdryl/isin/"), "{bound}");
        assert!(!root.join("home/.config").exists(), "nothing laid out yet");
        configured.merge(row("C-1")).unwrap();
        configured.commit().unwrap();
        assert!(
            root.join("home/.config/yggdryl/isin/part-0.arrows")
                .is_file()
        );
        let again = autoload(None, Some(home.clone())).unwrap();
        assert_eq!(
            again.get(super::HOLCIM).unwrap().get(&IdType::Common),
            Some("C-1")
        );
        assert!(!again.is_dirty());
        // An empty location reads as unset.
        assert_eq!(autoload(Some("  "), Some(home.clone())).unwrap().len(), 1);

        // An explicit location beats the home's folder: a leaf as a path or
        // a URL, a folder under `~`.
        let leaf = root.join("instruments.arrows");
        let spellings = [
            leaf.to_string_lossy().into_owned(),
            Url::from_path(&leaf).unwrap().to_string(),
        ];
        for spelling in spellings {
            let mut located = autoload(Some(&spelling), Some(home.clone())).unwrap();
            assert!(located.is_empty(), "{spelling}");
            located.merge(row("C-2")).unwrap();
            located.commit().unwrap();
            assert!(leaf.is_file(), "{spelling}");
            let back = autoload(Some(&spelling), Some(home.clone())).unwrap();
            assert_eq!(
                back.get(super::HOLCIM).unwrap().get(&IdType::Common),
                Some("C-2"),
                "{spelling}"
            );
            std::fs::remove_file(&leaf).unwrap();
        }
        let mut tilde = autoload(Some("~/instruments/"), Some(home.clone())).unwrap();
        tilde.merge(row("C-3")).unwrap();
        tilde.commit().unwrap();
        assert!(root.join("home/instruments/part-0.arrows").is_file());
        // A path under `~` is a platform path: a blank or a non-ASCII
        // character in it is the path's own; the value is read trimmed.
        let mut spaced = autoload(Some("  ~/caf\u{e9} instruments/ "), Some(home.clone())).unwrap();
        spaced.merge(row("C-4")).unwrap();
        spaced.commit().unwrap();
        assert!(
            root.join("home/caf\u{e9} instruments/part-0.arrows")
                .is_file()
        );
        let padded = format!(
            " {} ",
            root.join("padded").join("instruments.arrows").display()
        );
        let mut trimmed = autoload(Some(&padded), Some(home.clone())).unwrap();
        trimmed.merge(row("C-5")).unwrap();
        trimmed.commit().unwrap();
        assert!(root.join("padded/instruments.arrows").is_file());

        // Refused, never the empty registry.
        assert!(
            autoload(Some("~/instruments/"), None)
                .unwrap_err()
                .is_absent()
        );
        for home_itself in ["~", "~/", "~//"] {
            let error = autoload(Some(home_itself), Some(home.clone()))
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("home directory itself"),
                "{home_itself}: {error}"
            );
        }
        let error = autoload(Some("mem://1/isin/"), Some(home.clone()))
            .unwrap_err()
            .to_string();
        assert!(error.contains("mem"), "{error}");
        std::fs::write(
            root.join("home/.config/yggdryl/isin/part-0.arrows"),
            b"not an arrow stream",
        )
        .unwrap();
        assert!(autoload(None, Some(home)).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
