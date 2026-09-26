//! `rust/src/http/server/mount.rs`: the HTTP reading of the `IOBase` verbs
//! over a mounted holder - reads whole, by range or as a `304`, listings,
//! writes and removals.

use super::*;

#[test]
fn an_absent_leaf_is_404_and_a_post_to_a_mount_is_405() {
    let server = served();
    let response = get(&server, "/missing.json");
    assert_eq!(response.status(), Status::NOT_FOUND);
    let response = Request::post(&server.url_of("/rows.json").expect("url").to_string(), "x")
        .expect("request")
        .send()
        .expect("send");
    assert_eq!(response.status(), Status::METHOD_NOT_ALLOWED);
    assert_eq!(
        response.headers().get("allow"),
        Some("GET, HEAD, PUT, DELETE, OPTIONS")
    );
    assert_eq!(server.request_count(), 2);
}

#[test]
fn a_put_on_a_container_is_409() {
    let server = served();
    let response = Request::put(&server.url_of("/dir").expect("url").to_string(), "x")
        .expect("request")
        .send()
        .expect("send");
    assert_eq!(response.status(), Status::CONFLICT);
}

#[test]
fn a_range_past_the_end_is_416_naming_the_total() {
    let server = served();
    let (head, body) = raw(
        &server,
        &request_line("GET", "/rows.json", "Range: bytes=100-\r\n"),
    );
    assert_eq!(head.status, Status::RANGE_NOT_SATISFIABLE);
    assert_eq!(
        head.headers.get("content-range"),
        Some(format!("bytes */{}", ROWS.len()).as_str())
    );
    assert!(body.is_empty());
}

#[test]
fn a_delete_of_nothing_is_404() {
    let server = served();
    let response = Request::delete(&server.url_of("/nothing.bin").expect("url").to_string())
        .expect("request")
        .send()
        .expect("send");
    assert_eq!(response.status(), Status::NOT_FOUND);
    assert_eq!(server.request_count(), 1);
}

#[test]
fn a_get_of_a_leaf_streams_it_with_its_validators() {
    let server = served();
    let response = get(&server, "/rows.json");
    assert_eq!(response.status(), Status::OK);
    assert_eq!(&*response.bytes().expect("body"), ROWS);
    let headers = response.headers();
    assert_eq!(headers.get("content-type"), Some("application/json"));
    assert_eq!(
        headers.content_length().expect("length"),
        Some(ROWS.len() as u64)
    );
    assert_eq!(headers.get("accept-ranges"), Some("bytes"));
    let etag = headers.etag().expect("etag").expect("an etag");
    assert!(!etag.is_weak());
    assert_eq!(
        etag.opaque.len(),
        16,
        "the lower-case hex of a 64-bit digest"
    );
    assert!(headers.last_modified().expect("date").is_some());
    assert!(
        headers
            .get("server")
            .is_some_and(|server| server.starts_with("yggdryl/"))
    );
    assert!(headers.date().expect("date").is_some());
    assert_eq!(server.request_count(), 1);
}

#[test]
fn a_head_declares_the_length_and_writes_no_body() {
    let server = served();
    let (status, headers, body) = raw_head(&server, &request_line("HEAD", "/rows.json", ""));
    assert_eq!(status, Status::OK);
    assert_eq!(
        headers.content_length().expect("length"),
        Some(ROWS.len() as u64)
    );
    assert_eq!(headers.get("accept-ranges"), Some("bytes"));
    assert!(headers.get("etag").is_some());
    assert!(body.is_empty());
}

#[test]
fn a_single_byte_range_is_206_in_its_three_spellings() {
    let server = served();
    let total = ROWS.len();
    for (range, start, last) in [
        ("bytes=2-4", 2, 4),
        ("bytes=20-", 20, total - 1),
        ("bytes=-3", total - 3, total - 1),
        ("bytes=0-1000", 0, total - 1),
    ] {
        let (head, body) = raw(
            &server,
            &request_line("GET", "/rows.json", &format!("Range: {range}\r\n")),
        );
        assert_eq!(head.status, Status::PARTIAL_CONTENT, "{range}");
        assert_eq!(
            head.headers.get("content-range"),
            Some(format!("bytes {start}-{last}/{total}").as_str()),
            "{range}"
        );
        assert_eq!(body, &ROWS[start..=last], "{range}");
        assert_eq!(
            head.headers.content_length().expect("length"),
            Some(body.len() as u64)
        );
    }
}

