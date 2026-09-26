//! `rust/src/xmla/vocabulary.rs`: the words XML for Analysis 1.1 defines - the
//! two methods, the enumerations a property draws on, the request types, the
//! standard property names, the `PropertyList` and the `RestrictionList` - each
//! read case-insensitively, written canonically, and refused by name.

use std::collections::{BTreeSet, HashSet};
use std::fmt;
use std::str::FromStr;

use yggdryl::Error;
use yggdryl::xmla::{
    Access, AuthenticationMode, AxisFormat, Content, Format, MdxSupport, Method, PropertyList,
    ProviderType, RequestType, Restrictions, StateSupport, property,
};

/// The path and the reason of the one refusal the vocabulary raises, after
/// checking the message is the canonical rendering of both.
fn refused<T: fmt::Debug>(result: yggdryl::Result<T>) -> (String, String) {
    let error = result.expect_err("a refusal");
    let message = error.to_string();
    match error {
        Error::InvalidRecord { path, reason } => {
            assert_eq!(
                message,
                format!("invalid record value at {path}: {reason}"),
                "the message renders the path and the reason"
            );
            (path.to_string(), reason.to_string())
        }
        other => panic!("expected an invalid record refusal, got {other:?}"),
    }
}

/// Pins one enumeration: its members in order with their spellings, every
/// spelling read back in any ASCII case and with padding, `Display` writing
/// the canonical spelling, and one distinct description per member.
fn pin_enumeration<T>(
    all: &[T],
    spelled: &[(T, &str)],
    as_str: fn(T) -> &'static str,
    description: fn(T) -> &'static str,
) where
    T: Copy + PartialEq + fmt::Debug + fmt::Display + FromStr<Err = Error>,
{
    assert_eq!(
        all.to_vec(),
        spelled
            .iter()
            .map(|(member, _)| *member)
            .collect::<Vec<_>>(),
        "ALL lists every member in the specification's order"
    );
    for &(member, spelling) in spelled {
        assert_eq!(as_str(member), spelling);
        assert_eq!(member.to_string(), spelling, "Display is the spelling");
        assert_eq!(spelling.parse::<T>().expect(spelling), member);
        assert_eq!(
            spelling.to_ascii_lowercase().parse::<T>().expect(spelling),
            member,
            "read case-insensitively"
        );
        assert_eq!(
            spelling.to_ascii_uppercase().parse::<T>().expect(spelling),
            member,
            "read case-insensitively"
        );
        assert_eq!(
            format!(" \t{spelling}\r\n").parse::<T>().expect(spelling),
            member,
            "surrounding whitespace is not part of the spelling"
        );
        assert!(!description(member).is_empty(), "{member:?} is described");
    }
    let mut descriptions = all
        .iter()
        .map(|member| description(*member))
        .collect::<Vec<_>>();
    descriptions.sort_unstable();
    descriptions.dedup();
    assert_eq!(
        descriptions.len(),
        all.len(),
        "each member says its own thing"
    );
}

// Refusals first.

