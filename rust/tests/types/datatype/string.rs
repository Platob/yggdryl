//! One string family: eighteen leaves, a shape in a charset, each its own
//! column.

use arrow_schema::DataType as ArrowDataType;
use arrow_schema::extension::{EXTENSION_TYPE_METADATA_KEY, EXTENSION_TYPE_NAME_KEY};
use yggdryl::types::FieldValue as _;
use yggdryl::types::{INLINE_CAPACITY, STRING_EXTENSION_NAME, Str, StringType};
use yggdryl::{Charset, DataType, DataTypeId, Field, Scalar};

/// Every leaf in `StringType::ALL` order, with its identifier, its canonical
/// name and its charset; the numbered leaves state sixteen.
const LEAVES: [(StringType, DataTypeId, &str, Charset); 18] = [
    (
        StringType::Utf8String,
        DataTypeId::Utf8String,
        "utf8",
        Charset::Utf8,
    ),
    (
        StringType::LargeUtf8String,
        DataTypeId::LargeUtf8String,
        "large_utf8",
        Charset::Utf8,
    ),
    (
        StringType::Utf8StringView,
        DataTypeId::Utf8StringView,
        "utf8_view",
        Charset::Utf8,
    ),
    (
        StringType::LargeUtf8StringView,
        DataTypeId::LargeUtf8StringView,
        "large_utf8_view",
        Charset::Utf8,
    ),
    (
        StringType::FixedUtf8String(16),
        DataTypeId::FixedUtf8String,
        "fixed_utf8",
        Charset::Utf8,
    ),
    (
        StringType::SizedUtf8String(16),
        DataTypeId::SizedUtf8String,
        "sized_utf8",
        Charset::Utf8,
    ),
    (
        StringType::AsciiString,
        DataTypeId::AsciiString,
        "ascii",
        Charset::Ascii,
    ),
    (
        StringType::LargeAsciiString,
        DataTypeId::LargeAsciiString,
        "large_ascii",
        Charset::Ascii,
    ),
    (
        StringType::AsciiStringView,
        DataTypeId::AsciiStringView,
        "ascii_view",
        Charset::Ascii,
    ),
    (
        StringType::LargeAsciiStringView,
        DataTypeId::LargeAsciiStringView,
        "large_ascii_view",
        Charset::Ascii,
    ),
    (
        StringType::FixedAsciiString(16),
        DataTypeId::FixedAsciiString,
        "fixed_ascii",
        Charset::Ascii,
    ),
    (
        StringType::SizedAsciiString(16),
        DataTypeId::SizedAsciiString,
        "sized_ascii",
        Charset::Ascii,
    ),
    (
        StringType::Cp1252String,
        DataTypeId::Cp1252String,
        "cp1252",
        Charset::Cp1252,
    ),
    (
        StringType::LargeCp1252String,
        DataTypeId::LargeCp1252String,
        "large_cp1252",
        Charset::Cp1252,
    ),
    (
        StringType::Cp1252StringView,
        DataTypeId::Cp1252StringView,
        "cp1252_view",
        Charset::Cp1252,
    ),
    (
        StringType::LargeCp1252StringView,
        DataTypeId::LargeCp1252StringView,
        "large_cp1252_view",
        Charset::Cp1252,
    ),
    (
        StringType::FixedCp1252String(16),
        DataTypeId::FixedCp1252String,
        "fixed_cp1252",
        Charset::Cp1252,
    ),
    (
        StringType::SizedCp1252String(16),
        DataTypeId::SizedCp1252String,
        "sized_cp1252",
        Charset::Cp1252,
    ),
];

#[test]
fn every_leaf_is_one_datatype_under_every_spelling() {
    // The table is `ALL` with a number filled in: every leaf in the one
    // declaration order, six shapes per charset.
    assert_eq!(
        LEAVES.map(|(leaf, ..)| leaf.id()),
        StringType::ALL.map(StringType::id)
    );
    for (leaf, id, spelling, charset) in LEAVES {
        assert_eq!(leaf.as_str(), spelling);
        assert_eq!(leaf.id(), id);
        assert_eq!(leaf.charset(), charset);
        assert_eq!(StringType::from_id(id, 16), Some(leaf));
        assert_eq!(StringType::from_str(spelling).unwrap().id(), id);
        // The fold is the grammar's: case, underscores and hyphens all drop.
        assert_eq!(
            StringType::from_str(&spelling.to_uppercase()).unwrap().id(),
            id
        );
        assert_eq!(
            StringType::from_str(&spelling.replace('_', "-"))
                .unwrap()
                .id(),
            id
        );
        assert_eq!(
            StringType::from_str(&spelling.replace('_', ""))
                .unwrap()
                .id(),
            id
        );
        // Every string identifier is parameterized: the leaf is the datatype
        // and the identifier alone is not one, and the width is the leaf's
        // rather than the identifier's.
        assert!(id.is_parameterized(), "{id}");
        assert!(id.is_string(), "{id}");
        assert_eq!(id.fixed_byte_width(), None, "{id}");

        let dtype = DataType::string(leaf).unwrap();
        assert!(dtype.to_string().starts_with(spelling), "{dtype}");
        assert_eq!(DataType::from_str(&dtype.to_string()).unwrap(), dtype);
        assert_eq!(dtype.id(), id);
        assert_eq!(dtype.name(), spelling);
        assert_eq!(dtype.charset(), Some(charset));
        assert_eq!(dtype.string_parameters(), Some(leaf));
        assert_eq!(dtype.bytes_parameters(), None);
        assert!(dtype.is_string());
    }

    // The sugar constructors are the same datatypes, and `utf8` is the
    // family's default.
    assert_eq!(DataType::utf8(), DataType::String(StringType::default()));
    for (sugar, leaf) in [
        (DataType::utf8(), StringType::Utf8String),
        (DataType::large_utf8(), StringType::LargeUtf8String),
        (DataType::utf8_view(), StringType::Utf8StringView),
        (DataType::large_utf8_view(), StringType::LargeUtf8StringView),
        (
            DataType::fixed_utf8(8).unwrap(),
            StringType::FixedUtf8String(8),
        ),
        (
            DataType::sized_utf8(32).unwrap(),
            StringType::SizedUtf8String(32),
        ),
        (DataType::ascii(), StringType::AsciiString),
        (DataType::large_ascii(), StringType::LargeAsciiString),
        (DataType::ascii_view(), StringType::AsciiStringView),
        (
            DataType::large_ascii_view(),
            StringType::LargeAsciiStringView,
        ),
        (
            DataType::fixed_ascii(4).unwrap(),
            StringType::FixedAsciiString(4),
        ),
        (
            DataType::sized_ascii(4).unwrap(),
            StringType::SizedAsciiString(4),
        ),
        (DataType::cp1252(), StringType::Cp1252String),
        (DataType::large_cp1252(), StringType::LargeCp1252String),
        (DataType::cp1252_view(), StringType::Cp1252StringView),
        (
            DataType::large_cp1252_view(),
            StringType::LargeCp1252StringView,
        ),
        (
            DataType::fixed_cp1252(8).unwrap(),
            StringType::FixedCp1252String(8),
        ),
        (
            DataType::sized_cp1252(32).unwrap(),
            StringType::SizedCp1252String(32),
        ),
    ] {
        assert_eq!(sugar, DataType::string(leaf).unwrap());
    }
    assert_eq!(
        DataType::fixed_utf8(8).unwrap().to_string(),
        "fixed_utf8(8)"
    );
    assert_eq!(
        DataType::sized_cp1252(32).unwrap().to_string(),
        "sized_cp1252(32)"
    );
}

