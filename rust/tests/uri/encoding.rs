//! Percent-encoding edges, against the rules the specs and other
//! implementations disagree about.
//!
//! Every case here is one an outside implementation gets wrong, one a spec
//! pins exactly, or one a filename in the wild produces. The sources are named
//! per test: RFC 3986 (generic syntax and normalization), RFC 6874 (IPv6 zone
//! identifiers), RFC 8089 (the `file` scheme), RFC 8141 (URNs), and the WHATWG
//! URL corpus.

use std::path::PathBuf;

use yggdryl::{Authority, Error, Uri, UriPath, Url, Urn};

/// Return the target, byte offset, and reason a parse failure reports.
fn parse_failure(error: &Error) -> (&str, usize, &str) {
    match error {
        Error::Parse {
            target,
            position,
            reason,
        } => (target, *position, reason.as_str()),
        other => panic!("expected a parse error, got {other}"),
    }
}

/// Assert one input fails at one byte of the *original* text.
fn fails_at(input: &str, position: usize, reason: &str) {
    let error = Uri::from_str(input).expect_err(&format!("{input:?} must not parse"));
    let (_, reported, actual) = parse_failure(&error);
    assert_eq!(reported, position, "{input:?} reported the wrong byte");
    assert_eq!(actual, reason, "{input:?} reported the wrong reason");
    assert!(
        input.is_char_boundary(reported),
        "{input:?} reported byte {reported}, which splits a character"
    );
}

/// RFC 3986 s2.1: `pct-encoded = "%" HEXDIG HEXDIG`, exactly two digits.
///
/// WHATWG keeps a short escape as literal text instead; this crate validates,
/// so the escape is refused and the offset names the `%` that opened it, in the
/// original string rather than in the component the parser sliced out.
#[test]
fn a_short_percent_escape_is_refused_at_the_percent_that_opened_it() {
    const REASON: &str = "percent escape must contain exactly two hexadecimal digits";

    fails_at("https://example.test/a%", 22, REASON);
    fails_at("https://example.test/a%2", 22, REASON);
    fails_at("https://example.test/a%zz", 22, REASON);
    fails_at("https://example.test/a%2zb", 22, REASON);
    // The doubled percent is the failure: `%%` opens an escape whose first
    // digit is `%`, so it is refused where a lenient parser keeps `%%41`.
    fails_at("https://example.test/a%%41", 22, REASON);
    fails_at("https://example.test/%", 21, REASON);
    fails_at("https://example.test/?q=%", 24, REASON);
    fails_at("https://example.test/?q=a%2", 25, REASON);
    fails_at("https://example.test/#%", 22, REASON);
    fails_at("https://example.test/#a%2", 23, REASON);
    fails_at("https://a%/", 9, REASON);
    fails_at("https://a%2/", 9, REASON);
    fails_at("https://[fe80::1%eth0]/", 16, REASON);

    // The same rule on every component that takes text of its own.
    assert!(UriPath::from_str("/a%").is_err());
    assert!(Authority::from_str("a%").is_err());
    assert!(
        Uri::from_str("https://example.test/")
            .unwrap()
            .set_query(Some("a=%2"))
            .is_err()
    );
}

/// RFC 3986 s6.2.2.1: only the two hex digits case-normalize, and they go up.
///
/// WHATWG preserves the case an escape was written with; this crate follows the
/// RFC so that two spellings of one URI are one value, which is what lets `Eq`
/// and `stable_hash` answer equivalence.
#[test]
fn only_the_hex_digits_of_an_escape_change_case_and_they_uppercase() {
    let uri = Uri::from_str("HTTPS://example.test/%2fAbC%2Fdef?q=%c3%a9#%7bx%7D").unwrap();

    assert_eq!(
        uri.to_string(),
        "https://example.test/%2FAbC%2Fdef?q=%C3%A9#%7Bx%7D"
    );
    // Literal path text keeps its case; only the escapes moved.
    assert_eq!(uri.path().as_str(), "/%2FAbC%2Fdef");
    assert_eq!(
        Uri::from_str("https://example.test/%3A%3a%3C%3c")
            .unwrap()
            .path()
            .as_str(),
        "/%3A%3A%3C%3C"
    );
    // Two spellings of one URI are one value.
    assert_eq!(
        Uri::from_str("https://example.test/a%3ab").unwrap(),
        Uri::from_str("https://example.test/a%3Ab").unwrap()
    );
    assert_eq!(
        Uri::from_str("https://example.test/a%3ab")
            .unwrap()
            .stable_hash(),
        Uri::from_str("https://example.test/a%3Ab")
            .unwrap()
            .stable_hash()
    );
}

