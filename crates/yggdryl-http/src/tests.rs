//! Hermetic tests: a localhost `TcpListener` serves HEAD / `Range` / 429 /
//! mid-stream drops, so nothing here touches the network.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use yggdryl_core::{BytesIO, Io, Whence};

use crate::{HttpError, HttpRequest, HttpResponseBatch, HttpSession, Method, RetryConfig};

/// Opens a streamed body over a resource (the old `session.stream` helper): sends
/// the request without raising and returns the live body as a [`Box<dyn Io>`].
fn open_stream(session: &HttpSession, request: HttpRequest, keep_alive: bool) -> Box<dyn Io> {
    session
        .send(request, false, keep_alive, true)
        .unwrap()
        .into_io()
}

/// Spawns a one-shot localhost HTTP/1.1 server that replies with `reply` and
/// hands back the raw request line/headers it received. Hermetic — no network.
fn serve_once(reply: Vec<u8>) -> (String, std::sync::mpsc::Receiver<Vec<u8>>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            // Drain the whole request (headers + any chunked/fixed body) using a
            // short read timeout, so a multi-packet request is captured before we
            // reply — and so the client finishes writing before the socket closes.
            stream
                .set_read_timeout(Some(std::time::Duration::from_millis(150)))
                .ok();
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                match stream.read(&mut buf) {
                    Ok(0) => break,
                    Ok(count) => request.extend_from_slice(&buf[..count]),
                    Err(_) => break, // timed out: assume the request is complete
                }
            }
            tx.send(request).ok();
            let _ = stream.write_all(&reply);
            let _ = stream.flush();
        }
    });
    (url, rx)
}

fn ok_reply(content_type: &str, body: &[u8]) -> Vec<u8> {
    let mut reply = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    reply.extend_from_slice(body);
    reply
}

#[test]
fn method_parses_and_names() {
    assert_eq!(Method::from_str("get").unwrap(), Method::Get);
    assert_eq!(Method::from_str(" Post ").unwrap(), Method::Post);
    assert_eq!(Method::Delete.as_str(), "DELETE");
    assert!(Method::from_str("teleport").is_err());
}

#[test]
fn io_factory_opens_an_http_url() {
    let (url, _rx) = serve_once(ok_reply("text/plain", b"factory body"));
    // Building a session registers http/https with the yggdryl-io factory.
    let _session = HttpSession::new();
    let mut body = yggdryl_core::from_str(&url).unwrap();
    let mut out = Vec::new();
    body.read_to_end(&mut out).unwrap();
    assert_eq!(out, b"factory body");
}

#[test]
fn get_reads_status_headers_and_text() {
    let (url, _rx) = serve_once(ok_reply("text/plain", b"hello world"));
    let session = HttpSession::new().with_user_agent("yggdryl-http-test");
    let response = session.get(&url).unwrap();
    assert_eq!(response.status(), 200);
    assert!(response.ok());
    assert_eq!(response.content_type(), Some("text/plain"));
    assert_eq!(response.content_length(), Some(11));
    assert_eq!(response.text().unwrap(), "hello world");
}

#[test]
fn post_sends_method_headers_and_body() {
    let (url, rx) = serve_once(ok_reply("application/json", b"{}"));
    let session = HttpSession::new();
    let response = session
        .request(
            HttpRequest::post(&url)
                .unwrap()
                .with_header("x-custom", "42")
                .with_body(b"the-body".to_vec()),
            false,
        )
        .unwrap();
    assert_eq!(response.status(), 200);

    let request = String::from_utf8(rx.recv().unwrap()).unwrap();
    assert!(request.starts_with("POST / HTTP/1.1"), "{request}");
    assert!(request.to_lowercase().contains("x-custom: 42"), "{request}");
    assert!(request.ends_with("the-body"), "{request}");
}

#[test]
fn request_headers_override_session_defaults() {
    let (url, rx) = serve_once(ok_reply("text/plain", b"ok"));
    let session = HttpSession::new().with_header("x-tag", "session");
    session
        .request(
            HttpRequest::get(&url)
                .unwrap()
                .with_header("x-tag", "request"),
            false,
        )
        .unwrap();
    let request = String::from_utf8(rx.recv().unwrap())
        .unwrap()
        .to_lowercase();
    assert!(request.contains("x-tag: request"), "{request}");
    assert!(!request.contains("x-tag: session"), "{request}");
}

#[test]
fn streamed_request_body_from_an_io_handle() {
    let (url, rx) = serve_once(ok_reply("text/plain", b"ok"));
    let session = HttpSession::new();
    // Upload straight from a BytesIO handle, never buffering a Vec.
    let upload = BytesIO::from_bytes(b"streamed-upload-payload".to_vec());
    session
        .request(
            HttpRequest::put(&url).unwrap().with_body_reader(upload),
            false,
        )
        .unwrap();
    let request = String::from_utf8(rx.recv().unwrap()).unwrap();
    assert!(request.starts_with("PUT / HTTP/1.1"), "{request}");
    assert!(request.contains("streamed-upload-payload"), "{request}");
}

#[test]
fn io_body_sets_content_length_and_streams() {
    let (url, rx) = serve_once(ok_reply("text/plain", b"ok"));
    let session = HttpSession::new();
    // An Io body knows its length, so the request is framed with Content-Length.
    let upload = BytesIO::from_bytes(b"io-streamed-body".to_vec());
    session
        .request(HttpRequest::put(&url).unwrap().with_body_io(upload), false)
        .unwrap();
    let request = String::from_utf8(rx.recv().unwrap())
        .unwrap()
        .to_lowercase();
    assert!(request.contains("content-length: 16"), "{request}");
    assert!(request.contains("io-streamed-body"), "{request}");
    assert!(!request.contains("transfer-encoding: chunked"), "{request}");
}

#[test]
fn response_body_streams_into_a_bytesio() {
    let (url, _rx) = serve_once(ok_reply("application/octet-stream", &vec![7u8; 5000]));
    let session = HttpSession::new();
    let handle = session.get(&url).unwrap().into_bytesio().unwrap();
    assert_eq!(handle.len(), 5000);
}

#[test]
fn raise_for_status_flags_errors() {
    let reply =
        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec();
    let (url, _rx) = serve_once(reply);
    let session = HttpSession::new();
    // raise_error = false returns the 404 response instead of erroring.
    let response = session
        .request(HttpRequest::get(&url).unwrap(), false)
        .unwrap();
    assert_eq!(response.status(), 404);
    assert!(!response.ok());
    assert!(matches!(
        response.raise_for_status(),
        Err(HttpError::Status(404))
    ));
}

#[test]
fn get_raises_on_error_status_by_default() {
    // A 404 is not retried, so a single-shot server suffices; get() raises.
    let reply =
        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec();
    let (url, _rx) = serve_once(reply);
    assert!(matches!(
        HttpSession::new().get(&url),
        Err(HttpError::Status(404))
    ));
}

