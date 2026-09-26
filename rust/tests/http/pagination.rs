//! `rust/src/http/pagination.rs`: how the next page is found and where the rows are.

use smol_str::SmolStr;
use yggdryl::http::{Headers, NextPage, Pagination};
use yggdryl::{Error, FieldPath, FieldSegment, Scalar, Url, from_json_scalar};

const ORDERS: &str = "https://api.example.com/v1/orders?limit=2";

fn url(text: &str) -> Url {
    Url::from_str(text).expect("a URL")
}

fn json(text: &str) -> Scalar {
    from_json_scalar(text).expect("a JSON document")
}

fn headers(pairs: &[(&str, &str)]) -> Headers {
    Headers::from_entries(pairs.iter().copied()).expect("headers")
}

fn path(text: &str) -> FieldPath {
    FieldPath::from_str(text).expect("a path")
}

fn next_url(text: &str) -> Option<NextPage> {
    Some(NextPage::Url(url(text)))
}

fn parameter(name: &str, value: &str) -> Option<NextPage> {
    Some(NextPage::Parameter {
        name: SmolStr::new(name),
        value: value.to_owned(),
    })
}

/// A page of two rows under `data` with `extra` beside them.
fn page(extra: &str) -> Scalar {
    json(&format!(r#"{{"data":[{{"id":1}},{{"id":2}}],{extra}}}"#))
}

fn auto(url_text: &str, headers: &Headers, body: Option<&Scalar>) -> Option<NextPage> {
    Pagination::Auto
        .next(&url(url_text), headers, body, 0, 2)
        .expect("a verdict")
}

// --- refusals ----------------------------------------------------------------

#[test]
fn a_spelling_that_names_no_mode_or_a_malformed_argument_is_refused() {
    for (text, position, expected) in [
        ("scroll", 0, "one of auto"),
        ("", 0, "one of auto"),
        ("auto:x", 5, "no argument"),
        ("link:x", 5, "no argument"),
        ("header", 6, "`:`"),
        ("header:", 7, "header name"),
        ("header:a b", 7, "header name"),
        ("url:", 4, "field path"),
        ("url:a b", 6, "expected"),
        ("url:items[1:3]", 4, "selects several"),
        ("cursor:next_cursor", 7, "`<path>:<parameter>`"),
        ("cursor::after", 7, "field path"),
        ("cursor:next_cursor:", 19, "query parameter"),
        ("offset:offset", 7, "`<parameter>:<size>`"),
        ("offset:offset:0", 14, "positive page size"),
        ("offset:offset:many", 14, "positive page size"),
        ("offset::10", 7, "query parameter"),
        ("page:page", 5, "`<parameter>:<start>`"),
        ("page:page:first", 10, "first page number"),
    ] {
        let error = Pagination::from_str(text).expect_err(text);
        let Error::Parse {
            target,
            position: at,
            reason,
        } = &error
        else {
            panic!("{text:?}: {error:?}");
        };
        assert_eq!(*target, "pagination", "{text:?}");
        assert_eq!(*at, position, "{text:?}: {reason}");
        assert!(reason.contains(expected), "{text:?}: {reason}");
    }
}

#[test]
fn a_hand_built_path_selecting_several_values_is_refused_when_walked() {
    let pagination = Pagination::Url(path("data[0:1].next"));
    let error = pagination
        .next(
            &url(ORDERS),
            &Headers::new(),
            Some(&page(r#""next":"x""#)),
            0,
            2,
        )
        .expect_err("a refusal");
    assert!(
        matches!(
            error,
            Error::Parse {
                target: "pagination",
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn a_malformed_link_header_is_heard_rather_than_skipped() {
    let bad = headers(&[("Link", "no angle brackets; rel=next")]);
    assert!(
        Pagination::Link
            .next(&url(ORDERS), &bad, None, 0, 2)
            .is_err()
    );
    assert!(
        Pagination::Auto
            .next(&url(ORDERS), &bad, None, 0, 2)
            .is_err()
    );
}

// --- spellings ---------------------------------------------------------------

#[test]
fn every_mode_round_trips_through_its_spelling() {
    for (text, expected) in [
        ("auto", Pagination::Auto),
        ("none", Pagination::None),
        ("link", Pagination::Link),
        (
            "header:X-Next-Page",
            Pagination::Header(SmolStr::new("X-Next-Page")),
        ),
        ("url:paging.next", Pagination::Url(path("paging.next"))),
        (
            "cursor:meta.next_cursor:after",
            Pagination::Cursor {
                path: path("meta.next_cursor"),
                parameter: SmolStr::new("after"),
            },
        ),
        (
            "offset:offset:100",
            Pagination::Offset {
                parameter: SmolStr::new("offset"),
                page_size: 100,
                total: None,
            },
        ),
        (
            "offset:skip:50:meta.total",
            Pagination::Offset {
                parameter: SmolStr::new("skip"),
                page_size: 50,
                total: Some(path("meta.total")),
            },
        ),
        (
            "page:page:1",
            Pagination::Page {
                parameter: SmolStr::new("page"),
                start: 1,
            },
        ),
    ] {
        let parsed = Pagination::from_str(text).expect(text);
        assert_eq!(parsed, expected, "{text}");
        assert_eq!(parsed.to_string(), text);
        assert_eq!(text.parse::<Pagination>().expect(text), expected);
    }
    assert_eq!(
        Pagination::from_str("AUTO").expect("auto"),
        Pagination::Auto
    );
    assert_eq!(
        Pagination::from_str(" Link ").expect("link"),
        Pagination::Link
    );
    assert_eq!(Pagination::default(), Pagination::Auto);
    assert_eq!(
        Pagination::from_str("url:\"@odata.nextLink\"").expect("a quoted name"),
        Pagination::Url(FieldPath::new([FieldSegment::field("@odata.nextLink")]))
    );
}

// --- every mode ---------------------------------------------------------------

#[test]
fn none_ends_after_the_first_page_whatever_the_page_says() {
    let link = headers(&[("Link", "</v1/orders?page=2>; rel=\"next\"")]);
    let body = page(r#""next":"/v1/orders?page=2""#);
    assert_eq!(
        Pagination::None
            .next(&url(ORDERS), &link, Some(&body), 0, 2)
            .expect("a verdict"),
        None
    );
}

#[test]
fn link_reads_the_rel_next_target_resolved_against_the_page() {
    let link = headers(&[(
        "Link",
        "</v1/orders?limit=2&page=2>; rel=\"next\", <https://api.example.com/v1/orders?page=9>; rel=\"last\"",
    )]);
    assert_eq!(
        Pagination::Link
            .next(&url(ORDERS), &link, None, 0, 2)
            .expect("a verdict"),
        next_url("https://api.example.com/v1/orders?limit=2&page=2")
    );
    let absolute = headers(&[("Link", "<https://other.example/p2>; rel=next")]);
    assert_eq!(
        Pagination::Link
            .next(&url(ORDERS), &absolute, None, 0, 2)
            .expect("a verdict"),
        next_url("https://other.example/p2")
    );
    let last_only = headers(&[("Link", "</v1/orders?page=9>; rel=\"last\"")]);
    assert_eq!(
        Pagination::Link
            .next(
                &url(ORDERS),
                &last_only,
                Some(&page(r#""next":"/x""#)),
                0,
                2
            )
            .expect("a verdict"),
        None,
        "the body is not consulted"
    );
}

#[test]
fn header_reads_the_named_header_as_a_url() {
    let pagination = Pagination::Header(SmolStr::new("X-Continuation"));
    let present = headers(&[("x-continuation", "?limit=2&token=abc")]);
    assert_eq!(
        pagination
            .next(&url(ORDERS), &present, None, 0, 2)
            .expect("a verdict"),
        next_url("https://api.example.com/v1/orders?limit=2&token=abc")
    );
    assert_eq!(
        pagination
            .next(&url(ORDERS), &Headers::new(), None, 0, 2)
            .expect("a verdict"),
        None
    );
}

#[test]
fn url_reads_the_body_at_the_declared_path() {
    let pagination = Pagination::Url(path("paging.next_href"));
    let body = page(r#""paging":{"next_href":"https://api.example.com/v1/orders?after=2"}"#);
    assert_eq!(
        pagination
            .next(&url(ORDERS), &Headers::new(), Some(&body), 0, 2)
            .expect("a verdict"),
        next_url("https://api.example.com/v1/orders?after=2")
    );
    let absent = page(r#""paging":{"next_href":null}"#);
    assert_eq!(
        pagination
            .next(&url(ORDERS), &Headers::new(), Some(&absent), 0, 2)
            .expect("a verdict"),
        None
    );
    assert_eq!(
        pagination
            .next(&url(ORDERS), &Headers::new(), None, 0, 2)
            .expect("a verdict"),
        None
    );
}

#[test]
fn cursor_reads_the_body_and_sends_it_back_under_the_parameter() {
    let by_index = Pagination::Cursor {
        path: path("data[-1].id"),
        parameter: SmolStr::new("after"),
    };
    assert_eq!(
        by_index
            .next(&url(ORDERS), &Headers::new(), Some(&page(r#""n":2"#)), 0, 2)
            .expect("a verdict"),
        parameter("after", "2")
    );
    let by_key = Pagination::from_str("cursor:meta['next']:cursor").expect("a spelling");
    let body = page(r#""meta":{"next":"c2"}"#);
    assert_eq!(
        by_key
            .next(&url(ORDERS), &Headers::new(), Some(&body), 0, 2)
            .expect("a verdict"),
        parameter("cursor", "c2")
    );
    assert_eq!(
        by_key
            .next(
                &url("https://api.example.com/v1/orders?cursor=c2"),
                &Headers::new(),
                Some(&body),
                1,
                2
            )
            .expect("a verdict"),
        None,
        "a cursor the URL already carries is the same page"
    );
    let empty = page(r#""meta":{"next":""}"#);
    assert_eq!(
        by_key
            .next(&url(ORDERS), &Headers::new(), Some(&empty), 0, 2)
            .expect("a verdict"),
        None
    );
}

#[test]
fn offset_counts_up_by_the_page_size_and_ends_at_a_short_page_or_the_total() {
    let offset = Pagination::from_str("offset:offset:2").expect("a spelling");
    let no_headers = Headers::new();
    assert_eq!(
        offset
            .next(&url(ORDERS), &no_headers, Some(&page(r#""n":0"#)), 0, 2)
            .expect("a verdict"),
        parameter("offset", "2")
    );
    assert_eq!(
        offset
            .next(&url(ORDERS), &no_headers, None, 3, 2)
            .expect("a verdict"),
        parameter("offset", "8"),
        "the page index counts where no parameter does"
    );
    let at_four = url("https://api.example.com/v1/orders?limit=2&offset=4");
    assert_eq!(
        offset
            .next(&at_four, &no_headers, None, 2, 2)
            .expect("a verdict"),
        parameter("offset", "6")
    );
    assert_eq!(
        offset
            .next(&at_four, &no_headers, None, 2, 1)
            .expect("a verdict"),
        None,
        "a short page is the last"
    );
    let with_total = Pagination::from_str("offset:offset:2:meta.total").expect("a spelling");
    assert_eq!(
        with_total
            .next(
                &at_four,
                &no_headers,
                Some(&page(r#""meta":{"total":6}"#)),
                2,
                2
            )
            .expect("a verdict"),
        None
    );
    assert_eq!(
        with_total
            .next(
                &at_four,
                &no_headers,
                Some(&page(r#""meta":{"total":7}"#)),
                2,
                2
            )
            .expect("a verdict"),
        parameter("offset", "6")
    );
}

#[test]
fn page_counts_up_from_the_start_and_ends_at_an_empty_page() {
    let paged = Pagination::from_str("page:page:1").expect("a spelling");
    let no_headers = Headers::new();
    assert_eq!(
        paged
            .next(&url(ORDERS), &no_headers, None, 0, 2)
            .expect("a verdict"),
        parameter("page", "2")
    );
    assert_eq!(
        paged
            .next(&url(ORDERS), &no_headers, None, 4, 2)
            .expect("a verdict"),
        parameter("page", "6")
    );
    assert_eq!(
        paged
            .next(
                &url("https://api.example.com/v1/orders?page=7"),
                &no_headers,
                None,
                0,
                2
            )
            .expect("a verdict"),
        parameter("page", "8"),
        "the URL's own number wins over the count"
    );
    assert_eq!(
        paged
            .next(&url(ORDERS), &no_headers, None, 4, 0)
            .expect("a verdict"),
        None
    );
}

// --- the Auto ladder, rung by rung -------------------------------------------

#[test]
fn auto_reads_link_first_before_anything_in_the_body() {
    let link = headers(&[("Link", "</v1/orders?limit=2&page=2>; rel=\"next\"")]);
    let body = page(r#""next":"https://elsewhere.example/""#);
    assert_eq!(
        auto(ORDERS, &link, Some(&body)),
        next_url("https://api.example.com/v1/orders?limit=2&page=2")
    );
}

#[test]
fn auto_reads_the_next_headers_as_a_url_or_a_token_under_the_parameter_the_url_carries() {
    assert_eq!(
        auto(
            ORDERS,
            &headers(&[("X-Next-Page", "/v1/orders?limit=2&page=3")]),
            None
        ),
        next_url("https://api.example.com/v1/orders?limit=2&page=3")
    );
    assert_eq!(
        auto(ORDERS, &headers(&[("X-Next-Page", "3")]), None),
        parameter("page", "3")
    );
    assert_eq!(
        auto(
            "https://api.example.com/v1/orders?page_number=2",
            &headers(&[("Next-Page", "3")]),
            None
        ),
        parameter("page_number", "3")
    );
    assert_eq!(
        auto(ORDERS, &headers(&[("X-Next-Cursor", "c9")]), None),
        parameter("cursor", "c9")
    );
    assert_eq!(
        auto(
            "https://api.example.com/v1/orders?after=c1",
            &headers(&[("X-Next", "c9")]),
            None
        ),
        parameter("after", "c9")
    );
    assert_eq!(
        auto(
            "https://api.example.com/v1/orders?after=c9",
            &headers(&[("X-Next", "c9")]),
            None
        ),
        None,
        "a token already carried is the same page"
    );
    assert_eq!(auto(ORDERS, &headers(&[("X-Next-Page", "  ")]), None), None);
}

#[test]
fn auto_reads_every_next_url_path_of_the_body_in_order() {
    for (extra, expected) in [
        (
            r#""next":"/v1/orders?limit=2&page=2""#,
            "https://api.example.com/v1/orders?limit=2&page=2",
        ),
        (
            r#""next_url":"https://api.example.com/p2""#,
            "https://api.example.com/p2",
        ),
        (
            r#""nextUrl":"https://api.example.com/p3""#,
            "https://api.example.com/p3",
        ),
        (
            r#""next_page_url":"https://api.example.com/p4""#,
            "https://api.example.com/p4",
        ),
        (
            r#""nextLink":"https://api.example.com/p5""#,
            "https://api.example.com/p5",
        ),
        (
            r#""@odata.nextLink":"https://api.example.com/p6""#,
            "https://api.example.com/p6",
        ),
        (
            r#""links":{"next":"https://api.example.com/p7"}"#,
            "https://api.example.com/p7",
        ),
        (
            r#""links":{"next":{"href":"https://api.example.com/p8"}}"#,
            "https://api.example.com/p8",
        ),
        (
            r#""_links":{"next":{"href":"https://api.example.com/p9"}}"#,
            "https://api.example.com/p9",
        ),
        (
            r#""paging":{"next":"https://api.example.com/p10"}"#,
            "https://api.example.com/p10",
        ),
        (
            r#""meta":{"next":"https://api.example.com/p11"}"#,
            "https://api.example.com/p11",
        ),
        (
            r#""pagination":{"next":"orders?limit=2&page=12"}"#,
            "https://api.example.com/v1/orders?limit=2&page=12",
        ),
    ] {
        assert_eq!(
            auto(ORDERS, &Headers::new(), Some(&page(extra))),
            next_url(expected),
            "{extra}"
        );
    }
    // An earlier candidate wins.
    let two = page(
        r#""next":"https://api.example.com/first","nextUrl":"https://api.example.com/second""#,
    );
    assert_eq!(
        auto(ORDERS, &Headers::new(), Some(&two)),
        next_url("https://api.example.com/first")
    );
}

#[test]
fn auto_reads_every_cursor_path_of_the_body_sent_back_under_the_cursor_parameter() {
    for (extra, value) in [
        (r#""next_cursor":"c1""#, "c1"),
        (r#""nextCursor":"c2""#, "c2"),
        (r#""next_page_token":"t3""#, "t3"),
        (r#""nextPageToken":"t4""#, "t4"),
        (r#""cursor":"c5""#, "c5"),
        (r#""after":"c6""#, "c6"),
        (r#""meta":{"cursor":"c7"}"#, "c7"),
        (r#""pagination":{"cursor":"c8"}"#, "c8"),
        (r#""next_cursor":42"#, "42"),
    ] {
        assert_eq!(
            auto(ORDERS, &Headers::new(), Some(&page(extra))),
            parameter("cursor", value),
            "{extra}"
        );
    }
    assert_eq!(
        auto(
            "https://api.example.com/v1/orders?page_token=t1",
            &Headers::new(),
            Some(&page(r#""next_page_token":"t2""#))
        ),
        parameter("page_token", "t2"),
        "the parameter the URL already carries"
    );
    let null_next = page(r#""next":null,"next_cursor":"c1""#);
    assert_eq!(
        auto(ORDERS, &Headers::new(), Some(&null_next)),
        parameter("cursor", "c1"),
        "a null URL candidate is skipped, not the end"
    );
}

#[test]
fn auto_ends_the_walk_on_the_documented_stop_conditions() {
    let link = headers(&[("Link", "</v1/orders?limit=2&page=2>; rel=\"next\"")]);
    let has_more_false = page(r#""has_more":false,"next_cursor":"c1""#);
    assert_eq!(
        auto(ORDERS, &link, Some(&has_more_false)),
        None,
        "has_more false"
    );
    let has_more_camel = page(r#""hasMore":false,"next":"/v1/orders?page=2""#);
    assert_eq!(
        auto(ORDERS, &Headers::new(), Some(&has_more_camel)),
        None,
        "hasMore false"
    );
    let has_more_true = page(r#""has_more":true,"next_cursor":"c1""#);
    assert_eq!(
        auto(ORDERS, &Headers::new(), Some(&has_more_true)),
        parameter("cursor", "c1")
    );

    let empty_page = json(r#"{"data":[],"next":"/v1/orders?page=2"}"#);
    assert_eq!(
        Pagination::Auto
            .next(&url(ORDERS), &link, Some(&empty_page), 0, 0)
            .expect("a verdict"),
        None,
        "an empty page"
    );

    let same = page(r#""next":"https://api.example.com/v1/orders?limit=2""#);
    assert_eq!(
        auto(ORDERS, &Headers::new(), Some(&same)),
        None,
        "the same URL"
    );
    let same_link = headers(&[("Link", "</v1/orders?limit=2>; rel=\"next\"")]);
    assert_eq!(
        auto(ORDERS, &same_link, None),
        None,
        "the same URL from Link"
    );

    let carried = page(r#""next_cursor":"c2""#);
    assert_eq!(
        auto(
            "https://api.example.com/v1/orders?cursor=c2",
            &Headers::new(),
            Some(&carried)
        ),
        None,
        "a cursor already carried"
    );

    assert_eq!(auto(ORDERS, &Headers::new(), None), None, "nothing to read");
    assert_eq!(
        auto(ORDERS, &Headers::new(), Some(&page(r#""total":2"#))),
        None,
        "nothing found"
    );
}

// --- the records path ---------------------------------------------------------

#[test]
fn records_path_is_the_declared_one_the_root_a_known_key_or_the_largest_sequence() {
    let declared = path("payload.rows");
    assert_eq!(
        Pagination::records_path(&json("{}"), Some(&declared)),
        Some(declared.clone())
    );
    assert_eq!(
        Pagination::records_path(&json("[{\"id\":1}]"), None),
        Some(FieldPath::root())
    );
    for key in [
        "data", "items", "results", "records", "value", "rows", "entries", "elements", "content",
    ] {
        let body = json(&format!(r#"{{"count":9,"{key}":[{{"id":1}}]}}"#));
        assert_eq!(
            Pagination::records_path(&body, None),
            Some(path(key)),
            "{key}"
        );
    }
    let hits = json(r#"{"hits":{"total":1,"hits":[{"_id":"a"}]}}"#);
    assert_eq!(
        Pagination::records_path(&hits, None),
        Some(path("hits.hits"))
    );
    let precedence = json(r#"{"results":[1,2,3],"data":[1]}"#);
    assert_eq!(
        Pagination::records_path(&precedence, None),
        Some(path("data")),
        "the table's order"
    );
    let largest = json(r#"{"meta":{"n":2},"errors":[],"orders":[{"id":1},{"id":2}],"tags":["a"]}"#);
    assert_eq!(
        Pagination::records_path(&largest, None),
        Some(path("orders"))
    );
    let none = json(r#"{"meta":{"n":2},"data":{"id":1}}"#);
    assert_eq!(Pagination::records_path(&none, None), None);
    assert_eq!(Pagination::records_path(&json("42"), None), None);
}
