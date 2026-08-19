//! `{{ }}` placeholders: the closed grammar, the two typing rules, and the
//! security switches that keep the environment out unless asked for.

use yggdryl::io::{Buffer, IOBase};
use yggdryl::text::{self, Format, Limits, Loading, Placeholders};
use yggdryl::{Url, Value};

/// The three formats, each with the same document written its own way.
fn documents(scalar: &str) -> [(Format, String); 3] {
    [
        (Format::Json, format!("{{\"value\": {scalar:?}}}")),
        // YAML *requires* the quotes: a bare `{{ X }}` is a flow mapping.
        (Format::Yaml, format!("value: {scalar:?}\n")),
        (Format::Toml, format!("value = {scalar:?}\n")),
    ]
}

fn resolved(scalar: &str, placeholders: Placeholders) -> Vec<yggdryl::Result<Value>> {
    let loading = Loading::new().with_placeholders(placeholders);
    documents(scalar)
        .into_iter()
        .map(|(format, document)| {
            text::from_str_with(&document, format, &loading)
                .map(|value| value.get_key_str("value").cloned().unwrap_or(Value::Null))
        })
        .collect()
}

#[test]
fn a_whole_scalar_placeholder_adopts_the_resolved_value_s_own_type() {
    let placeholders = Placeholders::new()
        .with_variable("PORT", Value::I64(8080))
        .with_variable("DEBUG", Value::Bool(true))
        .with_variable("RATIO", Value::from(1.5))
        .with_variable("NOTHING", Value::Null);

    for value in resolved("{{ PORT }}", placeholders.clone()) {
        assert_eq!(value.unwrap(), Value::I64(8080));
    }
    for value in resolved("{{ DEBUG }}", placeholders.clone()) {
        assert_eq!(value.unwrap(), Value::Bool(true));
    }
    for value in resolved("{{ NOTHING }}", placeholders.clone()) {
        assert_eq!(value.unwrap(), Value::Null);
    }
    // A container is fine whole; only an embedded one has no text form.
    let sequenced = Placeholders::new().with_variable(
        "HOSTS",
        Value::from_sequence([Value::from("a"), Value::from("b")]),
    );
    for value in resolved("{{ HOSTS }}", sequenced) {
        assert_eq!(
            value.unwrap(),
            Value::from_sequence([Value::from("a"), Value::from("b")])
        );
    }
    // Whitespace inside the braces is not part of the name.
    for value in resolved("{{PORT}}", placeholders) {
        assert_eq!(value.unwrap(), Value::I64(8080));
    }
}

#[test]
fn an_embedded_placeholder_substitutes_textually_and_stays_a_string() {
    let placeholders = Placeholders::new()
        .with_variable("ROOT", Value::from("/var/log"))
        .with_variable("PORT", Value::I64(8080))
        .with_variable("PRICE", Value::decimal(150, 2));

    for value in resolved("{{ ROOT }}/app", placeholders.clone()) {
        assert_eq!(value.unwrap(), Value::from("/var/log/app"));
    }
    // A number embedded in text is its own spelling, and the result is text.
    for value in resolved("localhost:{{ PORT }}/health", placeholders.clone()) {
        assert_eq!(value.unwrap(), Value::from("localhost:8080/health"));
    }
    // A decimal never goes through a float: 1.50 stays 1.50.
    for value in resolved("price={{ PRICE }}", placeholders.clone()) {
        assert_eq!(value.unwrap(), Value::from("price=1.50"));
    }
    // Two in one scalar, and text on both sides of each.
    for value in resolved("{{ ROOT }}:{{ PORT }}!", placeholders.clone()) {
        assert_eq!(value.unwrap(), Value::from("/var/log:8080!"));
    }

    // Embedded, a container and a null have no text form and are refused
    // rather than rendered as something plausible.
    let containers = Placeholders::new()
        .with_variable("HOSTS", Value::from_sequence([Value::from("a")]))
        .with_variable("NOTHING", Value::Null);
    for value in resolved("hosts={{ HOSTS }}", containers.clone()) {
        let refused = value.unwrap_err().to_string();
        assert!(refused.contains("resolve to a scalar"), "{refused}");
    }
    for value in resolved("value={{ NOTHING }}", containers) {
        assert!(value.is_err());
    }
}