#[test]
fn retries_a_500_once_then_surfaces_it() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    let hits = Arc::new(AtomicU32::new(0));
    let counter = hits.clone();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let _ = read_request(&mut stream);
            counter.fetch_add(1, Ordering::SeqCst);
            let _ = stream.write_all(
                b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    let session = HttpSession::new().with_retry(RetryConfig {
        max_retries: 3,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(2),
    });
    // A 500 is retried exactly once (not the full max_retries), then surfaced.
    assert!(matches!(session.get(&url), Err(HttpError::Status(500))));
    assert_eq!(hits.load(Ordering::SeqCst), 2); // initial + one retry
}

#[test]
fn buffered_send_drains_the_body_and_releases_the_connection() {
    // stream = false drains the body into a BytesIO during send, so the connection
    // is released immediately and the same accessors expose the buffered body.
    let url = serve_ranges(stream_payload());
    let session = HttpSession::new();
    let response = session
        .send(HttpRequest::get(&url).unwrap(), false, true, false)
        .unwrap();
    assert_eq!(session.open_streams(), 0); // released during send, not held
    assert_eq!(response.status(), 200);
    let bytes = response.bytes().unwrap();
    assert_eq!(bytes, stream_payload());
}

#[cfg(feature = "compression")]
#[test]
fn gzip_response_is_decoded_transparently() {
    let body = b"this body was gzip-encoded on the wire".to_vec();
    let packed = yggdryl_core::Compression::Gzip.compress(&body).unwrap();
    let mut reply = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        packed.len()
    )
    .into_bytes();
    reply.extend_from_slice(&packed);

    let (url, _rx) = serve_once(reply);
    let response = HttpSession::new().get(&url).unwrap();
    assert_eq!(response.content_encoding(), Some("gzip"));
    assert_eq!(response.text().unwrap(), String::from_utf8(body).unwrap());
}

#[cfg(feature = "media")]
#[test]
fn response_mime_type_from_content_type() {
    let (url, _rx) = serve_once(ok_reply("application/json", b"{}"));
    let response = HttpSession::new().get(&url).unwrap();
    assert_eq!(
        response.mime_type(),
        Some(yggdryl_core::MimeType::from_str("application/json").unwrap())
    );
}

// --- multi-request server for the HttpStream / retry / send_many tests ---

/// Reads one HTTP request off `stream`, returning `(method, path, range)`.
#[allow(clippy::type_complexity)]
fn read_request(stream: &mut std::net::TcpStream) -> Option<(String, String, Option<(u64, u64)>)> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(0) | Err(_) => return None,
            Ok(_) => buf.push(byte[0]),
        }
    }
    let text = String::from_utf8_lossy(&buf);
    let mut lines = text.lines();
    let first = lines.next()?;
    let mut parts = first.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let range = text
        .lines()
        .find_map(|line| {
            line.strip_prefix("Range: ")
                .or_else(|| line.strip_prefix("range: "))
        })
        .and_then(|value| value.trim().strip_prefix("bytes="))
        .and_then(|spec| {
            let (start, end) = spec.split_once('-')?;
            // An open-ended range (`bytes=5000-`) has an empty end: treat it as
            // "to the end" (the server clamps to the payload length).
            let end = if end.is_empty() {
                u64::MAX
            } else {
                end.parse().ok()?
            };
            Some((start.parse().ok()?, end))
        });
    Some((method, path, range))
}

/// A looping server that serves `payload` with HEAD + `Range` (206) support.
/// Each connection runs on its own thread and serves **multiple** requests
/// (HTTP/1.1 keep-alive), so it exercises the pooled-connection reuse path.
fn serve_ranges(payload: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let payload = std::sync::Arc::new(payload);
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let payload = payload.clone();
            thread::spawn(move || {
                let mut stream = stream;
                let _ = stream.set_nodelay(true);
                let total = payload.len();
                // Keep serving requests on this connection until the peer
                // closes it or asks to (`Connection: close`).
                while let Some((method, _path, range)) = read_request(&mut stream) {
                    if method == "HEAD" {
                        let head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {total}\r\nAccept-Ranges: bytes\r\n\r\n"
                        );
                        if stream.write_all(head.as_bytes()).is_err() {
                            break;
                        }
                    } else if let Some((start, end)) = range {
                        let end = (end as usize).min(total.saturating_sub(1));
                        let slice = &payload[start as usize..=end];
                        let header = format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{total}\r\nContent-Length: {}\r\n\r\n",
                            slice.len()
                        );
                        if stream.write_all(header.as_bytes()).is_err()
                            || stream.write_all(slice).is_err()
                        {
                            break;
                        }
                    } else {
                        let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {total}\r\n\r\n");
                        if stream.write_all(header.as_bytes()).is_err()
                            || stream.write_all(&payload).is_err()
                        {
                            break;
                        }
                    }
                }
            });
        }
    });
    url
}

fn stream_payload() -> Vec<u8> {
    (0..10_000u32).map(|n| (n % 251) as u8).collect()
}

#[test]
fn httpstream_discovers_size_and_reads_sequentially() {
    let payload = stream_payload();
    let url = serve_ranges(payload.clone());
    let session = HttpSession::new();
    let mut stream = open_stream(&session, HttpRequest::get(&url).unwrap(), true);
    assert_eq!(stream.stream_len(), Some(payload.len() as u64));
    let mut out = Vec::new();
    stream.read_to_end(&mut out).unwrap();
    assert_eq!(out, payload);
}

#[test]
fn httpstream_seek_and_positional_pread() {
    let payload = stream_payload();
    let url = serve_ranges(payload.clone());
    let session = HttpSession::new();
    let mut stream = open_stream(&session, HttpRequest::get(&url).unwrap(), true);

    // Seek then sequential read.
    stream.seek(5000, Whence::Start).unwrap();
    let mut buf = [0u8; 100];
    stream.read(&mut buf).unwrap();
    assert_eq!(&buf[..], &payload[5000..5100]);

    // A footer pread leaves the cursor (still at 5100) untouched.
    let mut footer = [0u8; 20];
    let n = stream.pread(&mut footer, -20, Whence::End).unwrap();
    assert_eq!(&footer[..n], &payload[payload.len() - 20..]);
    assert_eq!(stream.stream_position(), 5100);

    // A Whence::Current pread *does* advance the cursor by what it read.
    let mut take = [0u8; 30];
    let n = stream.pread(&mut take, 0, Whence::Current).unwrap();
    assert_eq!(&take[..n], &payload[5100..5130]);
    assert_eq!(stream.stream_position(), 5130);
}

#[test]
fn httpstream_short_seek_back_is_served_from_the_cache() {
    let payload = stream_payload();
    let url = serve_ranges(payload.clone());
    let session = HttpSession::new();
    let mut stream = open_stream(&session, HttpRequest::get(&url).unwrap(), true);

    // Stream the first 2000 bytes, filling the sliding cache.
    let mut head = [0u8; 2000];
    let mut filled = 0;
    while filled < head.len() {
        let n = stream.read(&mut head[filled..]).unwrap();
        assert_ne!(n, 0);
        filled += n;
    }
    assert_eq!(&head[..], &payload[..2000]);

    // Seek back into the cached region and re-read — no new request needed.
    stream.seek(500, Whence::Start).unwrap();
    let mut again = [0u8; 100];
    stream.read(&mut again).unwrap();
    assert_eq!(&again[..], &payload[500..600]);

    // Seek forward (still cached) and read, then continue to the end.
    stream.seek(1500, Whence::Start).unwrap();
    let mut rest = Vec::new();
    stream.read_to_end(&mut rest).unwrap();
    assert_eq!(rest, payload[1500..]);
}