#[test]
fn every_spelling_renders_as_its_canonical_leaf() {
    // The charset-free spellings name the UTF-8 leaf of their shape and are
    // the only ones that take a `(charset)`; a charset-named spelling is the
    // leaf itself; a number after a plain spelling is a maximum, which is
    // its own leaf.
    for (spelling, canonical) in [
        ("string", "utf8"),
        ("str", "utf8"),
        ("text", "utf8"),
        ("varchar", "utf8"),
        ("nvarchar", "utf8"),
        ("character varying", "utf8"),
        ("utf8_string", "utf8"),
        ("string()", "utf8"),
        ("utf8(32)", "sized_utf8(32)"),
        ("varchar(32)", "sized_utf8(32)"),
        ("string(32)", "sized_utf8(32)"),
        ("sized_string(32)", "sized_utf8(32)"),
        ("sized_utf8(32)", "sized_utf8(32)"),
        ("string(utf-8,32)", "sized_utf8(32)"),
        ("char(8)", "fixed_utf8(8)"),
        ("fixed_string(8)", "fixed_utf8(8)"),
        ("FixedString(8)", "fixed_utf8(8)"),
        ("fixed_utf8(8)", "fixed_utf8(8)"),
        ("string_view", "utf8_view"),
        ("stringview", "utf8_view"),
        ("utf8view", "utf8_view"),
        ("LargeString", "large_utf8"),
        ("large-utf8", "large_utf8"),
        ("large_string_view", "large_utf8_view"),
        ("large_utf8_view", "large_utf8_view"),
        ("ascii", "ascii"),
        ("us-ascii", "ascii"),
        ("ascii_string", "ascii"),
        ("string(us-ascii)", "ascii"),
        ("ascii(4)", "sized_ascii(4)"),
        ("string(us-ascii,4)", "sized_ascii(4)"),
        ("sized_ascii(4)", "sized_ascii(4)"),
        ("fixed_ascii(4)", "fixed_ascii(4)"),
        ("fixed_string(us-ascii,4)", "fixed_ascii(4)"),
        ("large_ascii", "large_ascii"),
        ("large_string(us-ascii)", "large_ascii"),
        ("ascii_view", "ascii_view"),
        ("string_view(us-ascii)", "ascii_view"),
        ("large_ascii_view", "large_ascii_view"),
        ("large_string_view(us-ascii)", "large_ascii_view"),
        ("cp1252", "cp1252"),
        ("windows-1252", "cp1252"),
        ("string(windows-1252)", "cp1252"),
        ("string(cp1252)", "cp1252"),
        ("string(windows-1252,32)", "sized_cp1252(32)"),
        ("sized_cp1252(32)", "sized_cp1252(32)"),
        ("sized_windows_1252(32)", "sized_cp1252(32)"),
        ("fixed_string(windows-1252,8)", "fixed_cp1252(8)"),
        ("fixed_cp1252(8)", "fixed_cp1252(8)"),
        ("large_string(windows-1252)", "large_cp1252"),
        ("large_cp1252", "large_cp1252"),
        ("string_view(windows-1252)", "cp1252_view"),
        ("cp1252_view", "cp1252_view"),
        ("large_string_view(windows-1252)", "large_cp1252_view"),
        ("large_windows_1252_view", "large_cp1252_view"),
    ] {
        assert_eq!(
            DataType::from_str(spelling).unwrap().to_string(),
            canonical,
            "{spelling}"
        );
    }

    // A maximum only lands on a plain leaf: the large and the viewed shapes
    // refuse it naming the sized leaf a bounded column would be.
    for (spelling, sized) in [
        ("large_utf8(64)", "sized_utf8(maximum)"),
        ("utf8_view(8)", "sized_utf8(maximum)"),
        ("large_string_view(8)", "sized_utf8(maximum)"),
        ("large_ascii(16)", "sized_ascii(maximum)"),
        ("ascii_view(4)", "sized_ascii(maximum)"),
        ("large_string(windows-1252,32)", "sized_cp1252(maximum)"),
        ("cp1252_view(8)", "sized_cp1252(maximum)"),
    ] {
        let refusal = DataType::from_str(spelling).unwrap_err().to_string();
        assert!(refusal.contains(sized), "{spelling}: {refusal}");
    }
    // A charset-named spelling already answered the charset question.
    for spelling in [
        "utf8(windows-1252)",
        "fixed_utf8(windows-1252,8)",
        "ascii(utf-8)",
        "large_ascii(us-ascii)",
        "cp1252(utf-8)",
        "sized_cp1252(windows-1252,8)",
    ] {
        let refusal = DataType::from_str(spelling).unwrap_err().to_string();
        assert!(
            refusal.contains("string is the spelling that takes one"),
            "{spelling}: {refusal}"
        );
    }
    // Only three charsets have leaves; the rest stay the decoding vocabulary.
    for spelling in [
        "string(iso-8859-1)",
        "string(latin1)",
        "large_string(iso-8859-15)",
        "fixed_string(utf-16-le,8)",
    ] {
        let refusal = DataType::from_str(spelling).unwrap_err().to_string();
        assert!(
            refusal.contains("utf-8, us-ascii or windows-1252"),
            "{spelling}: {refusal}"
        );
    }
    // And a word that is no charset is refused as one.
    assert!(DataType::from_str("string(nowhere)").is_err());
}

