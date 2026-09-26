//! HTTP access, against the crate's own server on loopback.
//!
//! The six shapes a caller of an HTTP resource performs: the whole value,
//! one range out of the end (a footer read), a full streamed drain, one
//! small JSON exchange, a walk of a paginated API, and a fan-out of many
//! small requests over `send_all`. Each is a stated number of requests - one
//! `GET`, one ranged `GET`, one `GET`, one `POST`, one `GET` per page, one
//! `GET` per request - which the accounting tests hold; what is measured
//! here is everything around those round trips, so a per-request cost that
//! crept in shows beside the counts. Nothing leaves the machine: the server
//! is `yggdryl::http::Server` bound on `127.0.0.1:0`, serving a memory
//! folder and fixed answers.

use std::hint::black_box;
use std::io::Read as _;
use std::sync::Arc;

use criterion::{BenchmarkId, Criterion, Throughput};
use yggdryl::fs::{FsFolder, MemoryFileSystem};
use yggdryl::holder::Holder;
use yggdryl::http::{Method, Response, Server, Session, Status};
use yggdryl::{IOBase, Scalar};

/// The resource the byte shapes read, in bytes.
const PAYLOAD: usize = crate::bench_profile::corpus(4 * 1024 * 1024, 64 * 1024);

/// The range a footer read asks for, in bytes.
const FOOTER: usize = 8 * 1024;

/// The pages the paginated walk reads.
const PAGES: usize = crate::bench_profile::corpus(64, 4);

/// The rows one page holds.
const ROWS_PER_PAGE: usize = crate::bench_profile::corpus(100, 4);

/// The requests one `send_all` fan-out sends, and the threads it sends on.
const FAN_OUT: usize = crate::bench_profile::corpus(256, 8);
const FAN_OUT_THREADS: usize = 8;

/// Deterministic bytes that no coding shortens by accident.
fn payload(length: usize) -> Vec<u8> {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state.to_le_bytes()[0]
        })
        .collect()
}

/// A server with a memory folder mounted at `/` holding `data.bin`, a fixed
/// JSON answer to `POST /orders`, and `PAGES` pages under `/pages/<n>`,
/// each naming the next in its `next` member, the last naming none.
fn serve() -> Server {
    let server = Server::bind("127.0.0.1:0").expect("a loopback server");
    let folder = Holder::from(
        FsFolder::from_path(Arc::new(MemoryFileSystem::new()), "", None).expect("a memory root"),
    );
    folder
        .child_by_path("data.bin")
        .expect("a child")
        .write_all_bytes(&payload(PAYLOAD))
        .expect("the payload");
    server.mount("/", folder).expect("the mount");

    let accepted = yggdryl::json::from_utf8(r#"{"id":42,"state":"accepted"}"#).expect("a document");
    server.respond(
        Some(Method::Post),
        "/orders",
        Response::new(Status::CREATED)
            .with_json(&accepted)
            .expect("a JSON answer"),
    );

    for page in 0..PAGES {
        let rows = (0..ROWS_PER_PAGE)
            .map(|row| format!(r#"{{"id":{},"symbol":"MSFT"}}"#, page * ROWS_PER_PAGE + row))
            .collect::<Vec<_>>()
            .join(",");
        let next = if page + 1 < PAGES {
            format!(r#","next":"/pages/{}""#, page + 1)
        } else {
            String::new()
        };
        server.respond(
            Some(Method::Get),
            &format!("/pages/{page}"),
            Response::new(Status::OK)
                .with_header("content-type", "application/json")
                .expect("a content type")
                .with_body(format!(r#"{{"items":[{rows}]{next}}}"#)),
        );
    }
    server.set_recording(false);
    server
}

pub(crate) fn http_benchmarks(criterion: &mut Criterion) {
    let server = serve();
    let session = Session::new();
    let url = |path: &str| server.url_of(path).expect("a URL").to_string();
    let resource = session.get(&url("/data.bin")).expect("a request");

    let mut group = criterion.benchmark_group("http_bytes");
    group.throughput(Throughput::Bytes(PAYLOAD as u64));

    // A whole read: one GET, so this is the transfer and the framing around
    // it.
    group.bench_function("read_all", |bencher| {
        bencher.iter(|| black_box(black_box(&resource).read_all_bytes().expect("the body")));
    });

    // A footer read: one ranged GET, the range the transfer and not the
    // resource.
    group.throughput(Throughput::Bytes(FOOTER as u64));
    group.bench_function("read_footer", |bencher| {
        let offset = (PAYLOAD - FOOTER) as u64;
        bencher.iter(|| {
            black_box(
                black_box(&resource)
                    .read_range_bytes(offset, FOOTER)
                    .expect("the footer"),
            )
        });
    });

    // A streamed drain, which is what a record reader does: one GET,
    // consumed in bounded pieces through the resumable stream.
    group.throughput(Throughput::Bytes(PAYLOAD as u64));
    group.bench_function("stream_drain", |bencher| {
        bencher.iter(|| {
            let mut stream = black_box(&resource)
                .pstream_bytes(0, 64 * 1024)
                .expect("a stream");
            let mut window = vec![0_u8; 64 * 1024];
            let mut total = 0_usize;
            loop {
                let read = stream.read(&mut window).expect("a chunk");
                if read == 0 {
                    break;
                }
                total += read;
            }
            assert_eq!(total, PAYLOAD);
            black_box(total)
        });
    });
    group.finish();

    let mut group = criterion.benchmark_group("http_session");

    // One small JSON exchange: the body rendered, one POST, the answer read
    // whole and parsed under its media type.
    let order = yggdryl::json::from_utf8(r#"{"symbol":"MSFT","quantity":100,"side":"buy"}"#)
        .expect("a document");
    let orders = url("/orders");
    group.bench_function("send_json", |bencher| {
        bencher.iter(|| {
            let response = session
                .request(Method::Post, &orders)
                .expect("a request")
                .with_json(black_box(&order))
                .expect("a JSON body")
                .send()
                .expect("an answer");
            assert_eq!(response.status(), Status::CREATED);
            black_box(response.scalar().expect("a document"))
        });
    });

    // A walk of a paginated API: one GET per page, each next page found by
    // the automatic ladder in the page's own document.
    let first = url("/pages/0");
    group.throughput(Throughput::Elements(PAGES as u64));
    group.bench_with_input(
        BenchmarkId::new("pages", PAGES),
        &first,
        |bencher, first| {
            bencher.iter(|| {
                let walked = session
                    .get(first)
                    .expect("a request")
                    .pages()
                    .map(|page| page.expect("a page").scalar().expect("a document"))
                    .collect::<Vec<Scalar>>();
                assert_eq!(walked.len(), PAGES);
                black_box(walked)
            });
        },
    );

    // Many small requests on a thread pool, answered in order: what a
    // parallel walk to one host costs per request once its connections are
    // pooled.
    let quote = url("/pages/0");
    group.throughput(Throughput::Elements(FAN_OUT as u64));
    group.bench_with_input(
        BenchmarkId::new("send_all", FAN_OUT),
        &quote,
        |bencher, quote| {
            bencher.iter(|| {
                let requests = (0..FAN_OUT)
                    .map(|_| session.get(quote).expect("a request"))
                    .collect::<Vec<_>>();
                let answered = session
                    .send_all(requests, Some(FAN_OUT_THREADS))
                    .map(|answer| answer.expect("an answer").bytes().expect("a body").len())
                    .sum::<usize>();
                black_box(answered)
            });
        },
    );
    group.finish();
}
