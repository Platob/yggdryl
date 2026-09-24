//! `rust/src/auth/environment.rs`: the process environment, or a stand-in.

use std::collections::BTreeMap;

use yggdryl::internals::auth_environment::{Environment, is_true};

fn given(pairs: &[(&str, &str)]) -> Environment {
    Environment::Given(
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect::<BTreeMap<_, _>>(),
    )
}

#[test]
fn a_given_environment_answers_its_pairs_and_nothing_else() {
    let environment = given(&[("AWS_REGION", " eu-west-3 "), ("EMPTY", "   ")]);
    assert_eq!(environment.get("AWS_REGION").as_deref(), Some("eu-west-3"));
    assert_eq!(environment.get("EMPTY"), None, "blank is unset");
    assert_eq!(
        environment.get("PATH"),
        None,
        "the process is not consulted"
    );
}

#[test]
fn the_process_environment_is_the_process_s_own() {
    // PATH exists everywhere, so this proves the process is read without the
    // test setting a variable of its own - which it could not do in a crate
    // that denies unsafe code.
    assert!(Environment::Process.get("PATH").is_some());
    assert_eq!(
        Environment::Process.get("YGGDRYL_A_VARIABLE_NOBODY_SETS"),
        None
    );
}

#[test]
fn a_flag_reads_every_spelling_the_cloud_tools_accept() {
    for spelling in ["true", "TRUE", "1", "yes", "on", " Yes "] {
        assert!(is_true(spelling), "{spelling:?}");
    }
    for spelling in ["false", "0", "no", "off", "", "maybe"] {
        assert!(!is_true(spelling), "{spelling:?}");
    }
    let environment = given(&[("A", "on"), ("B", "off")]);
    assert_eq!(environment.flag("A"), Some(true));
    assert_eq!(environment.flag("B"), Some(false));
    assert_eq!(environment.flag("C"), None);
}
