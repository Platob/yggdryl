//! `rust/src/yaml/parser.rs`: a tag that declares a type reads its text
//! through that type's reader, and an untagged plain scalar is the format's own
//! set.

use yggdryl::Scalar;
use yggdryl::yaml;

/// The value under `key` of the one-entry mapping `document`.
fn entry(document: &str, key: &str) -> Result<Scalar, String> {
    yaml::from_utf8(document)
        .map(|value| value.get_key_str(key).cloned().expect("the key is read"))
        .map_err(|error| error.to_string())
}

#[test]
fn a_tagged_boolean_reads_every_spelling_the_boolean_reader_reads() {
    for (text, expected) in [
        ("true", true),
        ("False", false),
        ("yes", true),
        ("off", false),
        ("y", true),
        ("N", false),
        ("1", true),
        ("0", false),
        ("tr", true),
        ("\" true \"", true),
    ] {
        let read = entry(&format!("flag: !!bool {text}\n"), "flag");
        assert_eq!(read, Ok(Scalar::from(expected)), "{text}");
    }
}

#[test]
fn a_tagged_boolean_that_no_boolean_spells_is_refused() {
    for text in ["maybe", "2", "n/a", "\"\""] {
        let error = entry(&format!("flag: !!bool {text}\n"), "flag").unwrap_err();
        assert!(error.contains("invalid YAML boolean"), "{text}: {error}");
    }
}

#[test]
fn an_untagged_plain_scalar_keeps_the_formats_own_boolean_set() {
    // YAML resolves true, yes and on (and their negatives) to booleans; the
    // short spellings a column of flags reads are text until a tag or a field
    // declares them.
    for (text, expected) in [
        ("true", Scalar::from(true)),
        ("no", Scalar::from(false)),
        ("y", Scalar::from("y")),
        ("n", Scalar::from("n")),
        ("t", Scalar::from("t")),
    ] {
        let read = entry(&format!("flag: {text}\n"), "flag");
        assert_eq!(read, Ok(expected), "{text}");
    }
}

#[test]
fn a_tagged_binary_reads_base64_folded_over_lines_or_spaced() {
    let bytes = Scalar::from(vec![0_u8, 255]);
    for document in [
        "payload: !!binary AP8=\n",
        "payload: !!binary \"AP 8=\"\n",
        "payload: !!binary |\n  AP\n  8=\n",
    ] {
        assert_eq!(
            entry(document, "payload"),
            Ok(bytes.clone()),
            "{document:?}"
        );
    }
}

#[test]
fn a_tagged_binary_that_is_not_standard_base64_is_refused() {
    // The URL-safe alphabet and a missing pad are not RFC 4648 section 4.
    for text in ["not base64!", "AP8", "-_8="] {
        let error = entry(&format!("payload: !!binary \"{text}\"\n"), "payload").unwrap_err();
        assert!(
            error.contains("invalid YAML binary scalar"),
            "{text}: {error}"
        );
    }
}
