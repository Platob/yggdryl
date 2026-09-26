//! `rust/src/xml/scanner.rs`: the scanner every object-store XML answer is
//! read through.
//!
//! S3, Azure Blob Storage and STS each read their own vocabulary on top of
//! this scanner, and those readers are pinned beside their owners. What is
//! pinned here is the reading itself: what it tolerates around and inside a
//! root, what it decodes, what it answers for a child, and what it refuses -
//! every malformed document naming the byte where the scan stopped.

use yggdryl::internals::xml_scanner::{Element, XmlError, parse_document, parse_root};

/// The root of a document the test expects to be well formed.
fn document(xml: &str) -> Element {
    parse_document(xml.as_bytes()).expect("a well-formed document")
}

/// The message a malformed document is refused with.
fn refusal(xml: &[u8]) -> String {
    let error: XmlError = parse_document(xml).expect_err("a refusal");
    error.0
}

/// `levels` nested `<d>` elements around `innermost`.
fn nested(levels: usize, innermost: &str) -> String {
    format!(
        "{}{innermost}{}",
        "<d>".repeat(levels),
        "</d>".repeat(levels)
    )
}

#[test]
fn a_prolog_trailing_comments_and_a_byte_order_mark_surround_the_root() {
    let root = document(
        "\u{FEFF}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!-- issued by the token service -->\n\
         <?xml-stylesheet href=\"none\"?>\n\
         <AssumeRoleResponse><RequestId>c6104cbe</RequestId></AssumeRoleResponse>\n\
         <!-- trailer -->\n<?done?>\r\n\t ",
    );
    assert_eq!(
        root.name(),
        "AssumeRoleResponse",
        "the root is the one element past the prolog"
    );
    assert_eq!(
        root.child_text("RequestId"),
        Some("c6104cbe"),
        "the root's content is read"
    );
    assert_eq!(
        document("<Empty/>").name(),
        "Empty",
        "a bare self-closing root needs nothing around it"
    );
}

#[test]
fn namespace_prefixes_are_dropped_so_names_match_locally() {
    let root = document(
        "<sts:AssumeRoleResponse xmlns:sts=\"https://sts.amazonaws.com/doc/2011-06-15/\">\
         <sts:AssumeRoleResult><sts:Credentials>\
         <sts:AccessKeyId>ASIAEXAMPLE</sts:AccessKeyId>\
         </sts:Credentials></sts:AssumeRoleResult></sts:AssumeRoleResponse>",
    );
    assert_eq!(
        root.name(),
        "AssumeRoleResponse",
        "the root answers its local name"
    );
    let credentials = root
        .child("AssumeRoleResult")
        .and_then(|result| result.child("Credentials"))
        .expect("the credentials reached by local names");
    assert_eq!(
        credentials.child_text("AccessKeyId"),
        Some("ASIAEXAMPLE"),
        "a prefixed leaf reads by its local name"
    );
    assert!(
        root.child("sts:AssumeRoleResult").is_none(),
        "a prefixed name matches nothing: names are local"
    );
    // The prefix is dropped from the name, never from the matching: a closing
    // tag repeats the qualified name it closes.
    assert_eq!(
        refusal(b"<a:Key></b:Key>"),
        "<a:Key> closed by </b:Key> at byte 14",
        "another prefix does not close the element"
    );
    assert_eq!(
        refusal(b"<a:Key></Key>"),
        "<a:Key> closed by </Key> at byte 12",
        "the bare local name does not close a prefixed element"
    );
}

