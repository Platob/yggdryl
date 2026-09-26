//! `rust/src/http/server/connection.rs`: one connection - the head read
//! within its bound and its deadline, the body framed, the answer written and
//! the connection kept or closed - and the `CONNECT` tunnel, driven over raw
//! `TcpStream` writes.

use super::*;

#[test]
fn a_malformed_request_line_answers_400_and_closes() {
    let server = served();
    let (head, body) = raw(&server, b"GET\r\n\r\n");
    assert_eq!(head.status, Status::BAD_REQUEST);
    assert_eq!(head.headers.get("connection"), Some("close"));
    assert!(
        head.headers
            .get("server")
            .is_some_and(|server| server.starts_with("yggdryl/"))
    );
    assert!(head.headers.get("date").is_some());
    assert!(!body.is_empty(), "the refusal names what was wrong");
    assert_eq!(
        server.request_count(),
        0,
        "a head the grammar refuses is no request"
    );
}

#[test]
fn a_folded_field_line_answers_400() {
    let server = served();
    let (head, _) = raw(
        &server,
        b"GET /rows.json HTTP/1.1\r\nHost: test\r\n Folded: yes\r\n\r\n",
    );
    assert_eq!(head.status, Status::BAD_REQUEST);
}

#[test]
fn a_head_over_the_bound_answers_431() {
    let server = Server::bind_with(
        "127.0.0.1:0",
        ServerOptions::default().with_max_head_size(256),
    )
    .expect("bind");
    let filler = "x".repeat(300);
    let (head, _) = raw(
        &server,
        format!("GET / HTTP/1.1\r\nHost: test\r\nX-Filler: {filler}\r\n\r\n").as_bytes(),
    );
    assert_eq!(head.status.code(), 431);
    assert_eq!(server.request_count(), 0);
}

#[test]
fn a_body_over_the_bound_answers_413_and_closes() {
    let server = Server::bind_with(
        "127.0.0.1:0",
        ServerOptions::default().with_max_body_size(8),
    )
    .expect("bind");
    server.mount("/", memory_root()).expect("mount");
    let (head, _) = raw(
        &server,
        b"PUT /big.bin HTTP/1.1\r\nHost: test\r\nContent-Length: 9\r\n\r\n123456789",
    );
    assert_eq!(head.status.code(), 413);
    let (head, _) = raw(
        &server,
        b"PUT /big.bin HTTP/1.1\r\nHost: test\r\nTransfer-Encoding: chunked\r\n\r\n9\r\n123456789\r\n0\r\n\r\n",
    );
    assert_eq!(head.status.code(), 413);
    assert_eq!(server.request_count(), 0, "a refused body reaches no route");
    let (head, _) = raw(
        &server,
        b"PUT /ok.bin HTTP/1.1\r\nHost: test\r\nConnection: close\r\nContent-Length: 8\r\n\r\n12345678",
    );
    assert_eq!(
        head.status,
        Status::CREATED,
        "exactly the bound is accepted"
    );
}

#[test]
fn an_ambiguous_framing_answers_400() {
    let server = served();
    let (head, _) = raw(
        &server,
        b"PUT /x HTTP/1.1\r\nHost: test\r\nContent-Length: 1\r\nTransfer-Encoding: chunked\r\n\r\n1\r\nx\r\n0\r\n\r\n",
    );
    assert_eq!(head.status, Status::BAD_REQUEST);
}

#[test]
fn a_transfer_coding_on_http_1_0_or_not_ending_in_chunked_answers_400() {
    let server = served();
    for request in [
        &b"PUT /x HTTP/1.0\r\nHost: test\r\nTransfer-Encoding: chunked\r\n\r\n1\r\nx\r\n0\r\n\r\n"[..],
        b"PUT /x HTTP/1.1\r\nHost: test\r\nTransfer-Encoding: chunked, gzip\r\n\r\n1\r\nx\r\n0\r\n\r\n",
        b"PUT /x HTTP/1.1\r\nHost: test\r\nTransfer-Encoding: gzip\r\n\r\nx",
    ] {
        let (head, body) = raw(&server, request);
        assert_eq!(head.status, Status::BAD_REQUEST, "{request:?}");
        assert!(String::from_utf8_lossy(&body).contains("Transfer-Encoding"));
    }
    assert_eq!(
        server.request_count(),
        0,
        "a refused framing reaches no route"
    );
}