#[test]
fn an_embedded_geometry_renders_as_wkt_and_malformed_wkb_as_lossless_hex() {
    // One valid little-endian WKB `POINT (1 2)`.
    let mut wkb = vec![0x01_u8, 0x01, 0x00, 0x00, 0x00];
    wkb.extend_from_slice(&1.0_f64.to_le_bytes());
    wkb.extend_from_slice(&2.0_f64.to_le_bytes());
    let placeholders = Placeholders::new()
        .with_variable("SHAPE", Value::Geospatial(wkb.into()))
        .with_variable(
            "BROKEN",
            Value::Geospatial([0xff_u8, 0x00].as_slice().into()),
        );

    // A geometry's canonical text is WKT, the spelling geospatial readers read.
    for value in resolved("shape={{ SHAPE }}", placeholders.clone()) {
        assert_eq!(value.unwrap(), Value::from("shape=POINT (1 2)"));
    }
    // Malformed WKB still embeds losslessly, as the hex of its bytes.
    for value in resolved("shape={{ BROKEN }}", placeholders) {
        assert_eq!(value.unwrap(), Value::from("shape=ff00"));
    }
}

#[test]
fn a_missing_variable_is_a_typed_error_naming_it_and_its_position() {
    for value in resolved("{{ ROOT }}/app", Placeholders::new()) {
        let refused = value.unwrap_err().to_string();
        // Never an empty string: the name, and where in the value it sits.
        assert!(refused.contains("{{ ROOT }}"), "{refused}");
        assert!(refused.contains("at byte 0"), "{refused}");
        assert!(
            refused.contains("not consulted"),
            "the message says the environment was off: {refused}"
        );
    }
    for value in resolved("host=/{{ MISSING }}", Placeholders::new()) {
        let refused = value.unwrap_err().to_string();
        assert!(refused.contains("at byte 6"), "{refused}");
    }
    // And the path names which value in the document failed.
    let loading = Loading::new().with_placeholders(Placeholders::new());
    let refused = text::from_str_with(
        r#"{"server": {"hosts": ["ok", "{{ MISSING }}"]}}"#,
        Format::Json,
        &loading,
    )
    .unwrap_err()
    .to_string();
    assert!(refused.contains("$.server.hosts[1]"), "{refused}");
}

#[test]
fn a_default_makes_a_variable_optional_and_carries_its_own_type() {
    let empty = Placeholders::new();
    for value in resolved(r#"{{ PORT | default(8080) }}"#, empty.clone()) {
        assert_eq!(value.unwrap(), Value::I64(8080));
    }
    for value in resolved(r#"{{ ROOT | default("/tmp") }}"#, empty.clone()) {
        assert_eq!(value.unwrap(), Value::from("/tmp"));
    }
    for value in resolved(r#"{{ ON | default(true) }}"#, empty.clone()) {
        assert_eq!(value.unwrap(), Value::Bool(true));
    }
    for value in resolved(r#"{{ NOTHING | default(null) }}"#, empty.clone()) {
        assert_eq!(value.unwrap(), Value::Null);
    }
    // Embedded, the default renders as text like any other resolved value.
    for value in resolved(r#"{{ ROOT | default("/tmp") }}/logs"#, empty.clone()) {
        assert_eq!(value.unwrap(), Value::from("/tmp/logs"));
    }
    // A supplied value wins over the default.
    let supplied = Placeholders::new().with_variable("PORT", Value::I64(9090));
    for value in resolved(r#"{{ PORT | default(8080) }}"#, supplied) {
        assert_eq!(value.unwrap(), Value::I64(9090));
    }

    // There is exactly one filter, and anything else says so.
    for value in resolved(r#"{{ ROOT | upper }}"#, empty.clone()) {
        let refused = value.unwrap_err().to_string();
        assert!(refused.contains("default(LITERAL)"), "{refused}");
        assert!(refused.contains("no chains"), "{refused}");
    }
    // A container default is refused: a default is a scalar.
    for value in resolved(r#"{{ ROOT | default([1]) }}"#, empty) {
        assert!(value.is_err());
    }
}