/// RFC 3986 s2.2 and s6.2.2.2: an escaped reserved octet is data, not syntax.
///
/// Components are split before anything is decoded, so `%2F` stays inside the
/// segment that carried it. Decoding first is the path-traversal bug class
/// Tomcat, Spring, and IIS have each shipped.
#[test]
fn an_escaped_reserved_octet_is_data_and_never_becomes_structure() {
    let uri = Uri::from_str("https://example.test/a%2Fb/c").unwrap();
    assert_eq!(uri.path_segments().collect::<Vec<_>>(), ["a%2Fb", "c"]);
    assert_eq!(uri.path().segment_len(), 2);
    assert_eq!(uri.file_name(), Some("c"));

    // An escaped separator standing alone is one ordinary segment.
    assert_eq!(
        Uri::from_str("https://example.test/a/%2F/c")
            .unwrap()
            .path_segments()
            .collect::<Vec<_>>(),
        ["a", "%2F", "c"]
    );
    // A backslash is a separator on no host once it is escaped.
    assert_eq!(
        Uri::from_str("https://example.test/a%5Cb")
            .unwrap()
            .path_segments()
            .collect::<Vec<_>>(),
        ["a%5Cb"]
    );
    // Text decoding is not structure: `%2F` reads back as a slash inside the
    // segment, which is why segments read the encoded form instead.
    assert_eq!(uri.path_text(true).unwrap(), "/a/b/c");
    assert_eq!(uri.path_text(false).unwrap(), "/a%2Fb/c");

    // `..%2Fetc` is one name, not a traversal, in every structural view.
    let traversal = Uri::from_str("https://example.test/lake/..%2Fetc/passwd").unwrap();
    assert_eq!(traversal.parts(), ["lake", "..%2Fetc", "passwd"]);
    assert_eq!(
        traversal.path().normalize().unwrap().as_str(),
        "/lake/..%2Fetc/passwd"
    );
}

/// RFC 3986 s2.4: never encode or decode the same string twice.
///
/// `%2525` is a URI carrying the text `%25`, which is a URI carrying `%`. Each
/// direction moves exactly one level; the double-decode is the IIS
/// CVE-2001-0333 bug class and the double-encode its mirror.
#[test]
fn encoding_and_decoding_each_move_exactly_one_level() {
    let uri = Uri::from_str("https://example.test/%2525").unwrap();
    assert_eq!(uri.path().as_str(), "/%2525");
    assert_eq!(uri.path_text(true).unwrap(), "/%25");
    assert_eq!(uri.to_string(), "https://example.test/%2525");

    // A file name is data, so encoding it once is what a path conversion does.
    let path = Uri::from_path("/lake/50%25off").unwrap();
    assert_eq!(path.to_string(), "file:///lake/50%2525off");
    assert_eq!(
        path.clone().into_path().unwrap(),
        PathBuf::from("/lake/50%25off")
    );
    // And encoding the encoded form again is a different, equally exact URI.
    assert_eq!(
        Uri::from_path("/lake/50%2525off").unwrap().to_string(),
        "file:///lake/50%252525off"
    );
}

/// A literal `%` is an ordinary file name byte, on every operating system.
///
/// `100%.csv` is the name that breaks .NET's `System.Uri`, Node's
/// `new URL(path, 'file:')`, and a long tail of tooling. RFC 3986 s2.4 says the
/// data byte `%` is spelled `%25`, and the conversion is exact in both
/// directions.
#[test]
fn a_file_name_carrying_a_literal_percent_round_trips_exactly() {
    for name in [
        "100%.csv",
        "%20.txt",
        "a%b",
        "report %2F final.pdf",
        "50%25discount",
        "%",
        "%%",
        "% 20",
        "%zz",
        "e:%41foo%20bar%25.baz",
        "caf\u{e9} 100%.csv",
    ] {
        let source = format!("/lake/{name}");
        let uri = Uri::from_path(&source).unwrap();
        assert!(
            !uri.path().as_str().contains("%2525") || name.contains("%25"),
            "{name:?} encoded more than once: {uri}"
        );
        assert_eq!(
            uri.clone().into_path().unwrap(),
            PathBuf::from(&source),
            "{name:?} did not survive the round trip through {uri}"
        );
        // The URI it spells is itself parseable and canonical.
        assert_eq!(Uri::from_str(&uri.to_string()).unwrap(), uri);
    }

    assert_eq!(
        Uri::from_path("/lake/100%.csv").unwrap().to_string(),
        "file:///lake/100%25.csv"
    );
    // The bare `%` never reaches the URI, so nothing downstream can read it as
    // an escape that lost its digits.
    assert!(
        !Uri::from_path("/lake/a%b")
            .unwrap()
            .to_string()
            .contains("%b")
    );
}

/// An escape must not create a dot segment the URI does not show.
///
/// WHATWG treats `.`, `%2e`, `%2E` as the same segment and pops on
/// `..`, `.%2e`, `%2e.`, `%2e%2e`. This crate reads structure from the encoded
/// text, so those spellings stay opaque segments - `parts`, `normalize`, and
/// `segments_under` all see a name. That is safe only while the escape cannot
/// become a real dot segment later, so the platform-path conversion refuses it,
/// the same way it refuses an escaped separator. Node's `fileURLToPath` decodes
/// them instead and walks straight out of the directory it was given.
#[test]
fn percent_escapes_cannot_smuggle_a_dot_segment_into_a_platform_path() {
    const REASON: &str = "percent escapes cannot create a dot segment";

    for spelling in ["%2E%2E", "%2e%2e", ".%2E", "%2E.", "%2E", "%2e"] {
        let uri = Uri::from_str(&format!("file:///lake/{spelling}/etc/passwd")).unwrap();
        // Every structural view reads it as one ordinary name.
        assert!(
            uri.parts().contains(&uri.path().get_segment(1).unwrap()),
            "{spelling:?} lost its segment"
        );
        assert_eq!(uri.path().normalize().unwrap(), *uri.path());

        let error = uri
            .clone()
            .into_path()
            .expect_err(&format!("{spelling:?} must not reach a platform path"));
        let (target, position, reason) = parse_failure(&error);
        assert_eq!(target, "file URI path");
        assert_eq!(reason, REASON);
        assert_eq!(position, 6, "{spelling:?} named the wrong byte");
    }

    // A trailing escaped dot segment is refused too, and so is one under a UNC
    // authority, where the parent it names is the share root.
    assert!(
        Uri::from_str("file:///lake/%2E%2E")
            .unwrap()
            .into_path()
            .is_err()
    );
    assert!(
        Uri::from_str("file://server/share/%2E%2E/x")
            .unwrap()
            .into_path()
            .is_err()
    );

    // Only a whole segment is a dot segment: an escaped dot inside a longer
    // name is an ordinary byte and converts.
    for kept in ["%2Ehtml", "%2e.bar", "a%2Eb", "%2E%2Ebar", "..%2E"] {
        let uri = Uri::from_str(&format!("file:///lake/{kept}")).unwrap();
        assert!(
            uri.clone().into_path().is_ok(),
            "{kept:?} is a name, not a dot segment"
        );
    }

    // A dot segment the path spells outright is not an escape and is kept: the
    // caller's own resolution owns it.
    assert_eq!(
        Uri::from_str("file:///lake/../etc")
            .unwrap()
            .into_path()
            .unwrap(),
        PathBuf::from("/lake/../etc")
    );
}

