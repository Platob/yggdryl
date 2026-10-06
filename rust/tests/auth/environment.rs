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
fn a_raw_variable_set_empty_or_blank_is_set_and_one_never_set_is_not() {
    // A path variable set empty is the statement that there is no file, so
    // the empty text is answered where `get` answers nothing.
    let environment = given(&[
        ("AWS_CONFIG_FILE", ""),
        ("AWS_SHARED_CREDENTIALS_FILE", " 	 "),
        ("SPACED", "  /tmp/aws/config \n"),
    ]);
    assert_eq!(environment.raw("AWS_CONFIG_FILE").as_deref(), Some(""));
    assert_eq!(
        environment.raw("AWS_SHARED_CREDENTIALS_FILE").as_deref(),
        Some(""),
        "blank is set, trimmed to the empty text"
    );
    assert_eq!(environment.raw("UNSET"), None, "never set is not set");
    assert_eq!(environment.get("AWS_CONFIG_FILE"), None);
    assert_eq!(environment.get("AWS_SHARED_CREDENTIALS_FILE"), None);
    assert_eq!(
        environment.raw("SPACED").as_deref(),
        Some("/tmp/aws/config"),
        "a value is trimmed as `get` trims it"
    );
    assert_eq!(environment.raw("SPACED"), environment.get("SPACED"));
    assert_eq!(
        given(&[]).raw("AWS_CONFIG_FILE"),
        None,
        "the process is not consulted"
    );

    assert_eq!(
        Environment::Process.raw("YGGDRYL_A_VARIABLE_NOBODY_SETS"),
        None
    );
    assert_eq!(
        Environment::Process.raw("PATH"),
        Environment::Process.get("PATH")
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