#[test]
fn a_doubled_opener_is_a_literal_one_and_nothing_else_needs_escaping() {
    let placeholders = Placeholders::new().with_variable("NAME", Value::from("app"));
    for value in resolved("{{{{ NAME }}", placeholders.clone()) {
        assert_eq!(value.unwrap(), Value::from("{{ NAME }}"));
    }
    for value in resolved("{{{{ literal }} and {{ NAME }}", placeholders.clone()) {
        assert_eq!(value.unwrap(), Value::from("{{ literal }} and app"));
    }
    // A closing pair outside a placeholder is ordinary text.
    for value in resolved("}} {{ NAME }}", placeholders.clone()) {
        assert_eq!(value.unwrap(), Value::from("}} app"));
    }
    // An opener with no closer is a refusal, not a silent literal.
    for value in resolved("{{ NAME and more", placeholders) {
        let refused = value.unwrap_err().to_string();
        assert!(refused.contains("unterminated"), "{refused}");
    }
}

#[test]
fn placeholders_reach_keys_nested_structures_and_toml_tables() {
    let placeholders = Placeholders::new()
        .with_variable("SECTION", Value::from("server"))
        .with_variable("HOST", Value::from("db.internal"))
        .with_variable("PORT", Value::I64(5432));
    let loading = Loading::new().with_placeholders(placeholders);

    // A key is substituted exactly as a value is.
    let json = text::from_str_with(
        r#"{"{{ SECTION }}": {"host": "{{ HOST }}", "ports": ["{{ PORT }}", 1]}}"#,
        Format::Json,
        &loading,
    )
    .unwrap();
    let server = json.get_key_str("server").expect("the key resolved");
    assert_eq!(
        server.get_key_str("host").and_then(Value::as_str),
        Some("db.internal")
    );
    assert_eq!(
        server.get_key_str("ports").unwrap(),
        &Value::from_sequence([Value::I64(5432), Value::U64(1)])
    );

    // A TOML table, with the placeholder inside its own values.
    let table = text::from_str_with(
        "[database]\nhost = \"{{ HOST }}\"\nport = \"{{ PORT }}\"\n",
        Format::Toml,
        &loading,
    )
    .unwrap();
    let database = table.get_key_str("database").unwrap();
    assert_eq!(database.get_key_str("port").unwrap(), &Value::I64(5432));
}

#[test]
fn the_environment_is_a_second_switch_and_the_mapping_wins() {
    // A name this test owns, so nothing else can be reading or writing it.
    const NAME: &str = "YGGDRYL_PLACEHOLDER_TEST_VALUE";
    // SAFETY: `set_var` is `unsafe` because another thread reading the
    // environment concurrently is a data race. This name is used by this test
    // only, and the test does not spawn threads.
    unsafe { std::env::set_var(NAME, "from-environment") };

    let scalar = format!("{{{{ {NAME} }}}}");

    // Off: the variable is set and still does not resolve.
    for value in resolved(&scalar, Placeholders::new()) {
        assert!(value.is_err(), "the environment must not be consulted");
    }

    // On: it resolves, as text, because that is all an environment holds.
    for value in resolved(&scalar, Placeholders::new().with_environment(true)) {
        assert_eq!(value.unwrap(), Value::from("from-environment"));
    }

    // The supplied mapping wins, so a test overrides without touching the
    // process it runs in.
    let overridden = Placeholders::new()
        .with_environment(true)
        .with_variable(NAME, Value::from("from-mapping"));
    for value in resolved(&scalar, overridden) {
        assert_eq!(value.unwrap(), Value::from("from-mapping"));
    }

    // SAFETY: the same reasoning as above.
    unsafe { std::env::remove_var(NAME) };
}