#[test]
fn blank_lines_before_the_request_line_count_against_the_head_bound() {
    let server = Server::bind_with(
        "127.0.0.1:0",
        ServerOptions::default().with_max_head_size(64),
    )
    .expect("bind");
    let (head, _) = raw(&server, "\r\n".repeat(100).as_bytes());
    assert_eq!(head.status.code(), 431);
}

#[test]
fn a_head_trickled_past_the_read_timeout_is_closed() {
    let server = Server::bind_with(
        "127.0.0.1:0",
        ServerOptions::default().with_read_timeout(Duration::from_millis(400)),
    )
    .expect("bind");
    let mut stream = TcpStream::connect(server.address()).expect("connect");
    stream
        .write_all(b"GET / HTTP/1.1\r\nX-Slow: ")
        .expect("write");
    // One byte every 100 ms: each read is well inside the idle bound, and
    // only the head's deadline ends it.
    let mut trickle = stream.try_clone().expect("clone");
    let trickler = std::thread::spawn(move || {
        for _ in 0..50 {
            std::thread::sleep(Duration::from_millis(100));
            if trickle.write_all(b"x").is_err() {
                return;
            }
        }
    });
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    let started = Instant::now();
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "closed by the deadline, not by the trickle ending: {:?}",
        started.elapsed()
    );
    assert!(answer.is_empty(), "{answer:?}");
    assert_eq!(server.request_count(), 0);
    drop(stream);
    trickler.join().expect("trickler");
}

