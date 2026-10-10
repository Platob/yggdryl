//! `rust/market/src/instrument/env.rs`: the process default, resolved once from
//! `YGGDRYL_INSTRUMENTS_URI`, the home directory or nothing, in the
//! documented order, laid over the seed, and shared with the codec the
//! environment names.

use yggdryl::{Ccy, Isin, Mic, Url};
use yggdryl_market::{IdType, Instrument, Instruments, Listing};

const HOLCIM: &str = "CH0012214059";
const APPLE: &str = "US0378331005";
const MICROSOFT: &str = "US5949181045";

fn row(code: &str) -> Instrument {
    Instrument::for_security(Isin::new(HOLCIM).unwrap())
        .unwrap()
        .try_with_code(IdType::Common, code)
        .unwrap()
}

/// A store holding rows already is laid over the seed: a value the store
/// states wins over the seed's, a seed row it has no row of stands, a
/// fact only the seed states is kept beside the store's, and the default
/// is clean after the load.
#[test]
fn the_process_default_lays_the_store_over_the_seed() {
    crate::install::installed();
    if crate::run_isolated(
        "env::the_process_default_lays_the_store_over_the_seed",
        "over-seed",
        Some("instruments/"),
    ) {
        return;
    }
    let location = std::env::var("YGGDRYL_INSTRUMENTS_URI").expect("the child's location");
    let url = Url::from_location(&location).unwrap();
    let none: [(&str, &str); 0] = [];
    let mut store = Instruments::from_url(&url, none).expect("a first run");
    assert!(store.is_empty(), "bound without the seed");
    store
        .merge(
            Instrument::for_security(Isin::new(APPLE).unwrap())
                .unwrap()
                .with_listing(
                    Listing::new(Some(Mic::new("XNAS").unwrap()))
                        .with_ticker(Some("AAPL".into()))
                        .with_currency(Some(Ccy::new("CHF").unwrap())),
                )
                .unwrap(),
        )
        .unwrap();
    store.commit().expect("committed");

    let registry = Instruments::from_env().expect("the default resolves");
    let held = registry.lock().expect("the registry");
    assert!(!held.is_dirty(), "clean after the load");
    assert_eq!(held.len(), Instruments::seeded().len());
    let apple = held.get(APPLE).expect("the seed's and the store's");
    let nasdaq = Mic::new("XNAS").unwrap();
    assert_eq!(
        apple
            .listing(Some(&nasdaq))
            .and_then(|listing| listing.currency())
            .map(Ccy::as_str),
        Some("CHF"),
        "the store's value wins"
    );
    assert_eq!(
        apple.fisn(),
        Instruments::seeded().get(APPLE).and_then(Instrument::fisn),
        "a fact the store does not state stays the seed's"
    );
    assert!(apple.fisn().is_some());
    assert_eq!(
        held.get(MICROSOFT)
            .and_then(|held| held.listing(Some(&nasdaq)))
            .and_then(|listing| listing.currency())
            .map(Ccy::as_str),
        Some("USD"),
        "a seed row the store has none of"
    );
}

/// With nothing in the environment, the default is the home's own folder,
/// laid out by the first commit; an installed registry wins before anything
/// resolves one.
#[test]
fn the_process_default_is_the_homes_folder_and_an_install_wins_before_it_resolves() {
    crate::install::installed();
    if crate::run_isolated(
        "env::the_process_default_is_the_homes_folder_and_an_install_wins_before_it_resolves",
        "install-env",
        None,
    ) {
        return;
    }
    let mut installed = Instruments::new();
    installed.merge(row("C-INSTALLED")).unwrap();
    Instruments::install_env(installed).expect("installed first");
    let registry = Instruments::from_env().expect("the installed one");
    let held = registry.lock().expect("the registry");
    assert_eq!(
        held.get(HOLCIM).unwrap().get(&IdType::Common),
        Some("C-INSTALLED")
    );
    assert!(held.holder().is_none(), "installed, bound to nothing");
    drop(held);
    assert!(Instruments::install_env(Instruments::new()).is_err());
    let home = std::env::var("HOME").expect("the child's home");
    assert!(
        !std::path::Path::new(&home).join(".config").exists(),
        "nothing resolved the home's folder"
    );
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::local::LocalFolder;
    use yggdryl::{IOBase, Url};
    use yggdryl_market::IdType;
    use yggdryl_market::internals::instrument_env::autoload;

    use super::row;

    /// The pure step under `from_env`, driven by explicit inputs: nothing
    /// and no home the seed unbound; a home the folder under its
    /// configuration directory, a first run - the seed - laid out by the
    /// first commit and read back by the next resolution; an explicit location
    /// beating it, spelled as a path, a URL, a leaf or a `~` path; an empty
    /// location unset; a `~` path with no home, a scheme this build has no
    /// backend for and a store that cannot be read refused, never the seed
    /// alone.
    #[test]
    fn the_default_resolves_in_the_documented_order_from_explicit_inputs() {
        crate::install::installed();
        let root = crate::scratch("autoload");
        let home = LocalFolder::new(root.join("home")).unwrap();

        let seeded = yggdryl_market::Instruments::seeded().len();
        let unbound = autoload(None, None).unwrap();
        assert_eq!(unbound.len(), seeded);
        assert!(unbound.holder().is_none() && !unbound.is_dirty());

        let mut configured = autoload(None, Some(home.clone())).unwrap();
        assert_eq!(configured.len(), seeded);
        assert!(!configured.is_dirty());
        let bound = configured
            .holder()
            .expect("bound")
            .url()
            .expect("a URL")
            .to_string();
        assert!(
            bound.ends_with("/home/.config/yggdryl/instruments/"),
            "{bound}"
        );
        assert!(!root.join("home/.config").exists(), "nothing laid out yet");
        configured.merge(row("C-1")).unwrap();
        configured.commit().unwrap();
        assert!(
            root.join("home/.config/yggdryl/instruments/part-0.arrows")
                .is_file()
        );
        let again = autoload(None, Some(home.clone())).unwrap();
        assert_eq!(
            again.get(super::HOLCIM).unwrap().get(&IdType::Common),
            Some("C-1")
        );
        assert!(!again.is_dirty());
        // An empty location reads as unset: the home's folder, holding the
        // seed's rows the first commit wrote beside the one it moved.
        let unset = autoload(Some("  "), Some(home.clone())).unwrap();
        assert_eq!(unset.len(), seeded);
        assert_eq!(
            unset.get(super::HOLCIM).unwrap().get(&IdType::Common),
            Some("C-1")
        );

        // An explicit location beats the home's folder: a leaf as a path or
        // a URL, a folder under `~`.
        let leaf = root.join("instruments.arrows");
        let spellings = [
            leaf.to_string_lossy().into_owned(),
            Url::from_path(&leaf).unwrap().to_string(),
        ];
        for spelling in spellings {
            let mut located = autoload(Some(&spelling), Some(home.clone())).unwrap();
            assert_eq!(located.len(), seeded, "{spelling}");
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
            root.join("home/.config/yggdryl/instruments/part-0.arrows"),
            b"not an arrow stream",
        )
        .unwrap();
        assert!(autoload(None, Some(home)).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
