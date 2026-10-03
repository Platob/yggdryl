//! `rust/src/auth/environment.rs`: the process environment, or a stand-in.

use std::collections::BTreeMap;

use yggdryl::internals::auth_environment::Environment;

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
fn a_flag_reads_every_spelling_the_one_boolean_table_reads() {
    for spelling in [
        "true", "TRUE", "t", "tr", "tru", "1", "yes", "y", "Y", "ye", "on", " Yes ", "\tON\n",
    ] {
        assert_eq!(
            given(&[("A", spelling)]).flag("A"),
            Some(true),
            "{spelling:?}"
        );
    }
    for spelling in [
        "false", "f", "fa", "fal", "fals", "0", "no", "n", "N", "off", "of", " NO ",
    ] {
        assert_eq!(
            given(&[("A", spelling)]).flag("A"),
            Some(false),
            "{spelling:?}"
        );
    }
}

#[test]
fn a_flag_no_spelling_reads_is_false_and_a_blank_or_missing_one_is_unset() {
    // A toggle nobody can read is not one that was set: false, never a
    // refusal and never unset, which would let the next source answer.
    for spelling in ["maybe", "2", "yeah", "truee", "-1"] {
        assert_eq!(
            given(&[("A", spelling)]).flag("A"),
            Some(false),
            "{spelling:?}"
        );
    }
    for spelling in ["", "   "] {
        assert_eq!(given(&[("A", spelling)]).flag("A"), None, "{spelling:?}");
    }
    assert_eq!(given(&[]).flag("C"), None);
}