#[cfg(feature = "media")]
#[test]
fn httpstream_is_an_io_with_url_and_stats() {
    let payload = stream_payload();
    let url = serve_ranges(payload.clone());
    let session = HttpSession::new();
    let stream = open_stream(&session, HttpRequest::get(&url).unwrap(), true);
    assert_eq!(stream.url().scheme(), "http");
    let stats = stream.stats().unwrap();
    assert_eq!(stats.size(), payload.len() as u64);
}

#[test]
fn retries_429_then_succeeds() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    let hits = Arc::new(AtomicU32::new(0));
    let counter = hits.clone();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let _ = read_request(&mut stream);
            let n = counter.fetch_add(1, Ordering::SeqCst);
            if n < 2 {
                let _ = stream.write_all(
                    b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nContent-Length: 0\r\n\r\n",
                );
            } else {
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                );
            }
        }
    });
    let session = HttpSession::new().with_retry(RetryConfig {
        max_retries: 5,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(5),
    });
    let response = session.get(&url).unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(hits.load(Ordering::SeqCst), 3); // two 429s, then 200
}

#[test]
fn httpstream_resumes_after_a_dropped_connection() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    let payload = stream_payload();
    let served = payload.clone();
    let hits = Arc::new(AtomicU32::new(0));
    let counter = hits.clone();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let request = read_request(&mut stream);
            let n = counter.fetch_add(1, Ordering::SeqCst);
            let total = served.len();
            if n == 0 {
                // Initial streaming GET (no Range): promise the full body via
                // Content-Length but send only a prefix, then drop the socket —
                // the client sees a truncated body mid-stream.
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nAccept-Ranges: bytes\r\n\r\n"
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&served[..100]); // far short of Content-Length
                                                          // stream dropped here -> client sees a truncated/aborted body
            } else if let Some((_, _, Some((start, end)))) = request {
                // Resume: serve the requested range to the end.
                let end = (end as usize).min(total - 1);
                let slice = &served[start as usize..=end];
                let header = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{total}\r\nContent-Length: {}\r\n\r\n",
                    slice.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(slice);
            }
        }
    });
    let session = HttpSession::new().with_retry(RetryConfig {
        max_retries: 5,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(5),
    });
    let mut stream = open_stream(&session, HttpRequest::get(&url).unwrap(), true);
    let mut out = Vec::new();
    stream.read_to_end(&mut out).unwrap();
    assert_eq!(out, payload); // resumed and completed despite the mid-stream drop
    assert!(hits.load(Ordering::SeqCst) >= 2); // dropped streaming GET, then resumed range GET
}

#[test]
fn httpstream_rejects_a_reconnect_to_the_wrong_range() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    let payload = stream_payload();
    let served = payload.clone();
    let hits = Arc::new(AtomicU32::new(0));
    let counter = hits.clone();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let _ = read_request(&mut stream);
            let n = counter.fetch_add(1, Ordering::SeqCst);
            let total = served.len();
            if n == 0 {
                // Truncated streaming GET, then drop, forcing a resume from byte 100.
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nAccept-Ranges: bytes\r\n\r\n"
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&served[..100]);
            } else {
                // Misbehave: answer the resume Range with a 206 that starts at byte
                // 0, not the requested 100 — the client must reject this.
                let header = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-{}/{total}\r\nContent-Length: {total}\r\n\r\n",
                    total - 1
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&served);
            }
        }
    });
    let session = HttpSession::new().with_retry(RetryConfig {
        max_retries: 2,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(2),
    });
    let mut stream = open_stream(&session, HttpRequest::get(&url).unwrap(), true);
    let mut out = Vec::new();
    // The resume served the wrong offset, so the read fails rather than silently
    // returning corrupted bytes.
    assert!(stream.read_to_end(&mut out).is_err());
}

#[test]
fn httpstream_pread_during_an_active_stream() {
    let payload = stream_payload();
    let url = serve_ranges(payload.clone());
    let session = HttpSession::new();
    let mut stream = open_stream(&session, HttpRequest::get(&url).unwrap(), true);

    // Read 1000 bytes off the live stream.
    let mut head = [0u8; 1000];
    let mut filled = 0;
    while filled < head.len() {
        filled += stream.read(&mut head[filled..]).unwrap();
    }
    assert_eq!(&head[..], &payload[..1000]);

    // A positional footer pread does not disturb the cursor (still 1000)…
    let mut footer = [0u8; 10];
    let n = stream.pread(&mut footer, -10, Whence::End).unwrap();
    assert_eq!(&footer[..n], &payload[payload.len() - 10..]);
    assert_eq!(stream.stream_position(), 1000);

    // …and the live stream continues correctly from where it left off.
    let mut more = [0u8; 500];
    let mut got = 0;
    while got < more.len() {
        got += stream.read(&mut more[got..]).unwrap();
    }
    assert_eq!(&more[..], &payload[1000..1500]);
}

#[test]
fn httpstream_cache_evicts_and_seek_back_refetches() {
    // A payload larger than the 4 MiB cache, so a long read evicts the early bytes
    // and a seek back to them must re-fetch via a Range request.
    let payload: Vec<u8> = (0..5 * 1024 * 1024u32).map(|n| (n % 251) as u8).collect();
    let url = serve_ranges(payload.clone());
    let session = HttpSession::new();
    let mut stream = open_stream(&session, HttpRequest::get(&url).unwrap(), true);

    // Drain the whole 5 MiB so the first megabyte is evicted from the 4 MiB cache.
    let mut out = Vec::new();
    stream.read_to_end(&mut out).unwrap();
    assert_eq!(out, payload);

    // Seek back into the evicted region and read — a fresh Range fetch returns the
    // correct bytes despite the cache no longer holding them.
    stream.seek(1024, Whence::Start).unwrap();
    let mut back = [0u8; 64];
    let mut filled = 0;
    while filled < back.len() {
        filled += stream.read(&mut back[filled..]).unwrap();
    }
    assert_eq!(&back[..], &payload[1024..1024 + 64]);
}

#[test]
fn send_many_streams_batches() {
    let url = serve_ranges(b"hello".to_vec());
    let session = HttpSession::new()
        .with_max_concurrency(4)
        .with_batch_size(3);
    let requests: Vec<HttpRequest> = (0..7).map(|_| HttpRequest::get(&url).unwrap()).collect();
    let batches: Vec<HttpResponseBatch> = session.send_many(requests).collect();
    assert_eq!(batches.len(), 3); // 3 + 3 + 1
    let total: usize = batches.iter().map(|b| b.len()).sum();
    assert_eq!(total, 7);
    for batch in batches {
        for result in batch {
            assert_eq!(result.unwrap().status(), 200);
        }
    }
}

