//! `rust/src/http/headers.rs`: the field section over `Metadata` - the
//! refusals the metadata layer already makes, the case fold, the join of a
//! repeated name and its split, and every typed reader. The four helper
//! files beside it are pinned under `headers/`.

#[cfg(feature = "http")]
#[path = "headers/date.rs"]
mod date;
#[cfg(feature = "http")]
#[path = "headers/etag.rs"]
mod etag;
#[cfg(feature = "http")]
#[path = "headers/link.rs"]
mod link;
#[cfg(feature = "http")]
#[path = "headers/range.rs"]
mod range;

use std::time::Duration;

use yggdryl::http::{ContentRange, ETag, Headers};
use yggdryl::{Charset, Codec, Error, Metadata, MimeType};

/// `Sun, 06 Nov 1994 08:49:37 GMT` in nanoseconds.
const RFC_EXAMPLE_NS: i64 = 784_111_777 * 1_000_000_000;

fn headers(pairs: &[(&str, &str)]) -> Headers {
    Headers::from_entries(pairs.iter().copied()).unwrap()
}

fn header_refusal<T: std::fmt::Debug>(result: yggdryl::Result<T>) -> String {
    match result {
        Err(Error::Parse {
            target: "http header",
            reason,
            ..
        }) => reason.to_string(),
        other => panic!("expected an http header refusal, got {other:?}"),
    }
}

// Refusals: what the metadata layer already refuses, spoken as a header.

#[test]
fn a_name_that_is_no_token_is_refused() {
    let reason = header_refusal(Headers::from_entries([("Content Type", "text/plain")]));
    assert!(reason.starts_with("Content Type: "), "{reason}");
    assert!(reason.contains("ASCII token"), "{reason}");
    header_refusal(Headers::from_entries([("X-Trace\u{1}", "1")]));
    header_refusal(Headers::from_entries([("", "1")]));
    header_refusal(Headers::from_entries([("Host:", "example")]));
    header_refusal(Headers::from_entries([("Höst", "example")]));

    let mut section = Headers::new();
    header_refusal(section.insert("a b", "1"));
    header_refusal(section.append("a b", "1"));
    assert!(section.is_empty());
}

#[test]
fn a_value_holding_a_line_break_or_a_control_is_refused() {
    let reason = header_refusal(Headers::from_entries([("X-Note", "one\r\ntwo")]));
    assert!(reason.starts_with("X-Note: "), "{reason}");
    assert!(reason.contains("CR, LF"), "{reason}");
    header_refusal(Headers::from_entries([("X-Note", "one\ntwo")]));
    header_refusal(Headers::from_entries([("X-Note", "one\u{0}two")]));
    header_refusal(Headers::from_entries([("X-Note", "one\u{7f}two")]));
    // HTAB is the one control a value may hold.
    assert_eq!(
        headers(&[("X-Note", "one\ttwo")]).get("x-note"),
        Some("one\ttwo")
    );
}

#[test]
fn a_set_cookie_member_takes_the_same_validation() {
    let reason = header_refusal(Headers::from_entries([("Set-Cookie", "a=1\r")]));
    assert!(reason.starts_with("Set-Cookie: "), "{reason}");
    let mut section = headers(&[("Set-Cookie", "a=1")]);
    header_refusal(section.append("Set-Cookie", "b=2\u{0}"));
    assert_eq!(section.set_cookies(), ["a=1"]);
}

#[test]
fn a_malformed_content_length_is_refused_on_the_way_in() {
    let reason = header_refusal(Headers::from_entries([("Content-Length", "abc")]));
    assert!(reason.starts_with("Content-Length: "), "{reason}");
    assert!(reason.contains("unsigned 64-bit"), "{reason}");
    header_refusal(Headers::from_entries([("Content-Length", "-1")]));
    header_refusal(Headers::from_entries([("Content-Length", "12a")]));
    header_refusal(Headers::from_entries([("Content-Length", "")]));
    header_refusal(Headers::from_entries([(
        "Content-Length",
        "99999999999999999999",
    )]));
    // A repeated Content-Length joins into a list no length is.
    header_refusal(Headers::from_entries([
        ("Content-Length", "1"),
        ("Content-Length", "1"),
    ]));
}