#[test]
fn several_ranges_or_a_malformed_one_are_answered_whole() {
    let server = served();
    for range in ["bytes=0-1,3-4", "items=0-1", "bytes=x-y", "bytes=4-2"] {
        let (head, body) = raw(
            &server,
            &request_line("GET", "/rows.json", &format!("Range: {range}\r\n")),
        );
        assert_eq!(head.status, Status::OK, "{range}");
        assert_eq!(body, ROWS, "{range}");
    }
}

#[test]
fn if_range_holds_for_the_etag_and_the_date_and_fails_for_another() {
    let server = served();
    let response = get(&server, "/rows.json");
    let etag = response.headers().get("etag").expect("etag").to_owned();
    let date = response
        .headers()
        .get("last-modified")
        .expect("date")
        .to_owned();
    for validator in [etag.as_str(), date.as_str()] {
        let (head, body) = raw(
            &server,
            &request_line(
                "GET",
                "/rows.json",
                &format!("Range: bytes=0-2\r\nIf-Range: {validator}\r\n"),
            ),
        );
        assert_eq!(head.status, Status::PARTIAL_CONTENT, "{validator}");
        assert_eq!(body, &ROWS[..3]);
    }
    let (head, body) = raw(
        &server,
        &request_line(
            "GET",
            "/rows.json",
            "Range: bytes=0-2\r\nIf-Range: \"0000000000000000\"\r\n",
        ),
    );
    assert_eq!(head.status, Status::OK, "a mismatch answers the whole");
    assert_eq!(body, ROWS);
}

#[test]
fn if_none_match_and_if_modified_since_answer_304_with_the_validators() {
    let server = served();
    let response = get(&server, "/rows.json");
    let etag = response.headers().get("etag").expect("etag").to_owned();
    let date = response
        .headers()
        .get("last-modified")
        .expect("date")
        .to_owned();
    let (head, body) = raw(
        &server,
        &request_line(
            "GET",
            "/rows.json",
            &format!("If-None-Match: \"other\", {etag}\r\n"),
        ),
    );
    assert_eq!(head.status, Status::NOT_MODIFIED);
    assert_eq!(head.headers.get("etag"), Some(etag.as_str()));
    assert!(body.is_empty());
    assert_eq!(head.headers.get("content-length"), None);
    let (head, _) = raw(
        &server,
        &request_line(
            "GET",
            "/rows.json",
            &format!("If-Modified-Since: {date}\r\n"),
        ),
    );
    assert_eq!(head.status, Status::NOT_MODIFIED);
    assert_eq!(head.headers.get("last-modified"), Some(date.as_str()));
    let (head, body) = raw(
        &server,
        &request_line(
            "GET",
            "/rows.json",
            "If-Modified-Since: Wed, 01 Jan 2020 00:00:00 GMT\r\n",
        ),
    );
    assert_eq!(head.status, Status::OK, "modified since then");
    assert_eq!(body, ROWS);
    let (head, _) = raw(
        &server,
        &request_line("GET", "/rows.json", "If-None-Match: \"other\"\r\n"),
    );
    assert_eq!(head.status, Status::OK);
}

