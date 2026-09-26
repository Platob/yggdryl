//! `rust/src/http/pages.rs`: the walk over a paginated resource, one
//! response per page, and the Arrow reader laying every page out as one
//! batch under one root.
//!
//! Every mode the fixture speaks is walked to the end at one `GET` per page;
//! a failed page is yielded and resumed from its own request; a rate-limit
//! pause is observed between pages and bounded by `max_pause`.

use std::time::{Duration, Instant};

use arrow_array::RecordBatchReader;
use yggdryl::http::{HttpOptions, Pagination, Request, Session};
use yggdryl::{Error, FieldPath, Scalar};

use crate::http_server::RecordedExt as _;
use crate::http_server::{HttpServer, PageMode};

/// Three pages of two, two and one row.
fn pages() -> Vec<String> {
    vec![
        r#"[{"id":1,"name":"a"},{"id":2,"name":"b"}]"#.to_owned(),
        r#"[{"id":3,"name":"c"},{"id":4,"name":"d"}]"#.to_owned(),
        r#"[{"id":5,"name":"e"}]"#.to_owned(),
    ]
}

fn request(server: &HttpServer, path: &str) -> Request {
    Request::get(&server.url(path)).expect("a URL")
}

/// The `id` of every row on every page, in walk order.
fn ids(pages: yggdryl::http::Pages) -> Vec<Scalar> {
    let mut ids = Vec::new();
    for page in pages {
        let page = page.expect("a page");
        if !page.is_ok() {
            // A page counter runs off the end: the fixture's 404 is yielded
            // as the last page and holds no rows.
            continue;
        }
        let document = page.scalar().expect("a document");
        let rows = document
            .get_key_str("items")
            .and_then(Scalar::sequence_rows)
            .expect("rows");
        for row in rows.iter() {
            ids.push(row.get_key_str("id").cloned().expect("an id"));
        }
    }
    ids
}

// --- refusals ----------------------------------------------------------------

#[test]
fn a_page_whose_rows_the_first_pages_root_refuses_is_named() {
    let server = HttpServer::start();
    server.paginate(
        "/p",
        vec![
            r#"[{"id":1,"name":"a"}]"#.to_owned(),
            r#"[{"id":2,"name":"b"},{"id":"x","name":"c"}]"#.to_owned(),
        ],
        PageMode::Link,
    );
    let mut reader = request(&server, "/p")
        .pages()
        .into_arrow_reader(None, 0)
        .expect("a reader over the first page");
    let first = reader.next().expect("a batch").expect("the first page");
    assert_eq!(first.num_rows(), 1);
    let error = reader
        .next()
        .expect("a failure")
        .expect_err("a refused row");
    let text = error.to_string();
    assert!(text.contains("page=1"), "{text}");
    assert!(text.contains("row 1"), "{text}");
    assert!(reader.next().is_none());
    assert_eq!(server.request_count(), 2);
}

