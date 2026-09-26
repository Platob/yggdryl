//! `rust/src/http/server/h3.rs`: HTTP/3 on the UDP port beside the TCP one,
//! under a certificate the server signs for itself, advertised in `Alt-Svc`.

use super::*;
use yggdryl::http::HttpVersion;

fn answering_http3() -> Server {
    let server = Server::bind_with("127.0.0.1:0", ServerOptions::default().with_http3(true))
        .expect("bind with HTTP/3");
    let root = memory_root();
    root.child_by_path("rows.json")
        .expect("child")
        .write_all_bytes(ROWS)
        .expect("write");
    server.mount("/", root).expect("mount");
    server
}

fn over_http3(server: &Server) -> Session {
    let bundle = std::env::temp_dir().join(format!(
        "yggdryl-server-h3-{}-{}.pem",
        std::process::id(),
        server.port()
    ));
    std::fs::write(&bundle, server.certificate().expect("a certificate")).expect("a bundle");
    Session::with_options(
        HttpOptions::default()
            .with_ca_bundle(bundle)
            .with_http_version(Some(HttpVersion::Http3)),
    )
    .expect("a session")
}

#[test]
fn http1_answers_advertise_the_port_http3_is_answered_on() {
    let server = answering_http3();
    let response = get(&server, "/rows.json");
    assert_eq!(
        response.headers().get("alt-svc"),
        Some(format!("h3=\":{}\"; ma=86400", server.port()).as_str())
    );
    let plain = served();
    assert_eq!(get(&plain, "/rows.json").headers().get("alt-svc"), None);
}

#[test]
fn a_mounted_leaf_answers_a_range_over_quic() {
    let server = answering_http3();
    let response = over_http3(&server)
        .get(&format!("https://127.0.0.1:{}/rows.json", server.port()))
        .unwrap()
        .with_header("Range", "bytes=1-3")
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(response.version(), HttpVersion::Http3);
    assert_eq!(response.status().code(), 206);
    assert_eq!(&*response.bytes().unwrap(), &ROWS[1..=3]);
    assert_eq!(
        response.headers().get("content-range"),
        Some(format!("bytes 1-3/{}", ROWS.len()).as_str())
    );
}

#[test]
fn a_put_over_quic_writes_the_mount() {
    let server = answering_http3();
    let session = over_http3(&server);
    let target = format!("https://127.0.0.1:{}/new.txt", server.port());
    let created = session
        .put(&target, b"fresh".to_vec())
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(created.status().code(), 201);
    let read = session.get(&target).unwrap().send().unwrap();
    assert_eq!(read.text().unwrap(), "fresh");
}
