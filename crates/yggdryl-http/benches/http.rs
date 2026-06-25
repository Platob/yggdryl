//! Throughput benchmarks for the HTTP client against an in-process server.
//!
//! Run with `cargo bench -p yggdryl-http`. A plain `main` (the crate sets
//! `harness = false`) drives a localhost server, so there is no real network and
//! no benchmark-framework dependency. It compares the windowed [`HttpStream`]
//! against a one-shot download, and concurrent `send_many` against a sequential
//! request loop.

use std::hint::black_box;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Instant;

use yggdryl_core::{Io, Url};
use yggdryl_http::{HttpHeaders, HttpRequest, HttpResponseBatch, HttpSession, HttpVersion};

/// Reads one request off the stream, returning `(is_head, optional range)`.
fn read_request(stream: &mut std::net::TcpStream) -> Option<(bool, Option<(u64, u64)>)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 256];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return None,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let text = String::from_utf8_lossy(&buf);
    let is_head = text.starts_with("HEAD ");
    let range = text
        .lines()
        .find_map(|line| {
            line.strip_prefix("Range: ")
                .or_else(|| line.strip_prefix("range: "))
        })
        .and_then(|value| value.trim().strip_prefix("bytes="))
        .and_then(|spec| {
            let (start, end) = spec.split_once('-')?;
            Some((start.parse().ok()?, end.parse().ok()?))
        });
    Some((is_head, range))
}

/// Serves `payload` forever with HEAD + `Range` (206) support, handling each
/// connection on its own thread (so concurrent clients are served concurrently)
/// and sleeping `latency_ms` before replying to simulate network latency.
fn serve(payload: Vec<u8>, latency_ms: u64) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let payload = std::sync::Arc::new(payload);
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let payload = payload.clone();
            thread::spawn(move || {
                let mut stream = stream;
                let _ = stream.set_nodelay(true); // avoid Nagle/delayed-ACK stalls
                let total = payload.len();
                // Keep serving requests on this connection (HTTP/1.1 keep-alive) so
                // the pooled-connection-reuse path is exercised.
                while let Some((is_head, range)) = read_request(&mut stream) {
                    if latency_ms > 0 {
                        thread::sleep(std::time::Duration::from_millis(latency_ms));
                    }
                    let wrote = if is_head {
                        // HEAD: headers only, no body.
                        let header = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nAccept-Ranges: bytes\r\n\r\n"
                        );
                        stream.write_all(header.as_bytes())
                    } else if let Some((start, end)) = range {
                        let end = (end as usize).min(total.saturating_sub(1));
                        let slice = &payload[start as usize..=end];
                        let header = format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{total}\r\nContent-Length: {}\r\n\r\n",
                            slice.len()
                        );
                        stream
                            .write_all(header.as_bytes())
                            .and_then(|()| stream.write_all(slice))
                    } else {
                        let header = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nAccept-Ranges: bytes\r\n\r\n"
                        );
                        stream
                            .write_all(header.as_bytes())
                            .and_then(|()| stream.write_all(&payload))
                    };
                    if wrote.is_err() {
                        break;
                    }
                }
            });
        }
    });
    url
}

fn bench(name: &str, iters: u64, bytes: usize, mut f: impl FnMut()) {
    for _ in 0..iters / 5 + 1 {
        f();
    }
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    let secs = start.elapsed().as_secs_f64();
    let mib = (bytes as f64 * iters as f64) / (1024.0 * 1024.0);
    println!(
        "{name:<40} {:>8.1} MiB/s  ({:>7.2} ms/iter)",
        mib / secs,
        secs / iters as f64 * 1e3
    );
}

/// Times `f` over `iters` iterations (after a warm-up) and prints ns/iter, for the
/// CPU-only paths (URL resolve, header parsing) that move no payload.
fn bench_ns(name: &str, iters: u64, mut f: impl FnMut()) {
    for _ in 0..iters / 10 + 1 {
        f();
    }
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    let per = start.elapsed().as_nanos() as f64 / iters as f64;
    println!("{name:<40} {per:>9.1} ns/iter");
}

