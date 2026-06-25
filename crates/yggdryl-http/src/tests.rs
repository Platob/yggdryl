//! Hermetic tests: a localhost `TcpListener` serves HEAD / `Range` / 429 /
//! mid-stream drops, so nothing here touches the network.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use yggdryl_io::{BytesIO, Io, Whence};

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
    let packed = yggdryl_compression::Compression::Gzip
        .compress(&body)
        .unwrap();
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
        Some(yggdryl_media::MimeType::from_str("application/json").unwrap())
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