#[test]
fn an_empty_first_page_infers_no_root_without_a_field() {
    let server = HttpServer::start();
    server.paginate("/empty", vec!["[]".to_owned()], PageMode::Link);
    assert!(
        request(&server, "/empty")
            .pages()
            .into_arrow_reader(None, 0)
            .is_err()
    );
    let root = yggdryl::from_json_scalar(br#"[{"id":1}]"#)
        .expect("rows")
        .inferred_struct_field()
        .expect("a root");
    let mut reader = request(&server, "/empty")
        .pages()
        .into_arrow_reader(Some(&root), 0)
        .expect("a reader under the given root");
    assert_eq!(reader.schema().fields().len(), 1);
    assert!(reader.next().is_none(), "an empty page ends the walk");
    assert_eq!(server.request_count(), 2);
}

#[test]
fn a_page_that_is_no_document_is_named() {
    let server = HttpServer::start();
    server.put_resource("/text", b"not a document", Some("text/plain"));
    assert!(matches!(
        request(&server, "/text").pages().into_arrow_reader(None, 0),
        Err(Error::Codec { .. })
    ));
    server.put_resource("/nowhere", br#"{"meta":{"n":1}}"#, Some("application/json"));
    let error = match request(&server, "/nowhere")
        .pages()
        .into_arrow_reader(None, 0)
    {
        Ok(_) => panic!("expected no rows"),
        Err(error) => error,
    };
    match error {
        Error::Codec { format, reason, .. } => {
            assert_eq!(format, "http page");
            assert!(reason.contains("/nowhere"), "{reason}");
        }
        other => panic!("expected a page refusal, got {other:?}"),
    }
}

#[test]
fn a_failed_page_is_yielded_and_the_walk_resumes_from_that_page() {
    let server = HttpServer::start();
    server.paginate("/p", pages(), PageMode::Link);
    let mut walk = request(&server, "/p").pages();
    let first = walk.first_page().expect("a page").expect("the first page");
    assert_eq!(first.status().code(), 200);
    // The next three answers are refused before any byte: the client's three
    // attempts fail, and the failure is the caller's.
    server.fail_after("/p", 3);
    assert!(matches!(walk.next(), Some(Err(Error::Io(_)))));
    assert_eq!(server.request_count(), 4);
    // Asked again, the walk resumes from that page's own request.
    let second = walk.next().expect("a page").expect("the second page");
    assert_eq!(server.requests()[4].query("page"), Some("1"));
    assert_eq!(
        second
            .scalar()
            .expect("json")
            .path("items")
            .map(|items| items.len()),
        Some(2)
    );
    let third = walk.next().expect("a page").expect("the third page");
    assert_eq!(
        third
            .scalar()
            .expect("json")
            .path("items")
            .map(|items| items.len()),
        Some(1)
    );
    assert!(walk.next().is_none());
    assert_eq!(server.request_count(), 6);
}

// --- walking -----------------------------------------------------------------

#[test]
fn every_pagination_mode_walks_to_the_end_with_one_get_per_page() {
    let server = HttpServer::start();
    for (mode, pagination) in [
        (PageMode::Link, Pagination::Auto),
        (PageMode::Cursor, Pagination::Auto),
        (PageMode::Url, Pagination::Auto),
        (
            PageMode::Offset,
            Pagination::Offset {
                parameter: "offset".into(),
                page_size: 2,
                total: Some(FieldPath::from_str("total").expect("a path")),
            },
        ),
        (PageMode::Link, Pagination::Link),
        (
            PageMode::Cursor,
            Pagination::Cursor {
                path: FieldPath::from_str("next_cursor").expect("a path"),
                parameter: "cursor".into(),
            },
        ),
        (
            PageMode::Url,
            Pagination::Url(FieldPath::from_str("next").expect("a path")),
        ),
        (
            PageMode::Link,
            Pagination::Page {
                parameter: "page".into(),
                start: 0,
            },
        ),
    ] {
        server.clear_requests();
        server.paginate("/p", pages(), mode);
        let walk = request(&server, "/p")
            .with_pagination(pagination.clone())
            .pages();
        let walked = ids(walk);
        let expected_pages = if matches!(pagination, Pagination::Page { .. }) {
            // A page counter ends at an empty page, which the fixture
            // answers as a 404 the walk yields and stops at.
            4
        } else {
            3
        };
        assert_eq!(
            server.request_count(),
            expected_pages,
            "{mode:?} under {pagination}: one GET per page"
        );
        assert_eq!(
            walked,
            [1, 2, 3, 4, 5].map(Scalar::from),
            "{mode:?} under {pagination}"
        );
    }
    // The offset walk asks for each page by its offset.
    server.clear_requests();
    server.paginate("/p", pages(), PageMode::Offset);
    let walk = request(&server, "/p")
        .with_pagination(Pagination::Offset {
            parameter: "offset".into(),
            page_size: 2,
            total: None,
        })
        .pages();
    assert_eq!(ids(walk), [1, 2, 3, 4, 5].map(Scalar::from));
    let offsets: Vec<Option<String>> = server
        .requests()
        .iter()
        .map(|recorded| recorded.query("offset").map(str::to_owned))
        .collect();
    assert_eq!(offsets, [None, Some("2".to_owned()), Some("4".to_owned())]);
}

#[test]
fn pagination_none_and_the_page_limit_stop_the_walk() {
    let server = HttpServer::start();
    server.paginate("/p", pages(), PageMode::Link);
    let one = request(&server, "/p")
        .with_pagination(Pagination::None)
        .pages();
    assert_eq!(ids(one), [1, 2].map(Scalar::from));
    assert_eq!(server.request_count(), 1);

    server.clear_requests();
    let session =
        Session::with_options(HttpOptions::default().with_page_limit(Some(2))).expect("a session");
    let two = session.get(&server.url("/p")).expect("a URL").pages();
    assert_eq!(ids(two), [1, 2, 3, 4].map(Scalar::from));
    assert_eq!(server.request_count(), 2);
}

#[test]
fn a_next_url_already_visited_ends_the_walk() {
    let server = HttpServer::start();
    let back = server.url("/p?page=0");
    let forward = server.url("/p?page=1");
    server.paginate(
        "/p",
        vec![
            format!(r#"[{{"id":1,"next":"{forward}"}}]"#),
            format!(r#"[{{"id":2,"next":"{back}"}}]"#),
        ],
        PageMode::Link,
    );
    let walk = request(&server, "/p?page=0")
        .with_pagination(Pagination::Url(
            FieldPath::from_str("items[0].next").expect("a path"),
        ))
        .pages();
    assert_eq!(
        ids(walk),
        [1, 2].map(Scalar::from),
        "the loop back to the first page is not followed"
    );
    assert_eq!(server.request_count(), 2);
}

#[test]
fn a_rate_limit_pause_is_observed_between_pages_up_to_max_pause() {
    let server = HttpServer::start();
    server.paginate("/p", pages(), PageMode::Link);
    server.rate_limit("/p", 0, 1);
    let started = Instant::now();
    let walked = ids(request(&server, "/p").pages());
    let elapsed = started.elapsed();
    assert_eq!(walked, [1, 2, 3, 4, 5].map(Scalar::from));
    assert!(elapsed >= Duration::from_secs(1), "paused {elapsed:?}");
    assert!(elapsed < Duration::from_secs(10), "paused {elapsed:?}");
    assert_eq!(server.request_count(), 3);

    // The same pause under a shorter bound is cut to it.
    server.clear_requests();
    server.rate_limit("/p", 0, 1);
    let session =
        Session::with_options(HttpOptions::default().with_max_pause(Duration::from_millis(50)))
            .expect("a session");
    let started = Instant::now();
    let walked = ids(session.get(&server.url("/p")).expect("a URL").pages());
    assert_eq!(walked, [1, 2, 3, 4, 5].map(Scalar::from));
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(server.request_count(), 3);
}

// --- Arrow -------------------------------------------------------------------

#[test]
fn the_arrow_reader_answers_one_batch_per_page_under_one_schema() {
    let server = HttpServer::start();
    server.paginate("/p", pages(), PageMode::Link);
    let reader = request(&server, "/p")
        .pages()
        .into_arrow_reader(None, 0)
        .expect("a reader");
    let schema = reader.schema();
    let names: Vec<&str> = schema
        .fields()
        .iter()
        .map(|field| field.name().as_str())
        .collect();
    assert_eq!(names, ["id", "name"]);
    let batches: Vec<_> = reader.map(|batch| batch.expect("a batch")).collect();
    assert_eq!(
        batches
            .iter()
            .map(|batch| batch.num_rows())
            .collect::<Vec<_>>(),
        [2, 2, 1],
        "one batch per page"
    );
    assert!(batches.iter().all(|batch| batch.schema() == schema));
    assert_eq!(server.request_count(), 3);

    // A row bound splits a page into several batches, each still one page's.
    server.clear_requests();
    let reader = request(&server, "/p")
        .pages()
        .into_arrow_reader(None, 1)
        .expect("a reader");
    let rows: Vec<usize> = reader
        .map(|batch| batch.expect("a batch").num_rows())
        .collect();
    assert_eq!(rows, [1, 1, 1, 1, 1]);
    assert_eq!(server.request_count(), 3);

    // A declared root is laid out as given.
    server.clear_requests();
    let root = yggdryl::from_json_scalar(br#"[{"id":1,"name":"a"}]"#)
        .expect("rows")
        .inferred_struct_field()
        .expect("a root");
    let reader = request(&server, "/p")
        .pages()
        .into_arrow_reader(Some(&root), 0)
        .expect("a reader");
    assert_eq!(reader.schema(), schema);
    assert_eq!(reader.count(), 3);
    assert_eq!(server.request_count(), 3);
}

#[test]
fn the_serie_reader_yields_one_record_column_per_page() {
    let server = HttpServer::start();
    server.paginate("/p", pages(), PageMode::Cursor);
    let reader = request(&server, "/p")
        .pages()
        .into_serie_reader(None)
        .expect("a serie reader");
    let mut lengths = Vec::new();
    let mut first_id = None;
    for column in reader {
        let column = column.expect("a column");
        lengths.push(column.len());
        if first_id.is_none() {
            first_id = column
                .scalar(0)
                .ok()
                .and_then(|row| row.get(0).map(|id| id.into_owned()));
        }
    }
    assert_eq!(lengths, [2, 2, 1]);
    assert_eq!(first_id, Some(Scalar::from(1_i64)));
    assert_eq!(server.request_count(), 3);
}