/// An escaped separator or NUL cannot become a platform path either.
///
/// The escape is data in the URI - one segment, exactly as RFC 3986 s2.2 says -
/// so turning it into a path would either invent a separator or truncate the
/// name at the NUL. Node accepts the same input on one API and refuses it on
/// another; here both directions agree.
#[test]
fn escaped_separators_and_nul_stay_inside_the_uri() {
    for (input, reason) in [
        (
            "file:///lake/a%2Fb",
            "encoded path separators cannot be converted safely",
        ),
        (
            "file:///lake/a%5Cb",
            "encoded path separators cannot be converted safely",
        ),
        ("file:///lake/a%00b", "file path must not contain NUL"),
        (
            "file:///lake/a%252F%2Fb",
            "encoded path separators cannot be converted safely",
        ),
    ] {
        let uri = Uri::from_str(input).unwrap();
        assert_eq!(uri.path().segment_len(), 2, "{input:?} split a segment");
        let error = uri
            .into_path()
            .expect_err(&format!("{input:?} must not convert"));
        assert_eq!(parse_failure(&error).2, reason, "{input:?}");
    }

    // `%00` is legal URI data everywhere the generic syntax allows a byte, and
    // nothing truncates at it.
    let nul = Uri::from_str("https://example.test/a%00b?q=%00#%00").unwrap();
    assert_eq!(nul.path().as_str(), "/a%00b");
    assert_eq!(nul.path_text(true).unwrap().len(), 4);
    assert_eq!(nul.to_string(), "https://example.test/a%00b?q=%00#%00");
}

/// Escapes that are not UTF-8 stay bytes in the URI and refuse to be text.
///
/// `%C0%AF` is the overlong slash of IIS CVE-2000-0884 and `%ED%A0%80` a lone
/// surrogate: neither may decode to the character it imitates, and neither may
/// be silently replaced. Both survive as URI syntax and both refuse decoding.
#[test]
fn escapes_that_are_not_utf8_are_kept_as_bytes_and_never_decode() {
    for escape in ["%C0%AF", "%ED%A0%80", "%FF", "%EF", "%C0%80"] {
        let uri = Uri::from_str(&format!("https://example.test/{escape}?q={escape}")).unwrap();
        assert_eq!(uri.path().as_str(), format!("/{escape}"));
        assert_eq!(
            uri.to_string(),
            format!("https://example.test/{escape}?q={escape}")
        );

        let error = uri.path_text(true).expect_err("must not decode");
        assert_eq!(
            parse_failure(&error).2,
            "percent escapes must decode to UTF-8"
        );
        assert!(uri.query(true).is_err());

        let file = Uri::from_str(&format!("file:///lake/{escape}")).unwrap();
        assert_eq!(
            parse_failure(&file.into_path().expect_err("must not convert")).2,
            "file path percent escapes must decode to UTF-8"
        );
    }

    // The offset names the escape the decode stopped at, not the byte the
    // decoded buffer stopped at.
    let error = Uri::from_str("file:///lake/%41%FF.arrow")
        .unwrap()
        .into_path()
        .expect_err("must not convert");
    assert_eq!(parse_failure(&error).1, 9);
}

/// RFC 3987 s3.1: non-ASCII is UTF-8 first, then one escape per octet.
#[test]
fn non_ascii_names_become_uppercase_utf8_escapes_and_come_back() {
    let uri = Uri::from_path("/lake/r\u{e9}sum\u{e9}/\u{30a2}.csv").unwrap();

    assert_eq!(
        uri.to_string(),
        "file:///lake/r%C3%A9sum%C3%A9/%E3%82%A2.csv"
    );
    assert_eq!(
        uri.path_text(true).unwrap(),
        "/lake/r\u{e9}sum\u{e9}/\u{30a2}.csv"
    );
    assert_eq!(
        uri.clone().into_path().unwrap(),
        PathBuf::from("/lake/r\u{e9}sum\u{e9}/\u{30a2}.csv")
    );
    // A raw non-ASCII byte is not URI syntax: the text door encodes it, the
    // syntax door refuses it.
    assert!(Uri::from_str("https://example.test/r\u{e9}sum\u{e9}").is_err());
}