#[test]
fn a_malformed_content_range_is_refused_when_read() {
    let section = headers(&[("Content-Range", "items 0-9/10")]);
    let reason = header_refusal(section.content_range());
    assert!(reason.starts_with("Content-Range: "), "{reason}");
    header_refusal(headers(&[("Content-Range", "bytes 9-0/10")]).content_range());
}

#[test]
fn a_coding_this_crate_cannot_decode_is_refused_by_name() {
    let reason = header_refusal(headers(&[("Content-Encoding", "br")]).content_encoding());
    assert!(reason.starts_with("Content-Encoding: "), "{reason}");
    assert!(reason.contains("\"br\""), "{reason}");
    header_refusal(headers(&[("Content-Encoding", "gzip, compress")]).content_encoding());
    let reason = header_refusal(headers(&[("Content-Encoding", "nope")]).content_encoding());
    assert!(reason.contains("nope"), "{reason}");
    header_refusal(headers(&[("Content-Encoding", " , ")]).content_encoding());
}

#[test]
fn a_coding_chain_past_the_bound_is_refused() {
    assert_eq!(Headers::MAX_CODINGS, 5);
    let five = headers(&[("Content-Encoding", "gzip, gzip, gzip, gzip, gzip")]);
    assert_eq!(five.content_encoding().expect("five").len(), 5);
    let six = headers(&[("Content-Encoding", "gzip, gzip, gzip, gzip, gzip, gzip")]);
    let reason = header_refusal(six.content_encoding());
    assert!(reason.contains("more than 5 codings"), "{reason}");
}

// The fold.

#[test]
fn names_fold_to_lower_case_at_every_door() {
    let mut section = headers(&[("Content-Type", "text/plain")]);
    assert_eq!(section.get("content-type"), Some("text/plain"));
    assert_eq!(section.get("CONTENT-TYPE"), Some("text/plain"));
    assert_eq!(section.get("Content-Type"), Some("text/plain"));
    assert!(section.contains_key("cONTENT-tYPE"));
    assert!(!section.contains_key("content-length"));
    assert_eq!(section.len(), 1);

    assert_eq!(
        section.insert("CONTENT-TYPE", "text/html").unwrap(),
        Some("text/plain".to_owned())
    );
    assert_eq!(section.len(), 1);
    assert_eq!(section.insert("Accept", "*/*").unwrap(), None);
    assert_eq!(section.remove("ACCEPT"), Some("*/*".to_owned()));
    assert_eq!(section.remove("accept"), None);

    let names: Vec<&str> = section.iter().map(|(name, _)| name).collect();
    assert_eq!(names, ["content-type"]);
    assert_eq!(&section["Content-Type"], "text/html");
}

#[test]
fn the_stored_key_is_the_http_property_of_the_metadata() {
    let section = headers(&[("Content-Type", "text/plain"), ("ETag", "\"1\"")]);
    let metadata = section.as_metadata();
    assert_eq!(metadata.get("HTTP:content-type"), Some("text/plain"));
    assert_eq!(metadata.get("HTTP:etag"), Some("\"1\""));
    assert_eq!(metadata.as_http().get("content-type"), Some("text/plain"));
    let keys: Vec<&str> = metadata.iter().map(|(key, _)| key).collect();
    assert_eq!(keys, ["HTTP:content-type", "HTTP:etag"]);
    assert_eq!(section.clone().into_metadata(), *metadata);
}

#[test]
fn a_long_name_reads_the_same_as_a_short_one() {
    let long = "x-".repeat(80) + "name";
    let section = headers(&[(&long, "1"), ("Short", "2")]);
    assert_eq!(section.get(&long), Some("1"));
    assert_eq!(section.get(&long.to_ascii_uppercase()), Some("1"));
    assert_eq!(section.get("SHORT"), Some("2"));
    let mut section = section;
    assert_eq!(
        section.remove(&long.to_ascii_uppercase()),
        Some("1".to_owned())
    );
    assert_eq!(section.len(), 1);
}