#[test]
fn a_coded_leaf_is_served_coded_with_content_encoding() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    let root = memory_root();
    let coded = yggdryl::gzip::dump(ROWS).expect("gzip");
    root.child_by_path("rows.json.gz")
        .expect("child")
        .write_all_bytes(&coded)
        .expect("write");
    server.mount("/", root).expect("mount");
    let response = get(&server, "/rows.json.gz");
    assert_eq!(response.status(), Status::OK);
    assert_eq!(
        response.headers().get("content-type"),
        Some("application/json")
    );
    assert_eq!(response.headers().get("content-encoding"), Some("gzip"));
    // The byte surface is the body as sent; `bytes` is the decoded view.
    assert_eq!(
        response.read_all_bytes().expect("body"),
        coded,
        "the bytes as stored"
    );
    assert_eq!(&*response.bytes().expect("decoded"), ROWS);
}

#[test]
fn a_declared_media_type_is_served_with_its_charset() {
    let server = served();
    server
        .set_media_type(
            "/rows.json",
            MediaType::from_content_headers(Some("text/plain; charset=windows-1252"), None)
                .expect("media type"),
        )
        .expect("declare");
    let response = get(&server, "/rows.json");
    assert_eq!(
        response.headers().get("content-type"),
        Some("text/plain; charset=windows-1252")
    );
    let narrow = Server::bind("127.0.0.1:0").expect("bind");
    narrow.mount("/data", memory_root()).expect("mount");
    let error = narrow
        .set_media_type("/nowhere", MediaType::default())
        .expect_err("no mount covers the path");
    assert!(matches!(error, Error::Absent { .. }), "{error:?}");
}

#[test]
fn a_get_of_a_container_lists_its_children_as_json() {
    let server = served();
    let response = get(&server, "/");
    assert_eq!(response.status(), Status::OK);
    assert_eq!(
        response.headers().get("content-type"),
        Some("application/json")
    );
    let listing = response.scalar().expect("json");
    let rows = listing.sequence_rows().expect("an array");
    assert_eq!(rows.len(), 2);
    let text = response.text().expect("text");
    assert!(text.contains("\"name\":\"rows.json\""), "{text}");
    assert!(
        text.contains(&format!("\"url\":\"{}rows.json\"", server.url())),
        "{text}"
    );
    assert!(text.contains("\"kind\":\"file\""), "{text}");
    assert!(text.contains(&format!("\"size\":{}", ROWS.len())), "{text}");
    assert!(
        text.contains("\"media_type\":\"application/json\""),
        "{text}"
    );
    assert!(text.contains("\"name\":\"dir\""), "{text}");
    let response = get(&server, "/dir");
    let text = response.text().expect("text");
    assert!(
        text.contains(&format!("\"url\":\"{}dir/a.txt\"", server.url())),
        "{text}"
    );
}

#[test]
fn a_listed_name_is_the_decoded_name_and_its_url_reaches_the_child() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    let root = memory_root();
    for name in ["a b.txt", "q?x#y.txt", "100%.txt"] {
        root.child_by_path(name)
            .expect("child")
            .write_all_bytes(name.as_bytes())
            .expect("write");
    }
    server.mount("/files", root).expect("mount");
    let listing = get(&server, "/files").scalar().expect("json");
    let rows = listing.sequence_rows().expect("an array");
    let mut names = Vec::new();
    for row in rows.iter() {
        let name = row
            .get_key_str("name")
            .expect("a name")
            .as_str()
            .expect("text")
            .to_owned();
        let url = row
            .get_key_str("url")
            .expect("a url")
            .as_str()
            .expect("text")
            .to_owned();
        let body = Request::get(&url).expect("a URL").send().expect("send");
        assert_eq!(body.status(), Status::OK, "{url}");
        assert_eq!(body.text().expect("text"), name, "{url}");
        names.push(name);
    }
    names.sort();
    assert_eq!(names, ["100%.txt", "a b.txt", "q?x#y.txt"]);
}

