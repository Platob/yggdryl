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

use yggdryl_http::{HttpRequest, HttpResponseBatch, HttpSession};
use yggdryl_io::ReadBytes;

/// Reads one request off the stream, returning an optional `(start, end)` range.
fn read_range(stream: &mut std::net::TcpStream) -> Option<(u64, u64)> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(0) | Err(_) => return None,
            Ok(_) => buf.push(byte[0]),
        }
    }
    String::from_utf8_lossy(&buf)
        .lines()
        .find_map(|line| {
            line.strip_prefix("Range: ")
                .or_else(|| line.strip_prefix("range: "))
        })
        .and_then(|value| value.trim().strip_prefix("bytes="))
        .and_then(|spec| {
            let (start, end) = spec.split_once('-')?;
            Some((start.parse().ok()?, end.parse().ok()?))
        })
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
                let total = payload.len();
                let range = read_range(&mut stream);
                if latency_ms > 0 {
                    thread::sleep(std::time::Duration::from_millis(latency_ms));
                }
                match range {
                    Some((start, end)) => {
                        let end = (end as usize).min(total.saturating_sub(1));
                        let slice = &payload[start as usize..=end];
                        let header = format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{total}\r\nContent-Length: {}\r\n\r\n",
                            slice.len()
                        );
                        let _ = stream.write_all(header.as_bytes());
                        let _ = stream.write_all(slice);
                    }
                    None => {
                        let header = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nAccept-Ranges: bytes\r\n\r\n"
                        );
                        let _ = stream.write_all(header.as_bytes());
                        let _ = stream.write_all(&payload);
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

fn main() {
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
        let mut stream = session.stream(HttpRequest::get(&url).unwrap()).unwrap();
        let mut out = Vec::with_capacity(SIZE);
        stream.read_to_end(&mut out).unwrap();
        black_box(out);
    });

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
}