#[test]
fn attributes_are_skipped_whole_even_when_a_value_holds_markup() {
    let root = document(
        "<Root xmlns=\"urn:example\" test=\"a > b\" slash='x/>y'\n\t empty = \"\"  >\
         <Child quoted=\"it's\" other='say \"hi\"' angle=\"<not/>\">text</Child>\
         <Empty only='>' /></Root>",
    );
    assert_eq!(
        root.name(),
        "Root",
        "the root survives attributes holding `>` and `/>`"
    );
    assert_eq!(
        root.child_text("Child"),
        Some("text"),
        "no attribute value leaks into the text"
    );
    assert!(
        root.child("not").is_none(),
        "markup inside an attribute value is no element"
    );
    assert_eq!(
        root.child_text("Empty"),
        Some(""),
        "a self-closing element after an attribute holding `>`"
    );
    assert_eq!(
        refusal(b"<Key checked></Key>"),
        "expected `=` at byte 12",
        "an attribute without a value"
    );
    assert_eq!(
        refusal(b"<Key a=\"open></Key>"),
        "unterminated attribute value at byte 8",
        "a value whose quote never closes"
    );
}

#[test]
fn whitespace_inside_tags_is_tolerated() {
    let root = document("<Response\n>\r\n\t<Key >k</Key\t>\n<Empty\n/>\n</Response >");
    assert_eq!(
        root.child_text("Key"),
        Some("k"),
        "whitespace before `>` in an opening and a closing tag"
    );
    assert_eq!(root.child_text("Empty"), Some(""), "whitespace before `/>`");
}

#[test]
fn the_five_named_entities_and_decimal_and_hex_references_decode() {
    let root = document(
        "<Tags><Value>R&amp;D &lt;core&gt; &quot;q&quot; &apos;a&apos;</Value>\
         <Numeric>&#72;&#105;&#33; &#xe9;&#xE9; &#x263A; &#128512; &#10;end</Numeric></Tags>",
    );
    assert_eq!(
        root.child_text("Value"),
        Some("R&D <core> \"q\" 'a'"),
        "each of the five named entities"
    );
    assert!(
        root.child("Value")
            .expect("the value")
            .child("core")
            .is_none(),
        "a decoded `&lt;` is text, never markup"
    );
    assert_eq!(
        root.child_text("Numeric"),
        Some("Hi! \u{e9}\u{e9} \u{263A} \u{1F600} \nend"),
        "decimal references and hex references in either digit case"
    );
}

#[test]
fn a_reference_that_names_no_character_is_refused_where_its_text_begins() {
    let refused: [(&[u8], &str, &str); 7] = [
        (
            b"<Key><Part/>2 &copy; 2026</Key>",
            "unknown reference `&copy;` at byte 12",
            "an HTML entity XML does not define, in text after a child",
        ),
        (
            b"<Key>&#X41;</Key>",
            "unknown reference `&#X41;` at byte 5",
            "XML spells the hex marker in lower case",
        ),
        (
            b"<Key>&#x110000;</Key>",
            "unknown reference `&#x110000;` at byte 5",
            "a code point past Unicode",
        ),
        (
            b"<Key>&#99999999999;</Key>",
            "unknown reference `&#99999999999;` at byte 5",
            "a decimal reference past any code point",
        ),
        (
            b"<Key>&#12a;</Key>",
            "unknown reference `&#12a;` at byte 5",
            "a hex digit in a decimal reference",
        ),
        (
            b"<Key>AT&T</Key>",
            "unterminated reference `&T` at byte 5",
            "a bare ampersand",
        ),
        (
            b"<Key>&unterminated-and-long</Key>",
            "unterminated reference `&untermin` at byte 5",
            "the quoted text is bounded to eight characters",
        ),
    ];
    for (xml, message, case) in refused {
        assert_eq!(refusal(xml), message, "{case}");
    }
}

#[test]
fn cdata_comments_and_processing_instructions_inside_an_element_are_read_in_place() {
    let root = document(
        "<Policy><!-- before --><Statement>allow<!-- not text <Deny/> -->:\
         <![CDATA[<Action> & &amp; ]]b]]><?audit skip?>*</Statement><?after?><Effect/></Policy>",
    );
    assert_eq!(
        root.child_text("Statement"),
        Some("allow:<Action> & &amp; ]]b*"),
        "CDATA is verbatim, comments and processing instructions add nothing"
    );
    let statement = root.child("Statement").expect("the statement");
    assert!(
        statement.child("Deny").is_none() && statement.child("Action").is_none(),
        "markup inside a comment or a CDATA section is no element"
    );
    assert!(
        root.child("Effect").is_some(),
        "an element after a processing instruction is still a child"
    );
    assert_eq!(
        refusal(b"<Policy><![CDATA[open</Policy>"),
        "unterminated CDATA section at byte 17",
        "a CDATA section that never closes"
    );
    assert_eq!(
        refusal(b"<Policy><?audit</Policy>"),
        "unterminated processing instruction at byte 10",
        "a processing instruction that never closes"
    );
}