#[test]
fn a_document_without_placeholders_parses_identically_either_way() {
    let placeholders = Placeholders::new().with_variable("UNUSED", Value::from("x"));
    let on = Loading::new().with_placeholders(placeholders);
    let off = Loading::new();

    for (format, document) in [
        (
            Format::Json,
            r#"{"a": "plain", "b": [1, 2], "c": {"d": null}}"#,
        ),
        (Format::Yaml, "a: plain\nb:\n  - 1\n  - 2\nc:\n  d: null\n"),
        (Format::Toml, "a = \"plain\"\nb = [1, 2]\n[c]\n"),
    ] {
        assert_eq!(
            text::from_str_with(document, format, &on).unwrap(),
            text::from_str_with(document, format, &off).unwrap(),
        );
        // And a scalar that merely *mentions* a brace is not a placeholder.
        assert_eq!(
            text::from_str_with(r#"{"a": "a { b } c"}"#, Format::Json, &on).unwrap(),
            text::from_str_with(r#"{"a": "a { b } c"}"#, Format::Json, &off).unwrap(),
        );
    }
}

#[test]
fn every_entry_point_carries_the_same_loading() {
    let loading = Loading::new()
        .with_limits(Limits::default())
        .with_placeholders(Placeholders::new().with_variable("NAME", Value::from("app")));
    let document = r#"{"name": "{{ NAME }}"}"#;
    let expected = Value::from("app");

    let from_str = text::from_str_with(document, Format::Json, &loading).unwrap();
    let from_slice = text::from_slice_with(document.as_bytes(), Format::Json, &loading).unwrap();
    let from_reader =
        text::from_reader_with(std::io::Cursor::new(document), Format::Json, &loading).unwrap();
    assert_eq!(from_str, from_slice);
    assert_eq!(from_str, from_reader);
    assert_eq!(from_str.get_key_str("name"), Some(&expected));

    // Through a handle, with the coding peeled first: the guard and the
    // substitution both see the decoded document.
    let mut handle = Buffer::new().with_media_type(
        Url::from_str("file:///config.json.gz")
            .unwrap()
            .media_type(),
    );
    handle
        .write_all_bytes(&yggdryl::gzip::dump(document.as_bytes()).unwrap())
        .unwrap();
    let loaded = text::load_with(&handle, &loading).unwrap();
    assert_eq!(loaded.get_key_str("name"), Some(&expected));

    // Dumping never re-introduces a placeholder: substitution is a load-time
    // transformation, so a round trip yields the resolved document.
    let mut target =
        Buffer::new().with_media_type(Url::from_str("file:///out.json").unwrap().media_type());
    text::dump(&mut target, &loaded).unwrap();
    assert_eq!(target.read_all_bytes().unwrap(), br#"{"name":"app"}"#);
}

#[test]
fn an_unquoted_yaml_placeholder_is_what_yaml_says_it_is() {
    let loading = Loading::new()
        .with_placeholders(Placeholders::new().with_variable("PORT", Value::I64(8080)));

    // Quoted - the form the docs show - is a string scalar and resolves.
    let quoted = text::from_str_with("port: \"{{ PORT }}\"\n", Format::Yaml, &loading).unwrap();
    assert_eq!(quoted.get_key_str("port"), Some(&Value::I64(8080)));

    // Unquoted, YAML has already read `{{ PORT }}` as a flow mapping whose one
    // key is itself a flow mapping - there is no string scalar to substitute
    // into, and nothing here changes the document's shape to invent one.
    let bare = text::from_str_with("port: {{ PORT }}\n", Format::Yaml, &loading).unwrap();
    let port = bare.get_key_str("port").expect("a value under `port`");
    assert!(
        port.as_mapping().is_some(),
        "YAML read a flow mapping, not a placeholder: {port:?}"
    );
}