/// RFC 8089 s2: a rooted path is one path, whichever separator roots it.
///
/// `\data` is how Windows spells the root of the current drive. It has to land
/// on the same canonical absolute URI as `/data`, or the value it produces is
/// not the one a second pass through the same conversion produces.
#[test]
fn either_separator_roots_a_path_onto_the_same_absolute_file_uri() {
    for rooted in [r"\lake\100%.csv", r"/lake/100%.csv", r"\lake/100%.csv"] {
        let uri = Uri::from_path(rooted).unwrap();
        assert_eq!(uri.to_string(), "file:///lake/100%25.csv", "{rooted:?}");
        assert!(uri.has_authority(), "{rooted:?} lost its authority marker");
        assert_eq!(
            uri.clone().into_path().unwrap(),
            PathBuf::from("/lake/100%.csv")
        );
        // The conversion is its own fixed point.
        assert_eq!(
            Uri::from_path(uri.clone().into_path().unwrap()).unwrap(),
            uri
        );
        assert!(Url::from_uri(uri).is_ok());
    }

    // A relative path keeps no root and stays the relative `file:` form.
    assert_eq!(
        Uri::from_path(r"lake\100%.csv").unwrap().to_string(),
        "file:lake/100%25.csv"
    );
    assert_eq!(Uri::from_path(r"\").unwrap().to_string(), "file:///");
}

/// A path opening on two slashes names a UNC server, so it needs an authority.
///
/// RFC 8089 App. E.3.2 reads `file:////host/share` as a UNC string written into
/// the path. This crate spells UNC with the authority it has - `file://host/share` -
/// so the four-slash form has no host to give a platform path, and converting it
/// would invent one. Python's `url2pathname` guesses; this refuses.
#[test]
fn a_two_slash_path_without_an_authority_is_not_a_unc_server() {
    for input in ["file:////host/share/x", "file://///host/share/x"] {
        let uri = Uri::from_str(input).unwrap();
        assert_eq!(uri.to_string(), input);
        assert!(uri.authority().is_empty());

        let error = uri
            .into_path()
            .expect_err(&format!("{input:?} must not convert"));
        let (target, position, reason) = parse_failure(&error);
        assert_eq!(target, "file URI path");
        assert_eq!(position, 0);
        assert_eq!(
            reason,
            "a path opening on two slashes would name a UNC server this URI has no authority for"
        );
    }

    // The authority form converts, and a UNC platform path spells it back.
    let unc = Uri::from_path(r"\\server\share\100%.csv").unwrap();
    assert_eq!(unc.to_string(), "file://server/share/100%25.csv");
    assert_eq!(unc.authority().as_str(), "server");
    assert_eq!(
        unc.clone().into_path().unwrap(),
        PathBuf::from("//server/share/100%.csv")
    );
    assert_eq!(
        Uri::from_path(unc.clone().into_path().unwrap()).unwrap(),
        unc
    );
}

/// `join_path` takes names; `joinpath` takes URI text.
///
/// One door per kind of input: a platform component is data and is encoded, URI
/// text is syntax and is validated. Mixing them is why `new URL(name, 'file:')`
/// mangles a name holding `%` while `pathToFileURL` does not.
#[test]
fn join_path_spells_platform_names_and_joinpath_spells_uri_text() {
    let lake = Url::from_str("file:///lake").unwrap();

    for (name, expected) in [
        ("100%.csv", "file:///lake/100%25.csv"),
        ("a b.csv", "file:///lake/a%20b.csv"),
        ("caf\u{e9}.csv", "file:///lake/caf%C3%A9.csv"),
        ("a#b.csv", "file:///lake/a%23b.csv"),
        ("a?b.csv", "file:///lake/a%3Fb.csv"),
        ("a%2Fb.csv", "file:///lake/a%252Fb.csv"),
        ("year=2024", "file:///lake/year=2024"),
    ] {
        let joined = lake.join_path(name).unwrap();
        assert_eq!(joined.to_string(), expected, "{name:?}");
        assert_eq!(joined.file_name(), Some(&expected[13..]), "{name:?}");
        assert_eq!(
            joined.into_path().unwrap(),
            PathBuf::from(format!("/lake/{name}")),
            "{name:?} did not come back"
        );
    }

    // A separator inside the platform path is a component boundary, and a
    // separator inside one name is not.
    assert_eq!(
        lake.join_path("year=2024/a b.csv").unwrap().to_string(),
        "file:///lake/year=2024/a%20b.csv"
    );
    assert_eq!(
        lake.join_path("a/b").unwrap(),
        lake.joinpath("a").unwrap().joinpath("b").unwrap()
    );

    // The URI door takes syntax, so it refuses what a segment cannot carry and
    // keeps an escape the caller already wrote.
    assert_eq!(
        lake.joinpath("a%25b").unwrap().to_string(),
        "file:///lake/a%25b"
    );
    assert!(lake.joinpath("100%.csv").is_err());
    assert!(lake.joinpath("a b.csv").is_err());
    assert!(lake.joinpath("a#b").is_err());
}

/// RFC 6874 s2: `IPv6addrz = IPv6address "%25" ZoneID`, and the zone is
/// `1*( unreserved / pct-encoded )`.
///
/// The `%25` is the zone marker, not an escape to decode, so it survives
/// normalization untouched. WHATWG refuses zone identifiers outright; this
/// crate accepts the RFC form and refuses everything around it.
#[test]
fn ipv6_zone_identifiers_follow_the_percent_25_rule() {
    let uri = Uri::from_str("https://[fe80::1%25eth0]:8443/x").unwrap();
    assert_eq!(uri.authority().as_str(), "[fe80::1%25eth0]:8443");
    assert_eq!(uri.authority().host(), "fe80::1%25eth0");
    assert_eq!(uri.authority().port(), Some(8443));
    assert_eq!(uri.to_string(), "https://[fe80::1%25eth0]:8443/x");

    // A zone byte outside the unreserved set is spelled with an escape.
    assert!(Uri::from_str("https://[fe80::1%25en%201]/").is_ok());
    // A bare `%` is not the marker, and an empty zone is not a zone.
    assert!(Uri::from_str("https://[fe80::1%eth0]/").is_err());
    assert!(Uri::from_str("https://[fe80::1%25]/").is_err());
    assert!(Uri::from_str("https://[fe80::1%25 ]/").is_err());
    // A zone belongs to an IPv6 literal, not to IPvFuture or a registered name.
    assert!(Uri::from_str("https://[v1.fe80::a%25en1]/").is_err());
    assert_eq!(
        Uri::from_str("https://192.0.2.1%25eth0/")
            .unwrap()
            .authority()
            .host(),
        "192.0.2.1%25eth0"
    );
}

/// The user-information delimiter is the last `@`, so an escaped one is data.
///
/// `http://a%40b@example.com/` names the user `a%40b` on host `example.com`;
/// reading the first `@` instead hands the URI to the wrong host.
#[test]
fn escaped_authority_delimiters_do_not_move_the_real_ones() {
    let uri = Uri::from_str("https://a%40b:p%40ss@example.test:8443/x").unwrap();

    assert_eq!(uri.user(), Some("a%40b"));
    assert_eq!(uri.password(), Some("p%40ss"));
    assert_eq!(uri.hostname(), Some("example.test"));
    assert_eq!(uri.authority().port(), Some(8443));

    // An escaped colon stays inside the user name.
    let escaped_colon = Uri::from_str("https://user%3Aname:pass@example.test/").unwrap();
    assert_eq!(escaped_colon.user(), Some("user%3Aname"));
    assert_eq!(escaped_colon.password(), Some("pass"));

    // An escaped `%` in the user name is one escape, not a bare percent.
    let percent = Uri::from_str("https://%25DOMAIN:secret@example.test/").unwrap();
    assert_eq!(percent.user(), Some("%25DOMAIN"));
    assert_eq!(
        percent.to_string(),
        "https://%25DOMAIN:secret@example.test/"
    );

    // A second unescaped `@` is ambiguous and refused.
    assert!(Uri::from_str("https://a@b@example.test/").is_err());
}

/// A query pair encodes the three bytes that would end it, `+` included.
///
/// RFC 3986 reads `+` as a literal plus and HTML form decoding reads it as a
/// space; a value that survives both is spelled `%2B`. Reading and writing are
/// each other's inverse, which the S3 `+`-in-a-key bug class is what happens
/// without.
#[test]
fn query_pairs_encode_what_would_end_them_and_read_back_the_same() {
    for query in [
        "a", "a=", "=b", "a=b=c", "a&&b", "=", "%3D=%26", "a=b+c", "a%3Db=c", "a=%2B", "a=%25",
    ] {
        let url = Url::from_str(&format!("https://example.test/?{query}")).unwrap();
        let pairs: Vec<(String, String)> = url
            .parameters(true)
            .unwrap()
            .iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect();

        let mut written = url.clone();
        written
            .set_parameters(&url.parameters(true).unwrap().into_owned())
            .unwrap();
        let read_back: Vec<(String, String)> = written
            .parameters(true)
            .unwrap()
            .iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect();
        assert_eq!(pairs, read_back, "{query:?} is not a fixed point");
    }

    let mut url = Url::from_str("https://example.test/t").unwrap();
    let mut parameters = url.parameters(true).unwrap().into_owned();
    parameters.append("as of", "2026-01-02 09:30").unwrap();
    parameters.append("note", "a&b=c+d%e").unwrap();
    parameters.append("100%", "\u{e9}").unwrap();
    url.set_parameters(&parameters).unwrap();

    assert_eq!(
        url.query(false).unwrap().as_deref(),
        Some("as%20of=2026-01-02%2009:30&note=a%26b%3Dc%2Bd%25e&100%25=%C3%A9")
    );
    let read = url.parameters(true).unwrap();
    assert_eq!(read.get("as of"), Some("2026-01-02 09:30"));
    assert_eq!(read.get("note"), Some("a&b=c+d%e"));
    assert_eq!(read.get("100%"), Some("\u{e9}"));

    // A raw view speaks the query's own bytes and refuses text it cannot carry.
    let raw = url.parameters(false).unwrap();
    assert_eq!(raw.get("as%20of"), Some("2026-01-02%2009:30"));
    let mut raw = raw.into_owned();
    assert!(raw.insert("k", "a b").is_err());
    assert!(raw.insert("k", "a%2").is_err());
    assert!(raw.insert("k", "a&b").is_err());
}

/// A fragment is set from text, so setting and reading it is a round trip.
#[test]
fn a_fragment_set_from_text_reads_back_as_that_text() {
    for text in [
        "a%b",
        "100%",
        "a#b",
        "a+b",
        "a b",
        "a%41",
        "%",
        "caf\u{e9}",
        "a/b?c",
        "",
    ] {
        let mut url = Url::from_str("file:///lake/day.zip").unwrap();
        url.set_fragment(Some(text)).unwrap();
        assert_eq!(
            url.fragment(true).unwrap().as_deref(),
            Some(text),
            "{text:?}"
        );
        assert!(!url.to_string().contains(" "), "{text:?} left a raw space");
        assert_eq!(Uri::from_str(&url.to_string()).unwrap(), url.into_uri());
    }

    let mut url = Url::from_str("file:///lake/day.zip").unwrap();
    url.set_fragment(Some("trades/eu ndx.csv")).unwrap();
    assert_eq!(url.to_string(), "file:///lake/day.zip#trades/eu%20ndx.csv");
}

/// RFC 3986 s6.2.3: an empty component that is present is not an absent one.
#[test]
fn an_empty_query_or_fragment_delimiter_survives_canonicalization() {
    for input in [
        "https://example.test/?",
        "https://example.test/#",
        "https://example.test/?#",
        "https://example.test/?#x",
    ] {
        let uri = Uri::from_str(input).unwrap();
        assert_eq!(uri.to_string(), input);
        assert_eq!(Uri::from_str(&uri.to_string()).unwrap(), uri);
    }

    assert_eq!(
        Uri::from_str("https://example.test/?")
            .unwrap()
            .query(false)
            .unwrap()
            .as_deref(),
        Some("")
    );
    assert_eq!(
        Uri::from_str("https://example.test/")
            .unwrap()
            .query(false)
            .unwrap(),
        None
    );
    assert_ne!(
        Uri::from_str("https://example.test/?").unwrap(),
        Uri::from_str("https://example.test/").unwrap()
    );
}

/// RFC 8141 s2 and s3.1: the NSS carries escapes, and they are never decoded
/// for comparison, so `%2C` and `,` stay two different URNs.
#[test]
fn urn_namespace_specific_strings_keep_their_escapes() {
    let urn = Urn::from_str("URN:EXAMPLE:a123%2cz456").unwrap();

    // Scheme and namespace lowercase; the NSS keeps its case and its escapes,
    // whose hex digits uppercase.
    assert_eq!(urn.to_string(), "urn:example:a123%2Cz456");
    assert_eq!(urn.namespace(), "example");
    assert_eq!(urn.namespace_specific(), "a123%2Cz456");
    assert_ne!(urn, Urn::from_str("urn:example:a123,z456").unwrap());
    assert_ne!(urn, Urn::from_str("urn:example:A123%2Cz456").unwrap());

    // The NSS may hold `/`, and `?`, `#`, `[`, `]` only as escapes.
    assert_eq!(
        Urn::from_str("urn:example:1/406/47452/2")
            .unwrap()
            .file_name(),
        Some("2")
    );
    let escaped = Urn::from_str("urn:example:a%5Bb%5D%3Fc").unwrap();
    assert_eq!(escaped.namespace_specific(), "a%5Bb%5D%3Fc");
    assert_eq!(escaped.path_text(true).unwrap(), "example:a[b]?c");

    // A `?` that opens neither an r- nor a q-component is a syntax error.
    assert!(Urn::from_str("urn:example:foo?bar").is_err());
    let resolved = Urn::from_str("urn:example:foo?+CCResolve:cc=uk?=q1#f1").unwrap();
    assert_eq!(
        resolved.query(false).unwrap().as_deref(),
        Some("+CCResolve:cc=uk?=q1")
    );
    assert_eq!(resolved.fragment(false).unwrap().as_deref(), Some("f1"));
}

/// Escapes cannot spell a Windows drive designator into a platform path.
///
/// `file:///C%3A/x` is a URI whose first segment is the three-character name
/// `C%3A`, not the drive `C:`; WHATWG agrees the escape means it is not a drive
/// letter. Converting it would silently promote the name to a drive, so the
/// conversion is refused and the drive form has to be spelled outright.
#[test]
fn escaped_drive_designators_cannot_become_a_drive() {
    for input in [
        "file:///C%3A/x",
        "file:///%43%3A/x",
        "file:///%43:/x",
        "file:///c%3a/x",
    ] {
        let uri = Uri::from_str(input).unwrap();
        assert!(uri.into_path().is_err(), "{input:?} must not convert");
    }

    let drive = Uri::from_str("file:///c:/lake/100%25.csv").unwrap();
    assert_eq!(drive.to_string(), "file:///C:/lake/100%25.csv");
    assert_eq!(
        drive.into_path().unwrap(),
        PathBuf::from("C:/lake/100%.csv")
    );
    // The vertical-bar spelling is not URI syntax at all.
    assert!(Uri::from_str("file:///c|/x").is_err());
}

/// RFC 8089 s2: `file:/data` and `file:///data` name the same local path.
///
/// Both are valid syntax, so a canonical value has to pick one, and the one it
/// picks is the one a platform path converts to. Otherwise two values of the
/// same file compare and hash apart - which is exactly the mismatch Spark and
/// this crate produce when they write the same Iceberg table location.
#[test]
fn an_absolute_file_path_always_carries_its_authority_marker() {
    for spelled in ["file:/lake/100%25.csv", "file:///lake/100%25.csv"] {
        let uri = Uri::from_str(spelled).unwrap();
        assert_eq!(uri.to_string(), "file:///lake/100%25.csv", "{spelled:?}");
        assert!(uri.has_authority(), "{spelled:?}");
        assert_eq!(uri, Uri::from_path("/lake/100%.csv").unwrap());
        // The hierarchical form is what a URL requires, so both reach one.
        assert!(Url::from_str(spelled).is_ok(), "{spelled:?}");
    }

    // A relative `file:` path has no root, so it gains no marker.
    let relative = Uri::from_str("file:lake/100%25.csv").unwrap();
    assert!(!relative.has_authority());
    assert_eq!(relative.to_string(), "file:lake/100%25.csv");
    assert_eq!(relative, Uri::from_path("lake/100%.csv").unwrap());
}

/// A component after a UNC server is a share name, not a drive designator.
///
/// There is no drive at `\\server\c:`, and RFC 8089 s2 requires a file path's
/// case to be kept as given, so the drive rule only applies where no authority
/// names a host.
#[test]
fn a_share_name_shaped_like_a_drive_keeps_its_case() {
    for spelled in ["//server/c:/x", r"\\server\c:\x"] {
        let uri = Uri::from_path(spelled).unwrap();
        assert_eq!(uri.to_string(), "file://server/c:/x", "{spelled:?}");
        assert_eq!(uri.authority().as_str(), "server");
        assert_eq!(
            uri.clone().into_path().unwrap(),
            PathBuf::from("//server/c:/x")
        );
        assert_eq!(
            Uri::from_path(uri.clone().into_path().unwrap()).unwrap(),
            uri
        );
    }
    assert_eq!(
        Uri::from_str("file://server/c:/x").unwrap().to_string(),
        "file://server/c:/x"
    );

    // With no authority the first segment is a drive, and it uppercases.
    assert_eq!(Uri::from_path("/c:/x").unwrap().to_string(), "file:///C:/x");
}

/// A one-letter scheme is not a drive letter once it carries an authority.
///
/// `a://host/p` is what `from_parts` builds from the scheme `a`, so parsing its
/// own spelling has to give it back. `C:/x` and `C:\x` stay the drive reading:
/// nothing distinguishes them from a one-letter scheme with an absolute path,
/// and a platform path is what that spelling means in this crate.
#[test]
fn a_one_letter_scheme_with_an_authority_is_not_a_windows_drive() {
    for input in ["a://host/p", "c://bucket/key%25", "z://h"] {
        let uri = Uri::from_str(input).unwrap();
        assert_eq!(uri.to_string(), input, "{input:?}");
        assert_eq!(Uri::from_str(&uri.to_string()).unwrap(), uri);
        assert_ne!(uri.scheme(), &yggdryl::Scheme::FILE, "{input:?}");
    }

    // The drive reading, which the crate keeps for the separator-less spelling.
    assert_eq!(Uri::from_str("C:/x").unwrap().to_string(), "file:///C:/x");
    assert_eq!(Uri::from_str(r"C:\x").unwrap().to_string(), "file:///C:/x");
}

/// Resolving `.` and `..` never turns a relative path into an absolute one.
///
/// A relative path that resolves to no name at all is still relative: the
/// trailing separator says the location is a container, not that it is the
/// root.
#[test]
fn normalizing_a_relative_path_keeps_it_relative() {
    for (input, expected) in [
        ("./", ""),
        (".///", ""),
        ("a/../", ""),
        ("./a/", "a/"),
        ("a/./b", "a/b"),
        ("/./", "/"),
        ("/a/../", "/"),
        ("/", "/"),
        ("a/b/", "a/b/"),
    ] {
        let normalized = UriPath::from_str(input).unwrap().normalize().unwrap();
        assert_eq!(normalized.as_str(), expected, "{input:?}");
        assert_eq!(
            normalized.is_absolute(),
            UriPath::from_str(input).unwrap().is_absolute(),
            "{input:?} changed rootedness"
        );
    }
}

/// A refused conversion names the byte of the URI it refused.
#[test]
fn a_refused_path_conversion_points_at_the_offending_byte() {
    for (input, position) in [
        ("file:///x?q=1", 9),
        ("file:///x#rows", 9),
        ("file://host/x?q=1", 13),
        ("file://host/x#rows", 13),
    ] {
        let error = Uri::from_str(input)
            .unwrap()
            .into_path()
            .expect_err(&format!("{input:?} must not convert"));
        let (_, reported, reason) = parse_failure(&error);
        assert_eq!(reported, position, "{input:?}");
        assert_eq!(
            reason,
            "file URI query and fragment components cannot be represented by a path"
        );
        assert_eq!(
            &input[reported..reported + 1],
            if input.contains('?') { "?" } else { "#" }
        );
    }
}

/// A backslash settles the drive reading outright, because URI syntax has none.
///
/// `C:\x` and `C:\\x` can only be platform paths; `C:/x` is read as one too,
/// which is the ambiguity a one-letter scheme with an absolute path shares with
/// a drive. Only the authority marker separates them, and that case is covered
/// by [`a_one_letter_scheme_with_an_authority_is_not_a_windows_drive`].
#[test]
fn a_backslash_after_a_drive_letter_is_always_a_platform_path() {
    for input in [r"C:\x", r"C:\\x", r"D:\srv\100%.csv", r"c:\a%b"] {
        let uri = Uri::from_str(input).unwrap_or_else(|error| panic!("{input:?}: {error}"));
        assert_eq!(uri.scheme(), &yggdryl::Scheme::FILE, "{input:?}");
        assert!(
            uri.to_string().starts_with("file:///"),
            "{input:?} -> {uri}"
        );
        assert_eq!(uri, Uri::from_path(input).unwrap(), "{input:?}");
    }

    assert_eq!(
        Uri::from_str(r"D:\srv\100%.csv").unwrap().to_string(),
        "file:///D:/srv/100%25.csv"
    );
}

/// A failing escape in a query pair names its byte in the query it came from.
///
/// The pairs are a view over one component, so an offset relative to the half
/// pair a split produced addresses nothing the caller holds.
#[test]
fn a_query_pair_escape_failure_names_its_byte_in_the_query() {
    for (query, position, target) in [
        ("alphabeta=%FF", 10, "uri query value"),
        ("a=1&bcd=%FF", 8, "uri query value"),
        ("%FF=1", 0, "uri query key"),
        ("a=1&b=2&%FF=3", 8, "uri query key"),
        ("a=1&b=2&c=%C0%AF", 10, "uri query value"),
    ] {
        let error = yggdryl::Parameters::from_query(query, true)
            .err()
            .unwrap_or_else(|| panic!("{query:?} must not decode"));
        let (reported_target, reported, _) = parse_failure(&error);
        assert_eq!(reported_target, target, "{query:?}");
        assert_eq!(reported, position, "{query:?}");
        assert_eq!(&query[reported..reported + 1], "%", "{query:?}");
    }

    // The same offsets reach a caller through the URI's own view.
    let error = Uri::from_str("https://example.test/?a=1&note=%FF")
        .unwrap()
        .parameters(true)
        .expect_err("the escape must not decode");
    assert_eq!(parse_failure(&error).1, 9);
}

/// An absolute component replaces the URL rather than extending it twice.
///
/// `Path::components` yields the root and then the names under it, so the
/// conversion of the whole path already holds them: continuing the loop past
/// the root appends every one of them a second time.
#[test]
fn join_path_replaces_the_url_when_the_component_is_absolute() {
    let lake = Url::from_str("file:///lake/trades").unwrap();

    for (joined, expected) in [
        ("/a/b", "file:///a/b"),
        ("/x", "file:///x"),
        ("/a/b/100%.csv", "file:///a/b/100%25.csv"),
        ("/", "file:///"),
    ] {
        assert_eq!(
            lake.join_path(joined).unwrap().to_string(),
            expected,
            "{joined:?}"
        );
    }

    // A relative component still extends, and `.` and `..` still resolve.
    assert_eq!(
        lake.join_path("../lake2/a b.csv").unwrap().to_string(),
        "file:///lake/lake2/a%20b.csv"
    );
    assert_eq!(
        lake.join_path("./x").unwrap().to_string(),
        "file:///lake/trades/x"
    );
}

/// A file name is a name, so no mutation may turn one into a dot segment.
///
/// `..a` has the extension `a` and the stem `.`, so dropping the extension used
/// to leave the path addressing the directory that holds the file instead of
/// the file. The suffix removers keep the name; the setters refuse the value.
#[test]
fn a_filename_mutation_never_produces_a_dot_segment() {
    // Removing the one extension these names have would leave `.` or `..`.
    for name in ["..a", "...a"] {
        let source = Url::from_str(&format!("file:///lake/{name}")).unwrap();

        let mut removed = source.clone();
        assert!(!removed.remove_extension(), "{name:?}");
        assert_eq!(removed, source, "{name:?}");

        let mut cleared = source.clone();
        assert!(!cleared.clear_extensions(), "{name:?}");
        assert_eq!(cleared, source, "{name:?}");

        assert_eq!(source.parts(), ["lake", name], "{name:?}");
    }

    // A compound name keeps a usable stem, so one suffix comes off; clearing
    // every suffix would leave `.` again, so that one does not.
    let mut compound = Url::from_str("file:///lake/..a.b.c").unwrap();
    assert!(compound.remove_extension());
    assert_eq!(compound.to_string(), "file:///lake/..a.b");
    assert!(!compound.clear_extensions());
    assert_eq!(compound.to_string(), "file:///lake/..a.b");

    // The setters refuse a dot segment outright, and leave the path alone.
    let mut url = Url::from_str("file:///lake/report.csv").unwrap();
    for value in [".", ".."] {
        assert!(url.set_file_name(value).is_err(), "{value:?}");
        assert!(url.set_stem(value).is_err(), "{value:?}");
        assert_eq!(url.to_string(), "file:///lake/report.csv", "{value:?}");
    }

    // An ordinary dotfile is untouched by the rule.
    let mut hidden = Url::from_str("file:///lake/.env.local").unwrap();
    assert!(hidden.remove_extension());
    assert_eq!(hidden.to_string(), "file:///lake/.env");
}

/// A drive letter and a one-letter scheme are told apart by what follows.
///
/// A backslash is not URI syntax, so it settles the reading outright. After a
/// slash, syntax only a URI carries decides it: the `//` authority marker, a
/// `?`, or a `#`. Otherwise the drive reading stands, because a Windows path is
/// what that spelling means here.
#[test]
fn uri_only_syntax_after_a_drive_letter_makes_the_value_a_uri() {
    // The components survive as components rather than being escaped into the
    // path, which is what `from_parts` spells and has to parse back.
    let built = Uri::from_parts(
        yggdryl::Scheme::from_str("a").unwrap(),
        Authority::from_str("").unwrap(),
        UriPath::from_str("/b").unwrap(),
        Some("q=1".into()),
        Some("f".into()),
    )
    .unwrap();
    assert_eq!(built.to_string(), "a:/b?q=1#f");
    assert_eq!(Uri::from_str("a:/b?q=1#f").unwrap(), built);
    assert_eq!(
        Uri::from_str("a:/b?q=1#f")
            .unwrap()
            .query(false)
            .unwrap()
            .as_deref(),
        Some("q=1")
    );

    // A backslash keeps the drive reading whatever follows it, so a Windows
    // name carrying a `#` still becomes an escaped path segment.
    assert_eq!(
        Uri::from_str(r"C:\Users\a#b.txt").unwrap().to_string(),
        "file:///C:/Users/a%23b.txt"
    );
    assert_eq!(
        Uri::from_str("C:/Users/x").unwrap().to_string(),
        "file:///C:/Users/x"
    );
}