#[test]
fn an_element_holds_its_own_character_data_verbatim_and_not_its_childrens() {
    let root = document(
        "<Outer><Mixed> head <Inner>inside</Inner> tail </Mixed><Size>\n  7\n</Size></Outer>",
    );
    assert_eq!(
        root.child_text("Mixed"),
        Some(" head  tail "),
        "the text around a child, without the child's"
    );
    assert_eq!(
        root.child("Mixed")
            .and_then(|mixed| mixed.child_text("Inner")),
        Some("inside"),
        "the child's text is the child's"
    );
    assert_eq!(
        root.child_text("Size"),
        Some("\n  7\n"),
        "whitespace is kept: trimming is the reader's"
    );
}

#[test]
fn children_answers_every_match_in_document_order_and_child_the_first() {
    let root = document(
        "<Delete><Object><Key>b</Key></Object><Quiet>true</Quiet>\
         <s3:Object><Key>a</Key></s3:Object><!-- between --><Object><Key>c</Key></Object></Delete>",
    );
    let keys: Vec<&str> = root
        .children("Object")
        .map(|object| object.child_text("Key").expect("a key"))
        .collect();
    assert_eq!(
        keys,
        ["b", "a", "c"],
        "document order across other siblings and prefixes"
    );
    assert_eq!(
        root.children("Missing").count(),
        0,
        "no match is an empty iterator"
    );
    assert_eq!(
        root.child("Object")
            .and_then(|object| object.child_text("Key")),
        Some("b"),
        "child answers the first match"
    );
    let same_names = document("<x><x><x>deep</x></x></x>");
    assert_eq!(
        same_names
            .child("x")
            .and_then(|middle| middle.child_text("x")),
        Some("deep"),
        "a closing tag closes the innermost element of its name"
    );
}

#[test]
fn child_text_tells_an_empty_child_from_a_missing_one_and_required_names_the_missing_one() {
    let root = document(
        "<sts:Credentials xmlns:sts=\"urn:sts\"><AccessKeyId>ASIAFIRST</AccessKeyId>\
         <AccessKeyId>ASIASECOND</AccessKeyId><SessionToken></SessionToken><Expiration/>\
         </sts:Credentials>",
    );
    assert_eq!(
        root.required("AccessKeyId").expect("a present child"),
        "ASIAFIRST",
        "required answers the first match"
    );
    assert_eq!(
        root.child_text("SessionToken"),
        Some(""),
        "a present, empty child"
    );
    assert_eq!(
        root.child_text("Expiration"),
        Some(""),
        "a self-closing child reads as empty too"
    );
    assert_eq!(root.child_text("SecretAccessKey"), None, "an absent child");
    let error = root
        .required("SecretAccessKey")
        .expect_err("a missing child");
    assert_eq!(
        error.0, "<Credentials> without <SecretAccessKey>",
        "the parent by its local name, the child by the name asked for"
    );
    assert_eq!(error.to_string(), error.0, "Display is the message");
    let source: &dyn std::error::Error = &error;
    assert!(source.source().is_none(), "the refusal wraps nothing");
}