/// A server that never advertises a `Content-Length` (size unknown): a plain
/// `GET` streams the whole body then closes the socket (close-delimited), and a
/// `Range` request answers 206 from the offset, or 416 once it starts past the
/// end. Each connection is one-shot (closed after the reply) so the unknown
/// size is delimited by the close.
fn serve_unknown_size(payload: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let payload = std::sync::Arc::new(payload);
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let payload = payload.clone();
            thread::spawn(move || {
                let mut stream = stream;
                let _ = stream.set_nodelay(true);
                let Some((method, _path, range)) = read_request(&mut stream) else {
                    return;
                };
                let total = payload.len();
                if method == "HEAD" {
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                    );
                } else if let Some((start, end)) = range {
                    if start as usize >= total {
                        let _ = stream.write_all(b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    } else {
                        let end = (end as usize).min(total - 1);
                        let slice = &payload[start as usize..=end];
                        let header = format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            slice.len()
                        );
                        let _ = stream.write_all(header.as_bytes());
                        let _ = stream.write_all(slice);
                    }
                } else {
                    // Plain GET, no range: stream the whole body with no
                    // Content-Length and close — the client learns the size at
                    // EOF (the socket close).
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                    );
                    let _ = stream.write_all(&payload);
                }
            });
        }
    });
    url
}

#[test]
fn httpstream_unknown_size_reads_to_eof() {
    let payload = stream_payload();
    let url = serve_unknown_size(payload.clone());
    let session = HttpSession::new();
    let mut stream = open_stream(&session, HttpRequest::get(&url).unwrap(), false);
    assert_eq!(stream.stream_len(), None); // size not advertised
    let mut out = Vec::new();
    stream.read_to_end(&mut out).unwrap();
    assert_eq!(out, payload); // the close-delimited body ends the read cleanly
    assert_eq!(stream.stream_len(), Some(payload.len() as u64)); // discovered at EOF
}

#[test]
fn httpstream_range_past_end_is_clean_eof_via_416() {
    let payload = stream_payload();
    let url = serve_unknown_size(payload.clone());
    let session = HttpSession::new();
    let mut stream = open_stream(&session, HttpRequest::get(&url).unwrap(), false);
    // Size is unknown, so the past-the-end guard can't short-circuit: the
    // request is issued and the server's 416 surfaces as a clean 0-byte read.
    let mut buf = [0u8; 16];
    let n = stream
        .pread(&mut buf, payload.len() as i64, Whence::Start)
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn httpstream_close_releases_the_connection_and_reads_eof() {
    let payload = stream_payload();
    let url = serve_ranges(payload);
    let session = HttpSession::new();
    let mut stream = open_stream(&session, HttpRequest::get(&url).unwrap(), true);
    assert_eq!(session.open_streams(), 1); // the stream holds one connection
    let mut head = [0u8; 16];
    assert_eq!(stream.read(&mut head).unwrap(), 16);
    stream.close().unwrap();
    let mut more = [0u8; 16];
    assert_eq!(stream.read(&mut more).unwrap(), 0); // closed -> clean EOF
    drop(stream);
    assert_eq!(session.open_streams(), 0); // connection released on drop
}

#[test]
fn keep_alive_false_sends_connection_close() {
    let (url, rx) = serve_once(ok_reply("text/plain", b"ok"));
    HttpSession::new()
        .send(HttpRequest::get(&url).unwrap(), false, false, true)
        .unwrap();
    let request = String::from_utf8(rx.recv().unwrap())
        .unwrap()
        .to_lowercase();
    assert!(request.contains("connection: close"), "{request}");
}

#[test]
fn keep_alive_true_does_not_close_the_connection() {
    let (url, rx) = serve_once(ok_reply("text/plain", b"ok"));
    HttpSession::new()
        .send(HttpRequest::get(&url).unwrap(), false, true, true)
        .unwrap();
    let request = String::from_utf8(rx.recv().unwrap())
        .unwrap()
        .to_lowercase();
    assert!(!request.contains("connection: close"), "{request}");
}

#[test]
fn pool_safeguard_closes_extra_streams_when_saturated() {
    use std::sync::{Arc, Mutex};
    // Records every request line/headers, replying 200 with the payload.
    let payload = stream_payload();
    let served = payload.clone();
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let recorder = seen.clone();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let served = served.clone();
            let recorder = recorder.clone();
            thread::spawn(move || {
                let mut stream = stream;
                let _ = stream.set_nodelay(true);
                let mut buf = Vec::new();
                let mut byte = [0u8; 1];
                while !buf.ends_with(b"\r\n\r\n") {
                    match stream.read(&mut byte) {
                        Ok(0) | Err(_) => return,
                        Ok(_) => buf.push(byte[0]),
                    }
                }
                recorder
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf).into_owned());
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\n\r\n",
                    served.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&served);
            });
        }
    });
    // A pool that holds a single keep-alive connection.
    let session = HttpSession::new().with_pool_size(1);
    assert_eq!(session.pool_size(), 1);
    // The first stream fills the pool; the second is over capacity, so the
    // safeguard forces it to close (it must not starve the keep-alive pool).
    let s1 = open_stream(&session, HttpRequest::get(&url).unwrap(), true);
    let s2 = open_stream(&session, HttpRequest::get(&url).unwrap(), true);
    assert_eq!(session.open_streams(), 2);
    drop(s1);
    drop(s2);
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    let closing = seen
        .iter()
        .filter(|r| r.to_lowercase().contains("connection: close"))
        .count();
    assert_eq!(
        closing, 1,
        "exactly the over-capacity stream closes: {seen:?}"
    );
}

#[test]
fn keep_alive_requests_release_the_connection_on_eof() {
    let url = serve_ranges(stream_payload());
    let session = HttpSession::new();
    for _ in 0..5 {
        // Each drained response returns its connection to the pool (EOF), so no
        // stream is left holding one between requests.
        let _ = session.get(&url).unwrap().bytes().unwrap();
        assert_eq!(session.open_streams(), 0);
    }
}

#[test]
fn httpstream_reconnects_through_multiple_drops() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    let payload = stream_payload();
    let served = payload.clone();
    let hits = Arc::new(AtomicU32::new(0));
    let counter = hits.clone();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let request = read_request(&mut stream);
            let n = counter.fetch_add(1, Ordering::SeqCst);
            let total = served.len();
            // The resume cursor is the start of the requested range (0 first).
            let start = match &request {
                Some((_, _, Some((start, _)))) => *start as usize,
                _ => 0,
            };
            let header = if start == 0 {
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nAccept-Ranges: bytes\r\n\r\n"
                )
            } else {
                format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{}/{total}\r\nContent-Length: {}\r\n\r\n",
                    total - 1,
                    total - start
                )
            };
            let _ = stream.write_all(header.as_bytes());
            if n < 3 {
                // Truncate the body 1000 bytes in, then drop — forcing a fresh
                // mid-stream reconnect each time.
                let chunk_end = (start + 1000).min(total);
                let _ = stream.write_all(&served[start..chunk_end]);
            } else {
                // Finally serve the remainder in full.
                let _ = stream.write_all(&served[start..]);
            }
        }
    });
    let session = HttpSession::new().with_retry(RetryConfig {
        max_retries: 10,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(5),
    });
    let mut stream = open_stream(&session, HttpRequest::get(&url).unwrap(), true);
    let mut out = Vec::new();
    stream.read_to_end(&mut out).unwrap();
    assert_eq!(out, payload); // resumed through every drop and completed
    assert!(hits.load(Ordering::SeqCst) >= 4); // 3 truncated drops, then the full tail
}