#[test]
fn a_peer_that_stops_reading_frees_its_connection_at_the_write_timeout() {
    // One connection at a time: the second is served only once the first,
    // whose peer never reads its answer, is closed by the write timeout.
    let server = Server::bind_with(
        "127.0.0.1:0",
        ServerOptions::default()
            .with_max_connections(1)
            .with_write_timeout(Duration::from_millis(300)),
    )
    .expect("bind");
    let root = memory_root();
    root.child_by_path("big.bin")
        .expect("child")
        .write_all_bytes(&vec![7_u8; 64 << 20])
        .expect("write");
    root.child_by_path("small.txt")
        .expect("child")
        .write_all_bytes(b"small")
        .expect("write");
    server.mount("/", root).expect("mount");

    let mut stalled = TcpStream::connect(server.address()).expect("connect");
    stalled
        .write_all(b"GET /big.bin HTTP/1.1\r\nHost: test\r\n\r\n")
        .expect("write");

    let deadline = Instant::now() + Duration::from_secs(10);
    let served = loop {
        assert!(
            Instant::now() < deadline,
            "the stalled connection was never freed"
        );
        let mut stream = TcpStream::connect(server.address()).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("timeout");
        if stream
            .write_all(b"GET /small.txt HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n")
            .is_err()
        {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        let mut answer = Vec::new();
        let _ = stream.read_to_end(&mut answer);
        if answer.is_empty() {
            // Closed unread: the one slot is still taken.
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        break parse_response(&answer).expect("an answer");
    };
    assert_eq!(served.0.status, Status::OK);
    assert_eq!(served.1, b"small");
    assert!(server.connections() >= 2);
    drop(stalled);
}

#[test]
fn a_connect_to_a_server_that_does_not_tunnel_is_405_naming_the_allowed_methods() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    let (status, headers, _) = raw_head(
        &server,
        b"CONNECT 127.0.0.1:1 HTTP/1.1\r\nHost: 127.0.0.1:1\r\n\r\n",
    );
    assert_eq!(status, Status::METHOD_NOT_ALLOWED);
    assert_eq!(
        headers.get("allow"),
        Some("GET, HEAD, PUT, DELETE, OPTIONS")
    );
}

#[test]
fn a_tunnel_quiet_both_ways_for_the_read_timeout_is_closed() {
    let upstream = std::net::TcpListener::bind("127.0.0.1:0").expect("an upstream");
    let target = upstream.local_addr().expect("its address");
    let server = Server::bind_with(
        "127.0.0.1:0",
        ServerOptions::default()
            .with_tunnel(true)
            .with_read_timeout(Duration::from_millis(200)),
    )
    .expect("bind");
    let mut client = TcpStream::connect(server.address()).expect("connect");
    write!(
        client,
        "CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n"
    )
    .expect("send");
    let (_accepted, _) = upstream.accept().expect("the tunnel dials upstream");
    let mut head = [0_u8; 39];
    client.read_exact(&mut head).expect("the tunnel's answer");
    assert_eq!(&head, b"HTTP/1.1 200 Connection Established\r\n\r\n");
    // Nothing moves either way: the tunnel closes rather than holding its
    // connection for ever.
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("a bound");
    let started = Instant::now();
    let mut rest = Vec::new();
    client.read_to_end(&mut rest).expect("the tunnel closed");
    assert!(rest.is_empty());
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn a_chunked_route_answer_is_framed_in_chunks_and_a_head_declares_a_length() {
    let server = Server::bind("127.0.0.1:0").expect("bind");
    server.route(None, "/chunked", |_| {
        Response::new(Status::OK)
            .with_header("transfer-encoding", "chunked")
            .map(|response| response.with_body("chunky"))
    });
    let bytes = raw_bytes(&server, &request_line("GET", "/chunked", ""));
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("transfer-encoding: chunked\r\n"), "{text}");
    assert!(!text.contains("content-length"), "{text}");
    assert!(text.ends_with("6\r\nchunky\r\n0\r\n\r\n"), "{text}");
    let (head, body) = parse_response(&bytes).expect("a chunked message");
    assert_eq!(body, b"chunky");
    assert_eq!(
        head.headers.get("content-length"),
        Some("6"),
        "decoded by the grammar"
    );
    let (status, headers, body) = raw_head(&server, &request_line("HEAD", "/chunked", ""));
    assert_eq!(status, Status::OK);
    assert_eq!(headers.get("content-length"), Some("6"));
    assert_eq!(headers.get("transfer-encoding"), None);
    assert!(body.is_empty());
}

#[test]
fn cut_body_at_writes_the_head_and_that_many_bytes_then_closes() {
    let server = served();
    server.inject("/rows.json", Fault::CutBodyAt(5), 1);
    let bytes = raw_bytes(&server, &request_line("GET", "/rows.json", ""));
    let error = parse_response(&bytes).expect_err("a severed body");
    assert!(error.to_string().contains("incomplete body"), "{error}");
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains(&format!("content-length: {}\r\n", ROWS.len())),
        "{text}"
    );
    assert!(text.ends_with("\r\n\r\n[1, 2"), "{text}");
    let recorded = server.requests();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].status, Status::OK);
    assert_eq!(
        &*get(&server, "/rows.json").bytes().expect("body"),
        ROWS,
        "once only"
    );
}

#[test]
fn keep_alive_serves_two_requests_on_one_connection() {
    let server = served();
    let mut stream = TcpStream::connect(server.address()).expect("connect");
    stream
        .write_all(b"GET /rows.json HTTP/1.1\r\nHost: test\r\n\r\n")
        .expect("write");
    let (head, body) = read_message(&mut stream);
    assert_eq!(head.status, Status::OK);
    assert_eq!(body, ROWS);
    assert_eq!(head.headers.get("connection"), None);
    stream
        .write_all(b"GET /dir/a.txt HTTP/1.1\r\nHost: test\r\n\r\n")
        .expect("write");
    let (head, body) = read_message(&mut stream);
    assert_eq!(head.status, Status::OK);
    assert_eq!(body, b"alpha");
    assert_eq!(server.connections(), 1);
    assert_eq!(server.request_count(), 2);
}