#[test]
fn iteration_is_lexical_and_the_cursor_walks_it() {
    let section = headers(&[("Vary", "Accept"), ("Content-Type", "a"), ("Date", "b")]);
    let entries: Vec<(&str, &str)> = section.iter().collect();
    assert_eq!(
        entries,
        [("content-type", "a"), ("date", "b"), ("vary", "Accept")]
    );
    let back: Vec<&str> = section.iter().rev().map(|(name, _)| name).collect();
    assert_eq!(back, ["vary", "date", "content-type"]);

    assert_eq!(section.next_entry(None), Some(("content-type", "a")));
    assert_eq!(
        section.next_entry(Some("content-type")),
        Some(("date", "b"))
    );
    assert_eq!(section.next_entry(Some("DATE")), Some(("vary", "Accept")));
    assert_eq!(section.next_entry(Some("vary")), None);

    let borrowed: Vec<(&str, &str)> = (&section).into_iter().collect();
    assert_eq!(borrowed, entries);
    let owned = section.into_iter();
    assert_eq!(owned.len(), 3);
    let owned: Vec<(String, String)> = owned.collect();
    assert_eq!(owned[0], ("content-type".to_owned(), "a".to_owned()));
    assert_eq!(owned[2], ("vary".to_owned(), "Accept".to_owned()));
}

// The join and the split.

#[test]
fn a_repeated_name_joins_with_a_comma_and_splits_back() {
    let section = headers(&[
        ("Accept", "text/html"),
        ("Accept", "application/json"),
        ("accept", "*/*;q=0.1"),
    ]);
    assert_eq!(section.len(), 1);
    assert_eq!(
        section.get("accept"),
        Some("text/html, application/json, */*;q=0.1")
    );
    assert_eq!(
        section.get_all("Accept").collect::<Vec<_>>(),
        ["text/html", "application/json", "*/*;q=0.1"]
    );
    assert_eq!(section.get_all("missing").count(), 0);
}

#[test]
fn members_are_trimmed_empty_ones_dropped_and_quoted_commas_kept() {
    let section = headers(&[
        ("Vary", " Accept ,, Accept-Encoding,\t"),
        ("Link", "</a>; title=\"one, two\", </b>; anchor=\"x\\\",y\""),
    ]);
    assert_eq!(
        section.get_all("vary").collect::<Vec<_>>(),
        ["Accept", "Accept-Encoding"]
    );
    assert_eq!(
        section.get_all("link").collect::<Vec<_>>(),
        ["</a>; title=\"one, two\"", "</b>; anchor=\"x\\\",y\""]
    );
}

#[test]
fn set_cookie_joins_with_a_newline_and_splits_per_cookie() {
    let mut section = headers(&[
        (
            "Set-Cookie",
            "session=1; Path=/; Expires=Wed, 21 Oct 2015 07:28:00 GMT",
        ),
        ("set-cookie", "theme=dark"),
    ]);
    assert_eq!(section.len(), 1);
    assert_eq!(
        section.get("Set-Cookie"),
        Some("session=1; Path=/; Expires=Wed, 21 Oct 2015 07:28:00 GMT\ntheme=dark")
    );
    assert_eq!(
        section.set_cookies(),
        [
            "session=1; Path=/; Expires=Wed, 21 Oct 2015 07:28:00 GMT",
            "theme=dark"
        ]
    );
    assert_eq!(section.get_all("SET-COOKIE").count(), 2);
    section.append("Set-Cookie", "lang=fr").unwrap();
    assert_eq!(section.set_cookies().len(), 3);
    // Inserting replaces every cookie.
    section.insert("Set-Cookie", "only=1").unwrap();
    assert_eq!(section.set_cookies(), ["only=1"]);
    assert!(headers(&[]).set_cookies().is_empty());
}

#[test]
fn append_sets_an_absent_name_and_joins_a_present_one() {
    let mut section = Headers::new();
    section.append("Vary", "Accept").unwrap();
    assert_eq!(section.get("vary"), Some("Accept"));
    section.append("VARY", "Accept-Encoding").unwrap();
    assert_eq!(section.get("vary"), Some("Accept, Accept-Encoding"));
}