#[test]
fn a_nested_container_whose_name_needs_escapes_lists_urls_that_reach_its_children() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    let root = memory_root();
    root.child_by_path("a b?#%/x.txt")
        .expect("child")
        .write_all_bytes(b"nested")
        .expect("write");
    server.mount("/files", root).expect("mount");
    let listing = get(&server, "/files/a%20b%3F%23%25")
        .scalar()
        .expect("json");
    let rows = listing.sequence_rows().expect("an array");
    let row = rows.iter().next().expect("one child");
    let url = row
        .get_key_str("url")
        .expect("a url")
        .as_str()
        .expect("text")
        .to_owned();
    let body = Request::get(&url).expect("a URL").send().expect("send");
    assert_eq!(body.status(), Status::OK, "{url}");
    assert_eq!(body.text().expect("text"), "nested", "{url}");
}

#[test]
fn a_mounted_leaf_is_the_prefix_itself() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    let mut buffer = Holder::Buffer(Buffer::new());
    buffer.write_all_bytes(b"one leaf").expect("write");
    server.mount("/leaf", buffer).expect("mount");
    let response = get(&server, "/leaf");
    assert_eq!(response.status(), Status::OK);
    assert_eq!(&*response.bytes().expect("body"), b"one leaf");
    let (head, body) = raw(
        &server,
        &request_line("GET", "/leaf", "Range: bytes=4-\r\n"),
    );
    assert_eq!(head.status, Status::PARTIAL_CONTENT);
    assert_eq!(body, b"leaf");
    let response = Request::put(&server.url_of("/leaf").expect("url").to_string(), "two")
        .expect("request")
        .send()
        .expect("send");
    assert_eq!(response.status(), Status::NO_CONTENT);
    assert_eq!(&*get(&server, "/leaf").bytes().expect("body"), b"two");
}

#[test]
fn a_put_creates_then_replaces_and_declares_the_content_type() {
    let server = served();
    let url = server.url_of("/new.bin").expect("url").to_string();
    let response = Request::put(&url, "first")
        .expect("request")
        .with_header("content-type", "text/csv; charset=utf-8")
        .expect("header")
        .send()
        .expect("send");
    assert_eq!(response.status(), Status::CREATED);
    let response = Request::put(&url, "second")
        .expect("request")
        .send()
        .expect("send");
    assert_eq!(response.status(), Status::NO_CONTENT);
    let response = get(&server, "/new.bin");
    assert_eq!(&*response.bytes().expect("body"), b"second");
    assert_eq!(
        response.headers().get("content-type"),
        Some("text/csv; charset=utf-8"),
        "the declared type outlives the handle that wrote it"
    );
    assert_eq!(server.request_count(), 3);
}

#[test]
fn a_delete_removes_and_the_next_one_is_404() {
    let server = served();
    let url = server.url_of("/rows.json").expect("url").to_string();
    let response = Request::delete(&url)
        .expect("request")
        .send()
        .expect("send");
    assert_eq!(response.status(), Status::NO_CONTENT);
    let response = Request::delete(&url)
        .expect("request")
        .send()
        .expect("send");
    assert_eq!(response.status(), Status::NOT_FOUND);
    assert_eq!(get(&server, "/rows.json").status(), Status::NOT_FOUND);
    let statuses: Vec<u16> = server
        .requests()
        .iter()
        .map(|recorded| recorded.status.code())
        .collect();
    assert_eq!(statuses, [204, 404, 404]);
}

#[test]
fn options_names_the_five_methods() {
    let server = served();
    let (head, body) = raw(&server, &request_line("OPTIONS", "/rows.json", ""));
    assert_eq!(head.status, Status::NO_CONTENT);
    assert_eq!(
        head.headers.get("allow"),
        Some("GET, HEAD, PUT, DELETE, OPTIONS")
    );
    assert!(body.is_empty());
}

#[test]
fn the_etag_can_be_turned_off() {
    let server =
        Server::bind_with("127.0.0.1:0", ServerOptions::default().with_etag(false)).expect("bind");
    let root = memory_root();
    root.child_by_path("x.txt")
        .expect("child")
        .write_all_bytes(b"x")
        .expect("write");
    server.mount("/", root).expect("mount");
    let response = get(&server, "/x.txt");
    assert_eq!(response.headers().get("etag"), None);
    assert_eq!(response.headers().get("accept-ranges"), Some("bytes"));
}