#[test]
fn a_width_and_a_maximum_are_two_leaves_and_never_one_column() {
    // `sized_cp1252(32)` is a maximum of thirty-two bytes and
    // `fixed_cp1252(8)` an exact width; the two readings never both answer.
    let bounded = DataType::from_str("string(windows-1252,32)").unwrap();
    let parameters = bounded.string_parameters().unwrap();
    assert_eq!(parameters, StringType::SizedCp1252String(32));
    assert_eq!(parameters.max(), Some(32));
    assert_eq!(parameters.fixed(), None);
    assert_eq!(parameters.bound(), Some(32));
    assert!(parameters.is_bounded());
    assert!(!parameters.is_fixed());
    assert_eq!(bounded.fixed_byte_width(), None);
    assert_eq!(bounded.to_string(), "sized_cp1252(32)");
    assert_ne!(bounded, DataType::cp1252());

    let fixed = DataType::from_str("fixed_string(windows-1252,8)").unwrap();
    let parameters = fixed.string_parameters().unwrap();
    assert_eq!(parameters, StringType::FixedCp1252String(8));
    assert_eq!(parameters.fixed(), Some(8));
    assert_eq!(parameters.max(), None);
    assert!(parameters.is_fixed());
    assert_eq!(fixed.fixed_byte_width(), Some(8));
    assert_eq!(fixed.to_string(), "fixed_cp1252(8)");
    assert_ne!(bounded, fixed);

    // The value a sized column holds is the plain leaf of its charset: the
    // maximum is the column's rule and never the value's.
    assert_eq!(
        StringType::SizedUtf8String(16).storage(),
        StringType::Utf8String
    );
    assert_eq!(
        StringType::SizedAsciiString(4).storage(),
        StringType::AsciiString
    );
    assert_eq!(
        StringType::SizedCp1252String(32).storage(),
        StringType::Cp1252String
    );
    assert_eq!(
        StringType::FixedAsciiString(4).storage(),
        StringType::FixedAsciiString(4)
    );
    assert_eq!(
        StringType::LargeCp1252StringView.storage(),
        StringType::LargeCp1252StringView
    );

    // A column of no bytes is not a column, whichever leaf states the number.
    assert!(DataType::from_str("utf8(0)").is_err());
    assert!(DataType::from_str("fixed_ascii(0)").is_err());
    assert!(DataType::from_str("string(windows-1252,0)").is_err());
    assert!(DataType::fixed_utf8(0).is_err());
    assert!(DataType::sized_cp1252(0).is_err());
    assert!(StringType::FixedUtf8String(0).validate().is_err());
    assert!(StringType::SizedAsciiString(0).validate().is_err());
    let refusal = StringType::FixedCp1252String(0)
        .validate()
        .unwrap_err()
        .to_string();
    assert!(
        refusal.contains("expected a width of at least one byte, got 0"),
        "{refusal}"
    );
    assert!(
        DataType::String(StringType::FixedCp1252String(0))
            .validate()
            .is_err()
    );

    // A bare numbered spelling is a question rather than a declaration.
    for spelling in [
        "fixed_string",
        "fixedutf8",
        "fixed_ascii",
        "sized_string",
        "sized_ascii",
        "sized_cp1252()",
    ] {
        assert!(DataType::from_str(spelling).is_err(), "{spelling}");
    }

    // Which offsets, whether it is viewed and which charset are the leaf's,
    // and answered without a match.
    assert!(StringType::LargeAsciiStringView.is_view());
    assert!(StringType::LargeAsciiStringView.is_large());
    assert!(StringType::Cp1252StringView.is_view());
    assert!(!StringType::Cp1252StringView.is_large());
    assert!(StringType::LargeUtf8String.is_large());
    assert!(!StringType::LargeUtf8String.is_view());
    assert!(!StringType::SizedUtf8String(16).is_view());
    assert!(!StringType::SizedUtf8String(16).is_large());
    assert_eq!(StringType::SizedUtf8String(16).charset(), Charset::Utf8);
    assert_eq!(StringType::LargeAsciiString.charset(), Charset::Ascii);
    assert_eq!(StringType::FixedCp1252String(8).charset(), Charset::Cp1252);
}

#[test]
fn a_number_and_a_charset_restate_a_leaf_under_one_rule_each() {
    // A stated number: the width on a fixed leaf, the maximum on a plain or
    // a sized one, refused on the large and the viewed shapes.
    assert_eq!(
        StringType::Utf8String.with_bound(32).unwrap(),
        StringType::SizedUtf8String(32)
    );
    assert_eq!(
        StringType::AsciiString.with_bound(4).unwrap(),
        StringType::SizedAsciiString(4)
    );
    assert_eq!(
        StringType::SizedCp1252String(8).with_bound(32).unwrap(),
        StringType::SizedCp1252String(32)
    );
    assert_eq!(
        StringType::FixedAsciiString(1).with_bound(4).unwrap(),
        StringType::FixedAsciiString(4)
    );
    for leaf in [
        StringType::LargeUtf8String,
        StringType::Utf8StringView,
        StringType::LargeUtf8StringView,
        StringType::LargeAsciiString,
        StringType::AsciiStringView,
        StringType::LargeCp1252StringView,
    ] {
        let refusal = leaf.with_bound(8).unwrap_err().to_string();
        assert!(refusal.contains("expected no maximum"), "{leaf}: {refusal}");
        assert_eq!(leaf.with_declared_bound(None).unwrap(), leaf);
    }
    assert!(StringType::Utf8String.with_bound(0).is_err());

    // A number the caller may not have stated: the six numbered leaves are
    // their number and stand without none.
    assert_eq!(
        StringType::Utf8String.with_declared_bound(None).unwrap(),
        StringType::Utf8String
    );
    assert_eq!(
        StringType::SizedAsciiString(1)
            .with_declared_bound(Some(4))
            .unwrap(),
        StringType::SizedAsciiString(4)
    );
    let refusal = StringType::FixedUtf8String(1)
        .with_declared_bound(None)
        .unwrap_err()
        .to_string();
    assert!(
        refusal.contains("expected fixed_utf8(number), got none"),
        "{refusal}"
    );
    assert!(
        StringType::SizedCp1252String(1)
            .with_declared_bound(None)
            .is_err()
    );

    // A charset: the same shape in another family, the number kept.
    assert_eq!(
        StringType::Utf8String
            .with_charset(Charset::Cp1252)
            .unwrap(),
        StringType::Cp1252String
    );
    assert_eq!(
        StringType::FixedUtf8String(4)
            .with_charset(Charset::Ascii)
            .unwrap(),
        StringType::FixedAsciiString(4)
    );
    assert_eq!(
        StringType::SizedAsciiString(3)
            .with_charset(Charset::Utf8)
            .unwrap(),
        StringType::SizedUtf8String(3)
    );
    assert_eq!(
        StringType::LargeUtf8StringView
            .with_charset(Charset::Cp1252)
            .unwrap(),
        StringType::LargeCp1252StringView
    );
    for (leaf, _, _, charset) in LEAVES {
        assert_eq!(leaf.with_charset(charset).unwrap(), leaf);
        // Three families, one shape each: a round trip through the other
        // two comes back to the leaf.
        let ascii = leaf.with_charset(Charset::Ascii).unwrap();
        let latin = ascii.with_charset(Charset::Cp1252).unwrap();
        assert_eq!(latin.with_charset(charset).unwrap(), leaf, "{leaf}");
        assert_eq!(latin.bound(), leaf.bound(), "{leaf}");
        let refusal = leaf.with_charset(Charset::Latin1).unwrap_err().to_string();
        assert!(
            refusal.contains("utf-8, us-ascii or windows-1252"),
            "{leaf}: {refusal}"
        );
        assert!(leaf.with_charset(Charset::Utf16Le).is_err(), "{leaf}");
    }

    // A binding's `(layout, charset, bound)` arguments run the same rules in
    // the same order, so neither binding decides anything.
    assert_eq!(
        StringType::from_declaration("string", Some("windows-1252"), Some(32)).unwrap(),
        StringType::SizedCp1252String(32)
    );
    assert_eq!(
        StringType::from_declaration("fixed_string", Some("us-ascii"), Some(4)).unwrap(),
        StringType::FixedAsciiString(4)
    );
    assert_eq!(
        StringType::from_declaration("large-utf8", None, None).unwrap(),
        StringType::LargeUtf8String
    );
    assert_eq!(
        StringType::from_declaration("ASCII", None, Some(4)).unwrap(),
        StringType::SizedAsciiString(4)
    );
    let refusal = StringType::from_declaration("utf8", Some("windows-1252"), None)
        .unwrap_err()
        .to_string();
    assert!(
        refusal.contains("string is the spelling that takes one"),
        "{refusal}"
    );
    assert!(StringType::from_declaration("fixed_utf8", None, None).is_err());
    assert!(StringType::from_declaration("large_utf8", None, Some(64)).is_err());
    assert!(StringType::from_declaration("string", Some("latin1"), None).is_err());
    let refusal = StringType::from_declaration("blob", None, None)
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("expected a string layout"), "{refusal}");

    // The spellings, already folded: the charset-free ones are a subset.
    assert_eq!(
        StringType::general_spelling("string"),
        Some(StringType::Utf8String)
    );
    assert_eq!(
        StringType::general_spelling("fixedstring"),
        Some(StringType::FixedUtf8String(1))
    );
    assert_eq!(StringType::general_spelling("utf8"), None);
    assert_eq!(
        StringType::from_spelling("utf8"),
        Some(StringType::Utf8String)
    );
    assert_eq!(
        StringType::from_spelling("sizedwindows1252"),
        Some(StringType::SizedCp1252String(1))
    );
    assert_eq!(StringType::from_spelling("blob"), None);
    let refusal = StringType::from_str("blob").unwrap_err().to_string();
    assert!(refusal.contains("expected a string layout"), "{refusal}");
}

