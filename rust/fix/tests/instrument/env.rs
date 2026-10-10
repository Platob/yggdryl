//! `rust/market/src/instrument/env.rs`: the process default, resolved once from
//! `YGGDRYL_INSTRUMENTS_URI`, the home directory or nothing, in the
//! documented order, laid over the seed, and shared with the codec the
//! environment names.

use std::sync::Arc;

use yggdryl::{IOBase, Isin, Url};
use yggdryl_fix::{FixCodec, FixRegistry};
use yggdryl_market::{IdType, Instrument, Instruments};

const HOLCIM: &str = "CH0012214059";
const APPLE: &str = "US0378331005";

fn row(code: &str) -> Instrument {
    Instrument::for_security(Isin::new(HOLCIM).unwrap())
        .unwrap()
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
    let location = std::env::var("YGGDRYL_INSTRUMENTS_URI").expect("the child's location");
    let registry = Instruments::from_env().expect("the default resolves");
    {
        let mut held = registry.lock().expect("the registry");
        assert_eq!(
            held.len(),
            Instruments::seeded().len(),
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
        Instruments::from_env().expect("resolved once"),
        registry
    ));
    assert!(
        Instruments::install_env(Instruments::new())
            .unwrap_err()
            .to_string()
            .contains("already resolved")
    );
    // The codec the environment names shares it; one built by hand attaches
    // none.
    let codec = FixCodec::from_env().expect("a codec");
    assert!(Arc::ptr_eq(codec.instruments().expect("shared"), registry));
    assert!(
        FixCodec::new(Arc::new(FixRegistry::new()))
            .instruments()
            .is_none()
    );
}
