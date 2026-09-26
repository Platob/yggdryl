//! `rust/src/xml/element.rs`: the namespace-aware view over the natural XML
//! value - the scope a prefix resolves in, and an element's name, text,
//! attributes and children read by the namespace they are in rather than by
//! the prefix their author chose.

use yggdryl::serie::Run;
use yggdryl::xml::{Element, Scope, XSD_NAMESPACE, XSI_NAMESPACE};
use yggdryl::{DataType, Error, Scalar, Serie, from_xml_scalar, into_xml_scalar};

const SOAP: &str = "http://schemas.xmlsoap.org/soap/envelope/";
const XMLA: &str = "urn:schemas-microsoft-com:xml-analysis";
/// The namespace the reserved `xml` prefix is bound to by the recommendation.
const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";
/// The namespace the reserved `xmlns` prefix is bound to by the
/// recommendation, which every namespace declaration is in.
const XMLNS_NAMESPACE: &str = "http://www.w3.org/2000/xmlns/";

/// The natural value of a literal document.
fn parsed(xml: &str) -> Scalar {
    from_xml_scalar(xml).unwrap_or_else(|error| panic!("{xml}: {error}"))
}

fn record<const N: usize>(entries: [(&str, Scalar); N]) -> Scalar {
    Scalar::from_struct(entries).unwrap()
}

fn root(document: &Scalar) -> Element<'_> {
    Element::root(document).unwrap_or_else(|error| panic!("{document:?}: {error}"))
}

/// The refusal `Element::root` answers for `document`, which is a codec error
/// of the XML format.
fn refused_root(document: &Scalar) -> String {
    let error = Element::root(document).expect_err("a refusal");
    assert!(
        matches!(error, Error::Codec { format: "xml", .. }),
        "{error:?}"
    );
    error.to_string()
}

/// The refusal `one_child_in` answers, which is a codec error of the XML
/// format.
fn refused_one(element: &Element<'_>, namespace: &str, local: &str) -> String {
    let error = element
        .one_child_in(namespace, local)
        .expect_err("a refusal");
    assert!(
        matches!(error, Error::Codec { format: "xml", .. }),
        "{error:?}"
    );
    error.to_string()
}

/// Every child's name as spelled, in the order `children` answers them.
fn names<'a>(element: &'a Element<'_>) -> Vec<&'a str> {
    element.children().map(|child| child.name()).collect()
}

/// Every child's own text, in the order `children` answers them.
fn texts(element: &Element<'_>) -> Vec<Option<String>> {
    element
        .children()
        .map(|child| child.text().map(str::to_owned))
        .collect()
}

#[test]
fn the_schema_namespaces_are_the_w3c_names() {
    assert_eq!(XSD_NAMESPACE, "http://www.w3.org/2001/XMLSchema");
    assert_eq!(XSI_NAMESPACE, "http://www.w3.org/2001/XMLSchema-instance");
}

#[test]
fn an_empty_scope_resolves_no_prefix_and_no_default_namespace() {
    const EMPTY: Scope = Scope::new();
    assert_eq!(EMPTY, Scope::default());
    assert_eq!(
        EMPTY.resolve(""),
        None,
        "an undeclared default is no namespace"
    );
    assert_eq!(EMPTY.resolve("s"), None);
    assert_eq!(EMPTY.prefix_of(SOAP), None);
    assert_eq!(EMPTY.prefix_of(""), None);
}

#[test]
fn the_xml_prefix_resolves_in_every_scope_without_a_declaration() {
    assert_eq!(Scope::new().resolve("xml"), Some(XML_NAMESPACE));
    assert_eq!(
        Scope::new().with("xml", "urn:other").resolve("xml"),
        Some(XML_NAMESPACE),
        "the reserved binding is the recommendation's and no declaration moves it"
    );
    assert_eq!(
        Scope::new().resolve("XML"),
        None,
        "a prefix is case-sensitive"
    );
}

#[test]
fn a_binding_resolves_its_prefix_and_the_empty_prefix_is_the_default_namespace() {
    let scope = Scope::new().with("s", SOAP).with("", XMLA);
    assert_eq!(scope.resolve("s"), Some(SOAP));
    assert_eq!(scope.resolve(""), Some(XMLA));
    assert_eq!(
        scope.resolve("x"),
        None,
        "an unbound prefix is no namespace"
    );
    assert_eq!(scope.resolve("S"), None);

    let mut bound = Scope::new();
    bound.bind("s", SOAP);
    bound.bind("", XMLA);
    assert_eq!(bound, scope, "`with` is `bind` on an owned scope");
    assert_ne!(bound, Scope::new());
}

#[test]
fn the_innermost_binding_of_a_prefix_wins() {
    let scope = Scope::new()
        .with("p", "urn:one")
        .with("", "urn:outer")
        .with("p", "urn:two")
        .with("", "urn:inner");
    assert_eq!(scope.resolve("p"), Some("urn:two"));
    assert_eq!(scope.resolve(""), Some("urn:inner"));
}

#[test]
fn an_empty_namespace_undeclares_the_default_and_a_prefix() {
    let scope = Scope::new().with("", "urn:a").with("", "");
    assert_eq!(
        scope.resolve(""),
        None,
        "`xmlns=\"\"` undeclares the default"
    );
    assert_eq!(
        scope.clone().with("", "urn:b").resolve(""),
        Some("urn:b"),
        "a later declaration binds it again"
    );
    let scope = Scope::new().with("p", "urn:a").with("p", "");
    assert_eq!(scope.resolve("p"), None);
}

#[test]
fn prefix_of_answers_the_innermost_prefix_bound_to_a_namespace() {
    let scope = Scope::new().with("a", "urn:x").with("b", "urn:x");
    assert_eq!(scope.prefix_of("urn:x"), Some("b"));
    assert_eq!(scope.prefix_of("urn:y"), None);

    let scope = scope.with("", "urn:x");
    assert_eq!(
        scope.prefix_of("urn:x"),
        Some(""),
        "the default namespace is the empty prefix"
    );
    assert_eq!(Scope::new().with("s", SOAP).prefix_of(SOAP), Some("s"));
}