fn main() {
    cpu_paths();

    const SIZE: usize = 8 * 1024 * 1024; // 8 MiB, so HttpStream uses two 4 MiB windows
    let payload: Vec<u8> = (0..SIZE).map(|i| (i % 251) as u8).collect();
    let url = serve(payload.clone(), 0);
    let session = HttpSession::new();

    println!("== download ({} MiB) ==", SIZE / 1024 / 1024);
    bench("one-shot GET into_bytesio", 20, SIZE, || {
        let handle = session.get(&url).unwrap().into_bytesio().unwrap();
        black_box(handle);
    });
    bench("HttpStream windowed read_to_end", 20, SIZE, || {
        let mut stream = session
            .send(HttpRequest::get(&url).unwrap(), false, true, true)
            .unwrap()
            .into_io();
        let mut out = Vec::with_capacity(SIZE);
        stream.read_to_end(&mut out).unwrap();
        black_box(out);
    });

    // HttpStream's strength: random access. Read a 16-byte "footer" with a single
    // Range request instead of downloading the whole resource.
    println!(
        "\n== random access (16-byte footer of {} MiB) ==",
        SIZE / 1024 / 1024
    );
    let footer_reqs = 200u64;
    let start = Instant::now();
    for _ in 0..footer_reqs {
        let mut stream = session
            .send(HttpRequest::get(&url).unwrap(), false, true, true)
            .unwrap()
            .into_io();
        let mut footer = [0u8; 16];
        stream
            .pread(&mut footer, -16, yggdryl_core::Whence::End)
            .unwrap();
        black_box(footer);
    }
    println!(
        "HttpStream pread footer                  {:>7.2} ms/read (one Range request, no full download)",
        start.elapsed().as_secs_f64() / footer_reqs as f64 * 1e3
    );

    // Many small requests with 5 ms of simulated latency each: concurrent
    // send_many should beat the sequential loop roughly by the concurrency factor.
    let small = serve(b"small-response-body".to_vec(), 5);
    const N: usize = 64;
    println!("\n== {N} small requests, 5ms latency each ==");
    bench("sequential request loop", 4, N, || {
        for _ in 0..N {
            black_box(session.get(&small).unwrap());
        }
    });
    bench("send_many (concurrency 8)", 4, N, || {
        let requests: Vec<HttpRequest> =
            (0..N).map(|_| HttpRequest::get(&small).unwrap()).collect();
        let batches: Vec<HttpResponseBatch> = session.send_many(requests).collect();
        black_box(batches);
    });

    // Connection pooling: a keep-alive session reuses one warm connection across
    // requests, while keep_alive=false reconnects (a fresh TCP/TLS setup) each time.
    let tiny = serve(b"x".to_vec(), 0);
    const M: usize = 200;
    println!("\n== {M} tiny requests: pooled keep-alive vs reconnect-each ==");
    bench("keep_alive=true  (pooled reuse)", 6, M, || {
        for _ in 0..M {
            let response = session
                .send(HttpRequest::get(&tiny).unwrap(), false, true, true)
                .unwrap();
            black_box(response.bytes().unwrap());
        }
    });
    bench("keep_alive=false (reconnect each)", 6, M, || {
        for _ in 0..M {
            let response = session
                .send(HttpRequest::get(&tiny).unwrap(), false, false, true)
                .unwrap();
            black_box(response.bytes().unwrap());
        }
    });
}

/// CPU-only paths that need no server: base-URL resolution (the RFC 3986
/// relative-reference join the redirect layer also runs) and header parsing.
fn cpu_paths() {
    println!("== url resolve / header parsing (CPU only) ==");
    let based = HttpSession::new()
        .with_base_url(Url::from_str("https://api.example.com/v1/users/").unwrap());
    let n = 2_000_000;
    bench_ns("resolve_url (relative segment)", n, || {
        black_box(based.resolve_url(black_box("profile")).unwrap());
    });
    bench_ns("resolve_url (dot-segments ../)", n, || {
        black_box(based.resolve_url(black_box("../../v2/orders")).unwrap());
    });
    bench_ns("resolve_url (absolute, no join)", n, || {
        black_box(
            based
                .resolve_url(black_box("https://other.example.com/x"))
                .unwrap(),
        );
    });

    // Header parsing: content_size prefers the Content-Range total over
    // Content-Length; retry_after parses both the delta-seconds and HTTP-date forms.
    let range_headers = HttpHeaders::from_mapping([
        (
            "Content-Range".to_string(),
            "bytes 0-1023/8388608".to_string(),
        ),
        ("Content-Length".to_string(), "1024".to_string()),
    ]);
    bench_ns("content_size (Content-Range total)", n, || {
        black_box(black_box(&range_headers).content_size());
    });
    let retry_secs = HttpHeaders::from_mapping([("Retry-After".to_string(), "120".to_string())]);
    bench_ns("retry_after (delta seconds)", n, || {
        black_box(black_box(&retry_secs).retry_after());
    });
    let retry_date = HttpHeaders::from_mapping([(
        "Retry-After".to_string(),
        "Wed, 21 Oct 2026 07:28:00 GMT".to_string(),
    )]);
    bench_ns("retry_after (HTTP-date)", n, || {
        black_box(black_box(&retry_date).retry_after());
    });

    // Protocol-version selection, on the per-request path (parsed from a string in
    // the bindings, matched on every dispatch to choose the transport).
    bench_ns("HttpVersion::from_str (h2)", n, || {
        black_box(HttpVersion::from_str(black_box("h2")).unwrap());
    });
    bench_ns("HttpVersion::from_alpn (h3)", n, || {
        black_box(HttpVersion::from_alpn(black_box("h3")));
    });
    println!();
}