#[test]
fn merging_keeps_this_section_where_both_state_a_name() {
    let mine = headers(&[("Accept", "text/html"), ("X-Mine", "1")]);
    let theirs = headers(&[("Accept", "*/*"), ("X-Theirs", "2")]);
    let merged = mine.merge_with(&theirs).unwrap();
    assert_eq!(merged.get("accept"), Some("text/html"));
    assert_eq!(merged.get("x-mine"), Some("1"));
    assert_eq!(merged.get("x-theirs"), Some("2"));
    assert_eq!(merged.len(), 3);
    // The other way round, the other section wins.
    assert_eq!(theirs.merge_with(&mine).unwrap().get("accept"), Some("*/*"));
    // Cookies merge whole, newline join included.
    let cookies = headers(&[("Set-Cookie", "a=1"), ("Set-Cookie", "b=2")]);
    assert_eq!(
        Headers::new().merge_with(&cookies).unwrap().set_cookies(),
        ["a=1", "b=2"]
    );
}

#[test]
fn clearing_empties_and_the_empty_section_is_the_default() {
    let mut section = headers(&[("Accept", "text/html")]);
    section.clear();
    assert!(section.is_empty());
    assert_eq!(section, Headers::default());
    assert_eq!(section, Headers::new());
    assert_eq!(section.as_metadata(), &Metadata::new());
}

// The bridges.

#[test]
fn a_metadata_snapshot_keeps_its_http_properties_alone() {
    let metadata = Metadata::from_entries([
        ("HTTPS:Content-Type", "text/plain"),
        ("comment", "not a header"),
        ("FIX:tag", "35"),
        ("HTTP:etag", "\"1\""),
    ])
    .unwrap();
    let section = Headers::from_metadata(metadata);
    assert_eq!(section.len(), 2);
    assert_eq!(section.get("content-type"), Some("text/plain"));
    assert_eq!(section.get("etag"), Some("\"1\""));
    assert!(!section.as_metadata().contains_key("comment"));

    // A snapshot that is nothing but headers is kept whole.
    let only = Metadata::from_entries([("HTTP:accept", "*/*")]).unwrap();
    assert_eq!(Headers::from_metadata(only.clone()).into_metadata(), only);
}

#[test]
fn the_json_form_spells_bare_lower_case_names_and_reads_back() {
    let section = headers(&[("Content-Type", "text/plain"), ("Accept", "a, b")]);
    let json = section.to_string();
    assert_eq!(
        json,
        "{\"accept\":\"a, b\",\"content-type\":\"text/plain\"}"
    );
    assert_eq!(section.clone().into_json().unwrap(), json);
    assert_eq!(Headers::from_json(&json).unwrap(), section);
    assert_eq!(json.parse::<Headers>().unwrap(), section);
    // Names are folded on the way in, a value with a quote is escaped.
    let quoted = Headers::from_json("{\"ETag\": \"\\\"1\\\"\"}").unwrap();
    assert_eq!(quoted.get("etag"), Some("\"1\""));
    assert_eq!(quoted.to_string(), "{\"etag\":\"\\\"1\\\"\"}");
    // Cookies survive the round trip with their newline.
    let cookies = headers(&[("Set-Cookie", "a=1"), ("Set-Cookie", "b=2")]);
    assert_eq!(Headers::from_json(&cookies.to_string()).unwrap(), cookies);
    // What is no object, or no header, is refused.
    assert!(matches!(Headers::from_json("[]"), Err(Error::Json(_))));
    assert!(matches!(
        Headers::from_json("{\"a b\": \"1\"}"),
        Err(Error::Json(_))
    ));
    assert_eq!(Headers::from_json("{}").unwrap(), Headers::new());
}

#[test]
fn debug_shows_the_bare_names() {
    let section = headers(&[("Content-Type", "text/plain")]);
    assert_eq!(format!("{section:?}"), "{\"content-type\": \"text/plain\"}");
}