#[test]
fn retry_exhaustion_surfaces_the_error_status() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let _ = read_request(&mut stream);
            let _ = stream.write_all(
                b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    let session = HttpSession::new().with_retry(RetryConfig {
        max_retries: 2,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(2),
    });
    // After exhausting retries the persistent 503 is returned, and get() raises.
    assert!(matches!(session.get(&url), Err(HttpError::Status(503))));
}

#[test]
fn send_many_handles_an_empty_iterator() {
    let session = HttpSession::new();
    let batches: Vec<HttpResponseBatch> = session.send_many(Vec::new()).collect();
    assert!(batches.is_empty());
}

#[test]
fn timing_records_sent_and_received_after_full_read() {
    // A buffered send (stream = false) drains the body during `send`, so the
    // returned response already carries both timestamps: sent_at > 0 and
    // received_at >= sent_at, the "after a normal GET is fully read" invariant.
    let url = serve_ranges(stream_payload());
    let session = HttpSession::new();
    let response = session
        .send(HttpRequest::get(&url).unwrap(), false, true, false)
        .unwrap();
    let sent_at = response.sent_at();
    let received_at = response.received_at();
    assert!(sent_at > 0.0, "sent_at should be stamped: {sent_at}");
    assert!(
        received_at >= sent_at,
        "received_at {received_at} >= sent_at {sent_at}"
    );
    // The body is still readable through the same accessors.
    assert_eq!(response.bytes().unwrap(), stream_payload());
}

#[test]
fn timing_received_at_is_unset_until_a_streamed_body_drains() {
    // A streamed send leaves received_at at 0.0 until the caller drains or closes
    // the live HttpStream; sent_at is stamped immediately.
    let url = serve_ranges(stream_payload());
    let session = HttpSession::new();
    let response = session
        .send(HttpRequest::get(&url).unwrap(), false, true, true)
        .unwrap();
    assert!(response.sent_at() > 0.0);
    assert_eq!(response.received_at(), 0.0); // streamed body not yet drained
}

#[test]
fn io_json_parses_a_response_body() {
    let (url, _rx) = serve_once(ok_reply("application/json", br#"{"a":1,"b":[2,3]}"#));
    let mut handle = HttpSession::new()
        .get(&url)
        .unwrap()
        .into_bytesio()
        .unwrap();
    let value = handle.json().unwrap();
    assert_eq!(value["a"].as_u64(), Some(1));
    assert_eq!(value["b"][0].as_u64(), Some(2));
    assert_eq!(value["b"][1].as_u64(), Some(3));
}

// ---------------------------------------------------------------------------
// Redirect following + cookie jar (hermetic, scripted localhost server)
// ---------------------------------------------------------------------------

/// A reply builder, given the raw request text the server received (so a redirect
/// reply can echo state). Returns the raw bytes to write back.
type Reply = Box<dyn Fn(&str) -> Vec<u8> + Send>;

/// Spawns a localhost server that answers each accepted connection with the next
/// scripted `reply` in order (one request per connection — every reply uses
/// `Connection: close`), forwarding each received request's text on the channel.
/// Hermetic: nothing leaves the loopback interface.
fn serve_script(replies: Vec<Reply>) -> (String, std::sync::mpsc::Receiver<String>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        let mut replies = replies.into_iter();
        for stream in listener.incoming().flatten() {
            let Some(reply) = replies.next() else { break };
            let mut stream = stream;
            stream
                .set_read_timeout(Some(Duration::from_millis(200)))
                .ok();
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            // Read until the header terminator, then drain any pending body.
            loop {
                match stream.read(&mut buf) {
                    Ok(0) => break,
                    Ok(count) => {
                        request.extend_from_slice(&buf[..count]);
                        if request.windows(4).any(|w| w == b"\r\n\r\n")
                            && !request_has_pending_body(&request)
                        {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let text = String::from_utf8_lossy(&request).into_owned();
            tx.send(text.clone()).ok();
            let _ = stream.write_all(&reply(&text));
            let _ = stream.flush();
        }
    });
    (url, rx)
}

/// Whether a request still has body bytes outstanding (a Content-Length larger
/// than the bytes already after the header terminator), so the server keeps
/// reading instead of replying mid-upload.
fn request_has_pending_body(request: &[u8]) -> bool {
    let text = String::from_utf8_lossy(request);
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return false;
    };
    let length = head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    });
    matches!(length, Some(length) if body.len() < length)
}

/// A `301`/`302`/`303`/`307`/`308` reply pointing at `location`.
fn redirect_reply(status: u16, reason: &str, location: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status} {reason}\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .into_bytes()
}

#[test]
fn redirect_chain_lands_on_200() {
    // /a -> /b -> 200, all on the same host; the final URL and body reflect /b.
    let (url, rx) = serve_script(vec![
        Box::new(|_| redirect_reply(302, "Found", "/b")),
        Box::new(|_| ok_reply("text/plain", b"arrived")),
    ]);
    let session = HttpSession::new();
    let response = session.get(&url).unwrap();
    assert_eq!(response.status(), 200);
    let final_url = response.url().to_string();
    assert_eq!(response.text().unwrap(), "arrived");
    assert!(final_url.ends_with("/b"), "{final_url}");
    // First hop requested `/`, second `/b`.
    assert!(rx.recv().unwrap().starts_with("GET / HTTP/1.1"));
    assert!(rx.recv().unwrap().starts_with("GET /b HTTP/1.1"));
}

#[test]
fn redirect_301_downgrades_post_to_get() {
    // A 301 on a POST follows as a bodyless GET (the de-facto browser behaviour).
    let (url, rx) = serve_script(vec![
        Box::new(|_| redirect_reply(301, "Moved Permanently", "/next")),
        Box::new(|_| ok_reply("text/plain", b"ok")),
    ]);
    let session = HttpSession::new();
    let response = session
        .request(
            HttpRequest::post(&url)
                .unwrap()
                .with_body(b"payload".to_vec()),
            false,
        )
        .unwrap();
    assert_eq!(response.status(), 200);
    let first = rx.recv().unwrap();
    assert!(first.starts_with("POST / HTTP/1.1"), "{first}");
    assert!(first.contains("payload"), "{first}");
    let second = rx.recv().unwrap();
    assert!(second.starts_with("GET /next HTTP/1.1"), "{second}");
    assert!(!second.contains("payload"), "{second}");
}