#[test]
fn sql_reads_its_own_two_string_shapes() {
    // `varchar(n)` bounds; `char(n)` is blank-padded to exactly n.
    assert_eq!(
        DataType::from_str("varchar(32)").unwrap().to_string(),
        "sized_utf8(32)"
    );
    assert_eq!(
        DataType::from_str("character varying(32)")
            .unwrap()
            .to_string(),
        "sized_utf8(32)"
    );
    assert_eq!(
        DataType::from_str("char(8)").unwrap().to_string(),
        "fixed_utf8(8)"
    );
    // A width is what makes a string fixed, so a bare `char` is not.
    assert_eq!(DataType::from_str("char").unwrap(), DataType::utf8());
    // And the SQL spellings are charset-free, so they take one.
    assert_eq!(
        DataType::from_str("char(us-ascii,8)").unwrap().to_string(),
        "fixed_ascii(8)"
    );
    assert_eq!(
        DataType::from_str("varchar(windows-1252,32)")
            .unwrap()
            .to_string(),
        "sized_cp1252(32)"
    );
}

#[test]
fn every_string_datatype_answers_one_question_about_its_charset() {
    // One accessor for every string, whatever its spelling.
    assert_eq!(DataType::utf8().charset(), Some(Charset::Utf8));
    assert_eq!(DataType::utf8_view().charset(), Some(Charset::Utf8));
    assert_eq!(DataType::ascii().charset(), Some(Charset::Ascii));
    assert_eq!(
        DataType::fixed_ascii(3).unwrap().charset(),
        Some(Charset::Ascii)
    );
    assert_eq!(
        DataType::fixed_ascii(3)
            .unwrap()
            .string_parameters()
            .unwrap()
            .fixed(),
        Some(3)
    );
    assert_eq!(
        DataType::from_str("large_string(windows-1252)")
            .unwrap()
            .charset(),
        Some(Charset::Cp1252)
    );
    // A charset with no leaf is not a string datatype anyone can name.
    assert!(DataType::from_str("large_string(iso-8859-15)").is_err());

    // A code is an identity over a registry, not a string with a charset:
    // it stores as text without declaring one.
    assert_eq!(DataType::Currency.charset(), None);
    assert!(!DataType::Currency.is_string());
    assert!(DataType::utf8().is_string());
}