#[test]
fn debug_never_prints_a_credential_and_display_is_the_data() {
    let section = headers(&[
        ("Authorization", "Bearer t-1"),
        ("Cookie", "sid=secret"),
        ("Proxy-Authorization", "Basic cHJveHk="),
        ("Set-Cookie", "sid=secret; Path=/"),
        ("Accept", "text/plain"),
    ]);
    let debug = format!("{section:?}");
    assert_eq!(
        debug,
        "{\"accept\": \"text/plain\", \"authorization\": <redacted>, \"cookie\": <redacted>, \
         \"proxy-authorization\": <redacted>, \"set-cookie\": <redacted>}"
    );
    // Display and serde are the data forms: every value as it is.
    assert!(section.to_string().contains("Bearer t-1"));
}

#[test]
fn equality_order_and_hash_read_the_entries_alone() {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let one = headers(&[("A", "1"), ("B", "2")]);
    let other = headers(&[("b", "2"), ("a", "1")]);
    assert_eq!(one, other);
    assert_eq!(one.stable_hash(), other.stable_hash());
    assert_eq!(one.cmp(&other), std::cmp::Ordering::Equal);
    let mut left = DefaultHasher::new();
    one.hash(&mut left);
    let mut right = DefaultHasher::new();
    other.hash(&mut right);
    assert_eq!(left.finish(), right.finish());

    let different = headers(&[("A", "1"), ("B", "3")]);
    assert_ne!(one, different);
    assert_ne!(one.stable_hash(), different.stable_hash());
    assert!(one < different);
    // The stable hash is the display's, so it is pinned by the display.
    assert_eq!(Headers::new().stable_hash(), headers(&[]).stable_hash());
}

#[test]
#[should_panic(expected = "header \"x-missing\" is not present")]
fn indexing_an_absent_header_panics() {
    let section = headers(&[("A", "1")]);
    let _ = &section["x-missing"];
}

// The typed readers.

#[test]
fn content_length_and_content_type_read_what_the_headers_state() {
    let section = headers(&[
        ("Content-Length", "0042"),
        ("Content-Type", "text/csv; charset=windows-1252"),
    ]);
    // The length is canonicalized on the way in.
    assert_eq!(section.get("content-length"), Some("42"));
    assert_eq!(section.content_length().unwrap(), Some(42));
    assert_eq!(
        section.content_type(),
        Some("text/csv; charset=windows-1252")
    );
    assert_eq!(section.mime_type().unwrap(), MimeType::CSV);
    assert_eq!(section.charset().unwrap(), Some(Charset::Cp1252));

    let empty = Headers::new();
    assert_eq!(empty.content_length().unwrap(), None);
    assert_eq!(empty.content_type(), None);
    assert_eq!(empty.mime_type().unwrap(), MimeType::default());
    assert_eq!(empty.charset().unwrap(), None);
    assert!(
        headers(&[("Content-Type", "nonsense")])
            .mime_type()
            .is_err()
    );
    assert!(
        headers(&[("Content-Type", "text/plain; charset=klingon")])
            .charset()
            .is_err()
    );
}

#[test]
fn the_media_type_composes_the_type_its_charset_and_the_coding() {
    let section = headers(&[
        ("Content-Type", "application/json; charset=utf-8"),
        ("Content-Encoding", "gzip"),
    ]);
    let media_type = section.media_type().unwrap();
    assert_eq!(*media_type.base(), MimeType::JSON);
    assert_eq!(media_type.charset(), Some(Charset::Utf8));
    assert_eq!(media_type.encodings(), [MimeType::GZIP]);
    assert_eq!(Codec::from_media_type(&media_type), Codec::Gzip);

    let plain = headers(&[("Content-Type", "text/plain")])
        .media_type()
        .unwrap();
    assert_eq!(*plain.base(), MimeType::PLAIN_TEXT);
    assert!(plain.encodings().is_empty());
    assert_eq!(plain.charset(), None);
    assert_eq!(
        *Headers::new().media_type().unwrap().base(),
        MimeType::default()
    );
}