#[test]
fn redirect_307_preserves_post_and_body() {
    // A 307 preserves the method and replays the (in-memory) body.
    let (url, rx) = serve_script(vec![
        Box::new(|_| redirect_reply(307, "Temporary Redirect", "/again")),
        Box::new(|_| ok_reply("text/plain", b"ok")),
    ]);
    let session = HttpSession::new();
    let response = session
        .request(
            HttpRequest::post(&url)
                .unwrap()
                .with_body(b"keep-me".to_vec()),
            false,
        )
        .unwrap();
    assert_eq!(response.status(), 200);
    let _first = rx.recv().unwrap();
    let second = rx.recv().unwrap();
    assert!(second.starts_with("POST /again HTTP/1.1"), "{second}");
    assert!(second.contains("keep-me"), "{second}");
}

#[test]
fn redirect_dropping_the_body_strips_entity_headers() {
    // A 303 (and a POST->GET downgrade) drops the body; the entity headers that
    // described it must not linger on the now-bodyless GET.
    let (url, rx) = serve_script(vec![
        Box::new(|_| redirect_reply(303, "See Other", "/done")),
        Box::new(|_| ok_reply("text/plain", b"ok")),
    ]);
    let session = HttpSession::new();
    let response = session
        .request(
            HttpRequest::post(&url)
                .unwrap()
                .with_header("content-type", "application/json")
                .with_body(b"{\"a\":1}".to_vec()),
            false,
        )
        .unwrap();
    assert_eq!(response.status(), 200);
    let _first = rx.recv().unwrap();
    let second = rx.recv().unwrap().to_lowercase();
    assert!(second.starts_with("get /done http/1.1"), "{second}");
    assert!(!second.contains("content-type"), "{second}");
    assert!(!second.contains("content-length"), "{second}");
    assert!(!second.contains("{\"a\":1}"), "{second}");
}

#[test]
fn redirect_loop_errors() {
    // /loop -> /loop forever: detected as a loop and surfaced as an error.
    let (url, _rx) = serve_script(vec![
        Box::new(|_| redirect_reply(302, "Found", "/loop")),
        Box::new(|_| redirect_reply(302, "Found", "/loop")),
        Box::new(|_| redirect_reply(302, "Found", "/loop")),
    ]);
    let session = HttpSession::new();
    let result = session.send(
        HttpRequest::from_url(
            Method::Get,
            yggdryl_core::Url::from_str(&format!("{url}/loop")).unwrap(),
        ),
        false,
        true,
        false,
    );
    assert!(
        matches!(result, Err(HttpError::TooManyRedirects(_))),
        "expected TooManyRedirects, got {}",
        result.map(|response| response.status()).unwrap_or(0)
    );
}

#[test]
fn allow_redirect_false_returns_the_3xx() {
    // With redirects disabled the 3xx is returned untouched.
    let (url, _rx) = serve_script(vec![Box::new(|_| {
        redirect_reply(302, "Found", "/somewhere")
    })]);
    let session = HttpSession::new();
    let response = session
        .send(
            HttpRequest::get(&url).unwrap().with_allow_redirect(false),
            false,
            true,
            false,
        )
        .unwrap();
    assert_eq!(response.status(), 302);
    assert_eq!(response.header("location"), Some("/somewhere"));
}

#[test]
fn exceeding_max_redirects_errors() {
    let (url, _rx) = serve_script(vec![
        Box::new(|_| redirect_reply(302, "Found", "/1")),
        Box::new(|_| redirect_reply(302, "Found", "/2")),
        Box::new(|_| redirect_reply(302, "Found", "/3")),
    ]);
    let session = HttpSession::new().with_max_redirects(1);
    let result = session.send(HttpRequest::get(&url).unwrap(), false, true, false);
    assert!(
        matches!(result, Err(HttpError::TooManyRedirects(_))),
        "expected TooManyRedirects, got {}",
        result.map(|response| response.status()).unwrap_or(0)
    );
}

#[test]
fn cross_host_redirect_strips_authorization() {
    // The first host redirects to a *different* host; the Authorization header
    // must not be carried across (the second host never sees it).
    let (host_b, rx_b) = serve_script(vec![Box::new(|_| ok_reply("text/plain", b"b"))]);
    let location = format!("{host_b}/secure");
    let (host_a, rx_a) = serve_script(vec![Box::new(move |_| {
        redirect_reply(302, "Found", &location)
    })]);
    let session = HttpSession::new();
    let response = session
        .request(
            HttpRequest::get(&host_a)
                .unwrap()
                .with_header("authorization", "Bearer secret"),
            false,
        )
        .unwrap();
    assert_eq!(response.status(), 200);
    let first = rx_a.recv().unwrap().to_lowercase();
    assert!(first.contains("authorization: bearer secret"), "{first}");
    let second = rx_b.recv().unwrap().to_lowercase();
    assert!(
        !second.contains("authorization"),
        "second hop leaked auth: {second}"
    );
}

#[test]
fn set_cookie_is_sent_back_on_a_follow_up_request() {
    // First response sets a cookie; the next request to the same host sends it.
    let (url, rx) = serve_script(vec![
        Box::new(|_| {
            let mut reply = b"HTTP/1.1 200 OK\r\nSet-Cookie: sid=abc123; Path=/\r\nContent-Length: 2\r\nConnection: close\r\n\r\n".to_vec();
            reply.extend_from_slice(b"ok");
            reply
        }),
        Box::new(|_| ok_reply("text/plain", b"second")),
    ]);
    let session = HttpSession::new();
    session.get(&url).unwrap();
    let _first = rx.recv().unwrap();
    assert!(session.cookies().get("sid").is_some());
    session.get(&url).unwrap();
    let second = rx.recv().unwrap().to_lowercase();
    assert!(second.contains("cookie: sid=abc123"), "{second}");
}

#[test]
fn an_expired_cookie_is_not_sent() {
    // A Max-Age=0 cookie expires at once, so it is never sent on the next request.
    let (url, rx) = serve_script(vec![
        Box::new(|_| {
            let mut reply = b"HTTP/1.1 200 OK\r\nSet-Cookie: gone=x; Path=/; Max-Age=0\r\nContent-Length: 2\r\nConnection: close\r\n\r\n".to_vec();
            reply.extend_from_slice(b"ok");
            reply
        }),
        Box::new(|_| ok_reply("text/plain", b"second")),
    ]);
    let session = HttpSession::new();
    session.get(&url).unwrap();
    let _first = rx.recv().unwrap();
    session.get(&url).unwrap();
    let second = rx.recv().unwrap().to_lowercase();
    assert!(
        !second.contains("cookie:"),
        "expired cookie was sent: {second}"
    );
}