/// A prefix a later binding moved to another namespace, or undeclared, is not
/// bound to the first one "here" any longer: answering it names a prefix
/// that resolves elsewhere. What `prefix_of` answers, `resolve` reads back as
/// the namespace asked.
#[test]
fn prefix_of_never_answers_a_prefix_a_later_binding_shadowed() {
    let cases = [
        (
            "`p` rebound to urn:two",
            Scope::new().with("p", "urn:one").with("p", "urn:two"),
            "urn:one",
        ),
        (
            "the default undeclared",
            Scope::new().with("", "urn:a").with("", ""),
            "urn:a",
        ),
        (
            "`p` undeclared",
            Scope::new().with("p", "urn:a").with("p", ""),
            "urn:a",
        ),
    ];
    // Each answer beside what `resolve` reads that prefix as in the same
    // scope: a prefix bound to the namespace here resolves back to it.
    let answers: Vec<_> = cases
        .iter()
        .map(|(case, scope, namespace)| {
            let prefix = scope.prefix_of(namespace);
            (
                *case,
                prefix,
                prefix.and_then(|prefix| scope.resolve(prefix)),
            )
        })
        .collect();
    assert_eq!(
        answers,
        [
            ("`p` rebound to urn:two", None, None),
            ("the default undeclared", None, None),
            ("`p` undeclared", None, None),
        ],
        "no prefix in scope names the namespace a later binding took it from"
    );

    let scope = Scope::new()
        .with("p", "urn:one")
        .with("q", "urn:one")
        .with("q", "urn:two");
    assert_eq!(
        scope.prefix_of("urn:one"),
        Some("p"),
        "an outer prefix still bound to the namespace answers when the inner one moved"
    );
    assert_eq!(scope.resolve("p"), Some("urn:one"));

    let scope = Scope::new().with("p", "urn:a").with("p", "");
    assert_eq!(
        scope.prefix_of(""),
        None,
        "an undeclaration binds no prefix to the empty namespace"
    );
    assert_eq!(Scope::new().with("", "").prefix_of(""), None);
}

/// `resolve` answers the reserved `xml` binding in every scope; the prefix
/// bound to that namespace is therefore `xml` in every scope too.
#[test]
fn prefix_of_answers_the_reserved_xml_prefix_for_the_xml_namespace() {
    let scope = Scope::new();
    assert_eq!(scope.resolve("xml"), Some(XML_NAMESPACE));
    assert_eq!(scope.prefix_of(XML_NAMESPACE), Some("xml"));
    assert_eq!(
        Scope::new().with("s", SOAP).prefix_of(XML_NAMESPACE),
        Some("xml"),
        "other declarations leave the reserved binding alone"
    );

    let scope = Scope::new().with("xml", "urn:other");
    assert_eq!(
        scope.prefix_of("urn:other"),
        None,
        "a declaration cannot move `xml`, so `xml` names no other namespace"
    );
    assert_eq!(scope.prefix_of(XML_NAMESPACE), Some("xml"));
}

#[test]
fn the_root_is_the_document_element_read_in_the_empty_scope() {
    let document = parsed(
        "<SOAP-ENV:Envelope xmlns:SOAP-ENV=\"http://schemas.xmlsoap.org/soap/envelope/\">\
           <SOAP-ENV:Body/>\
         </SOAP-ENV:Envelope>",
    );
    let envelope = root(&document);
    assert_eq!(envelope.name(), "SOAP-ENV:Envelope");
    assert_eq!(envelope.local_name(), "Envelope");
    assert_eq!(envelope.prefix(), Some("SOAP-ENV"));
    assert_eq!(envelope.namespace(), Some(SOAP));
    assert_eq!(envelope.scope(), &Scope::new().with("SOAP-ENV", SOAP));
    assert_eq!(
        envelope.value(),
        document
            .as_struct()
            .unwrap()
            .get("SOAP-ENV:Envelope")
            .unwrap(),
        "the view borrows the one entry the record names"
    );
}

#[test]
fn the_root_resolves_an_unprefixed_name_under_its_own_default_declaration() {
    let document = parsed("<Discover xmlns=\"urn:schemas-microsoft-com:xml-analysis\"/>");
    let discover = root(&document);
    assert_eq!(discover.name(), "Discover");
    assert_eq!(discover.local_name(), "Discover");
    assert_eq!(discover.prefix(), None);
    assert_eq!(discover.namespace(), Some(XMLA));

    let document = parsed("<a/>");
    let bare = root(&document);
    assert_eq!(bare.namespace(), None, "no declaration is no namespace");
    assert_eq!(bare.scope(), &Scope::new());
    assert!(bare.is_nil());
}

/// The module's own reading: `SOAP-ENV:Envelope`, `soap:Envelope` and a bare
/// `Envelope` under a default declaration are one element.
#[test]
fn the_three_spellings_of_the_envelope_are_one_element() {
    for xml in [
        "<SOAP-ENV:Envelope xmlns:SOAP-ENV=\"http://schemas.xmlsoap.org/soap/envelope/\"/>",
        "<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\"/>",
        "<Envelope xmlns=\"http://schemas.xmlsoap.org/soap/envelope/\"/>",
    ] {
        let document = parsed(xml);
        let envelope = root(&document);
        assert_eq!(envelope.namespace(), Some(SOAP), "{xml}");
        assert_eq!(envelope.local_name(), "Envelope", "{xml}");
        assert!(envelope.is(Some(SOAP), "Envelope"), "{xml}");
        assert!(envelope.is_in(SOAP, "Envelope"), "{xml}");
        assert!(!envelope.is(None, "Envelope"), "{xml}");
    }
}

#[test]
fn the_root_refuses_a_value_that_is_not_a_record_naming_its_element() {
    for (value, kind) in [
        (Scalar::from("<a/>"), "string"),
        (Scalar::Null, "null"),
        (Scalar::from_sequence([Scalar::from("a")]), "serie"),
        (Scalar::from(7_i64), "i64"),
        (Scalar::from(true), "boolean"),
    ] {
        let message = refused_root(&value);
        assert_eq!(
            message,
            format!(
                "invalid xml data at byte 0: expected an XML document, \
                 the record naming its document element, got {kind}"
            )
        );
    }
}

#[test]
fn the_root_refuses_an_empty_record_and_one_with_several_entries() {
    let message = refused_root(&record([]));
    assert_eq!(
        message,
        "invalid xml data at byte 0: expected an XML document naming its document element, \
         got an empty record"
    );
    let message = refused_root(&record([
        ("a", Scalar::from("1")),
        ("b", Scalar::from("2")),
    ]));
    assert_eq!(
        message,
        "invalid xml data at byte 0: expected an XML document naming one document element, \
         got several entries"
    );
}

#[test]
fn an_element_built_on_an_explicit_scope_resolves_through_it_and_its_own_declarations() {
    let scope = Scope::new().with("s", SOAP);
    let value = record([
        ("@xmlns:m", Scalar::from(XMLA)),
        ("m:Discover", Scalar::from("x")),
    ]);
    let body = Element::new("s:Body", &value, &scope);
    assert_eq!(body.namespace(), Some(SOAP));
    assert_eq!(body.scope(), &scope.clone().with("m", XMLA));
    assert_eq!(
        scope,
        Scope::new().with("s", SOAP),
        "the caller's scope is read, never extended in place"
    );
    let discover = body.child(Some(XMLA), "Discover").expect("the method");
    assert_eq!(discover.name(), "m:Discover");
    assert_eq!(discover.scope().resolve("s"), Some(SOAP));

    let leaf = Scalar::from("text");
    let leaf = Element::new("s:Fault", &leaf, &scope);
    assert_eq!(leaf.scope(), &scope, "a leaf declares nothing");
    assert_eq!(leaf.namespace(), Some(SOAP));
}