#[test]
fn connection_close_and_http_1_0_end_the_connection_after_the_answer() {
    let server = served();
    let (head, body) = raw(&server, &request_line("GET", "/rows.json", ""));
    assert_eq!(head.headers.get("connection"), Some("close"));
    assert_eq!(body, ROWS);
    let (head, body) = raw(&server, b"GET /rows.json HTTP/1.0\r\nHost: test\r\n\r\n");
    assert_eq!(head.version.as_str(), "HTTP/1.1");
    assert_eq!(head.headers.get("connection"), Some("close"));
    assert_eq!(body, ROWS);
    let mut stream = TcpStream::connect(server.address()).expect("connect");
    stream
        .write_all(b"GET /rows.json HTTP/1.0\r\nHost: test\r\nConnection: keep-alive\r\n\r\n")
        .expect("write");
    let (head, _) = read_message(&mut stream);
    assert_eq!(
        head.headers.get("connection"),
        None,
        "HTTP/1.0 asked to stay"
    );
    stream
        .write_all(b"GET /dir/a.txt HTTP/1.0\r\nHost: test\r\nConnection: close\r\n\r\n")
        .expect("write");
    let (_, body) = read_message(&mut stream);
    assert_eq!(body, b"alpha");
    assert_eq!(server.connections(), 3);
}

#[test]
fn keep_alive_off_closes_every_connection() {
    let server = Server::bind_with(
        "127.0.0.1:0",
        ServerOptions::default().with_keep_alive(false),
    )
    .expect("bind");
    server.mount("/", memory_root()).expect("mount");
    let (head, _) = raw(&server, b"GET /none HTTP/1.1\r\nHost: test\r\n\r\n");
    assert_eq!(head.status, Status::NOT_FOUND);
    assert_eq!(head.headers.get("connection"), Some("close"));
}

#[test]
fn a_chunked_request_body_is_decoded_and_its_trailer_folded_in() {
    let server = served();
    let (head, _) = raw(
        &server,
        b"PUT /up.bin HTTP/1.1\r\nHost: test\r\nConnection: close\r\nTransfer-Encoding: chunked\r\nTrailer: X-Sum\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\nX-Sum: 5\r\n\r\n",
    );
    assert_eq!(head.status, Status::CREATED);
    assert_eq!(&*get(&server, "/up.bin").bytes().expect("body"), b"abcde");
    let recorded = &server.requests()[0];
    assert_eq!(recorded.method, Method::Put);
    assert_eq!(recorded.body_len, 5);
    assert_eq!(recorded.headers.get("content-length"), Some("5"));
    assert_eq!(recorded.headers.get("transfer-encoding"), None);
    assert_eq!(recorded.headers.get("x-sum"), Some("5"));
}

#[test]
fn expect_100_continue_is_acknowledged_before_the_body_is_read() {
    let server = served();
    let mut stream = TcpStream::connect(server.address()).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    stream
        .write_all(
            b"PUT /expected.bin HTTP/1.1\r\nHost: test\r\nExpect: 100-continue\r\nContent-Length: 4\r\n\r\n",
        )
        .expect("write");
    let mut interim = [0_u8; 25];
    stream.read_exact(&mut interim).expect("the interim answer");
    assert_eq!(&interim, b"HTTP/1.1 100 Continue\r\n\r\n");
    stream.write_all(b"body").expect("write");
    let (head, _) = read_message(&mut stream);
    assert_eq!(head.status, Status::CREATED);
    assert_eq!(
        &*get(&server, "/expected.bin").bytes().expect("body"),
        b"body"
    );
}