#[test]
fn redirect_307_with_a_streamed_body_returns_the_3xx() {
    // A 307 must preserve method *and* body, but a streamed (single-shot) body
    // cannot be replayed — so the 3xx is returned untouched rather than
    // re-dispatched with a silently emptied body.
    let (url, _rx) = serve_script(vec![
        Box::new(|_| redirect_reply(307, "Temporary Redirect", "/again")),
        Box::new(|_| ok_reply("text/plain", b"unreachable")),
    ]);
    let session = HttpSession::new();
    let response = session
        .send(
            HttpRequest::post(&url)
                .unwrap()
                .with_body_io(BytesIO::from_bytes(b"streamed".to_vec())),
            false,
            true,
            false,
        )
        .unwrap();
    assert_eq!(response.status(), 307);
    assert_eq!(response.header("location"), Some("/again"));
}

#[test]
fn a_cookie_set_by_a_redirect_is_re_derived_on_the_next_hop() {
    // The redirecting response sets a cookie; the followed hop (same host) must
    // send *that* cookie, not carry a stale value from the first hop.
    let (url, rx) = serve_script(vec![
        Box::new(|_| {
            let mut reply = b"HTTP/1.1 302 Found\r\nLocation: /next\r\nSet-Cookie: sid=fromredirect; Path=/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec();
            reply.extend_from_slice(b"");
            reply
        }),
        Box::new(|_| ok_reply("text/plain", b"done")),
    ]);
    let session = HttpSession::new();
    let response = session.get(&url).unwrap();
    assert_eq!(response.status(), 200);
    let _first = rx.recv().unwrap();
    let second = rx.recv().unwrap().to_lowercase();
    assert!(second.contains("cookie: sid=fromredirect"), "{second}");
}

#[test]
fn retry_after_accepts_both_delta_seconds_and_http_date() {
    // Delta-seconds form.
    let mut headers = crate::HttpHeaders::new();
    headers.insert("retry-after", "120");
    assert_eq!(headers.retry_after(), Some(Duration::from_secs(120)));

    // A far-future HTTP-date yields a positive (and large) delay from now.
    let mut future = crate::HttpHeaders::new();
    future.insert("retry-after", "Wed, 21 Oct 2099 07:28:00 GMT");
    let delay = future.retry_after().expect("date form should parse");
    assert!(delay > Duration::from_secs(60 * 60 * 24 * 365), "{delay:?}");

    // A past HTTP-date clamps to zero rather than going negative.
    let mut past = crate::HttpHeaders::new();
    past.insert("retry-after", "Wed, 21 Oct 2015 07:28:00 GMT");
    assert_eq!(past.retry_after(), Some(Duration::ZERO));
}

#[test]
fn a_secure_cookie_is_withheld_over_http() {
    // A Secure cookie set over the (test) http connection must not be sent back
    // over plain http — the domain matches but the scheme rule withholds it.
    let url = yggdryl_core::Url::from_str("https://example.com/").unwrap();
    let mut jar = crate::HttpCookies::new();
    let mut headers = crate::HttpHeaders::new();
    headers.insert("set-cookie", "tok=v; Path=/; Secure");
    jar.set_from_response(&url, &headers);
    // Over https the cookie is offered.
    assert_eq!(jar.header_for(&url).as_deref(), Some("tok=v"));
    // Over plain http to the same host it is withheld.
    let http = yggdryl_core::Url::from_str("http://example.com/").unwrap();
    assert_eq!(jar.header_for(&http), None);
}

#[test]
fn http_version_parses_names_and_alpn() {
    use crate::HttpVersion;
    // Selectors parse case-insensitively across the common spellings.
    for value in ["auto", "AUTO", "negotiate", ""] {
        assert_eq!(HttpVersion::from_str(value).unwrap(), HttpVersion::Auto);
    }
    for value in ["1.1", "http/1.1", "H1", "http11"] {
        assert_eq!(HttpVersion::from_str(value).unwrap(), HttpVersion::Http11);
    }
    assert_eq!(HttpVersion::from_str("h2").unwrap(), HttpVersion::Http2);
    assert_eq!(HttpVersion::from_str("http/3").unwrap(), HttpVersion::Http3);
    // An unknown selector names the accepted values.
    let err = HttpVersion::from_str("spdy").unwrap_err();
    assert!(err.to_string().contains("expected auto"), "{err}");

    // Names, ALPN ids and the round-trip back from an ALPN id.
    assert_eq!(HttpVersion::Http11.as_str(), "HTTP/1.1");
    assert_eq!(HttpVersion::Http2.alpn(), Some("h2"));
    assert_eq!(HttpVersion::Auto.alpn(), None);
    assert_eq!(HttpVersion::from_alpn("h3"), Some(HttpVersion::Http3));
    assert_eq!(HttpVersion::from_alpn("spdy/3"), None);

    // Only HTTP/1.1 (and Auto, which negotiates to it) is wired today.
    assert!(HttpVersion::Http11.is_available());
    assert!(HttpVersion::Auto.is_available());
    assert!(!HttpVersion::Http2.is_available());
    assert!(!HttpVersion::Http3.is_available());
}

#[test]
fn negotiated_version_is_http11_and_pinning_an_unwired_version_errors() {
    use crate::HttpVersion;
    let (url, _rx) = serve_once(ok_reply("text/plain", b"ok"));
    // A normal request negotiates (Auto) down to the only wired transport, and the
    // response reports the version it was delivered over.
    let session = HttpSession::new();
    let response = session
        .send(HttpRequest::get(&url).unwrap(), false, false, false)
        .unwrap();
    assert_eq!(response.negotiated_version(), HttpVersion::Http11);

    // Pinning HTTP/2 (no transport yet) errors before any bytes leave, naming the
    // alternative, rather than silently downgrading.
    let (url2, _rx2) = serve_once(ok_reply("text/plain", b"ok"));
    let pinned = HttpRequest::get(&url2)
        .unwrap()
        .with_http_version(HttpVersion::Http2);
    let err = match session.send(pinned, false, false, false) {
        Ok(_) => panic!("pinning an unwired HTTP/2 should error"),
        Err(err) => err,
    };
    assert!(matches!(err, HttpError::Unsupported(_)), "{err:?}");
    assert!(err.to_string().contains("HTTP/2"), "{err}");

    // A session-level default applies to requests that do not pin their own.
    let h2_session = HttpSession::new().with_http_version(HttpVersion::Http2);
    assert_eq!(h2_session.http_version(), HttpVersion::Http2);
    let (url3, _rx3) = serve_once(ok_reply("text/plain", b"ok"));
    assert!(h2_session
        .send(HttpRequest::get(&url3).unwrap(), false, false, false)
        .is_err());
}