#[test]
fn an_elements_own_declaration_applies_to_its_own_name() {
    let value = record([("@xmlns:p", Scalar::from("urn:two"))]);
    let element = Element::new("p:a", &value, &Scope::new().with("p", "urn:one"));
    assert_eq!(element.namespace(), Some("urn:two"));

    let value = record([("@xmlns", Scalar::from("urn:inner"))]);
    let element = Element::new("a", &value, &Scope::new().with("", "urn:outer"));
    assert_eq!(element.namespace(), Some("urn:inner"));
}

/// Only an `xmlns` attribute or an `xmlns:` one declares: an attribute whose
/// name merely starts with `xmlns`, and a child element spelled like a
/// declaration, bind nothing.
#[test]
fn only_an_xmlns_attribute_declares_a_prefix() {
    let document = parsed("<q:e xmlnsq=\"urn:q\"/>");
    let e = root(&document);
    assert_eq!(e.scope(), &Scope::new());
    assert_eq!(e.namespace(), None, "`xmlnsq` is an ordinary attribute");
    assert_eq!(e.attribute_in(None, "xmlnsq"), Some("urn:q"));

    let value = record([
        ("xmlns:q", Scalar::from("urn:q")),
        ("xmlns", Scalar::from("urn:d")),
    ]);
    let e = Element::new("q:e", &value, &Scope::new().with("", "urn:outer"));
    assert_eq!(e.scope(), &Scope::new().with("", "urn:outer"));
    assert_eq!(e.namespace(), None);
    assert_eq!(names(&e), ["xmlns", "xmlns:q"], "both are child elements");
    assert_eq!(
        e.child(Some("urn:outer"), "xmlns")
            .map(|child| child.name()),
        Some("xmlns"),
        "an unprefixed child keeps the outer default"
    );
}

#[test]
fn children_inherit_the_declarations_in_scope_above_them() {
    let document = parsed(
        "<r xmlns=\"urn:d\" xmlns:p=\"urn:p\">\
           <c><p:g>1</p:g><h>2</h></c>\
         </r>",
    );
    let r = root(&document);
    assert_eq!(r.namespace(), Some("urn:d"));
    let c = r
        .child(Some("urn:d"), "c")
        .expect("c in the default namespace");
    assert_eq!(
        c.scope(),
        r.scope(),
        "a child that declares nothing inherits"
    );
    let g = c
        .child(Some("urn:p"), "g")
        .expect("g under the inherited prefix");
    assert_eq!(g.text(), Some("1"));
    let h = c
        .child(Some("urn:d"), "h")
        .expect("h under the inherited default");
    assert_eq!(h.text(), Some("2"));
    assert!(
        c.child(None, "h").is_none(),
        "h is in urn:d, not in no namespace"
    );
}

#[test]
fn a_child_that_rebinds_a_prefix_or_the_default_resolves_under_its_own_binding() {
    let document = parsed(
        "<p:r xmlns:p=\"urn:one\" xmlns=\"urn:outer\">\
           <p:c xmlns:p=\"urn:two\"><p:g/></p:c>\
           <d xmlns=\"urn:inner\"><e/></d>\
           <f/>\
         </p:r>",
    );
    let r = root(&document);
    assert_eq!(r.namespace(), Some("urn:one"));
    let c = r
        .child(Some("urn:two"), "c")
        .expect("c under its own binding");
    assert!(r.child(Some("urn:one"), "c").is_none());
    let g = c.children().next().expect("g");
    assert_eq!(g.namespace(), Some("urn:two"));

    let d = r
        .child(Some("urn:inner"), "d")
        .expect("d under its own default");
    let e = d.children().next().expect("e");
    assert_eq!(e.namespace(), Some("urn:inner"));
    let f = r
        .child(Some("urn:outer"), "f")
        .expect("f keeps the outer default");
    assert_eq!(f.name(), "f");
}

#[test]
fn a_declaration_reaches_no_sibling() {
    let document = parsed("<r><a xmlns:p=\"urn:p\"><p:x/></a><p:b/></r>");
    let r = root(&document);
    let b = r.children().find(|child| child.name() == "p:b").expect("b");
    assert_eq!(
        b.namespace(),
        None,
        "`p` was declared on a sibling, not here"
    );
    let a = r.child(None, "a").expect("a");
    assert_eq!(a.children().next().expect("x").namespace(), Some("urn:p"));
}

#[test]
fn an_empty_default_declaration_puts_a_subtree_in_no_namespace() {
    let document = parsed("<r xmlns=\"urn:d\"><c xmlns=\"\"><d/></c></r>");
    let r = root(&document);
    let c = r.child(None, "c").expect("c in no namespace");
    assert_eq!(c.namespace(), None);
    let d = c.child(None, "d").expect("d in no namespace");
    assert!(d.is(None, "d"));
    assert!(!d.is(Some("urn:d"), "d"));
}

#[test]
fn two_prefixes_bound_to_one_namespace_name_one_element() {
    let document = parsed(
        "<r xmlns:a=\"urn:x\" xmlns:b=\"urn:x\" xmlns=\"urn:x\">\
           <a:k>1</a:k><b:k>2</b:k><k>3</k>\
         </r>",
    );
    let r = root(&document);
    let found: Vec<_> = r
        .children_in(Some("urn:x"), "k")
        .iter()
        .map(|child| (child.name(), child.text().map(str::to_owned)))
        .collect();
    assert_eq!(
        found,
        [
            ("a:k", Some("1".to_owned())),
            ("b:k", Some("2".to_owned())),
            ("k", Some("3".to_owned())),
        ]
    );
    let message = refused_one(&r, "urn:x", "k");
    assert!(
        message.ends_with("expected one `k` element under `r`, found several"),
        "{message}"
    );
}