#[test]
fn every_leaf_rides_its_arrow_storage_and_all_but_three_ride_the_document() {
    // Arrow's string layouts declare UTF-8, so text that is not UTF-8 rides
    // the binary layout beside them and the charset rides the document;
    // US-ASCII is UTF-8 and rides the text layouts under the same document.
    // Plain `utf8`, `large_utf8` and `utf8_view` are Arrow's own and cross
    // bare; every other leaf is written whole, charset beside the name, since
    // the charset is the one fact Arrow cannot state.
    let cases: [(&str, ArrowDataType, Option<&str>); 18] = [
        ("utf8", ArrowDataType::Utf8, None),
        ("large_utf8", ArrowDataType::LargeUtf8, None),
        ("utf8_view", ArrowDataType::Utf8View, None),
        (
            "large_utf8_view",
            ArrowDataType::Utf8View,
            Some(r#"{"layout":"large_utf8_view","charset":"utf-8"}"#),
        ),
        (
            "fixed_utf8(8)",
            ArrowDataType::FixedSizeBinary(8),
            Some(r#"{"layout":"fixed_utf8","charset":"utf-8","fixed":8}"#),
        ),
        (
            "utf8(32)",
            ArrowDataType::Utf8,
            Some(r#"{"layout":"sized_utf8","charset":"utf-8","max":32}"#),
        ),
        (
            "ascii",
            ArrowDataType::Utf8,
            Some(r#"{"layout":"ascii","charset":"us-ascii"}"#),
        ),
        (
            "large_ascii",
            ArrowDataType::LargeUtf8,
            Some(r#"{"layout":"large_ascii","charset":"us-ascii"}"#),
        ),
        (
            "ascii_view",
            ArrowDataType::Utf8View,
            Some(r#"{"layout":"ascii_view","charset":"us-ascii"}"#),
        ),
        (
            "large_ascii_view",
            ArrowDataType::Utf8View,
            Some(r#"{"layout":"large_ascii_view","charset":"us-ascii"}"#),
        ),
        (
            "fixed_ascii(4)",
            ArrowDataType::FixedSizeBinary(4),
            Some(r#"{"layout":"fixed_ascii","charset":"us-ascii","fixed":4}"#),
        ),
        (
            "sized_ascii(16)",
            ArrowDataType::Utf8,
            Some(r#"{"layout":"sized_ascii","charset":"us-ascii","max":16}"#),
        ),
        (
            "string(windows-1252)",
            ArrowDataType::Binary,
            Some(r#"{"layout":"cp1252","charset":"windows-1252"}"#),
        ),
        (
            "largestring(windows-1252)",
            ArrowDataType::LargeBinary,
            Some(r#"{"layout":"large_cp1252","charset":"windows-1252"}"#),
        ),
        (
            "stringview(windows-1252)",
            ArrowDataType::BinaryView,
            Some(r#"{"layout":"cp1252_view","charset":"windows-1252"}"#),
        ),
        (
            "large_cp1252_view",
            ArrowDataType::BinaryView,
            Some(r#"{"layout":"large_cp1252_view","charset":"windows-1252"}"#),
        ),
        (
            "fixed_string(windows-1252,8)",
            ArrowDataType::FixedSizeBinary(8),
            Some(r#"{"layout":"fixed_cp1252","charset":"windows-1252","fixed":8}"#),
        ),
        (
            "string(windows-1252,32)",
            ArrowDataType::Binary,
            Some(r#"{"layout":"sized_cp1252","charset":"windows-1252","max":32}"#),
        ),
    ];
    for (spelling, storage, document) in cases {
        let dtype = DataType::from_str(spelling).unwrap();
        assert_eq!(
            dtype.clone().into_arrow_datatype().unwrap(),
            storage,
            "{spelling}"
        );
        // A bare Arrow text storage is the plain UTF-8 leaf it declares.
        if document.is_none() {
            assert_eq!(
                DataType::from_arrow_datatype(&storage).unwrap(),
                dtype,
                "{spelling}"
            );
        }

        let field = dtype.clone().nullable_field("value");
        let arrow = field.clone().into_arrow_field().unwrap();
        assert_eq!(arrow.data_type(), &storage, "{spelling}");
        assert_eq!(
            arrow
                .metadata()
                .get(EXTENSION_TYPE_NAME_KEY)
                .map(String::as_str),
            document.map(|_| STRING_EXTENSION_NAME),
            "{spelling}"
        );
        assert_eq!(
            arrow
                .metadata()
                .get(EXTENSION_TYPE_METADATA_KEY)
                .map(String::as_str),
            document,
            "{spelling}"
        );
        assert_eq!(
            Field::from_arrow_field(&arrow).unwrap(),
            field,
            "{spelling}"
        );
    }

    // The document round-trips through its own door for every leaf, and an
    // older document that named a shape beside a charset reads as the leaf
    // it meant: a maximum makes the column sized whichever unbounded shape
    // was named.
    for (leaf, ..) in LEAVES {
        assert_eq!(
            StringType::from_extension_json(&leaf.extension_json()).unwrap(),
            leaf,
            "{leaf}"
        );
    }
    for (document, leaf) in [
        (
            r#"{"layout":"string","charset":"us-ascii"}"#,
            StringType::AsciiString,
        ),
        (
            r#"{"layout":"string","charset":"us-ascii","max":4}"#,
            StringType::SizedAsciiString(4),
        ),
        (
            r#"{"layout":"fixed_string","charset":"us-ascii","fixed":4}"#,
            StringType::FixedAsciiString(4),
        ),
        (
            r#"{"layout":"large_string","charset":"utf-8","max":64}"#,
            StringType::SizedUtf8String(64),
        ),
        (
            r#"{"layout":"large_string","charset":"windows-1252"}"#,
            StringType::LargeCp1252String,
        ),
        (
            r#"{"layout":"string_view","charset":"windows-1252","max":32}"#,
            StringType::SizedCp1252String(32),
        ),
        (
            r#"{"layout":"large_string_view","charset":"utf-8"}"#,
            StringType::LargeUtf8StringView,
        ),
        (
            r#"{"layout":"utf8","max":16}"#,
            StringType::SizedUtf8String(16),
        ),
        (
            r#"{"layout":"sized_cp1252","max":32}"#,
            StringType::SizedCp1252String(32),
        ),
        (
            r#"{"layout":"fixed_ascii","fixed":4}"#,
            StringType::FixedAsciiString(4),
        ),
    ] {
        assert_eq!(
            StringType::from_extension_json(document).unwrap(),
            leaf,
            "{document}"
        );
    }
    for retired in [
        r#"{"layout":"fixed_utf8"}"#,
        r#"{"layout":"fixed_utf8","max":8}"#,
        r#"{"layout":"utf8","fixed":4}"#,
        r#"{"layout":"sized_utf8"}"#,
        r#"{"layout":"fixed_ascii","fixed":4,"max":4}"#,
        r#"{"layout":"string","charset":"iso-8859-1"}"#,
        r#"{"layout":"utf8","max":0}"#,
        r#"{"layout":"blob"}"#,
        r#""utf8""#,
    ] {
        assert!(
            StringType::from_extension_json(retired).is_err(),
            "{retired}"
        );
    }

    // A document over a storage it does not describe imports as the storage.
    let latin = DataType::cp1252().string_parameters().unwrap();
    let foreign = arrow_schema::Field::new("value", ArrowDataType::Utf8, true).with_metadata(
        [
            (
                EXTENSION_TYPE_NAME_KEY.to_owned(),
                STRING_EXTENSION_NAME.to_owned(),
            ),
            (
                EXTENSION_TYPE_METADATA_KEY.to_owned(),
                latin.extension_json(),
            ),
        ]
        .into_iter()
        .collect(),
    );
    assert_eq!(
        Field::from_arrow_field(&foreign).unwrap().dtype(),
        &DataType::utf8()
    );
}

#[test]
fn the_two_view_layouts_share_one_arrow_layout_and_stay_distinct_here() {
    // Arrow has one view layout; this crate declares two, and the difference
    // travels in the metadata rather than in the buffers.
    let view = DataType::utf8_view();
    let large = DataType::from_str("large_utf8_view").unwrap();
    assert_ne!(view, large);
    assert_eq!(
        view.clone().into_arrow_datatype().unwrap(),
        large.clone().into_arrow_datatype().unwrap()
    );
    let field = large.clone().nullable_field("value");
    assert_eq!(
        Field::from_arrow_field(&field.clone().into_arrow_field().unwrap()).unwrap(),
        field
    );
}

#[test]
fn a_string_column_stores_the_bytes_its_charset_writes() {
    let dtype = DataType::from_str("string(windows-1252)").unwrap();
    let value = dtype.scalar("Grüße").unwrap();
    let text = value.as_str().unwrap();
    assert_eq!(text, "Grüße");
    assert_eq!(value.id(), DataTypeId::Cp1252String);
    assert_eq!(value.dtype().unwrap(), dtype);

    // The characters are UTF-8 in memory and windows-1252 on the way out.
    let encoded = Charset::Cp1252.encode("Grüße").unwrap();
    assert_eq!(encoded.len(), 5);
    assert_eq!(encoded.as_ref(), &[0x47, 0x72, 0xFC, 0xDF, 0x65]);
}

#[test]
fn bytes_arriving_at_a_string_column_are_read_through_its_charset() {
    let dtype = DataType::from_str("string(windows-1252)").unwrap();
    let value = dtype
        .scalar(Scalar::from(vec![0x47_u8, 0x72, 0xFC, 0xDF, 0x65]))
        .unwrap();
    assert_eq!(value.as_str(), Some("Grüße"));

    // And a byte the charset leaves unassigned is transcribed rather than
    // refusing the whole value: `0x81` is a C1 control in ISO 8859-1, which
    // is what the WHATWG index maps it to too.
    let recovered = dtype
        .scalar(Scalar::from(vec![0x6F_u8, 0x6B, 0x81]))
        .unwrap();
    assert_eq!(recovered.as_str(), Some("ok\u{0081}"));
}

#[test]
fn utf8_reads_bytes_strictly_in_every_layout() {
    // UTF-8 is a validated repertoire: bytes that are not UTF-8 are refused
    // naming the charset, under every UTF-8 leaf, while a legacy charset
    // transcribes the same bytes.
    let damaged = vec![0x6F_u8, 0x6B, 0x81];
    for spelling in [
        "utf8",
        "utf8(8)",
        "fixed_utf8(8)",
        "large_utf8",
        "utf8_view",
        "large_utf8_view",
    ] {
        let dtype = DataType::from_str(spelling).unwrap();
        let refused = dtype
            .scalar(Scalar::from(damaged.clone()))
            .unwrap_err()
            .to_string();
        assert!(refused.contains("utf-8"), "{spelling}: {refused}");
        assert_eq!(
            dtype
                .scalar(Scalar::from("caf\u{e9}".as_bytes().to_vec()))
                .unwrap()
                .as_str(),
            Some("caf\u{e9}"),
            "{spelling}"
        );
    }
    assert!(Str::from_bytes(&damaged, StringType::default()).is_err());
    assert_eq!(
        Str::from_bytes(&damaged, StringType::Cp1252String).unwrap(),
        "ok\u{0081}"
    );
}

#[test]
fn a_maximum_is_counted_in_the_bytes_the_charset_stores() {
    let dtype = DataType::from_str("string(windows-1252,4)").unwrap();
    // Five scalars, five stored bytes: over the bound.
    assert!(dtype.scalar("Grüße").is_err());
    // Four scalars in one byte each: within it.
    assert_eq!(dtype.scalar("Grüß").unwrap().as_str(), Some("Grüß"));

    // UTF-8 counts the bytes it stores, which is not the scalar count.
    let utf8 = DataType::from_str("utf8(4)").unwrap();
    assert!(utf8.scalar("Grüß").is_err());
    assert_eq!(utf8.scalar("Grü").unwrap().as_str(), Some("Grü"));
}

#[test]
fn a_fixed_string_pads_its_storage_and_reads_back_trimmed() {
    let dtype = DataType::from_str("fixed_string(windows-1252,8)").unwrap();
    let value = dtype.scalar("café").unwrap();
    assert_eq!(value.as_str(), Some("café"));
    assert_eq!(value.dtype().unwrap(), dtype);
    // Four stored bytes in windows-1252, so it fits eight; five in UTF-8
    // would too, and neither is the scalar count.
    assert!(
        DataType::from_str("fixedstring(windows-1252,3)")
            .unwrap()
            .scalar("café")
            .is_err()
    );
}

#[test]
fn a_string_restates_into_another_layout_without_copying_its_characters() {
    // Long enough to be heap storage: below the inline buffer a rewrite
    // copies the bytes into the new value and no pointer identity could be
    // observed, so a short value cannot witness this claim at all.
    const LONG: &str = "a value well past the twenty-three byte inline buffer";
    assert!(LONG.len() > INLINE_CAPACITY);

    let source = DataType::utf8().scalar(LONG).unwrap();
    let origin = source.as_str().unwrap().as_ptr();
    let target = DataType::from_str("large_string(windows-1252)").unwrap();
    let restated = target.scalar(source).unwrap();
    assert_eq!(restated.as_str(), Some(LONG));
    assert_eq!(restated.id(), DataTypeId::LargeCp1252String);
    assert_eq!(restated.dtype().unwrap(), target);
    // The leaf is the offset width and the charset, not the bytes: the
    // rewrite adopts the storage handle rather than copying the characters.
    assert!(
        std::ptr::eq(restated.as_str().unwrap().as_ptr(), origin),
        "restating a leaf should share its storage"
    );

    let short = DataType::utf8().scalar("AAPL").unwrap();
    assert_eq!(target.scalar(short).unwrap().as_str(), Some("AAPL"));
}

#[test]
fn str_is_the_compact_string_and_compares_by_its_characters() {
    // Short text lives inside the value, a static spelling costs nothing,
    // and longer text is one shared handle.
    let short = Str::new("AAPL");
    assert!(short.is_inline());
    assert!(Str::new("a".repeat(INLINE_CAPACITY)).is_inline());
    assert!(!Str::new("a".repeat(INLINE_CAPACITY + 1)).is_inline());
    assert_eq!(std::mem::size_of::<Str>(), 32);
    assert_eq!(Str::new_static("AAPL"), short);
    assert_eq!(Str::default(), "");
    assert_eq!(short.parameters(), StringType::Utf8String);

    // Equality, order and hash read the characters alone, so a value is one
    // value whichever column holds it.
    let latin = short
        .clone()
        .try_with_parameters(StringType::LargeCp1252String)
        .unwrap();
    assert_eq!(latin, short);
    assert_eq!(latin, "AAPL");
    assert_eq!("AAPL", latin);
    assert_eq!(latin, String::from("AAPL"));
    assert_eq!(latin.charset(), Charset::Cp1252);
    assert_eq!(latin.parameters(), StringType::LargeCp1252String);
    assert!(latin.parameters().is_large());
    assert_eq!(latin.dtype().unwrap(), DataType::large_cp1252());
    assert_ne!(format!("{latin:?}"), format!("{short:?}"));
    assert_eq!(latin.to_string(), "AAPL");
    let mut members = std::collections::BTreeMap::new();
    members.insert(latin, 1);
    assert_eq!(members.get("AAPL"), Some(&1));

    // The conversions every string API leans on.
    assert_eq!(Str::from(String::from("x")), "x");
    assert_eq!(Str::from('x'), "x");
    assert_eq!(String::from(Str::new("x")), "x");
    assert_eq!("a b".parse::<Str>().unwrap(), "a b");
    assert_eq!(["a", "b"].into_iter().collect::<Str>(), "ab");
    assert_eq!(&*Str::new("deref"), "deref");
    assert_eq!(Scalar::from("x"), Scalar::String(Str::new("x")));

    // A maximum is never carried: the value answers the plain leaf its
    // storage is.
    let bounded = Str::new("USD")
        .try_with_parameters(StringType::SizedUtf8String(4))
        .unwrap();
    assert_eq!(bounded.parameters(), StringType::default());
    assert!(
        Str::new("EURO!")
            .try_with_parameters(StringType::SizedUtf8String(4))
            .is_err()
    );
    // A fixed width is: the value pads to it on the way out.
    let fixed = Str::new("USD\0")
        .try_with_parameters(StringType::FixedUtf8String(4))
        .unwrap();
    assert_eq!(fixed, "USD");
    assert_eq!(fixed.fixed(), Some(4));
    assert_eq!(fixed.encode().unwrap().as_ref(), b"USD\0");
    assert_eq!(fixed.encoded_len(), 4);
    assert_eq!(fixed.dtype().unwrap(), DataType::fixed_utf8(4).unwrap());
    assert!(
        Str::new("x")
            .try_with_parameters(StringType::FixedUtf8String(0))
            .is_err()
    );
}

#[test]
fn a_string_datatype_survives_both_serde_doors() {
    // One `string` tag for every string, with only what it declares beside
    // it: the leaf when not `utf8`, and the number under the key its leaf
    // gives it. The leaf's name says its charset, so no `charset` key is
    // written.
    for (spelling, json) in [
        ("utf8", r#"{"type":"string"}"#),
        (
            "utf8(32)",
            r#"{"type":"string","layout":"sized_utf8","max":32}"#,
        ),
        (
            "fixed_utf8(8)",
            r#"{"type":"string","layout":"fixed_utf8","fixed":8}"#,
        ),
        ("large_utf8", r#"{"type":"string","layout":"large_utf8"}"#),
        ("utf8_view", r#"{"type":"string","layout":"utf8_view"}"#),
        (
            "large_utf8_view",
            r#"{"type":"string","layout":"large_utf8_view"}"#,
        ),
        ("ascii", r#"{"type":"string","layout":"ascii"}"#),
        (
            "sized_ascii(3)",
            r#"{"type":"string","layout":"sized_ascii","max":3}"#,
        ),
        (
            "fixed_ascii(4)",
            r#"{"type":"string","layout":"fixed_ascii","fixed":4}"#,
        ),
        (
            "string(windows-1252)",
            r#"{"type":"string","layout":"cp1252"}"#,
        ),
        (
            "fixed_string(windows-1252,8)",
            r#"{"type":"string","layout":"fixed_cp1252","fixed":8}"#,
        ),
        (
            "string(windows-1252,32)",
            r#"{"type":"string","layout":"sized_cp1252","max":32}"#,
        ),
        (
            "string_view(windows-1252)",
            r#"{"type":"string","layout":"cp1252_view"}"#,
        ),
        (
            "large_cp1252_view",
            r#"{"type":"string","layout":"large_cp1252_view"}"#,
        ),
    ] {
        let dtype = DataType::from_str(spelling).unwrap();
        assert_eq!(dtype.clone().into_json().unwrap(), json, "{spelling}");
        assert_eq!(DataType::from_json(json).unwrap(), dtype, "{spelling}");
        assert_eq!(
            serde_json::from_str::<DataType>(&serde_json::to_string(&dtype).unwrap()).unwrap(),
            dtype,
            "{spelling}"
        );

        let value = dtype.clone().into_value();
        assert_eq!(DataType::from_value(value).unwrap(), dtype, "{spelling}");
    }

    // An older document spelled a shape beside a charset, and a maximum
    // beside a plain or a large shape; each still reads as the leaf it
    // meant, through both doors.
    for (document, spelling) in [
        (r#"{"type":"string","charset":"us-ascii"}"#, "ascii"),
        (
            r#"{"type":"string","charset":"us-ascii","max":3}"#,
            "sized_ascii(3)",
        ),
        (
            r#"{"type":"string","layout":"fixed_string","charset":"us-ascii","fixed":4}"#,
            "fixed_ascii(4)",
        ),
        (
            r#"{"type":"string","layout":"large_string","charset":"windows-1252"}"#,
            "large_cp1252",
        ),
        (
            r#"{"type":"string","layout":"string_view","charset":"windows-1252","max":32}"#,
            "sized_cp1252(32)",
        ),
        (r#"{"type":"string","max":32}"#, "sized_utf8(32)"),
        (
            r#"{"type":"string","layout":"large_string","max":64}"#,
            "sized_utf8(64)",
        ),
    ] {
        let dtype = DataType::from_str(spelling).unwrap();
        assert_eq!(DataType::from_json(document).unwrap(), dtype, "{document}");
        let entries = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(document)
            .unwrap()
            .into_iter()
            .map(|(key, value)| {
                let value = match value {
                    serde_json::Value::String(text) => Scalar::from(text.as_str()),
                    serde_json::Value::Number(number) => Scalar::from(number.as_i64().unwrap()),
                    other => panic!("{document}: {other}"),
                };
                (Scalar::from(key.as_str()), value)
            });
        let value = Scalar::from_mapping(entries).unwrap();
        assert_eq!(DataType::from_value(value).unwrap(), dtype, "{document}");
    }

    // The retired tags are nobody's, a number the leaf does not read is
    // refused, and so is a charset with no leaf.
    for retired in [
        r#"{"type":"utf8"}"#,
        r#"{"type":"large_utf8"}"#,
        r#"{"type":"utf8_view"}"#,
        r#"{"type":"ascii"}"#,
        r#"{"type":"fixed_ascii","width":4}"#,
        r#"{"type":"sized_utf8","max":4}"#,
        r#"{"type":"string","layout":"fixed_string"}"#,
        r#"{"type":"string","layout":"fixed_utf8"}"#,
        r#"{"type":"string","layout":"fixed_utf8","max":4}"#,
        r#"{"type":"string","layout":"sized_utf8"}"#,
        r#"{"type":"string","fixed":4}"#,
        r#"{"type":"string","max":0}"#,
        r#"{"type":"string","charset":"iso-8859-1"}"#,
        r#"{"type":"string","layout":"blob"}"#,
    ] {
        assert!(DataType::from_json(retired).is_err(), "{retired}");
    }
}

#[test]
fn a_string_value_survives_the_scalar_wire_format() {
    for spelling in [
        "string(windows-1252)",
        "fixed_string(windows-1252,8)",
        "large_utf8_view",
        "utf8(32)",
        "ascii",
        "sized_ascii(4)",
        "large_cp1252_view",
    ] {
        let dtype = DataType::from_str(spelling).unwrap();
        // US-ASCII is a repertoire, so its columns get text they can hold.
        let text = match dtype.charset() {
            Some(Charset::Ascii) => "cafe",
            _ => "caf\u{e9}",
        };
        let value = dtype.scalar(text).unwrap();
        let json = serde_json::to_string(&value).unwrap();
        let read: Scalar = serde_json::from_str(&json).unwrap();
        assert_eq!(read, value, "{spelling}");
        // A value declares the leaf it is stored in; the maximum is the
        // column's rule about values, not part of one.
        assert_eq!(read.id(), value.id(), "{spelling}");
        assert_eq!(dtype.scalar(read).unwrap(), value, "{spelling}");
    }

    // The ordinary string still writes its characters and nothing else; a
    // value that declares more writes the leaf and a width, never a
    // maximum and never a charset - the leaf's name says it.
    assert_eq!(
        serde_json::to_string(&Scalar::from("AAPL")).unwrap(),
        r#"{"type":"string","value":"AAPL"}"#
    );
    assert_eq!(
        serde_json::to_string(
            &DataType::from_str("utf8(32)")
                .unwrap()
                .scalar("AAPL")
                .unwrap()
        )
        .unwrap(),
        r#"{"type":"string","value":"AAPL"}"#
    );
    assert_eq!(
        serde_json::to_string(&DataType::fixed_ascii(4).unwrap().scalar("USD").unwrap()).unwrap(),
        r#"{"type":"string","value":{"layout":"fixed_ascii","fixed":4,"text":"USD"}}"#
    );
    assert_eq!(
        serde_json::to_string(&DataType::ascii().scalar("USD").unwrap()).unwrap(),
        r#"{"type":"string","value":{"layout":"ascii","text":"USD"}}"#
    );
    assert_eq!(
        serde_json::to_string(&DataType::sized_cp1252(8).unwrap().scalar("x").unwrap()).unwrap(),
        r#"{"type":"string","value":{"layout":"cp1252","text":"x"}}"#
    );

    // An older value document spelled a shape beside a charset, and reads
    // as the leaf it meant.
    for (document, spelling) in [
        (
            r#"{"type":"string","value":{"layout":"fixed_string","charset":"us-ascii","fixed":4,"text":"USD"}}"#,
            "fixed_ascii(4)",
        ),
        (
            r#"{"type":"string","value":{"layout":"string","charset":"us-ascii","text":"USD"}}"#,
            "ascii",
        ),
        (
            r#"{"type":"string","value":{"layout":"large_string","charset":"windows-1252","text":"USD"}}"#,
            "large_cp1252",
        ),
    ] {
        let read: Scalar = serde_json::from_str(document).unwrap();
        assert_eq!(
            read,
            DataType::from_str(spelling).unwrap().scalar("USD").unwrap(),
            "{document}"
        );
        assert_eq!(read.dtype().unwrap().to_string(), spelling, "{document}");
    }
    // A numbered leaf with no width, a width on a leaf that takes none, and
    // text that does not fit are refused.
    for retired in [
        r#"{"type":"string","value":{"layout":"fixed_utf8","text":"x"}}"#,
        r#"{"type":"string","value":{"layout":"large_utf8","fixed":4,"text":"x"}}"#,
        r#"{"type":"string","value":{"layout":"fixed_ascii","fixed":4,"text":"EURO!"}}"#,
        r#"{"type":"string","value":{"layout":"ascii","text":"café"}}"#,
        r#"{"type":"string","value":{"layout":"string","charset":"iso-8859-1","text":"x"}}"#,
    ] {
        assert!(
            serde_json::from_str::<Scalar>(retired).is_err(),
            "{retired}"
        );
    }
}

#[test]
fn a_hand_built_string_with_no_width_is_refused_before_a_boundary() {
    // The variant is public, so a caller can build what the constructor would
    // have refused; `validate` is where that stops.
    let unwidened = DataType::String(StringType::FixedCp1252String(0));
    assert!(unwidened.validate().is_err());
    assert!(unwidened.clone().into_arrow_datatype().is_err());
    assert!(
        unwidened
            .clone()
            .nullable_field("value")
            .into_arrow_field()
            .is_err()
    );
}

#[test]
fn a_cast_into_another_charset_re_encodes_and_refuses_what_it_cannot_spell() {
    use arrow_array::{Array, ArrayRef, BinaryArray, StringArray};
    use std::sync::Arc;
    use yggdryl::ArrowCastOptions;

    // Arrow's kernel would hand a UTF-8 buffer to a windows-1252 column and
    // call it a framing change, and the column would read back as mojibake.
    // The string cast re-encodes every cell instead, and a scalar the
    // charset has no byte for is refused naming the charset, the row and
    // the column.
    let source: ArrayRef = Arc::new(StringArray::from(vec![Some("café"), None]));
    let target = DataType::from_str("string(windows-1252)")
        .unwrap()
        .nullable_field("value");
    let cast = target
        .cast_arrow_array(source, ArrowCastOptions::new().with_safe(false))
        .unwrap();
    let cast = cast.as_any().downcast_ref::<BinaryArray>().unwrap();
    assert_eq!(cast.value(0), &[0x63, 0x61, 0x66, 0xE9]);
    assert!(cast.is_null(1));

    let source: ArrayRef = Arc::new(StringArray::from(vec![Some("café"), Some("東京")]));
    let refusal = target
        .cast_arrow_array(
            Arc::clone(&source),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("windows-1252"), "{refusal}");
    assert!(refusal.contains("row 1"), "{refusal}");
    assert!(refusal.contains("\"value\""), "{refusal}");
    // Under `safe`, the cell that cannot be spelled becomes null.
    let lenient = target
        .cast_arrow_array(source, ArrowCastOptions::new())
        .unwrap();
    let lenient = lenient.as_any().downcast_ref::<BinaryArray>().unwrap();
    assert_eq!(lenient.value(0), &[0x63, 0x61, 0x66, 0xE9]);
    assert!(lenient.is_null(1));

    // A field already in that exact string reads back as itself.
    let arrow = target.clone().into_arrow_field().unwrap();
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), target);
}

#[test]
fn a_charset_a_column_cannot_write_is_refused_where_the_bytes_are_written() {
    // The value door counts rather than judges, because the permissive read
    // recovers damage precisely by answering scalars the charset does not
    // assign. The write is where a scalar with no byte is refused, by name.
    let dtype = DataType::from_str("string(windows-1252)").unwrap();
    let value = dtype.scalar("東京").unwrap();
    assert_eq!(value.as_str(), Some("東京"));

    let field = dtype.nullable_field("value");
    let refusal = yggdryl::FieldScalar::new(&field, value)
        .unwrap()
        .into_arrow_array()
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("windows-1252"), "{refusal}");

    // A recovered byte survives the round trip the strict door would refuse.
    let damaged = DataType::from_str("string(windows-1252)")
        .unwrap()
        .scalar(Scalar::from(vec![0x6F_u8, 0x6B, 0x81]))
        .unwrap();
    assert_eq!(damaged.as_str(), Some("ok\u{0081}"));
}