#[test]
fn content_encoding_reads_the_codings_in_application_order() {
    assert_eq!(
        Headers::new().content_encoding().unwrap(),
        [Codec::Identity]
    );
    assert_eq!(
        headers(&[("Content-Encoding", "gzip")])
            .content_encoding()
            .unwrap(),
        [Codec::Gzip]
    );
    // HTTP's `deflate` is the zlib framing, and the coding folds case.
    assert_eq!(
        headers(&[("Content-Encoding", "DEFLATE, zstd")])
            .content_encoding()
            .unwrap(),
        [Codec::Zlib, Codec::Zstd]
    );
    assert_eq!(
        headers(&[("Content-Encoding", "identity")])
            .content_encoding()
            .unwrap(),
        [Codec::Identity]
    );
    // Two fields join into one list.
    assert_eq!(
        headers(&[("Content-Encoding", "gzip"), ("Content-Encoding", "zstd")])
            .content_encoding()
            .unwrap(),
        [Codec::Gzip, Codec::Zstd]
    );
}

#[test]
fn ranges_are_read_off_content_range_and_accept_ranges() {
    let section = headers(&[
        ("Content-Range", "bytes 0-1023/4096"),
        ("Accept-Ranges", "none, BYTES"),
    ]);
    assert_eq!(
        section.content_range().unwrap(),
        Some(ContentRange::Bytes {
            start: 0,
            end: 1023,
            total: Some(4096)
        })
    );
    assert!(section.accept_ranges());
    assert_eq!(
        headers(&[("Content-Range", "bytes */4096")])
            .content_range()
            .unwrap(),
        Some(ContentRange::Unsatisfied { total: 4096 })
    );
    assert_eq!(Headers::new().content_range().unwrap(), None);
    assert!(!Headers::new().accept_ranges());
    assert!(!headers(&[("Accept-Ranges", "none")]).accept_ranges());
}

#[test]
fn validators_and_instants_parse_once() {
    let section = headers(&[
        ("ETag", "W/\"v2\""),
        ("Last-Modified", "Sun, 06 Nov 1994 08:49:37 GMT"),
        ("Date", "Sun Nov  6 08:49:37 1994"),
        ("Location", "/next"),
    ]);
    assert_eq!(
        section.etag().unwrap(),
        Some(ETag {
            opaque: "v2".to_owned(),
            weak: true
        })
    );
    assert_eq!(section.last_modified().unwrap(), Some(RFC_EXAMPLE_NS));
    assert_eq!(section.date().unwrap(), Some(RFC_EXAMPLE_NS));
    assert_eq!(section.location(), Some("/next"));

    let empty = Headers::new();
    assert_eq!(empty.etag().unwrap(), None);
    assert_eq!(empty.last_modified().unwrap(), None);
    assert_eq!(empty.date().unwrap(), None);
    assert_eq!(empty.location(), None);
    header_refusal(headers(&[("ETag", "v2")]).etag());
    assert!(matches!(
        headers(&[("Last-Modified", "yesterday")]).last_modified(),
        Err(Error::Parse {
            target: "http date",
            ..
        })
    ));
    assert!(headers(&[("Date", "1994-11-06")]).date().is_err());
}

#[test]
fn retry_after_is_delta_seconds_or_the_time_until_a_date() {
    let now = RFC_EXAMPLE_NS;
    assert_eq!(Headers::new().retry_after(now).unwrap(), None);
    assert_eq!(
        headers(&[("Retry-After", "120")]).retry_after(now).unwrap(),
        Some(Duration::from_secs(120))
    );
    assert_eq!(
        headers(&[("Retry-After", " 0 ")]).retry_after(now).unwrap(),
        Some(Duration::ZERO)
    );
    // Ninety seconds past the example instant.
    assert_eq!(
        headers(&[("Retry-After", "Sun, 06 Nov 1994 08:51:07 GMT")])
            .retry_after(now)
            .unwrap(),
        Some(Duration::from_secs(90))
    );
    // A date already passed is no pause at all.
    assert_eq!(
        headers(&[("Retry-After", "Sun, 06 Nov 1994 08:00:00 GMT")])
            .retry_after(now)
            .unwrap(),
        Some(Duration::ZERO)
    );
    let reason = header_refusal(headers(&[("Retry-After", "soon")]).retry_after(now));
    assert!(reason.starts_with("Retry-After: "), "{reason}");
    header_refusal(headers(&[("Retry-After", "-5")]).retry_after(now));
    header_refusal(headers(&[("Retry-After", "99999999999999999999")]).retry_after(now));
}