#[test]
fn is_matches_the_local_name_in_exactly_the_namespace_given() {
    let document = parsed("<s:Body xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"/>");
    let body = root(&document);
    assert!(body.is(Some(SOAP), "Body"));
    assert!(
        !body.is(None, "Body"),
        "a qualified element is in its namespace"
    );
    assert!(!body.is(Some(XMLA), "Body"));
    assert!(
        !body.is(Some(SOAP), "body"),
        "a local name is case-sensitive"
    );
    assert!(
        !body.is(Some(SOAP), "s:Body"),
        "the local name carries no prefix"
    );

    let document = parsed("<Body/>");
    let bare = root(&document);
    assert!(bare.is(None, "Body"));
    assert!(!bare.is(Some(SOAP), "Body"));
}

#[test]
fn an_undeclared_prefix_is_in_no_namespace() {
    let document = parsed("<q:Body/>");
    let body = root(&document);
    assert_eq!(body.prefix(), Some("q"));
    assert_eq!(body.local_name(), "Body");
    assert_eq!(body.namespace(), None);
    assert!(body.is(None, "Body"));
    assert!(
        body.is_in(SOAP, "Body"),
        "no namespace is the intake reading"
    );
}

#[test]
fn is_in_accepts_the_namespace_given_or_none_and_refuses_another() {
    let document = parsed(
        "<r xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
            xmlns:m=\"urn:schemas-microsoft-com:xml-analysis\">\
           <s:Body/><Body/><m:Body/>\
         </r>",
    );
    let r = root(&document);
    let verdicts: Vec<_> = r
        .children()
        .map(|child| (child.name(), child.is_in(SOAP, "Body")))
        .collect();
    assert_eq!(
        verdicts,
        [("Body", true), ("m:Body", false), ("s:Body", true)]
    );
    assert!(
        r.children().all(|child| !child.is_in(SOAP, "Header")),
        "the local name must match too"
    );
}

#[test]
fn a_leaf_answers_its_text_and_an_emptied_element_the_empty_text() {
    let document = parsed("<r><a>x</a><b></b></r>");
    let r = root(&document);
    let a = r.child(None, "a").expect("a");
    assert_eq!(a.text(), Some("x"));
    assert!(!a.is_nil());
    let b = r.child(None, "b").expect("b");
    assert_eq!(
        b.text(),
        Some(""),
        "an emptied element is present and empty"
    );
    assert!(!b.is_nil());
}

#[test]
fn a_self_closed_element_is_nil_and_has_no_text() {
    let document = parsed("<r><a/><b k=\"v\"/></r>");
    let r = root(&document);
    let a = r.child(None, "a").expect("a");
    assert!(a.is_nil());
    assert_eq!(a.text(), None);
    assert_eq!(a.value(), &Scalar::Null);
    assert_eq!(a.attributes().count(), 0);

    let b = r.child(None, "b").expect("b");
    assert!(!b.is_nil(), "an attribute is something the element says");
    assert_eq!(
        b.text(),
        None,
        "a self-closed element has no text of its own"
    );
}

#[test]
fn text_beside_attributes_is_the_elements_own_text() {
    let document = parsed("<r><a k=\"v\">x</a><b k=\"v\"></b></r>");
    let r = root(&document);
    assert_eq!(r.child(None, "a").expect("a").text(), Some("x"));
    assert_eq!(r.child(None, "b").expect("b").text(), Some(""));
}

#[test]
fn an_element_holding_children_has_no_text_unless_it_carries_its_own() {
    let document = parsed("<r><a><b>1</b></a><c>lead<d>2</d></c></r>");
    let r = root(&document);
    assert_eq!(r.text(), None);
    let a = r.child(None, "a").expect("a");
    assert_eq!(a.text(), None);
    let c = r.child(None, "c").expect("c");
    assert_eq!(c.text(), Some("lead"));
    assert_eq!(names(&c), ["d"], "`#text` is not a child");
}

#[test]
fn escaped_and_cdata_text_reads_decoded() {
    let document = parsed(
        "<a k=\"x&amp;y &lt;z&gt; &quot;q&quot;\">\
           &amp; &lt;tag&gt; &quot;q&quot; &apos;s&apos; &#233;&#x263A;<![CDATA[<raw & kept>]]>\
         </a>",
    );
    let a = root(&document);
    assert_eq!(
        a.text(),
        Some("& <tag> \"q\" 's' \u{e9}\u{263a}<raw & kept>")
    );
    assert_eq!(a.attribute("k"), Some(&Scalar::from("x&y <z> \"q\"")));
}

#[test]
fn unicode_names_and_text_read_as_written() {
    let document = parsed(
        "<ñ:café xmlns:ñ=\"urn:ñ\" ñ:note=\"thé\">\
           <ñ:thé>☕ chaud</ñ:thé><日本>語</日本>\
         </ñ:café>",
    );
    let cafe = root(&document);
    assert_eq!(cafe.name(), "ñ:café");
    assert_eq!(cafe.prefix(), Some("ñ"));
    assert_eq!(cafe.local_name(), "café");
    assert_eq!(cafe.namespace(), Some("urn:ñ"));
    assert_eq!(cafe.attribute_in(Some("urn:ñ"), "note"), Some("thé"));
    assert_eq!(names(&cafe), ["ñ:thé", "日本"]);
    let the = cafe.child(Some("urn:ñ"), "thé").expect("the tea");
    assert_eq!(the.text(), Some("☕ chaud"));
    let japan = cafe.child(None, "日本").expect("the unqualified child");
    assert_eq!(japan.text(), Some("語"));
}

#[test]
fn whitespace_in_a_leaf_is_kept_and_between_children_is_dropped() {
    let document = parsed("<r>\n  <a>  x  </a>\n  <b>   </b>\n\t<c>\n</c>\n</r>");
    let r = root(&document);
    assert_eq!(r.text(), None, "indentation is no text of its own");
    assert_eq!(
        texts(&r),
        [
            Some("  x  ".to_owned()),
            Some("   ".to_owned()),
            Some("\n".to_owned())
        ]
    );
}

#[test]
fn whitespace_only_text_beside_attributes_is_kept_and_line_ends_read_as_line_feeds() {
    let document = parsed("<r>\r\n\t<a k=\"v\">  </a>\r\n\t<b>x\r\ny\rz</b>\r\n</r>");
    let r = root(&document);
    assert_eq!(
        r.text(),
        None,
        "CRLF indentation between children is no text"
    );
    assert_eq!(
        texts(&r),
        [Some("  ".to_owned()), Some("x\ny\nz".to_owned())],
        "text beside an attribute is the element's own, and every line end is a line feed"
    );
}

#[test]
fn xsi_nil_under_the_conventional_prefix_is_nil() {
    let document = parsed(
        "<r xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
           <a xsi:nil=\"true\"/><b xsi:nil=\"1\"></b><c xsi:nil=\"false\"/>\
         </r>",
    );
    let r = root(&document);
    let a = r.child(None, "a").expect("a");
    assert!(a.is_nil());
    assert_eq!(a.text(), None);
    let b = r.child(None, "b").expect("b");
    assert!(b.is_nil(), "`1` is the other spelling of true");
    let c = r.child(None, "c").expect("c");
    assert!(!c.is_nil(), "`false` marks nothing absent");
    assert_eq!(c.attribute_in(Some(XSI_NAMESPACE), "nil"), Some("false"));
}

/// The codec matches the conventional `xsi:nil` spelling as written and the
/// natural value it answers is null: the XML module's own table lists
/// `<a xsi:nil="true"/>` as null with no declaration beside it, so the view
/// has no attribute left to resolve.
#[test]
fn the_conventional_nil_spelling_reads_null_even_where_xsi_is_undeclared() {
    let document = parsed("<r><a xsi:nil=\"true\"/></r>");
    let a = root(&document).child(None, "a").map(|a| a.is_nil());
    assert_eq!(a, Some(true));
}

/// A nil mark is the `nil` attribute of the XML Schema instance namespace,
/// whatever prefix spells it: WCF's `i:nil` is the same mark as `xsi:nil`.
/// The codec matches only the conventional spelling, so this one reaches the
/// view as a record and `is_nil` resolves the attribute itself.
#[test]
fn a_nil_mark_under_any_prefix_bound_to_the_xsi_namespace_is_nil() {
    let document =
        parsed("<r xmlns:i=\"http://www.w3.org/2001/XMLSchema-instance\"><a i:nil=\"true\"/></r>");
    let r = root(&document);
    let a = r.child(None, "a").expect("a");
    assert_eq!(a.attribute_in(Some(XSI_NAMESPACE), "nil"), Some("true"));
    assert_eq!(a.text(), None);
    assert!(
        a.is_nil(),
        "`i:nil=\"true\"` under the XSI namespace is a nil mark"
    );
}

/// A namespace declaration is not an attribute the element says anything
/// with, so declaring `xsi` on the nilled element itself still marks it nil.
#[test]
fn a_nil_mark_whose_element_declares_its_own_xsi_prefix_is_nil() {
    let document = parsed(
        "<r><a xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:nil=\"true\"/></r>",
    );
    let r = root(&document);
    let a = r.child(None, "a").expect("a");
    assert_eq!(a.attribute_in(Some(XSI_NAMESPACE), "nil"), Some("true"));
    assert!(a.is_nil(), "the element is marked `xsi:nil`");
}

/// `xsi:nil` is an XML Schema boolean: `true` and `1` mark the element
/// absent, `false` and `0` do not, and a spelling outside the lexical space
/// marks nothing.
#[test]
fn a_nil_mark_is_true_or_one_and_every_other_value_marks_nothing() {
    let document = parsed(
        "<r xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
           <a xsi:nil=\"true\"/><b xsi:nil=\"1\"/><c xsi:nil=\"false\"/>\
           <d xsi:nil=\"0\"/><e xsi:nil=\"TRUE\"/><f xsi:nil=\"\"/><g xsi:nil=\"yes\"/>\
         </r>",
    );
    let r = root(&document);
    let verdicts: Vec<_> = r
        .children()
        .map(|child| (child.name(), child.is_nil()))
        .collect();
    assert_eq!(
        verdicts,
        [
            ("a", true),
            ("b", true),
            ("c", false),
            ("d", false),
            ("e", false),
            ("f", false),
            ("g", false),
        ]
    );
}

/// XML Schema Part 1 §2.6.2: an element marked `xsi:nil="true"` must be
/// empty "but can carry attributes"; the mark beside another attribute
/// still says the value is absent, which the codec leaves to the view.
#[test]
fn a_nil_mark_beside_another_attribute_is_nil() {
    let document = parsed(
        "<r xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
           <a xsi:nil=\"true\" k=\"v\"/>\
         </r>",
    );
    let r = root(&document);
    let a = r.child(None, "a").expect("a");
    assert_eq!(
        a.attribute("k"),
        Some(&Scalar::from("v")),
        "the attribute is kept"
    );
    assert_eq!(a.attribute_in(Some(XSI_NAMESPACE), "nil"), Some("true"));
    assert!(a.is_nil(), "the element is marked `xsi:nil`");
}

/// `xsi:nil` is an XML Schema boolean, whose whitespace facet is `collapse`
/// (XML Schema Part 2 §3.2.2): ` true ` is the value `true`.
#[test]
fn a_nil_mark_with_surrounding_whitespace_is_nil() {
    let document = parsed(
        "<r xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
           <a xsi:nil=\" true \"/>\
         </r>",
    );
    let r = root(&document);
    let a = r.child(None, "a").expect("a");
    assert_eq!(a.text(), None);
    assert!(a.is_nil(), "` true ` collapses to `true`");

    let document = parsed(
        "<r xmlns:i=\"http://www.w3.org/2001/XMLSchema-instance\">\
           <a i:nil=\"&#9;1&#10;\"/><b i:nil=\" false \"/>\
         </r>",
    );
    let r = root(&document);
    let verdicts: Vec<_> = r
        .children()
        .map(|child| (child.name(), child.is_nil()))
        .collect();
    assert_eq!(verdicts, [("a", true), ("b", false)]);
}

/// The mark is read by the namespace it is in: a `nil` attribute in no
/// namespace, or under a prefix bound to another namespace, marks nothing.
#[test]
fn a_nil_attribute_outside_the_xsi_namespace_marks_nothing() {
    let document = parsed(
        "<r xmlns:xsi=\"urn:not-xsi\" xmlns:i=\"http://www.w3.org/2001/XMLSchema-instance\">\
           <a nil=\"true\"/><b xsi:nil=\"true\" k=\"v\"/><c i:type=\"xsd:string\"/>\
         </r>",
    );
    let r = root(&document);
    let verdicts: Vec<_> = r
        .children()
        .map(|child| (child.name(), child.is_nil()))
        .collect();
    assert_eq!(
        verdicts,
        [("a", false), ("b", false), ("c", false)],
        "an unqualified `nil`, a `nil` in urn:not-xsi, another XSI attribute"
    );
    let b = r.child(None, "b").expect("b");
    assert_eq!(b.attribute_in(Some("urn:not-xsi"), "nil"), Some("true"));
}

/// The conventional spelling under a prefix the document bound elsewhere is
/// the same case as the one above with nothing beside it: `xsi:nil` here is
/// `nil` in urn:not-xsi, which marks nothing, and the attribute says
/// something the view must still be able to read.
#[test]
fn the_conventional_nil_spelling_is_null_whatever_xsi_is_bound_to() {
    // The codec's own rule, stated in its table: the literal spelling
    // `xsi:nil="true"` on an otherwise empty element is null before any view
    // reads it, so the mark is gone and no attribute is left to resolve.
    let document = parsed("<r xmlns:xsi=\"urn:not-xsi\"><a xsi:nil=\"true\"/></r>");
    let r = root(&document);
    let a = r.child(None, "a").expect("a");
    assert_eq!(
        (a.is_nil(), a.attribute_in(Some("urn:not-xsi"), "nil")),
        (true, None),
        "the codec reads the conventional spelling literally"
    );
}

/// A nil mark says the element is absent even where the codec kept content
/// beside it: the mark is what `is_nil` answers, and the text is still read.
#[test]
fn a_nil_mark_beside_content_is_nil_and_the_content_is_still_read() {
    let document = parsed(
        "<r xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
           <a xsi:nil=\"true\">x</a><b xsi:nil=\"true\"><c/></b>\
         </r>",
    );
    let r = root(&document);
    let a = r.child(None, "a").expect("a");
    assert!(a.is_nil());
    assert_eq!(a.text(), Some("x"));
    let b = r.child(None, "b").expect("b");
    assert!(b.is_nil());
    assert_eq!(names(&b), ["c"]);
}

#[test]
fn attributes_are_the_at_entries_in_name_order_declarations_included() {
    let document = parsed("<a z=\"1\" b=\"2\" xmlns:p=\"urn:p\" p:m=\"3\" xmlns=\"urn:d\">t</a>");
    let a = root(&document);
    let attributes: Vec<_> = a
        .attributes()
        .map(|(name, value)| (name, value.as_str()))
        .collect();
    assert_eq!(
        attributes,
        [
            ("b", Some("2")),
            ("p:m", Some("3")),
            ("xmlns", Some("urn:d")),
            ("xmlns:p", Some("urn:p")),
            ("z", Some("1")),
        ],
        "a declaration is the `@xmlns` attribute it is written as, and `#text` is none"
    );

    let document = parsed("<r><leaf>x</leaf></r>");
    let leaf = root(&document)
        .child(None, "leaf")
        .map(|leaf| leaf.attributes().count());
    assert_eq!(leaf, Some(0), "a leaf has no attributes");
}

#[test]
fn attribute_reads_a_name_as_spelled_prefix_included() {
    let document = parsed("<a xmlns:p=\"urn:p\" p:m=\"3\" k=\"\"/>");
    let a = root(&document);
    assert_eq!(a.attribute("p:m"), Some(&Scalar::from("3")));
    assert_eq!(a.attribute("m"), None, "the name is matched as spelled");
    assert_eq!(
        a.attribute("@p:m"),
        None,
        "the `@` is the codec's, not the name's"
    );
    assert_eq!(
        a.attribute("k"),
        Some(&Scalar::from("")),
        "an empty value is present"
    );
    assert_eq!(a.attribute("xmlns:p"), Some(&Scalar::from("urn:p")));
    assert_eq!(a.attribute("absent"), None);

    let document = parsed("<r><leaf>x</leaf><nil/></r>");
    let r = root(&document);
    assert_eq!(
        r.child(None, "leaf")
            .and_then(|leaf| leaf.attribute("k").cloned()),
        None
    );
    assert_eq!(
        r.child(None, "nil")
            .and_then(|nil| nil.attribute("k").cloned()),
        None
    );
}

#[test]
fn attribute_in_resolves_a_prefix_through_the_scope_and_an_unprefixed_attribute_is_in_no_namespace()
{
    let document = parsed(
        "<r xmlns:p=\"urn:p\" xmlns=\"urn:d\">\
           <a p:m=\"3\" k=\"v\" xml:lang=\"fr\"/>\
         </r>",
    );
    let r = root(&document);
    let a = r.child(Some("urn:d"), "a").expect("a");
    assert_eq!(
        a.attribute_in(Some("urn:p"), "m"),
        Some("3"),
        "the prefix was declared on an ancestor"
    );
    assert_eq!(a.attribute_in(None, "m"), None, "`p:m` is in urn:p");
    assert_eq!(a.attribute_in(None, "k"), Some("v"));
    assert_eq!(
        a.attribute_in(Some("urn:d"), "k"),
        None,
        "the default namespace never applies to an attribute"
    );
    assert_eq!(a.attribute_in(Some(XML_NAMESPACE), "lang"), Some("fr"));
    assert_eq!(
        a.attribute_in(None, "lang"),
        None,
        "`xml:lang` is in the reserved namespace, declared or not"
    );
    assert_eq!(a.attribute_in(Some("urn:p"), "absent"), None);
}

#[test]
fn attribute_in_reads_an_attributes_prefix_under_the_innermost_declaration() {
    let document = parsed("<r xmlns:p=\"urn:outer\"><a xmlns:p=\"urn:inner\" p:k=\"v\"/></r>");
    let r = root(&document);
    let a = r.child(None, "a").expect("a");
    assert_eq!(
        a.attribute_in(Some("urn:inner"), "k"),
        Some("v"),
        "the element's own declaration binds its own attributes"
    );
    assert_eq!(a.attribute_in(Some("urn:outer"), "k"), None);
}

#[test]
fn attribute_in_passes_over_a_same_named_attribute_in_another_namespace() {
    let document = parsed(
        "<a xmlns:p=\"urn:p\" xmlns:q=\"urn:q\" \
            p:k=\"1\" q:k=\"2\" k=\"3\" xml:k=\"4\" e=\"\"/>",
    );
    let a = root(&document);
    assert_eq!(
        [
            a.attribute_in(Some("urn:p"), "k"),
            a.attribute_in(Some("urn:q"), "k"),
            a.attribute_in(None, "k"),
            a.attribute_in(Some(XML_NAMESPACE), "k"),
            a.attribute_in(Some("urn:r"), "k"),
            a.attribute_in(None, "e"),
        ],
        [Some("1"), Some("2"), Some("3"), Some("4"), None, Some("")],
        "each `k` is read by its namespace, whichever comes first by name"
    );
}

#[test]
fn attribute_in_reads_an_undeclared_prefix_as_no_namespace() {
    let document = parsed("<a q:k=\"v\"/>");
    let a = root(&document);
    assert_eq!(a.attribute_in(None, "k"), Some("v"));
    assert_eq!(a.attribute_in(Some("urn:q"), "k"), None);
}

/// A namespace declaration - `xmlns`, `xmlns:p` - is a binding rather than an
/// attribute, so no `attribute_in` lookup reaches one: not as an unqualified
/// attribute, and not in the namespace the recommendation binds `xmlns` to.
/// `attribute` and `attributes` still read it as the `@xmlns` entry it is
/// written as.
#[test]
fn a_namespace_declaration_is_no_attribute_in_any_namespace() {
    let document = parsed("<a xmlns=\"urn:d\" xmlns:p=\"urn:p\" p=\"own\"/>");
    let a = root(&document);
    assert_eq!(
        [
            a.attribute_in(None, "xmlns"),
            a.attribute_in(Some(XMLNS_NAMESPACE), "p"),
            a.attribute_in(Some(XMLNS_NAMESPACE), "xmlns"),
            a.attribute_in(Some("urn:d"), "p"),
        ],
        [None, None, None, None]
    );
    assert_eq!(
        a.attribute_in(None, "p"),
        Some("own"),
        "the unqualified `p` is the attribute, never the `xmlns:p` declaration"
    );
    assert_eq!(a.attribute("xmlns:p"), Some(&Scalar::from("urn:p")));

    let document = parsed("<a xmlns:p=\"urn:p\"/>");
    let a = root(&document);
    assert_eq!(
        a.attribute_in(None, "p"),
        None,
        "`xmlns:p` declares a prefix; it is no unqualified attribute `p`"
    );
}

#[test]
fn children_come_in_name_order_and_a_repeated_name_in_document_order() {
    let document = parsed("<r><b>2</b><a>1</a><b>3</b><c/><b>4</b></r>");
    let r = root(&document);
    assert_eq!(names(&r), ["a", "b", "b", "b", "c"]);
    assert_eq!(
        texts(&r),
        [
            Some("1".to_owned()),
            Some("2".to_owned()),
            Some("3".to_owned()),
            Some("4".to_owned()),
            None
        ]
    );
}

#[test]
fn children_of_different_names_come_in_byte_order_of_the_name_as_spelled() {
    let document = parsed("<r xmlns:p=\"urn:p\"><b/><p:a/><é/><B/><a/></r>");
    let r = root(&document);
    assert_eq!(
        names(&r),
        ["B", "a", "b", "p:a", "é"],
        "upper case before lower, and the prefix is part of the name ordered"
    );
}

#[test]
fn a_repeated_element_is_one_child_per_item_nil_and_empty_included() {
    let document = parsed("<r><leg>1</leg><leg/><leg></leg><leg k=\"v\">4</leg></r>");
    let r = root(&document);
    let legs = r.children_in(None, "leg");
    assert_eq!(legs.len(), 4);
    let read: Vec<_> = legs
        .iter()
        .map(|leg| (leg.text(), leg.is_nil(), leg.attribute_in(None, "k")))
        .collect();
    assert_eq!(
        read,
        [
            (Some("1"), false, None),
            (None, true, None),
            (Some(""), false, None),
            (Some("4"), false, Some("v")),
        ]
    );
    assert!(legs.iter().all(|leg| leg.name() == "leg"));
}

#[test]
fn a_repeated_element_inherits_the_scope_item_by_item() {
    let document =
        parsed("<r xmlns:p=\"urn:p\"><p:leg>1</p:leg><p:leg xmlns:p=\"urn:other\">2</p:leg></r>");
    let r = root(&document);
    let namespaces: Vec<_> = r
        .children()
        .map(|leg| leg.namespace().map(str::to_owned))
        .collect();
    assert_eq!(
        namespaces,
        [Some("urn:p".to_owned()), Some("urn:other".to_owned())]
    );
}

#[test]
fn attributes_and_own_text_are_not_children() {
    let value = record([
        ("@id", Scalar::from("7")),
        ("#text", Scalar::from("note")),
        ("#comment", Scalar::from("skipped")),
        ("leg", Scalar::from("1")),
    ]);
    let order = Element::new("order", &value, &Scope::new());
    assert_eq!(names(&order), ["leg"]);
    assert_eq!(order.text(), Some("note"));
}

#[test]
fn a_leaf_or_nil_element_has_no_children() {
    let document = parsed("<r><leaf>x</leaf><nil/></r>");
    let r = root(&document);
    for child in r.children() {
        assert_eq!(child.children().count(), 0, "{}", child.name());
        assert!(child.child(None, "x").is_none());
    }
}

#[test]
fn a_repeated_element_laid_out_as_a_column_yields_one_child_per_row() {
    let column = Serie::from_scalars(
        DataType::utf8().nullable_field("leg"),
        [Scalar::from("1"), Scalar::Null, Scalar::from("")],
    )
    .expect("a column");
    assert!(
        Scalar::from(column.clone()).as_sequence().is_none(),
        "a column lends no row"
    );
    let value = record([
        ("@xmlns", Scalar::from("urn:d")),
        ("leg", Scalar::from(column)),
    ]);
    let order = Element::new("order", &value, &Scope::new());
    let legs = order.children_in(Some("urn:d"), "leg");
    let read: Vec<_> = legs.iter().map(|leg| (leg.text(), leg.is_nil())).collect();
    assert_eq!(read, [(Some("1"), false), (None, true), (Some(""), false)]);
}

#[test]
fn every_serie_layout_under_a_key_repeats_the_element_once_per_item() {
    let items = || {
        Serie::Run(Run::new(vec![
            Scalar::from("1"),
            Scalar::Null,
            Scalar::from(""),
        ]))
    };
    for (layout, held) in [
        ("serie", Scalar::Serie(items())),
        ("serie_view", Scalar::SerieView(items())),
        ("fixed_size_serie", Scalar::FixedSizeSerie(items())),
        ("large_serie", Scalar::LargeSerie(items())),
        ("large_serie_view", Scalar::LargeSerieView(items())),
    ] {
        let value = record([("@xmlns:p", Scalar::from("urn:p")), ("p:leg", held)]);
        let order = Element::new("order", &value, &Scope::new());
        let legs = order.children_in(Some("urn:p"), "leg");
        let read: Vec<_> = legs
            .iter()
            .map(|leg| (leg.name(), leg.text(), leg.is_nil()))
            .collect();
        assert_eq!(
            read,
            [
                ("p:leg", Some("1"), false),
                ("p:leg", None, true),
                ("p:leg", Some(""), false),
            ],
            "{layout}"
        );
    }
}

#[test]
fn an_empty_sequence_or_column_under_a_key_is_no_child_and_one_item_is_one() {
    let empty = Serie::from_scalars(DataType::utf8().nullable_field("b"), Vec::<Scalar>::new())
        .expect("an empty column");
    let one = Serie::from_scalars(DataType::utf8().required_field("d"), [Scalar::from("x")])
        .expect("a one-row required column");
    let value = record([
        ("a", Scalar::from_sequence(Vec::<Scalar>::new())),
        ("b", Scalar::from(empty)),
        ("c", Scalar::from_sequence([Scalar::from("1")])),
        ("d", Scalar::from(one)),
    ]);
    let order = Element::new("order", &value, &Scope::new());
    assert_eq!(names(&order), ["c", "d"], "zero rows are zero elements");
    assert_eq!(texts(&order), [Some("1".to_owned()), Some("x".to_owned())]);
    assert!(order.child(None, "a").is_none());
    assert!(order.children_in(None, "b").is_empty());
    let message = refused_one(&order, "urn:x", "a");
    assert!(message.ends_with("found none"), "{message}");
    assert!(
        order.one_child_in("urn:x", "d").is_ok(),
        "one row is one element"
    );
}

#[test]
fn children_in_and_child_select_by_namespace_and_local_name() {
    let document = parsed(
        "<r xmlns:x=\"urn:x\">\
           <x:k>1</x:k><k>2</k><x:k>3</x:k><x:j>4</x:j>\
         </r>",
    );
    let r = root(&document);
    let in_x: Vec<_> = r
        .children_in(Some("urn:x"), "k")
        .iter()
        .map(|child| child.text().map(str::to_owned))
        .collect();
    assert_eq!(in_x, [Some("1".to_owned()), Some("3".to_owned())]);
    let bare: Vec<_> = r
        .children_in(None, "k")
        .iter()
        .map(|child| child.text().map(str::to_owned))
        .collect();
    assert_eq!(bare, [Some("2".to_owned())]);
    assert!(r.children_in(Some("urn:y"), "k").is_empty());

    assert_eq!(
        r.child(Some("urn:x"), "k")
            .and_then(|k| k.text().map(str::to_owned)),
        Some("1".to_owned()),
        "the first in document order"
    );
    assert_eq!(
        r.child(None, "k").and_then(|k| k.text().map(str::to_owned)),
        Some("2".to_owned())
    );
    assert!(r.child(None, "j").is_none(), "`x:j` is in urn:x");
    assert!(r.child(Some("urn:x"), "absent").is_none());
}

/// `children_in` promises document order. Two spellings of one element in
/// one namespace are two entries of the natural value, which keeps them in
/// name order, so the order the document wrote them in is gone.
#[test]
fn children_in_answers_spellings_in_name_order_and_one_spelling_in_document_order() {
    // Two spellings of one name in one namespace are two entries of the
    // natural record, so they come in name order; a parsed document keeps no
    // other order across names.
    let document = parsed(
        "<r xmlns=\"urn:x\" xmlns:p=\"urn:x\"><p:k>1</p:k><k>2</k><p:k>3</p:k></r>",
    );
    let r = root(&document);
    let found: Vec<_> = r
        .children_in(Some("urn:x"), "k")
        .iter()
        .map(|k| (k.name(), k.text().map(str::to_owned)))
        .collect();
    assert_eq!(
        found,
        [
            ("k", Some("2".to_owned())),
            ("p:k", Some("1".to_owned())),
            ("p:k", Some("3".to_owned())),
        ],
        "`k` sorts before `p:k`, and the two `p:k` keep their document order"
    );
}

/// `child` and `child_in` answer the first match in the order `children`
/// answers: name order across spellings, document order within one.
#[test]
fn child_and_child_in_answer_the_first_match_in_the_order_children_answers() {
    let document = parsed(
        "<r xmlns:t=\"http://schemas.xmlsoap.org/soap/envelope/\" \
            xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\">\
           <t:Body>1</t:Body><s:Body>2</s:Body><Body>3</Body><s:Body>4</s:Body>\
         </r>",
    );
    let r = root(&document);
    assert_eq!(names(&r), ["Body", "s:Body", "s:Body", "t:Body"]);
    assert_eq!(
        r.child(Some(SOAP), "Body").map(|body| body.name()),
        Some("s:Body")
    );
    assert_eq!(
        r.child(Some(SOAP), "Body")
            .and_then(|body| body.text().map(str::to_owned)),
        Some("2".to_owned()),
        "the first `s:Body` in the document"
    );
    assert_eq!(
        r.child_in(SOAP, "Body").map(|body| body.name()),
        Some("Body"),
        "the unqualified body sorts first"
    );
}

#[test]
fn child_in_takes_the_namespace_given_or_an_unqualified_element() {
    let document = parsed(
        "<r xmlns:m=\"urn:schemas-microsoft-com:xml-analysis\" xmlns:o=\"urn:other\">\
           <m:RequestType>A</m:RequestType><Restrictions>B</Restrictions><o:Properties>C</o:Properties>\
         </r>",
    );
    let r = root(&document);
    assert_eq!(
        r.child_in(XMLA, "RequestType")
            .and_then(|c| c.text().map(str::to_owned)),
        Some("A".to_owned())
    );
    assert_eq!(
        r.child_in(XMLA, "Restrictions")
            .and_then(|c| c.text().map(str::to_owned)),
        Some("B".to_owned()),
        "an unqualified element is the intake reading"
    );
    assert!(
        r.child_in(XMLA, "Properties").is_none(),
        "an element in another namespace is a different element"
    );
}

#[test]
fn one_child_in_answers_the_one_matching_child() {
    let document = parsed(
        "<SOAP-ENV:Envelope xmlns:SOAP-ENV=\"http://schemas.xmlsoap.org/soap/envelope/\">\
           <SOAP-ENV:Header/><SOAP-ENV:Body><x/></SOAP-ENV:Body>\
         </SOAP-ENV:Envelope>",
    );
    let envelope = root(&document);
    let body = envelope.one_child_in(SOAP, "Body").expect("one body");
    assert_eq!(body.name(), "SOAP-ENV:Body");
    assert_eq!(names(&body), ["x"]);

    let document = parsed("<Envelope><Body/></Envelope>");
    let envelope = root(&document);
    let body = envelope
        .one_child_in(SOAP, "Body")
        .expect("the unqualified body");
    assert!(body.is_nil());

    let document = parsed(
        "<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
            xmlns:o=\"urn:other\">\
           <o:Body>other</o:Body><s:Body>soap</s:Body>\
         </s:Envelope>",
    );
    let envelope = root(&document);
    let body = envelope
        .one_child_in(SOAP, "Body")
        .expect("a body in another namespace is not counted");
    assert_eq!(body.name(), "s:Body");
    assert_eq!(body.text(), Some("soap"));
}

#[test]
fn one_child_in_refuses_none_naming_the_element_and_the_local_name() {
    let document = parsed(
        "<SOAP-ENV:Envelope xmlns:SOAP-ENV=\"http://schemas.xmlsoap.org/soap/envelope/\" \
            xmlns:o=\"urn:other\">\
           <SOAP-ENV:Header/><o:Body/>\
         </SOAP-ENV:Envelope>",
    );
    let envelope = root(&document);
    let message = refused_one(&envelope, SOAP, "Body");
    assert_eq!(
        message,
        "invalid xml data at byte 0: expected one `Body` element under `SOAP-ENV:Envelope`, \
         found none",
        "a body in another namespace is none"
    );

    let document = parsed("<leaf>text</leaf>");
    let message = refused_one(&root(&document), SOAP, "Body");
    assert!(
        message.ends_with("expected one `Body` element under `leaf`, found none"),
        "{message}"
    );

    let document = parsed("<s:Envelope xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"/>");
    let message = refused_one(&root(&document), SOAP, "Body");
    assert!(
        message.ends_with("expected one `Body` element under `s:Envelope`, found none"),
        "the element is named as spelled, even where its prefix is undeclared: {message}"
    );
    let document = parsed("<Envelope/>");
    let message = refused_one(&root(&document), SOAP, "Body");
    assert!(
        message.ends_with("expected one `Body` element under `Envelope`, found none"),
        "a nil element holds no child: {message}"
    );
}

#[test]
fn one_child_in_refuses_several_naming_the_element() {
    let document = parsed(
        "<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\">\
           <s:Body/><s:Body/>\
         </s:Envelope>",
    );
    let message = refused_one(&root(&document), SOAP, "Body");
    assert_eq!(
        message,
        "invalid xml data at byte 0: expected one `Body` element under `s:Envelope`, found several"
    );

    let document = parsed(
        "<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\">\
           <s:Body/><Body/>\
         </s:Envelope>",
    );
    let message = refused_one(&root(&document), SOAP, "Body");
    assert!(
        message.ends_with("expected one `Body` element under `s:Envelope`, found several"),
        "a qualified and an unqualified body are two: {message}"
    );
}

#[test]
fn a_document_the_writer_wrote_reads_back_through_the_view() {
    let value = record([(
        "s:Envelope",
        record([
            ("@xmlns:s", Scalar::from(SOAP)),
            (
                "s:Body",
                record([(
                    "Discover",
                    record([
                        ("@xmlns", Scalar::from(XMLA)),
                        ("RequestType", Scalar::from("DISCOVER_DATASOURCES")),
                        (
                            "Restrictions",
                            Scalar::from_sequence([Scalar::from("a & b"), Scalar::from("é")]),
                        ),
                    ]),
                )]),
            ),
        ]),
    )]);
    let written = into_xml_scalar(&value).expect("the writer");
    let read = parsed(&written);
    assert_eq!(read, value);

    let envelope = root(&read);
    let discover = envelope
        .one_child_in(SOAP, "Body")
        .expect("the body")
        .children()
        .next()
        .map(|discover| {
            (
                discover.namespace().map(str::to_owned),
                discover
                    .one_child_in(XMLA, "RequestType")
                    .ok()
                    .and_then(|kind| kind.text().map(str::to_owned)),
                discover
                    .children_in(Some(XMLA), "Restrictions")
                    .iter()
                    .map(|item| item.text().map(str::to_owned))
                    .collect::<Vec<_>>(),
            )
        });
    assert_eq!(
        discover,
        Some((
            Some(XMLA.to_owned()),
            Some("DISCOVER_DATASOURCES".to_owned()),
            vec![Some("a & b".to_owned()), Some("é".to_owned())],
        ))
    );
}
