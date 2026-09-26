//! `rust/src/http/server/trace.rs`: the exchange trace - the numbering
//! across connections, what a connection that sends nothing writes, the
//! answer's file created as the exchange begins, and an answer appended in
//! pieces landing whole.

use super::*;

#[test]
fn two_requests_on_two_connections_are_traced_0000_and_0001() {
    let (dir, trace) = trace_folder("two-connections");
    let server =
        Server::bind_with("127.0.0.1:0", ServerOptions::default().with_trace(trace)).expect("bind");
    server.mount("/", memory_root()).expect("mount");
    let _ = raw_bytes(&server, &request_line("GET", "/none", ""));
    let _ = raw_bytes(&server, &request_line("GET", "/none", ""));
    wait_for(&dir.join("0001-response.http"), |bytes| !bytes.is_empty());
    assert!(dir.join("0000-request.http").exists());
    assert!(dir.join("0000-response.http").exists());
    assert!(dir.join("0001-request.http").exists());
    assert!(!dir.join("0002-request.http").exists());
}

#[test]
fn a_connection_closed_without_a_byte_writes_no_trace_files() {
    let (dir, trace) = trace_folder("silent");
    let server =
        Server::bind_with("127.0.0.1:0", ServerOptions::default().with_trace(trace)).expect("bind");
    server.mount("/", memory_root()).expect("mount");
    drop(TcpStream::connect(server.address()).expect("connect"));
    // A real request afterwards still opens exchange 0000: nothing was
    // numbered for the connection that sent no byte.
    let _ = raw_bytes(&server, &request_line("GET", "/none", ""));
    wait_for(&dir.join("0000-response.http"), |bytes| !bytes.is_empty());
    assert!(!dir.join("0001-request.http").exists());
}

#[test]
fn a_request_closed_before_its_answer_leaves_its_bytes_and_an_empty_answer() {
    let (dir, trace) = trace_folder("closed-before-answer");
    let server =
        Server::bind_with("127.0.0.1:0", ServerOptions::default().with_trace(trace)).expect("bind");
    server.mount("/", memory_root()).expect("mount");
    server.inject("/gone", Fault::CloseBeforeAnswer, 1);
    let sent = request_line("GET", "/gone", "");
    let answer = raw_bytes(&server, &sent);
    assert!(
        answer.is_empty(),
        "the fault closed the connection unanswered"
    );
    let request = wait_for(&dir.join("0000-request.http"), |bytes| {
        bytes.len() >= sent.len()
    });
    assert_eq!(request, sent);
    assert_eq!(
        std::fs::read(dir.join("0000-response.http")).expect("the answer's file"),
        b"",
        "created as the exchange began, and nothing was answered"
    );
}

#[test]
fn a_written_answer_past_the_spill_lands_whole_in_its_file() {
    let (dir, trace) = trace_folder("spill");
    let server =
        Server::bind_with("127.0.0.1:0", ServerOptions::default().with_trace(trace)).expect("bind");
    let payload: Arc<[u8]> = (0..200_000_u32).map(|at| (at % 251) as u8).collect();
    let served = Arc::clone(&payload);
    server.route(Some(Method::Get), "/big", move |_| {
        let payload = Arc::clone(&served);
        Ok(Response::new(Status::OK).with_writer(move |body| {
            for piece in payload.chunks(10_000) {
                body.write_all(piece)?;
            }
            Ok(())
        }))
    });
    let answer = raw_bytes(&server, &request_line("GET", "/big", ""));
    let traced = wait_for(&dir.join("0000-response.http"), |bytes| {
        bytes.len() >= answer.len()
    });
    assert_eq!(traced, answer, "every piece appended, in order");
    let (_, body) = parse_response(&traced).expect("the traced answer parses");
    assert_eq!(&body[..], &payload[..]);
}