#[test]
fn links_and_the_next_link_read_the_link_header() {
    let section = headers(&[
        ("Link", "</items?page=2>; rel=\"next\""),
        ("Link", "</items?page=9>; rel=\"last\""),
    ]);
    let links = section.links().unwrap();
    assert_eq!(links.len(), 2);
    assert_eq!(links[0].target, "/items?page=2");
    assert!(links[1].has_rel("last"));
    assert_eq!(section.next_link().unwrap(), Some("/items?page=2"));

    assert!(Headers::new().links().unwrap().is_empty());
    assert_eq!(Headers::new().next_link().unwrap(), None);
    assert_eq!(
        headers(&[("Link", "</items?page=9>; rel=\"last\"")])
            .next_link()
            .unwrap(),
        None
    );
    // A rel among several, and the next of several links.
    assert_eq!(
        headers(&[("Link", "</p>; rel=prev, </n>; rel=\"alternate next\"")])
            .next_link()
            .unwrap(),
        Some("/n")
    );
    header_refusal(headers(&[("Link", "rel=next")]).links());
    header_refusal(headers(&[("Link", "rel=next")]).next_link());
}

#[test]
fn transfer_encoding_and_connection_read_their_tokens() {
    assert!(headers(&[("Transfer-Encoding", "gzip, Chunked")]).transfer_encoding_chunked());
    assert!(!headers(&[("Transfer-Encoding", "gzip")]).transfer_encoding_chunked());
    assert!(!Headers::new().transfer_encoding_chunked());
    assert!(headers(&[("Connection", "keep-alive, CLOSE")]).connection_close());
    assert!(!headers(&[("Connection", "keep-alive")]).connection_close());
    assert!(!Headers::new().connection_close());
}

#[test]
fn a_rate_limit_pauses_only_when_nothing_remains_and_a_reset_is_stated() {
    let now = RFC_EXAMPLE_NS;
    assert_eq!(Headers::new().rate_limit_pause(now).unwrap(), None);
    assert_eq!(
        headers(&[("RateLimit-Remaining", "5"), ("RateLimit-Reset", "30")])
            .rate_limit_pause(now)
            .unwrap(),
        None
    );
    assert_eq!(
        headers(&[("RateLimit-Remaining", "0")])
            .rate_limit_pause(now)
            .unwrap(),
        None
    );
    // A small reset is delta seconds.
    assert_eq!(
        headers(&[("RateLimit-Remaining", "0"), ("RateLimit-Reset", "30")])
            .rate_limit_pause(now)
            .unwrap(),
        Some(Duration::from_secs(30))
    );
    // A large one is epoch seconds: sixty past the example instant.
    assert_eq!(
        headers(&[
            ("X-RateLimit-Remaining", "0"),
            ("X-RateLimit-Reset", "784111837")
        ])
        .rate_limit_pause(now)
        .unwrap(),
        Some(Duration::from_secs(60))
    );
    // An epoch reset already passed, and a fraction dropped.
    assert_eq!(
        headers(&[
            ("X-RateLimit-Remaining", "0"),
            ("X-RateLimit-Reset", "784111000.5")
        ])
        .rate_limit_pause(now)
        .unwrap(),
        Some(Duration::ZERO)
    );
    // The standard pair is read before the prefixed one.
    assert_eq!(
        headers(&[
            ("RateLimit-Remaining", "3"),
            ("X-RateLimit-Remaining", "0"),
            ("X-RateLimit-Reset", "10"),
        ])
        .rate_limit_pause(now)
        .unwrap(),
        None
    );
    let reason = header_refusal(
        headers(&[("RateLimit-Remaining", "none"), ("RateLimit-Reset", "10")])
            .rate_limit_pause(now),
    );
    assert!(reason.starts_with("ratelimit-remaining: "), "{reason}");
    header_refusal(
        headers(&[("RateLimit-Remaining", "0"), ("RateLimit-Reset", "-1")]).rate_limit_pause(now),
    );
}
