//! `rust/market/src/isin_registry/env.rs`: the process default, resolved once from
//! `YGGDRYL_ISIN_REGISTRY_URI`, the home directory or nothing, in the
//! documented order, laid over the seed, and shared with the codec the
//! environment names.

use std::sync::Arc;

use yggdryl::{IOBase, Isin, Url};
use yggdryl_fix::{FixCodec, FixRegistry};
use yggdryl_market::{IdType, IsinEntry, IsinRegistry};

const HOLCIM: &str = "CH0012214059";
const APPLE: &str = "US0378331005";

fn row(code: &str) -> IsinEntry {
    IsinEntry::new(Isin::new(HOLCIM).unwrap())
        .try_with_code(IdType::Common, code)
        .unwrap()
}

/// `from_env` resolves once and reads the environment, so it runs in a
/// child whose environment names a scratch store: the registry it answers
/// is bound there, the seed alone and clean on a first run, committed on
/// request, the same `Arc` on every call, what `FixCodec::from_env` shares
/// and what no later install replaces.
#[test]
fn the_process_default_binds_what_the_environment_names_and_a_codec_shares_it() {
    crate::install::installed();
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
        assert_eq!(
            held.len(),
            IsinRegistry::seeded().len(),
            "a first run: the seed"
        );
        assert!(held.get(APPLE).is_some());
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