#[test]
fn parse_root_answers_the_named_root_and_refuses_another_by_name() {
    let answer: &[u8] = b"<sts:AssumeRoleResponse xmlns:sts=\"urn:sts\">\
        <sts:ResponseMetadata/></sts:AssumeRoleResponse>";
    let root = parse_root(answer, "AssumeRoleResponse").expect("the named root");
    assert!(
        root.child("ResponseMetadata").is_some(),
        "the root answered is the whole document"
    );
    let cases: [(&[u8], &str, &str, &str); 4] = [
        (
            b"<ErrorResponse><Error><Code>ExpiredToken</Code></Error></ErrorResponse>",
            "AssumeRoleResponse",
            "expected <AssumeRoleResponse>, found <ErrorResponse>",
            "another root is refused by both names",
        ),
        (
            b"<assumeroleresponse/>",
            "AssumeRoleResponse",
            "expected <AssumeRoleResponse>, found <assumeroleresponse>",
            "names are matched case-sensitively",
        ),
        (
            answer,
            "sts:AssumeRoleResponse",
            "expected <sts:AssumeRoleResponse>, found <AssumeRoleResponse>",
            "the root is matched by its local name only",
        ),
        (
            b"<AssumeRoleResponse>",
            "AssumeRoleResponse",
            "<AssumeRoleResponse> never closed at byte 20",
            "a malformed document is refused before its root is named",
        ),
    ];
    for (xml, expected, message, case) in cases {
        assert_eq!(
            parse_root(xml, expected).expect_err("a refusal").0,
            message,
            "{case}"
        );
    }
}

#[test]
fn malformed_documents_are_refused_naming_the_byte_where_the_scan_stopped() {
    let malformed: [(&[u8], &str, &str); 12] = [
        (
            b"<Response><Result></Response>",
            "<Result> closed by </Response> at byte 28",
            "a mismatched closing tag",
        ),
        (
            b"<Code>x</code>",
            "<Code> closed by </code> at byte 13",
            "names are case-sensitive",
        ),
        (
            b"<Key></Key",
            "expected `>` at byte 10",
            "a closing tag cut short",
        ),
        (
            b"<Response><Result>partial",
            "<Result> never closed at byte 25",
            "an unterminated element names the innermost one open",
        ),
        (
            b"<Response><Result/>",
            "<Response> never closed at byte 19",
            "an unterminated root after a complete child",
        ),
        (
            b"<Response",
            "expected a name at byte 9",
            "an opening tag cut short",
        ),
        (b"<>", "expected a name at byte 1", "a tag without a name"),
        (
            b"<Response/><Response/>",
            "content after the root element at byte 11",
            "a second root",
        ),
        (
            b"<Response/>\n  trailing text",
            "content after the root element at byte 14",
            "text after the root",
        ),
        (
            b"  \n<!-- c -->oops<Response/>",
            "expected the root element at byte 13",
            "text before the root, past a comment",
        ),
        (
            b"<?xml version=\"1.0\"",
            "unterminated processing instruction at byte 2",
            "a declaration that never closes",
        ),
        (
            b"<Key>caf\xE9</Key>",
            "not UTF-8: invalid utf-8 sequence of 1 bytes from index 8",
            "a windows-1252 byte where UTF-8 was promised",
        ),
    ];
    for (xml, message, case) in malformed {
        assert_eq!(refusal(xml), message, "{case}");
    }
    assert_eq!(
        refusal(b"<Key>\xE2\x82"),
        "not UTF-8: incomplete utf-8 byte sequence from index 5",
        "a body cut inside a character"
    );
}

#[test]
fn nesting_is_bounded_at_thirty_two_levels_below_the_root() {
    // The root is level zero, so an element thirty-two levels below it is the
    // deepest one read.
    let root = document(&nested(32, "<floor>reached</floor>"));
    let mut element = &root;
    for _ in 0..31 {
        element = element.child("d").expect("one level down");
    }
    assert_eq!(
        element.child_text("floor"),
        Some("reached"),
        "thirty-two levels below the root are read"
    );
    assert_eq!(
        refusal(nested(33, "<floor/>").as_bytes()),
        "elements nested deeper than 32 at byte 99",
        "an empty element one level past the bound is refused where it opens"
    );
    assert_eq!(
        refusal(nested(34, "text").as_bytes()),
        "elements nested deeper than 32 at byte 99",
        "a deeper document stops at the first element past the bound"
    );
}