#[test]
fn an_unknown_method_is_refused_naming_both_methods() {
    let (path, reason) = refused("Fetch".parse::<Method>());
    assert_eq!(path, "$.method");
    assert_eq!(reason, r#"expected Discover or Execute, got "Fetch""#);

    let (_, reason) = refused("".parse::<Method>());
    assert_eq!(reason, r#"expected Discover or Execute, got """#);

    let (_, reason) = refused("xmla:Discover".parse::<Method>());
    assert_eq!(
        reason, r#"expected Discover or Execute, got "xmla:Discover""#,
        "a prefixed name is not the method"
    );
    let (_, reason) = refused("Dis cover".parse::<Method>());
    assert_eq!(reason, r#"expected Discover or Execute, got "Dis cover""#);
    let (_, reason) = refused("DiscoverResponse".parse::<Method>());
    assert_eq!(
        reason, r#"expected Discover or Execute, got "DiscoverResponse""#,
        "the response element is not the method"
    );
}

#[test]
fn an_unknown_member_of_each_enumeration_is_refused_listing_every_member() {
    let (path, reason) = refused("Flat".parse::<Format>());
    assert_eq!(path, "$.Format");
    assert_eq!(
        reason,
        r#"expected Tabular, Multidimensional, Native, got "Flat""#
    );

    let (path, reason) = refused("Rows".parse::<Content>());
    assert_eq!(path, "$.Content");
    assert_eq!(
        reason,
        r#"expected None, Schema, Data, SchemaData, DataOmitDefaultSlicer, DataIncludeDefaultSlicer, got "Rows""#
    );

    let (path, reason) = refused("RowFormat".parse::<AxisFormat>());
    assert_eq!(path, "$.AxisFormat");
    assert_eq!(
        reason,
        r#"expected TupleFormat, ClusterFormat, CustomFormat, got "RowFormat""#
    );

    let (path, reason) = refused("OLAP".parse::<ProviderType>());
    assert_eq!(path, "$.ProviderType");
    assert_eq!(reason, r#"expected TDP, MDP, DMP, got "OLAP""#);

    let (path, reason) = refused("Kerberos".parse::<AuthenticationMode>());
    assert_eq!(path, "$.AuthenticationMode");
    assert_eq!(
        reason,
        r#"expected Unauthenticated, Authenticated, Integrated, got "Kerberos""#
    );

    let (path, reason) = refused("Execute".parse::<Access>());
    assert_eq!(
        path, "$.PropertyAccessType",
        "the refusal names the column the enumeration fills"
    );
    assert_eq!(reason, r#"expected Read, Write, ReadWrite, got "Execute""#);

    let (path, reason) = refused("Pooled".parse::<StateSupport>());
    assert_eq!(path, "$.StateSupport");
    assert_eq!(reason, r#"expected None, Sessions, got "Pooled""#);

    let (path, reason) = refused("Full".parse::<MdxSupport>());
    assert_eq!(path, "$.MDXSupport");
    assert_eq!(reason, r#"expected Core, got "Full""#);
}

#[test]
fn an_empty_or_blank_member_is_refused_rather_than_defaulted() {
    let (path, reason) = refused("".parse::<Format>());
    assert_eq!(path, "$.Format");
    assert_eq!(
        reason,
        r#"expected Tabular, Multidimensional, Native, got """#
    );
    let (_, reason) = refused(" \t\n".parse::<Content>());
    assert_eq!(
        reason,
        r#"expected None, Schema, Data, SchemaData, DataOmitDefaultSlicer, DataIncludeDefaultSlicer, got """#,
        "the refusal names the trimmed spelling"
    );
}

#[test]
fn a_member_spelled_with_inner_whitespace_or_a_lookalike_letter_is_refused() {
    let (_, reason) = refused("Schema Data".parse::<Content>());
    assert_eq!(
        reason,
        r#"expected None, Schema, Data, SchemaData, DataOmitDefaultSlicer, DataIncludeDefaultSlicer, got "Schema Data""#
    );
    // The second `a` is CYRILLIC SMALL LETTER A, which no ASCII fold reaches.
    let (_, reason) = refused("N\u{430}tive".parse::<Format>());
    assert_eq!(
        reason,
        "expected Tabular, Multidimensional, Native, got \"N\u{430}tive\""
    );
    let (_, reason) = refused("Read-Write".parse::<Access>());
    assert_eq!(
        reason,
        r#"expected Read, Write, ReadWrite, got "Read-Write""#
    );
}

#[test]
fn a_refused_spelling_is_quoted_and_escaped() {
    let (_, reason) = refused("Tab\"u\u{0}lar".parse::<Format>());
    assert_eq!(
        reason, r#"expected Tabular, Multidimensional, Native, got "Tab\"u\0lar""#,
        "a quote and a control character stay visible"
    );
}

#[test]
fn a_refused_spelling_longer_than_the_error_budget_is_elided() {
    let long = "x".repeat(200);
    let (_, reason) = refused(long.parse::<Format>());
    assert_eq!(
        reason,
        format!(
            "expected Tabular, Multidimensional, Native, got \"{}\u{2026}\"",
            "x".repeat(64)
        ),
        "sixty-four bytes of the value, then an ellipsis"
    );

    // Two bytes a character: the cut falls on a character boundary.
    let accented = "\u{e9}".repeat(40);
    let (_, reason) = refused(accented.parse::<Method>());
    assert_eq!(
        reason,
        format!(
            "expected Discover or Execute, got \"{}\u{2026}\"",
            "\u{e9}".repeat(32)
        )
    );

    // Three bytes a character: 64 is not a boundary, so 63 bytes are kept.
    let wide = "\u{20ac}".repeat(30);
    let (_, reason) = refused(wide.parse::<Content>());
    assert_eq!(
        reason,
        format!(
            "expected None, Schema, Data, SchemaData, DataOmitDefaultSlicer, DataIncludeDefaultSlicer, got \"{}\u{2026}\"",
            "\u{20ac}".repeat(21)
        )
    );

    let exact = "y".repeat(64);
    let (_, reason) = refused(exact.parse::<Format>());
    assert_eq!(
        reason,
        format!("expected Tabular, Multidimensional, Native, got \"{exact}\""),
        "a value within the budget is named whole"
    );
}

#[test]
fn an_empty_or_blank_request_type_is_refused() {
    let (path, reason) = refused("".parse::<RequestType>());
    assert_eq!(path, "$.RequestType");
    assert_eq!(reason, r#"expected a rowset name, got """#);

    let (path, reason) = refused(" \t\r\n".parse::<RequestType>());
    assert_eq!(path, "$.RequestType");
    assert_eq!(reason, r#"expected a rowset name, got """#);
}

#[test]
fn a_blank_or_lookalike_method_is_refused() {
    let (path, reason) = refused(" \t\r\n".parse::<Method>());
    assert_eq!(path, "$.method");
    assert_eq!(
        reason, r#"expected Discover or Execute, got """#,
        "the refusal names the trimmed spelling"
    );
    // The `i` is CYRILLIC SMALL LETTER BYELORUSSIAN-UKRAINIAN I.
    let (_, reason) = refused("D\u{456}scover".parse::<Method>());
    assert_eq!(
        reason,
        "expected Discover or Execute, got \"D\u{456}scover\""
    );
    let (_, reason) = refused("Execute\u{0}".parse::<Method>());
    assert_eq!(
        reason, r#"expected Discover or Execute, got "Execute\0""#,
        "a control character is not padding"
    );
}

#[test]
fn an_empty_timeout_is_set_and_refused_as_not_a_count_of_seconds() {
    // `timeout` answers `None` when the property is unset and refuses a value
    // that is not a count of seconds. An empty value is set - `get` answers
    // it - and is no count, exactly as a blank one is refused and as an empty
    // `Format` or `Content` is; only `catalog` and `data_source_info` state
    // that an empty value reads as absent.
    let list = PropertyList::new().with(property::TIMEOUT, "");
    assert_eq!(
        list.get(property::TIMEOUT),
        Some(""),
        "an empty timeout is set"
    );
    let (path, reason) = refused(list.timeout());
    assert_eq!(path, "$.Timeout");
    assert_eq!(reason, r#"expected a count of seconds, got """#);
}

#[test]
fn a_timeout_refusal_names_the_standard_property_and_elides_a_long_value() {
    let (path, reason) = refused(PropertyList::new().with("TIMEOUT", "soon").timeout());
    assert_eq!(
        path, "$.Timeout",
        "the standard property, not the case it was written in"
    );
    assert_eq!(reason, r#"expected a count of seconds, got "soon""#);

    let long = "9".repeat(200);
    let (_, reason) = refused(PropertyList::new().with(property::TIMEOUT, long).timeout());
    assert_eq!(
        reason,
        format!(
            "expected a count of seconds, got \"{}\u{2026}\"",
            "9".repeat(64)
        ),
        "sixty-four bytes of the value, then an ellipsis"
    );

    // FULLWIDTH DIGIT THREE and FULLWIDTH DIGIT ZERO: digits to Unicode, not
    // to a count of seconds.
    let (_, reason) = refused(
        PropertyList::new()
            .with(property::TIMEOUT, "\u{ff13}\u{ff10}")
            .timeout(),
    );
    assert_eq!(
        reason,
        "expected a count of seconds, got \"\u{ff13}\u{ff10}\""
    );
}

#[test]
fn a_property_list_names_a_refused_member_trimmed_under_the_standard_property() {
    let list = PropertyList::new()
        .with("FORMAT", "  Flat\t")
        .with("content", "\nRows ")
        .with("AXISFORMAT", " RowFormat ");
    let (path, reason) = refused(list.format());
    assert_eq!(path, "$.Format");
    assert_eq!(
        reason,
        r#"expected Tabular, Multidimensional, Native, got "Flat""#
    );
    let (path, reason) = refused(list.content());
    assert_eq!(path, "$.Content");
    assert_eq!(
        reason,
        r#"expected None, Schema, Data, SchemaData, DataOmitDefaultSlicer, DataIncludeDefaultSlicer, got "Rows""#
    );
    let (path, reason) = refused(list.axis_format());
    assert_eq!(path, "$.AxisFormat");
    assert_eq!(
        reason,
        r#"expected TupleFormat, ClusterFormat, CustomFormat, got "RowFormat""#
    );
}

#[test]
fn a_timeout_that_is_not_a_count_of_seconds_is_refused_naming_the_value() {
    let (path, reason) = refused(
        PropertyList::new()
            .with(property::TIMEOUT, "soon")
            .timeout(),
    );
    assert_eq!(path, "$.Timeout");
    assert_eq!(reason, r#"expected a count of seconds, got "soon""#);

    for spelled in ["-1", "1.5", "4294967296", "30s", "1e3", "0x10"] {
        let (path, reason) = refused(PropertyList::new().with("Timeout", spelled).timeout());
        assert_eq!(path, "$.Timeout", "{spelled}");
        assert_eq!(
            reason,
            format!("expected a count of seconds, got {spelled:?}"),
            "{spelled}"
        );
    }

    let (_, reason) = refused(PropertyList::new().with("Timeout", " 3 0 ").timeout());
    assert_eq!(
        reason, r#"expected a count of seconds, got " 3 0 ""#,
        "the refusal names the value as it was written"
    );
    let (_, reason) = refused(PropertyList::new().with("Timeout", "   ").timeout());
    assert_eq!(
        reason, r#"expected a count of seconds, got "   ""#,
        "a blank value is set, and is not a count"
    );
}

#[test]
fn a_property_list_refuses_an_enumeration_value_naming_it() {
    let list = PropertyList::new()
        .with(property::FORMAT, "Flat")
        .with(property::CONTENT, "Rows")
        .with(property::AXIS_FORMAT, "RowFormat");

    let (path, reason) = refused(list.format());
    assert_eq!(path, "$.Format");
    assert_eq!(
        reason,
        r#"expected Tabular, Multidimensional, Native, got "Flat""#
    );

    let (path, reason) = refused(list.content());
    assert_eq!(path, "$.Content");
    assert_eq!(
        reason,
        r#"expected None, Schema, Data, SchemaData, DataOmitDefaultSlicer, DataIncludeDefaultSlicer, got "Rows""#
    );

    let (path, reason) = refused(list.axis_format());
    assert_eq!(path, "$.AxisFormat");
    assert_eq!(
        reason,
        r#"expected TupleFormat, ClusterFormat, CustomFormat, got "RowFormat""#
    );
}

#[test]
fn an_empty_enumeration_property_is_set_and_refused_rather_than_defaulted() {
    let list = PropertyList::new().with("Format", "").with("Content", "");
    let (_, reason) = refused(list.format());
    assert_eq!(
        reason,
        r#"expected Tabular, Multidimensional, Native, got """#
    );
    let (_, reason) = refused(list.content());
    assert_eq!(
        reason,
        r#"expected None, Schema, Data, SchemaData, DataOmitDefaultSlicer, DataIncludeDefaultSlicer, got """#
    );
}

// Methods.

#[test]
fn the_methods_are_discover_then_execute() {
    assert_eq!(Method::ALL, [Method::Discover, Method::Execute]);
    assert_eq!(Method::Discover.as_str(), "Discover");
    assert_eq!(Method::Execute.as_str(), "Execute");
    assert_eq!(Method::Discover.to_string(), "Discover");
    assert_eq!(Method::Execute.to_string(), "Execute");
    assert!(Method::Discover < Method::Execute);
}

#[test]
fn each_method_answers_under_its_response_element() {
    assert_eq!(Method::Discover.response_name(), "DiscoverResponse");
    assert_eq!(Method::Execute.response_name(), "ExecuteResponse");
    for method in Method::ALL {
        assert_eq!(
            method.response_name(),
            format!("{}Response", method.as_str())
        );
    }
}

#[test]
fn each_method_carries_its_soap_action() {
    assert_eq!(
        Method::Discover.soap_action(),
        "urn:schemas-microsoft-com:xml-analysis:Discover"
    );
    assert_eq!(
        Method::Execute.soap_action(),
        "urn:schemas-microsoft-com:xml-analysis:Execute"
    );
    for method in Method::ALL {
        assert_eq!(
            method.soap_action(),
            format!("{}:{}", yggdryl::xmla::NAMESPACE, method.as_str()),
            "the action is the method in the XMLA namespace"
        );
    }
}

#[test]
fn a_method_reads_case_insensitively_and_trimmed() {
    for method in Method::ALL {
        let spelled = method.as_str();
        assert_eq!(spelled.parse::<Method>().unwrap(), method);
        assert_eq!(
            spelled.to_ascii_lowercase().parse::<Method>().unwrap(),
            method
        );
        assert_eq!(
            spelled.to_ascii_uppercase().parse::<Method>().unwrap(),
            method
        );
        assert_eq!(
            format!("\n  {spelled}\t").parse::<Method>().unwrap(),
            method
        );
        assert_eq!(method.to_string().parse::<Method>().unwrap(), method);
    }
    assert_eq!("dIsCoVeR".parse::<Method>().unwrap(), Method::Discover);
}

// Enumerations.

#[test]
fn the_format_enumeration_is_the_specifications() {
    pin_enumeration(
        Format::ALL,
        &[
            (Format::Tabular, "Tabular"),
            (Format::Multidimensional, "Multidimensional"),
            (Format::Native, "Native"),
        ],
        Format::as_str,
        Format::description,
    );
    assert_eq!(Format::DEFAULT, Format::Native);
    assert_eq!(
        Format::Tabular.description(),
        "A flat or hierarchical rowset, the one shape a Discover answers."
    );
    assert_eq!(
        Format::Multidimensional.description(),
        "An MDDataSet, the shape a multidimensional Execute answers."
    );
    assert_eq!(
        Format::Native.description(),
        "Whatever shape suits the request; the result's namespace says which."
    );
}

#[test]
fn the_content_enumeration_is_the_specifications() {
    pin_enumeration(
        Content::ALL,
        &[
            (Content::None, "None"),
            (Content::Schema, "Schema"),
            (Content::Data, "Data"),
            (Content::SchemaData, "SchemaData"),
            (Content::DataOmitDefaultSlicer, "DataOmitDefaultSlicer"),
            (
                Content::DataIncludeDefaultSlicer,
                "DataIncludeDefaultSlicer",
            ),
        ],
        Content::as_str,
        Content::description,
    );
    assert_eq!(Content::DEFAULT, Content::SchemaData);
    assert_eq!(
        Content::None.description(),
        "Verify the command, execute nothing, answer no rows."
    );
    assert_eq!(
        Content::Schema.description(),
        "The XML Schema describing the result's columns, and no rows."
    );
    assert_eq!(Content::Data.description(), "The rows, and no schema.");
    assert_eq!(
        Content::SchemaData.description(),
        "The schema, then the rows: the default."
    );
}

#[test]
fn a_content_says_whether_it_carries_the_schema_and_the_rows() {
    let table = [
        (Content::None, false, false),
        (Content::Schema, true, false),
        (Content::Data, false, true),
        (Content::SchemaData, true, true),
        // The two slicer contents are rows and no schema: a multidimensional
        // result's default slicer is nothing a rowset has.
        (Content::DataOmitDefaultSlicer, false, true),
        (Content::DataIncludeDefaultSlicer, false, true),
    ];
    assert_eq!(table.len(), Content::ALL.len(), "every member is stated");
    for (content, schema, data) in table {
        assert_eq!(content.has_schema(), schema, "{content} has_schema");
        assert_eq!(content.has_data(), data, "{content} has_data");
    }
    assert!(
        Content::DEFAULT.has_schema() && Content::DEFAULT.has_data(),
        "the default carries both"
    );
}

#[test]
fn the_axis_format_enumeration_is_the_specifications() {
    pin_enumeration(
        AxisFormat::ALL,
        &[
            (AxisFormat::TupleFormat, "TupleFormat"),
            (AxisFormat::ClusterFormat, "ClusterFormat"),
            (AxisFormat::CustomFormat, "CustomFormat"),
        ],
        AxisFormat::as_str,
        AxisFormat::description,
    );
    assert_eq!(
        AxisFormat::TupleFormat.description(),
        "Each axis as its tuples: the default."
    );
    assert_eq!(
        AxisFormat::ClusterFormat.description(),
        "Each axis as clusters of members."
    );
    assert_eq!(
        AxisFormat::CustomFormat.description(),
        "A layout the provider defines."
    );
}

#[test]
fn the_provider_type_enumeration_is_the_specifications() {
    pin_enumeration(
        ProviderType::ALL,
        &[
            (ProviderType::Tdp, "TDP"),
            (ProviderType::Mdp, "MDP"),
            (ProviderType::Dmp, "DMP"),
        ],
        ProviderType::as_str,
        ProviderType::description,
    );
    assert_eq!("tdp".parse::<ProviderType>().unwrap(), ProviderType::Tdp);
    assert_eq!(
        ProviderType::Tdp.description(),
        "A tabular data provider: rows answered to a text command."
    );
    assert_eq!(
        ProviderType::Mdp.description(),
        "A multidimensional data provider: cubes answered to MDX."
    );
    assert_eq!(
        ProviderType::Dmp.description(),
        "A data mining provider, per OLE DB for Data Mining."
    );
}

#[test]
fn the_authentication_mode_enumeration_is_the_specifications() {
    pin_enumeration(
        AuthenticationMode::ALL,
        &[
            (AuthenticationMode::Unauthenticated, "Unauthenticated"),
            (AuthenticationMode::Authenticated, "Authenticated"),
            (AuthenticationMode::Integrated, "Integrated"),
        ],
        AuthenticationMode::as_str,
        AuthenticationMode::description,
    );
    assert_eq!(
        AuthenticationMode::Unauthenticated.description(),
        "No user name or password is sent."
    );
    assert_eq!(
        AuthenticationMode::Authenticated.description(),
        "A user name and password are sent in the properties."
    );
    assert_eq!(
        AuthenticationMode::Integrated.description(),
        "The transport authenticates: HTTP or the operating system."
    );
}

#[test]
fn the_property_access_enumeration_is_the_specifications() {
    pin_enumeration(
        Access::ALL,
        &[
            (Access::Read, "Read"),
            (Access::Write, "Write"),
            (Access::ReadWrite, "ReadWrite"),
        ],
        Access::as_str,
        Access::description,
    );
    assert_eq!(
        Access::Read.description(),
        "The provider answers the property; a client cannot set it."
    );
    assert_eq!(
        Access::Write.description(),
        "A client sets the property; the provider does not answer it."
    );
    assert_eq!(
        Access::ReadWrite.description(),
        "Set by the client and answered by the provider."
    );
}

#[test]
fn the_state_support_enumeration_is_the_specifications() {
    pin_enumeration(
        StateSupport::ALL,
        &[
            (StateSupport::None, "None"),
            (StateSupport::Sessions, "Sessions"),
        ],
        StateSupport::as_str,
        StateSupport::description,
    );
    assert_eq!(
        StateSupport::None.description(),
        "No sessions: every request stands alone."
    );
    assert_eq!(
        StateSupport::Sessions.description(),
        "BeginSession, Session and EndSession headers are honoured."
    );
}

#[test]
fn the_mdx_support_enumeration_is_the_specifications() {
    pin_enumeration(
        MdxSupport::ALL,
        &[(MdxSupport::Core, "Core")],
        MdxSupport::as_str,
        MdxSupport::description,
    );
    assert_eq!(
        MdxSupport::Core.description(),
        "The core MDX grammar, the one value the specification defines."
    );
}

#[test]
fn members_order_as_the_specification_lists_them() {
    fn ascending<T: Ord + fmt::Debug>(all: &[T]) {
        assert!(
            all.windows(2).all(|pair| pair[0] < pair[1]),
            "{all:?} sorts in the order it is listed"
        );
    }
    ascending(&Method::ALL);
    ascending(Format::ALL);
    ascending(Content::ALL);
    ascending(AxisFormat::ALL);
    ascending(ProviderType::ALL);
    ascending(AuthenticationMode::ALL);
    ascending(Access::ALL);
    ascending(StateSupport::ALL);
    ascending(MdxSupport::ALL);
    ascending(RequestType::ALL);

    let sorted = [
        "zz_rowset",
        "MDSCHEMA_SETS",
        "B_ROWSET",
        "discover_datasources",
        "A_ROWSET",
        "DBSCHEMA_TABLES",
    ]
    .into_iter()
    .map(|spelled| spelled.parse::<RequestType>().unwrap())
    .collect::<BTreeSet<_>>();
    assert_eq!(
        sorted.iter().map(RequestType::as_str).collect::<Vec<_>>(),
        [
            "DISCOVER_DATASOURCES",
            "DBSCHEMA_TABLES",
            "MDSCHEMA_SETS",
            "A_ROWSET",
            "B_ROWSET",
            "zz_rowset"
        ],
        "the defined request types in the specification's order, then the undefined ones by spelling"
    );
}

// Request types.

/// Every request type the specification defines, as it spells each one.
const REQUEST_TYPES: [(RequestType, &str); 26] = [
    (RequestType::DiscoverDatasources, "DISCOVER_DATASOURCES"),
    (RequestType::DiscoverProperties, "DISCOVER_PROPERTIES"),
    (
        RequestType::DiscoverSchemaRowsets,
        "DISCOVER_SCHEMA_ROWSETS",
    ),
    (RequestType::DiscoverEnumerators, "DISCOVER_ENUMERATORS"),
    (RequestType::DiscoverKeywords, "DISCOVER_KEYWORDS"),
    (RequestType::DiscoverLiterals, "DISCOVER_LITERALS"),
    (RequestType::DbschemaCatalogs, "DBSCHEMA_CATALOGS"),
    (RequestType::DbschemaColumns, "DBSCHEMA_COLUMNS"),
    (
        RequestType::DbschemaProviderTypes,
        "DBSCHEMA_PROVIDER_TYPES",
    ),
    (RequestType::DbschemaSchemata, "DBSCHEMA_SCHEMATA"),
    (RequestType::DbschemaTables, "DBSCHEMA_TABLES"),
    (RequestType::DbschemaTablesInfo, "DBSCHEMA_TABLES_INFO"),
    (RequestType::MdschemaActions, "MDSCHEMA_ACTIONS"),
    (RequestType::MdschemaCubes, "MDSCHEMA_CUBES"),
    (RequestType::MdschemaDimensions, "MDSCHEMA_DIMENSIONS"),
    (RequestType::MdschemaFunctions, "MDSCHEMA_FUNCTIONS"),
    (RequestType::MdschemaHierarchies, "MDSCHEMA_HIERARCHIES"),
    (RequestType::MdschemaLevels, "MDSCHEMA_LEVELS"),
    (RequestType::MdschemaMeasures, "MDSCHEMA_MEASURES"),
    (RequestType::MdschemaMembers, "MDSCHEMA_MEMBERS"),
    (RequestType::MdschemaProperties, "MDSCHEMA_PROPERTIES"),
    (RequestType::MdschemaSets, "MDSCHEMA_SETS"),
    (RequestType::MdschemaKpis, "MDSCHEMA_KPIS"),
    (RequestType::MdschemaMeasuregroups, "MDSCHEMA_MEASUREGROUPS"),
    (
        RequestType::MdschemaMeasuregroupDimensions,
        "MDSCHEMA_MEASUREGROUP_DIMENSIONS",
    ),
    (
        RequestType::MdschemaInputDatasources,
        "MDSCHEMA_INPUT_DATASOURCES",
    ),
];

#[test]
fn each_request_type_is_spelled_as_the_specification_spells_it() {
    assert_eq!(
        RequestType::ALL.to_vec(),
        REQUEST_TYPES
            .iter()
            .map(|(known, _)| known.clone())
            .collect::<Vec<_>>(),
        "ALL lists every defined request type in the specification's order"
    );
    for (known, spelled) in &REQUEST_TYPES {
        assert_eq!(known.as_str(), *spelled);
        assert_eq!(known.to_string(), *spelled, "Display is the spelling");
    }
    assert!(
        !RequestType::ALL
            .iter()
            .any(|known| matches!(known, RequestType::Other(_))),
        "ALL holds only the defined request types"
    );
}

#[test]
fn every_defined_request_type_reads_back_in_any_case_and_padding() {
    for (known, spelled) in &REQUEST_TYPES {
        assert_eq!(&spelled.parse::<RequestType>().unwrap(), known);
        assert_eq!(
            &spelled.to_ascii_lowercase().parse::<RequestType>().unwrap(),
            known,
            "read case-insensitively and written canonically"
        );
        assert_eq!(
            spelled
                .to_ascii_lowercase()
                .parse::<RequestType>()
                .unwrap()
                .as_str(),
            *spelled
        );
        assert_eq!(
            &format!("\t {spelled}\r\n").parse::<RequestType>().unwrap(),
            known
        );
        assert_eq!(&known.to_string().parse::<RequestType>().unwrap(), known);
    }
    assert_eq!(
        "Discover_Schema_Rowsets".parse::<RequestType>().unwrap(),
        RequestType::DiscoverSchemaRowsets
    );
}

#[test]
fn an_undefined_request_type_is_kept_as_spelled() {
    let other = " MY_ROWSET ".parse::<RequestType>().unwrap();
    assert_eq!(other, RequestType::Other("MY_ROWSET".into()));
    assert_eq!(other.as_str(), "MY_ROWSET", "trimmed, never refused");
    assert_eq!(other.to_string(), "MY_ROWSET");

    let mixed = "custom_Rowset".parse::<RequestType>().unwrap();
    assert_eq!(
        mixed.as_str(),
        "custom_Rowset",
        "an undefined name keeps its own case"
    );

    let unicode = "ROWSET_\u{3a9}_\u{e9}t\u{e9}"
        .parse::<RequestType>()
        .unwrap();
    assert_eq!(unicode.as_str(), "ROWSET_\u{3a9}_\u{e9}t\u{e9}");

    let near = "DISCOVER_DATASOURCE".parse::<RequestType>().unwrap();
    assert_eq!(
        near,
        RequestType::Other("DISCOVER_DATASOURCE".into()),
        "a near miss is not the defined request type"
    );
    let near = "DISCOVER DATASOURCES".parse::<RequestType>().unwrap();
    assert_eq!(near, RequestType::Other("DISCOVER DATASOURCES".into()));
}

#[test]
fn an_undefined_request_type_is_kept_whole_however_long() {
    let long = format!("PROVIDER_{}", "X".repeat(500));
    let other = long.parse::<RequestType>().unwrap();
    assert_eq!(other.as_str(), long, "never elided, never refused");
    assert_eq!(other.to_string(), long);
}

#[test]
fn undefined_request_types_differing_only_in_case_are_two_spellings_in_one_family() {
    let upper = "DISCOVER_XML_METADATA".parse::<RequestType>().unwrap();
    let lower = "discover_xml_metadata".parse::<RequestType>().unwrap();
    assert_ne!(upper, lower, "an undefined name is kept as it was spelled");
    assert!(upper.is_discover() && lower.is_discover());
    assert_eq!(lower.as_str(), "discover_xml_metadata");
}

#[test]
fn a_request_type_hashes_as_the_value_it_reads_as() {
    let mut seen = HashSet::new();
    for spelled in ["DBSCHEMA_TABLES", "dbschema_tables", " Dbschema_Tables\n"] {
        seen.insert(spelled.parse::<RequestType>().unwrap());
    }
    assert_eq!(
        seen.len(),
        1,
        "a defined request type is one key in any case"
    );
    assert!(seen.contains(&RequestType::DbschemaTables));
    for spelled in ["MY_ROWSET", "my_rowset", " MY_ROWSET "] {
        seen.insert(spelled.parse::<RequestType>().unwrap());
    }
    assert_eq!(seen.len(), 3, "an undefined one is keyed by its spelling");
    assert!(seen.contains(&RequestType::Other("my_rowset".into())));
}

#[test]
fn reading_a_hand_built_request_type_back_canonicalizes_it() {
    let hand_built = RequestType::Other("dbschema_tables".into());
    assert_eq!(hand_built.as_str(), "dbschema_tables", "built as spelled");
    assert!(hand_built.is_dbschema());
    let read = hand_built.to_string().parse::<RequestType>().unwrap();
    assert_eq!(
        read,
        RequestType::DbschemaTables,
        "the text door is what makes a defined name canonical"
    );
    assert_eq!(read.as_str(), "DBSCHEMA_TABLES");
}

#[test]
fn a_family_is_its_whole_prefix_at_the_opening_of_the_name() {
    for spelled in [
        "DISCOVER",
        "DB",
        "MDSCHEMA",
        "X_DISCOVER_Y",
        "DISCOVERX_Y",
        "MD_SCHEMA_CUBES",
        "DBSCHEMA-TABLES",
        "_DISCOVER_X",
    ] {
        let other = spelled.parse::<RequestType>().unwrap();
        assert!(
            !other.is_discover() && !other.is_dbschema() && !other.is_mdschema(),
            "{spelled} opens with no family's prefix"
        );
    }
    for (spelled, discover, dbschema, mdschema) in [
        ("discover_", true, false, false),
        ("DbSchema_", false, true, false),
        ("MDSCHEMA_", false, false, true),
    ] {
        let bare = spelled.parse::<RequestType>().unwrap();
        assert_eq!(
            (bare.is_discover(), bare.is_dbschema(), bare.is_mdschema()),
            (discover, dbschema, mdschema),
            "{spelled}: the prefix alone opens with the prefix"
        );
    }
}

#[test]
fn a_non_ascii_opening_is_in_no_family_and_is_classified_without_panicking() {
    let hand_built_empty = RequestType::Other("".into());
    let spellings = [
        // `\u{e9}` is two bytes and straddles the prefix's ninth byte.
        "DISCOVER\u{e9}X",
        // LATIN CAPITAL LETTER I WITH DOT ABOVE folds to `i` in Unicode alone.
        "D\u{130}SCOVER_X",
        // CYRILLIC CAPITAL LETTER ES looks like `C`.
        "DBS\u{421}HEMA_TABLES",
        "MDSCHEM\u{c5}_CUBES",
        "\u{e9}",
        "\u{1f4c8}\u{1f4c8}\u{1f4c8}",
    ];
    for other in spellings
        .into_iter()
        .map(|spelled| spelled.parse::<RequestType>().unwrap())
        .chain([hand_built_empty])
    {
        assert!(matches!(other, RequestType::Other(_)));
        assert!(
            !other.is_discover() && !other.is_dbschema() && !other.is_mdschema(),
            "{other:?} is in no family"
        );
    }
}

#[test]
fn each_defined_request_type_is_in_exactly_one_family() {
    let mut families = [0usize; 3];
    for (known, spelled) in &REQUEST_TYPES {
        let flags = [
            known.is_discover(),
            known.is_dbschema(),
            known.is_mdschema(),
        ];
        assert_eq!(
            flags.iter().filter(|flag| **flag).count(),
            1,
            "{spelled} is in one family"
        );
        for (count, flag) in families.iter_mut().zip(flags) {
            *count += usize::from(flag);
        }
        assert_eq!(known.is_discover(), spelled.starts_with("DISCOVER_"));
        assert_eq!(known.is_dbschema(), spelled.starts_with("DBSCHEMA_"));
        assert_eq!(known.is_mdschema(), spelled.starts_with("MDSCHEMA_"));
    }
    assert_eq!(
        families,
        [6, 6, 14],
        "six provider rowsets, six OLE DB rowsets, fourteen OLE DB for OLAP rowsets"
    );
    assert!(RequestType::DiscoverSchemaRowsets.is_discover());
    assert!(RequestType::DbschemaTablesInfo.is_dbschema());
    assert!(RequestType::MdschemaInputDatasources.is_mdschema());
}

#[test]
fn an_undefined_request_type_is_classified_by_its_prefix() {
    let discover = "DISCOVER_XML_METADATA".parse::<RequestType>().unwrap();
    assert!(matches!(discover, RequestType::Other(_)));
    assert!(discover.is_discover());
    assert!(!discover.is_dbschema() && !discover.is_mdschema());

    let dbschema = "DBSCHEMA_INDEXES".parse::<RequestType>().unwrap();
    assert!(dbschema.is_dbschema());
    assert!(!dbschema.is_discover() && !dbschema.is_mdschema());

    let mdschema = "MDSCHEMA_MEMBERS_EXTRA".parse::<RequestType>().unwrap();
    assert!(mdschema.is_mdschema());
    assert!(!mdschema.is_discover() && !mdschema.is_dbschema());

    let neither = "MY_ROWSET".parse::<RequestType>().unwrap();
    assert!(!neither.is_discover() && !neither.is_dbschema() && !neither.is_mdschema());

    let bare = "DISCOVER".parse::<RequestType>().unwrap();
    assert!(
        !bare.is_discover(),
        "the family is the prefix with its underscore"
    );
}

#[test]
fn an_undefined_request_type_is_classified_by_its_prefix_in_any_case() {
    // A name is read case-insensitively: `discover_xml_metadata` and
    // `DISCOVER_XML_METADATA` are one request type, so they are in one
    // family, the way `discover_datasources` is the defined provider rowset.
    assert!(
        "discover_datasources"
            .parse::<RequestType>()
            .unwrap()
            .is_discover()
    );
    let lower = "discover_xml_metadata".parse::<RequestType>().unwrap();
    assert!(
        lower.is_discover(),
        "{lower} is a DISCOVER_* rowset read case-insensitively"
    );
    let lower = "dbschema_indexes".parse::<RequestType>().unwrap();
    assert!(lower.is_dbschema(), "{lower} is a DBSCHEMA_* rowset");
    let lower = "Mdschema_Members_Extra".parse::<RequestType>().unwrap();
    assert!(lower.is_mdschema(), "{lower} is an MDSCHEMA_* rowset");
}

// Property names.

#[test]
fn the_standard_property_names_are_spelled_as_the_specification_spells_them() {
    let names = [
        (property::AXIS_FORMAT, "AxisFormat"),
        (property::BEGIN_RANGE, "BeginRange"),
        (property::CATALOG, "Catalog"),
        (property::CONTENT, "Content"),
        (property::CUBE, "Cube"),
        (property::DATA_SOURCE_INFO, "DataSourceInfo"),
        (property::DBMS_VERSION, "DBMSVersion"),
        (property::END_RANGE, "EndRange"),
        (property::FORMAT, "Format"),
        (property::LOCALE_IDENTIFIER, "LocaleIdentifier"),
        (property::MDX_SUPPORT, "MDXSupport"),
        (property::PASSWORD, "Password"),
        (property::PROVIDER_NAME, "ProviderName"),
        (property::PROVIDER_VERSION, "ProviderVersion"),
        (property::STATE_SUPPORT, "StateSupport"),
        (property::TIMEOUT, "Timeout"),
        (property::USER_NAME, "UserName"),
        (property::VISUAL_MODE, "VisualMode"),
    ];
    for (constant, spelled) in names {
        assert_eq!(constant, spelled);
    }
    let mut distinct = names
        .iter()
        .map(|(constant, _)| constant.to_ascii_lowercase())
        .collect::<Vec<_>>();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        names.len(),
        "no two names fold to one property"
    );
}

// Property lists.

/// A list's entries as name and value pairs.
fn properties(list: &PropertyList) -> Vec<(&str, &str)> {
    list.entries()
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect()
}

#[test]
fn a_new_property_list_holds_nothing() {
    let list = PropertyList::new();
    assert!(list.is_empty());
    assert_eq!(list.len(), 0);
    assert!(list.entries().is_empty());
    assert_eq!(list.get(property::CATALOG), None);
    assert_eq!(
        PropertyList::default(),
        list,
        "the default is the empty list"
    );
}

#[test]
fn setting_a_property_appends_it_in_the_order_written() {
    let mut list = PropertyList::new();
    list.set(property::CATALOG, "market");
    list.set(property::FORMAT, "Tabular");
    list.set("ProviderSpecific", "on");
    assert_eq!(
        properties(&list),
        [
            ("Catalog", "market"),
            ("Format", "Tabular"),
            ("ProviderSpecific", "on")
        ],
        "a property the specification does not define travels too"
    );
    assert!(!list.is_empty());
    assert_eq!(list.len(), 3);
}

#[test]
fn setting_a_property_again_replaces_its_value_in_place() {
    let mut list = PropertyList::new()
        .with("catalog", "market")
        .with("Format", "Tabular")
        .with("Content", "Data");
    list.set("CATALOG", "archive");
    assert_eq!(
        properties(&list),
        [
            ("catalog", "archive"),
            ("Format", "Tabular"),
            ("Content", "Data")
        ],
        "the name keeps the spelling it was first written with, and its place"
    );
    assert_eq!(list.len(), 3, "each property once");
    list.set(String::from("format"), String::from("Native"));
    assert_eq!(list.get("Format"), Some("Native"));
    assert_eq!(list.len(), 3);
}

#[test]
fn a_property_is_read_by_name_in_any_case() {
    let list = PropertyList::new().with("DataSourceInfo", "Provider=yggdryl");
    assert_eq!(list.get("DataSourceInfo"), Some("Provider=yggdryl"));
    assert_eq!(list.get("datasourceinfo"), Some("Provider=yggdryl"));
    assert_eq!(list.get("DATASOURCEINFO"), Some("Provider=yggdryl"));
    assert_eq!(list.get("DataSource"), None, "a prefix is not the name");
    assert_eq!(list.get(""), None);
}

#[test]
fn with_answers_the_list_with_the_property_set() {
    let list = PropertyList::new()
        .with(property::CATALOG, "market")
        .with(property::TIMEOUT, "30")
        .with(property::CATALOG, "archive");
    assert_eq!(
        properties(&list),
        [("Catalog", "archive"), ("Timeout", "30")]
    );
}

#[test]
fn a_property_value_is_kept_as_written() {
    let list = PropertyList::new()
        .with("Comment", "  padded  ")
        .with("Label", "\u{e9}t\u{e9} \u{1f4c8}")
        .with("Empty", "");
    assert_eq!(list.get("Comment"), Some("  padded  "));
    assert_eq!(list.get("Label"), Some("\u{e9}t\u{e9} \u{1f4c8}"));
    assert_eq!(list.get("Empty"), Some(""), "an empty value is still set");
    assert_eq!(list.len(), 3);
}

#[test]
fn removing_a_property_answers_its_value_and_keeps_the_rest_in_order() {
    let mut list = PropertyList::new()
        .with("Catalog", "market")
        .with("Format", "Tabular")
        .with("Content", "Data");
    assert_eq!(list.remove("FORMAT"), Some(String::from("Tabular")));
    assert_eq!(
        properties(&list),
        [("Catalog", "market"), ("Content", "Data")]
    );
    assert_eq!(list.remove("Format"), None, "a removed property is gone");
    assert_eq!(
        list.remove("Cube"),
        None,
        "an unset property removes nothing"
    );
    assert_eq!(list.len(), 2);
    assert_eq!(list.remove("catalog"), Some(String::from("market")));
    assert_eq!(list.remove("Content"), Some(String::from("Data")));
    assert!(list.is_empty());
    assert_eq!(list, PropertyList::new());
}

#[test]
fn a_property_list_collects_from_pairs_each_name_once() {
    let list: PropertyList = [
        ("Catalog", "market"),
        ("Format", "Tabular"),
        ("catalog", "archive"),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        properties(&list),
        [("Catalog", "archive"), ("Format", "Tabular")],
        "a repeated name replaces the value it held"
    );

    let owned: PropertyList = vec![(String::from("Timeout"), String::from("5"))]
        .into_iter()
        .collect();
    assert_eq!(owned.get("timeout"), Some("5"));

    let empty: PropertyList = std::iter::empty::<(&str, &str)>().collect();
    assert_eq!(empty, PropertyList::new());
}

#[test]
fn an_unset_enumeration_property_reads_as_its_default() {
    let list = PropertyList::new();
    assert_eq!(list.format().unwrap(), Format::DEFAULT);
    assert_eq!(list.format().unwrap(), Format::Native);
    assert_eq!(list.content().unwrap(), Content::DEFAULT);
    assert_eq!(list.content().unwrap(), Content::SchemaData);
    assert_eq!(list.axis_format().unwrap(), AxisFormat::TupleFormat);
}

#[test]
fn a_set_enumeration_property_reads_as_its_member() {
    let list = PropertyList::new()
        .with("format", " tabular ")
        .with("CONTENT", "SCHEMA")
        .with("axisformat", "clusterformat");
    assert_eq!(
        list.format().unwrap(),
        Format::Tabular,
        "the name and the value are both read case-insensitively"
    );
    assert_eq!(list.content().unwrap(), Content::Schema);
    assert_eq!(list.axis_format().unwrap(), AxisFormat::ClusterFormat);

    for format in Format::ALL {
        let list = PropertyList::new().with(property::FORMAT, format.as_str());
        assert_eq!(list.format().unwrap(), *format);
    }
    for content in Content::ALL {
        let list = PropertyList::new().with(property::CONTENT, content.to_string());
        assert_eq!(list.content().unwrap(), *content);
    }
    for axis in AxisFormat::ALL {
        let list = PropertyList::new().with(property::AXIS_FORMAT, axis.as_str());
        assert_eq!(list.axis_format().unwrap(), *axis);
    }
}

#[test]
fn the_catalog_and_the_data_source_are_absent_when_unset_or_empty() {
    let unset = PropertyList::new();
    assert_eq!(unset.catalog(), None);
    assert_eq!(unset.data_source_info(), None);

    let empty = PropertyList::new()
        .with(property::CATALOG, "")
        .with(property::DATA_SOURCE_INFO, "");
    assert_eq!(empty.catalog(), None);
    assert_eq!(empty.data_source_info(), None);

    let set = PropertyList::new()
        .with("CATALOG", "market")
        .with("datasourceinfo", "Provider=yggdryl;Data Source=local");
    assert_eq!(set.catalog(), Some("market"));
    assert_eq!(
        set.data_source_info(),
        Some("Provider=yggdryl;Data Source=local")
    );

    let unicode = PropertyList::new().with(property::CATALOG, "march\u{e9}");
    assert_eq!(unicode.catalog(), Some("march\u{e9}"));
}

#[test]
fn a_timeout_reads_as_a_count_of_seconds() {
    assert_eq!(PropertyList::new().timeout().unwrap(), None, "unset");
    let read = |spelled: &str| {
        PropertyList::new()
            .with(property::TIMEOUT, spelled)
            .timeout()
            .unwrap()
    };
    assert_eq!(read("30"), Some(30));
    assert_eq!(read(" 30\n"), Some(30), "surrounding whitespace is dropped");
    assert_eq!(read("0"), Some(0));
    assert_eq!(read("007"), Some(7));
    assert_eq!(read("4294967295"), Some(u32::MAX));
    assert_eq!(
        PropertyList::new().with("TIMEOUT", "5").timeout().unwrap(),
        Some(5),
        "the name is read case-insensitively"
    );
}

#[test]
fn a_blank_catalog_or_data_source_is_kept_as_written() {
    let list = PropertyList::new()
        .with(property::CATALOG, "   ")
        .with(property::DATA_SOURCE_INFO, "\t");
    assert_eq!(list.catalog(), Some("   "), "only an empty value is absent");
    assert_eq!(list.data_source_info(), Some("\t"));
}

#[test]
fn a_property_name_folds_ascii_case_and_nothing_else() {
    let mut list = PropertyList::new().with("\u{dc}nit", "upper");
    list.set("\u{fc}nit", "lower");
    assert_eq!(
        properties(&list),
        [("\u{dc}nit", "upper"), ("\u{fc}nit", "lower")],
        "a non-ASCII letter in another case is another name"
    );
    list.set(" Format", "Tabular");
    assert_eq!(list.get("Format"), None, "padding is part of a name");
    assert_eq!(list.get(" format"), Some("Tabular"));
    list.set("", "unnamed");
    assert_eq!(list.get(""), Some("unnamed"), "an empty name is a name");
    assert_eq!(list.len(), 4);
}

#[test]
fn a_property_list_equals_one_written_alike() {
    let written = PropertyList::new()
        .with("Catalog", "market")
        .with("Format", "Tabular");
    assert_eq!(written.clone(), written);
    assert_eq!(
        written,
        [("Catalog", "market"), ("Format", "Tabular")]
            .into_iter()
            .collect::<PropertyList>()
    );
    assert_eq!(
        written,
        PropertyList::new()
            .with("Format", "Tabular")
            .with("Catalog", "market"),
        "the order is not part of the list: a document's element view holds children by name"
    );
    assert_ne!(
        written,
        PropertyList::new()
            .with("Catalog", "archive")
            .with("Format", "Tabular")
    );
}

// Restrictions.

/// The restrictions as column names and the values each admits.
fn restricted(restrictions: &Restrictions) -> Vec<(&str, Vec<&str>)> {
    restrictions
        .entries()
        .iter()
        .map(|(name, values)| {
            (
                name.as_str(),
                values.iter().map(String::as_str).collect::<Vec<_>>(),
            )
        })
        .collect()
}

#[test]
fn new_restrictions_restrict_nothing() {
    let restrictions = Restrictions::new();
    assert!(restrictions.is_empty());
    assert_eq!(restrictions.len(), 0);
    assert!(restrictions.entries().is_empty());
    assert_eq!(restrictions.get("TABLE_NAME"), None);
    assert_eq!(Restrictions::default(), restrictions);
}

#[test]
fn a_restriction_written_once_admits_one_value() {
    let mut restrictions = Restrictions::new();
    restrictions.push("TABLE_NAME", "trades");
    restrictions.push("TABLE_SCHEMA", "market");
    assert_eq!(
        restricted(&restrictions),
        [
            ("TABLE_NAME", vec!["trades"]),
            ("TABLE_SCHEMA", vec!["market"])
        ]
    );
    assert_eq!(
        restrictions.get("TABLE_NAME"),
        Some(&[String::from("trades")][..])
    );
    assert_eq!(restrictions.len(), 2);
    assert!(!restrictions.is_empty());
}

#[test]
fn a_restriction_written_again_admits_each_value_in_order() {
    let mut restrictions = Restrictions::new();
    restrictions.push("TABLE_NAME", "trades");
    restrictions.push("TABLE_SCHEMA", "market");
    restrictions.push("table_name", "fills");
    restrictions.push(String::from("Table_Name"), String::from("trades"));
    assert_eq!(
        restricted(&restrictions),
        [
            ("TABLE_NAME", vec!["trades", "fills", "trades"]),
            ("TABLE_SCHEMA", vec!["market"])
        ],
        "one column, its first spelling and its place kept, every value appended"
    );
    assert_eq!(
        restrictions.len(),
        2,
        "the length counts columns, not values"
    );
}

#[test]
fn a_restriction_is_read_by_name_in_any_case() {
    let restrictions = Restrictions::new().with("CATALOG_NAME", "market");
    let expected: &[String] = &[String::from("market")];
    assert_eq!(restrictions.get("CATALOG_NAME"), Some(expected));
    assert_eq!(restrictions.get("catalog_name"), Some(expected));
    assert_eq!(
        restrictions.get("CATALOG"),
        None,
        "a prefix is not the name"
    );
    assert_eq!(restrictions.get("TABLE_NAME"), None);
}

#[test]
fn with_answers_the_restrictions_admitting_the_value() {
    let restrictions = Restrictions::new()
        .with("TABLE_NAME", "trades")
        .with("TABLE_NAME", "fills")
        .with("TABLE_TYPE", "TABLE");
    assert_eq!(
        restricted(&restrictions),
        [
            ("TABLE_NAME", vec!["trades", "fills"]),
            ("TABLE_TYPE", vec!["TABLE"])
        ]
    );
}

#[test]
fn a_restriction_value_is_kept_as_written() {
    let restrictions = Restrictions::new()
        .with("TABLE_NAME", "")
        .with("TABLE_NAME", " spaced ")
        .with("TABLE_NAME", "\u{e9}t\u{e9}");
    assert_eq!(
        restricted(&restrictions),
        [("TABLE_NAME", vec!["", " spaced ", "\u{e9}t\u{e9}"])]
    );
}

#[test]
fn restrictions_collect_from_pairs_appending_repeated_names() {
    let restrictions: Restrictions = [
        ("TABLE_NAME", "trades"),
        ("TABLE_SCHEMA", "market"),
        ("table_name", "fills"),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        restricted(&restrictions),
        [
            ("TABLE_NAME", vec!["trades", "fills"]),
            ("TABLE_SCHEMA", vec!["market"])
        ]
    );
    assert_eq!(
        restrictions,
        Restrictions::new()
            .with("TABLE_NAME", "trades")
            .with("TABLE_SCHEMA", "market")
            .with("table_name", "fills"),
        "collecting is pushing each pair in order"
    );

    let empty: Restrictions = std::iter::empty::<(String, String)>().collect();
    assert_eq!(empty, Restrictions::new());
}

#[test]
fn a_restriction_name_folds_ascii_case_and_nothing_else() {
    let restrictions = Restrictions::new()
        .with("\u{c9}TAT", "open")
        .with("\u{e9}tat", "closed")
        .with(" TABLE_NAME", "trades");
    assert_eq!(
        restricted(&restrictions),
        [
            ("\u{c9}TAT", vec!["open"]),
            ("\u{e9}tat", vec!["closed"]),
            (" TABLE_NAME", vec!["trades"])
        ],
        "a non-ASCII letter in another case, or padding, makes another column"
    );
    assert_eq!(restrictions.get("TABLE_NAME"), None);
    let expected: &[String] = &[String::from("")];
    assert_eq!(
        Restrictions::new().with("TABLE_NAME", "").get("table_name"),
        Some(expected),
        "an empty value is admitted as written"
    );
}

#[test]
fn restrictions_equal_ones_written_alike() {
    let written = Restrictions::new()
        .with("TABLE_NAME", "trades")
        .with("TABLE_NAME", "fills");
    assert_eq!(written.clone(), written);
    assert_ne!(
        written,
        Restrictions::new()
            .with("TABLE_NAME", "fills")
            .with("TABLE_NAME", "trades"),
        "the order the values were written in is part of the restrictions"
    );
    assert_ne!(written, Restrictions::new().with("TABLE_NAME", "trades"));
}