#[cfg(feature = "serde")]
#[test]
fn http_data_types_serde_round_trip() {
    use crate::{Cookie, HttpCookies, HttpHeaders, HttpVersion};

    // Method serialises as its variant name and parses back.
    let method = Method::Post;
    assert_eq!(
        serde_json::from_str::<Method>(&serde_json::to_string(&method).unwrap()).unwrap(),
        method
    );

    // HttpVersion round-trips its variant name.
    let version = HttpVersion::Http2;
    assert_eq!(
        serde_json::from_str::<HttpVersion>(&serde_json::to_string(&version).unwrap()).unwrap(),
        version
    );

    // RetryConfig round-trips its whole policy.
    let retry = RetryConfig::default();
    let back: RetryConfig = serde_json::from_str(&serde_json::to_string(&retry).unwrap()).unwrap();
    assert_eq!(back.max_retries, retry.max_retries);
    assert_eq!(back.base_delay, retry.base_delay);

    // Headers preserve order and casing.
    let mut headers = HttpHeaders::new();
    headers.insert("Content-Type", "text/plain");
    headers.insert("X-Trace", "abc");
    assert_eq!(
        serde_json::from_str::<HttpHeaders>(&serde_json::to_string(&headers).unwrap()).unwrap(),
        headers
    );

    // The cookie jar round-trips, so a session's cookies can be persisted.
    let url = yggdryl_core::Url::from_str("https://example.com/").unwrap();
    let jar = HttpCookies::new().with_cookie(Cookie::new("sid", "abc", &url).unwrap());
    let back: HttpCookies = serde_json::from_str(&serde_json::to_string(&jar).unwrap()).unwrap();
    assert_eq!(
        back.get("sid").map(|c| c.value().to_string()),
        Some("abc".to_string())
    );
}

#[test]
fn shared_session_singleton_and_set() {
    // Every call hands back the same pooled session (the same `Arc`)…
    let a = HttpSession::shared();
    let b = HttpSession::shared();
    assert!(std::sync::Arc::ptr_eq(&a, &b));
    // …until `set_shared` replaces it — here with one carrying a base URL, so the
    // module-level verbs resolve relative targets against it.
    let base = yggdryl_core::Url::from_str("https://api.example.com/v1/").unwrap();
    HttpSession::set_shared(HttpSession::new().with_base_url(base));
    let c = HttpSession::shared();
    assert_eq!(
        c.base_url().map(ToString::to_string),
        Some("https://api.example.com/v1/".to_string())
    );
    assert!(!std::sync::Arc::ptr_eq(&a, &c));
}

#[test]
fn base_url_resolves_relative_targets() {
    let base = yggdryl_core::Url::from_str("https://api.example.com/v1/").unwrap();
    let session = HttpSession::new().with_base_url(base);
    // A bare name joins onto the base path; an absolute path replaces it.
    assert_eq!(
        session.resolve_url("users").unwrap().to_string(),
        "https://api.example.com/v1/users"
    );
    assert_eq!(
        session.resolve_url("/users").unwrap().to_string(),
        "https://api.example.com/users"
    );
    // An absolute URL bypasses the base entirely.
    assert_eq!(
        session
            .resolve_url("https://other.test/x")
            .unwrap()
            .to_string(),
        "https://other.test/x"
    );
    // With no base URL a relative target is an error; an absolute one parses.
    let plain = HttpSession::new();
    assert!(plain.resolve_url("relative").is_err());
    assert!(plain.resolve_url("https://h/p").is_ok());
}

#[test]
fn redirect_resolve_normalizes_dot_segments_and_fragment() {
    use crate::redirect;
    let base = yggdryl_core::Url::from_str("https://h/v1/users/42").unwrap();
    // `../` ascends and is normalized (no literal "/v1/users/../other").
    assert_eq!(
        redirect::resolve(&base, "../other").unwrap().path(),
        "/v1/other"
    );
    assert_eq!(
        redirect::resolve(&base, "./x").unwrap().path(),
        "/v1/users/x"
    );
    // An absolute path's own dot-segments resolve too.
    assert_eq!(redirect::resolve(&base, "/a/../b").unwrap().path(), "/b");
    // A `#fragment` is split off the path, not embedded in it.
    let r = redirect::resolve(&base, "/p#frag").unwrap();
    assert_eq!(r.path(), "/p");
    assert_eq!(r.fragment(), Some("frag"));
    // A relative redirect with a query keeps the path and sets the query; it does
    // not inherit the base's (absent) query/fragment.
    let r2 = redirect::resolve(&base, "next?x=1").unwrap();
    assert_eq!(r2.path(), "/v1/users/next");
    assert_eq!(r2.query(), Some("x=1"));
}

#[test]
fn cross_domain_set_cookie_is_rejected() {
    use crate::Cookie;
    let evil = yggdryl_core::Url::from_str("https://a.evil.test/").unwrap();
    // A `Domain` the response host does not domain-match is rejected (injection).
    assert!(Cookie::from_set_cookie("x=1; Domain=example.com", &evil).is_none());
    // A single-label / public-suffix `Domain` is rejected.
    assert!(Cookie::from_set_cookie("x=1; Domain=test", &evil).is_none());
    // A host-only cookie (no `Domain`) is always accepted.
    assert!(Cookie::from_set_cookie("y=2; Path=/", &evil).is_some());
    // A `Domain` that is a parent of the response host is accepted.
    let sub = yggdryl_core::Url::from_str("https://www.example.com/").unwrap();
    let cookie = Cookie::from_set_cookie("x=1; Domain=example.com", &sub).unwrap();
    assert_eq!(cookie.domain(), "example.com");
    // A single-label `Domain` equal to the request host (e.g. localhost) is allowed.
    let local = yggdryl_core::Url::from_str("http://localhost/").unwrap();
    assert!(Cookie::from_set_cookie("x=1; Domain=localhost", &local).is_some());
}

#[test]
fn cookie_header_orders_by_descending_path_length() {
    use crate::{HttpCookies, HttpHeaders};
    let url = yggdryl_core::Url::from_str("https://h/app/page").unwrap();
    let mut jar = HttpCookies::new();
    let mut headers = HttpHeaders::new();
    headers.insert("set-cookie", "a=1; Path=/");
    headers.insert("set-cookie", "b=2; Path=/app");
    jar.set_from_response(&url, &headers);
    // The longer path (`/app`) is listed first (RFC 6265 §5.4).
    assert_eq!(jar.header_for(&url).as_deref(), Some("b=2; a=1"));
}

#[test]
fn head_response_with_content_length_drains_empty() {
    // A HEAD reply advertises Content-Length but sends no body. Draining it must
    // yield an empty body, not an UnexpectedEof — the body size is zero for HEAD
    // regardless of the header (the binding path drains buffered during send).
    let reply = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 11\r\nConnection: close\r\n\r\n".to_vec();
    let (url, _rx) = serve_once(reply);
    let session = HttpSession::new();
    let response = session
        .send(HttpRequest::head(&url).unwrap(), false, true, false)
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.bytes().unwrap(), b"");
}

#[test]
fn no_content_204_drains_empty() {
    // 204 No Content (and 304) carry no body even with a Content-Length header.
    let reply =
        b"HTTP/1.1 204 No Content\r\nContent-Length: 5\r\nConnection: close\r\n\r\n".to_vec();
    let (url, _rx) = serve_once(reply);
    let session = HttpSession::new();
    let response = session
        .send(HttpRequest::get(&url).unwrap(), false, true, false)
        .unwrap();
    assert_eq!(response.status(), 204);
    assert_eq!(response.bytes().unwrap(), b"");
}
