//! `rust/src/http/server/forwarded.rs`: the base a request was made under
//! behind a proxy - the public URL, the forwarded fields a trusted peer is
//! believed for, the path prefix, the `Host` - and the `IpNetwork` and
//! `ForwardedHeader` vocabularies it is stated in, driven over raw
//! `TcpStream` writes.

use super::*;
use yggdryl::Url;
use yggdryl::http::{ForwardedHeader, IpNetwork};

/// A server with `options`, routing `/echo` for every method to a text of
/// the request's URL.
fn echoing(options: ServerOptions) -> Server {
    let server = Server::bind_with("127.0.0.1:0", options).expect("bind");
    server.route(None, "/echo", |request| {
        Ok(Response::new(Status::OK).with_text(&request.url().to_string()))
    });
    server
}

/// The options trusting the loopback peer every test connects from.
fn trusting_loopback() -> ServerOptions {
    ServerOptions::default()
        .with_trusted_proxies(["127.0.0.1", "::1"])
        .expect("loopback networks")
}

/// The text a raw `GET` of `target` with `fields` answers; every request
/// states `Host: test`.
fn echoed(server: &Server, target: &str, fields: &str) -> String {
    let (head, body) = raw(server, &request_line("GET", target, fields));
    assert_eq!(
        head.status,
        Status::OK,
        "{}",
        String::from_utf8_lossy(&body)
    );
    String::from_utf8(body).expect("UTF-8")
}

/// The client the last recorded request names.
fn last_client(server: &Server) -> Option<std::net::IpAddr> {
    server
        .requests()
        .last()
        .and_then(|recorded| recorded.client)
}

fn address(text: &str) -> std::net::IpAddr {
    text.parse().expect("an address")
}

#[test]
fn an_untrusted_peers_forwarded_fields_are_ignored() {
    // Every field listed, and none believed: the peer is not a proxy.
    let server = echoing(ServerOptions::default().with_forwarded_headers(ForwardedHeader::ALL));
    let text = echoed(
        &server,
        "/echo",
        "X-Forwarded-Proto: https\r\nX-Forwarded-Host: pub.example\r\nX-Forwarded-For: 203.0.113.9\r\nX-Forwarded-Prefix: /olap\r\nForwarded: for=203.0.113.9;proto=https\r\n",
    );
    assert_eq!(text, "http://test/echo");
    let recorded = server.requests();
    assert_eq!(recorded.len(), 1);
    assert_eq!(
        recorded[0].peer.map(|peer| peer.ip()),
        Some(address("127.0.0.1"))
    );
    assert_eq!(recorded[0].client, Some(address("127.0.0.1")));
}

#[test]
fn by_default_a_trusted_peer_states_only_the_client_and_the_scheme() {
    // The documented nginx setup: the proxy sets `Host`, overwrites
    // `X-Forwarded-Proto` and appends to `X-Forwarded-For`; everything else
    // below is the client's own, passed through.
    let server = echoing(
        trusting_loopback()
            .with_path_prefix("/olap")
            .expect("a prefix"),
    );
    assert_eq!(
        server.options().forwarded_headers(),
        ForwardedHeader::DEFAULT
    );
    let text = echoed(
        &server,
        "/olap/echo",
        "Forwarded: for=6.6.6.6;host=evil.example;proto=http\r\nX-Forwarded-Host: evil.example\r\nX-Forwarded-Port: 1234\r\nX-Forwarded-Prefix: /evil\r\nX-Forwarded-Proto: https\r\nX-Forwarded-For: 6.6.6.6, 203.0.113.9\r\n",
    );
    assert_eq!(text, "https://test/olap/echo");
    assert_eq!(last_client(&server), Some(address("203.0.113.9")));

    // A malformed `Forwarded` beside them changes nothing either.
    let text = echoed(
        &server,
        "/olap/echo",
        "Forwarded: x\r\nX-Forwarded-Proto: https\r\nX-Forwarded-For: 203.0.113.9\r\n",
    );
    assert_eq!(text, "https://test/olap/echo");
    assert_eq!(last_client(&server), Some(address("203.0.113.9")));

    // An empty list reads nothing: the trusted peer is the client.
    let server = echoing(trusting_loopback().with_forwarded_headers([]));
    let text = echoed(
        &server,
        "/echo",
        "X-Forwarded-Proto: https\r\nX-Forwarded-For: 203.0.113.9\r\n",
    );
    assert_eq!(text, "http://test/echo");
    assert_eq!(last_client(&server), Some(address("127.0.0.1")));
}

#[test]
fn a_trusted_peer_is_believed_for_every_field_the_options_list() {
    let server = echoing(trusting_loopback().with_forwarded_headers(ForwardedHeader::ALL));
    let text = echoed(
        &server,
        "/echo?x=1",
        "X-Forwarded-Proto: https\r\nX-Forwarded-Host: pub.example\r\nX-Forwarded-Port: 8443\r\nX-Forwarded-Prefix: /olap\r\nX-Forwarded-For: 203.0.113.9, 127.0.0.1\r\n",
    );
    assert_eq!(text, "https://pub.example:8443/olap/echo?x=1");
    assert_eq!(last_client(&server), Some(address("203.0.113.9")));

    // The scheme's own port is not spelled, and a host that carries a port
    // keeps it.
    let text = echoed(
        &server,
        "/echo",
        "X-Forwarded-Proto: HTTPS\r\nX-Forwarded-Host: pub.example\r\nX-Forwarded-Port: 443\r\n",
    );
    assert_eq!(text, "https://pub.example/echo");
    let text = echoed(
        &server,
        "/echo",
        "X-Forwarded-Host: pub.example:9000\r\nX-Forwarded-Port: 8443\r\n",
    );
    assert_eq!(text, "http://pub.example:9000/echo");

    // A scheme that is neither, or a host that is no authority, is left
    // unread rather than served under.
    let text = echoed(
        &server,
        "/echo",
        "X-Forwarded-Proto: gopher\r\nX-Forwarded-Host: not a host\r\n",
    );
    assert_eq!(text, "http://test/echo");
    let text = echoed(&server, "/echo", "X-Forwarded-Host: user@pub.example\r\n");
    assert_eq!(text, "http://test/echo");

    // Only the fields listed: `X-Forwarded-Host` alone states the host,
    // and the client stays the peer.
    let server =
        echoing(trusting_loopback().with_forwarded_headers([ForwardedHeader::XForwardedHost]));
    let text = echoed(
        &server,
        "/echo",
        "X-Forwarded-Proto: https\r\nX-Forwarded-Host: pub.example\r\nX-Forwarded-For: 203.0.113.9\r\n",
    );
    assert_eq!(text, "http://pub.example/echo");
    assert_eq!(last_client(&server), Some(address("127.0.0.1")));
}

#[test]
fn an_appended_x_forwarded_list_is_read_where_the_trusted_proxy_wrote_it() {
    let options = ServerOptions::default()
        .with_trusted_proxies(["127.0.0.0/8", "10.0.0.0/8"])
        .expect("networks")
        .with_forwarded_headers(ForwardedHeader::ALL);
    let server = echoing(options);

    // One proxy appending to every list: its members sit at the client's
    // position in `X-Forwarded-For`, the client's own before them.
    let text = echoed(
        &server,
        "/echo",
        "X-Forwarded-Host: evil.example, data.example.com\r\nX-Forwarded-Proto: http, https\r\nX-Forwarded-For: 198.51.100.7, 203.0.113.9\r\n",
    );
    assert_eq!(text, "https://data.example.com/echo");
    assert_eq!(last_client(&server), Some(address("203.0.113.9")));

    // Two proxies appending: the edge that saw the client wrote the public
    // host and scheme, the inner one its own after them.
    let text = echoed(
        &server,
        "/echo",
        "X-Forwarded-Host: data.example.com, inner.local\r\nX-Forwarded-Proto: https, http\r\nX-Forwarded-For: 203.0.113.9, 10.0.0.7\r\n",
    );
    assert_eq!(text, "https://data.example.com/echo");
    assert_eq!(last_client(&server), Some(address("203.0.113.9")));

    // A list that does not line up with `X-Forwarded-For` is read at its
    // last member, the one the proxy nearest this server wrote - never its
    // first, which the client sent.
    let text = echoed(
        &server,
        "/echo",
        "X-Forwarded-Host: evil.example, data.example.com\r\nX-Forwarded-Proto: http, https\r\nX-Forwarded-For: 203.0.113.9\r\n",
    );
    assert_eq!(text, "https://data.example.com/echo");
    let text = echoed(
        &server,
        "/echo",
        "X-Forwarded-Host: evil.example, data.example.com\r\nX-Forwarded-Proto: http, https\r\n",
    );
    assert_eq!(text, "https://data.example.com/echo");
}

#[test]
fn a_forwarded_chain_is_walked_from_the_right_past_the_trusted_proxies() {
    let options = ServerOptions::default()
        .with_trusted_proxies(["127.0.0.0/8", "10.0.0.0/8"])
        .expect("networks")
        .with_forwarded_headers(ForwardedHeader::ALL);
    let server = echoing(options);

    // The edge proxy at 10.0.0.7 saw the client under the public host.
    let text = echoed(
        &server,
        "/echo",
        "Forwarded: for=203.0.113.9;host=\"pub.example:8443\";proto=https, for=10.0.0.7;proto=http\r\n",
    );
    assert_eq!(text, "https://pub.example:8443/echo");
    assert_eq!(last_client(&server), Some(address("203.0.113.9")));

    // An untrusted hop ends the walk: what the proxies before it stated is
    // not believed, and it is the client.
    let text = echoed(
        &server,
        "/echo",
        "Forwarded: for=203.0.113.9;host=pub.example;proto=https, for=\"[2001:db8::1]:4711\";proto=https, for=10.0.0.7\r\n",
    );
    assert_eq!(text, "https://test/echo");
    assert_eq!(last_client(&server), Some(address("2001:db8::1")));

    // `unknown` names no client, so the peer is the client; an IPv4-mapped
    // IPv6 proxy address is the IPv4 address it maps.
    let text = echoed(
        &server,
        "/echo",
        "Forwarded: for=unknown;proto=https, for=\"[::ffff:10.0.0.7]\"\r\n",
    );
    assert_eq!(text, "https://test/echo");
    assert_eq!(last_client(&server), Some(address("127.0.0.1")));

    // `X-Forwarded-For` is walked the same way when there is no `Forwarded`.
    echoed(
        &server,
        "/echo",
        "X-Forwarded-For: 198.51.100.4, 203.0.113.9, 10.0.0.7\r\n",
    );
    assert_eq!(last_client(&server), Some(address("203.0.113.9")));

    // `Forwarded` states the fact before the `X-Forwarded-*` field stating
    // the same one, and a fact it leaves out is read from the other.
    let text = echoed(
        &server,
        "/echo",
        "Forwarded: for=203.0.113.9;proto=https\r\nX-Forwarded-Proto: http\r\nX-Forwarded-Host: pub.example\r\nX-Forwarded-For: 198.51.100.4\r\n",
    );
    assert_eq!(text, "https://pub.example/echo");
    assert_eq!(last_client(&server), Some(address("203.0.113.9")));

    // Listed alone, `Forwarded` is the whole of what is read.
    let server = echoing(
        ServerOptions::default()
            .with_trusted_proxies(["127.0.0.1"])
            .expect("a network")
            .with_forwarded_headers([ForwardedHeader::Forwarded]),
    );
    let text = echoed(
        &server,
        "/echo",
        "Forwarded: for=203.0.113.9;proto=https\r\nX-Forwarded-Host: evil.example\r\nX-Forwarded-For: 6.6.6.6\r\n",
    );
    assert_eq!(text, "https://test/echo");
    assert_eq!(last_client(&server), Some(address("203.0.113.9")));
}

#[test]
fn a_forwarded_prefix_goes_before_the_servers_own_and_one_no_path_is_left_unread() {
    let options = trusting_loopback()
        .with_path_prefix("/b")
        .expect("a prefix")
        .with_forwarded_headers([
            ForwardedHeader::XForwardedFor,
            ForwardedHeader::XForwardedProto,
            ForwardedHeader::XForwardedPrefix,
        ]);
    let server = echoing(options);
    // What the proxy stripped, then what this server stripped.
    assert_eq!(
        echoed(&server, "/b/echo", "X-Forwarded-Prefix: /a\r\n"),
        "http://test/a/b/echo"
    );
    assert_eq!(
        echoed(&server, "/echo", "X-Forwarded-Prefix: /a/\r\n"),
        "http://test/a/echo"
    );
    assert_eq!(
        echoed(&server, "/b/echo", "X-Forwarded-Prefix: /\r\n"),
        "http://test/b/echo"
    );
    // A prefix holding a byte the path grammar refuses is left unread, as a
    // host that is no authority is, and the request is answered.
    for prefix in ["/a b", "/a\"b", "/a<b", "/a?b", "/a#b"] {
        assert_eq!(
            echoed(
                &server,
                "/b/echo",
                &format!("X-Forwarded-Prefix: {prefix}\r\n")
            ),
            "http://test/b/echo",
            "{prefix:?}"
        );
    }
}

#[test]
fn the_public_url_wins_over_the_forwarded_fields_and_the_host() {
    let options = trusting_loopback()
        .with_forwarded_headers(ForwardedHeader::ALL)
        .with_public_url(Url::from_str("https://data.example.com/olap").expect("a URL"));
    let server = echoing(options);
    let text = echoed(
        &server,
        "/echo?x=1",
        "X-Forwarded-Proto: http\r\nX-Forwarded-Host: other.example\r\nX-Forwarded-Prefix: /elsewhere\r\n",
    );
    assert_eq!(text, "https://data.example.com/olap/echo?x=1");
    assert_eq!(
        server.public_url_of("/echo").expect("a URL").to_string(),
        "https://data.example.com/olap/echo"
    );
    assert_eq!(
        server.public_url_of("echo").expect("a URL").to_string(),
        "https://data.example.com/olap/echo"
    );
    assert!(
        server
            .url_of("/echo")
            .expect("a URL")
            .to_string()
            .starts_with("http://127.0.0.1:")
    );
    let plain = echoing(ServerOptions::default());
    assert_eq!(
        plain.public_url_of("/echo").expect("a URL"),
        plain.url_of("/echo").expect("a URL")
    );
}

#[test]
fn a_path_prefix_comes_off_before_routing_and_goes_back_on_the_url() {
    let options = ServerOptions::default()
        .with_path_prefix("/olap/")
        .expect("a prefix");
    assert_eq!(options.path_prefix(), Some("/olap"));
    let server = echoing(options);
    server.route(None, "/x y", |request| {
        Ok(Response::new(Status::OK).with_text(&request.url().to_string()))
    });
    assert_eq!(
        echoed(&server, "/olap/echo?x=1", ""),
        "http://test/olap/echo?x=1"
    );
    assert_eq!(echoed(&server, "/echo", ""), "http://test/echo");
    assert_eq!(echoed(&server, "/olap/x%20y", ""), "http://test/olap/x%20y");

    // A prefix holding a byte a URL escapes goes back on as it was sent.
    let spaced = echoing(
        ServerOptions::default()
            .with_path_prefix("/my olap")
            .expect("a prefix"),
    );
    assert_eq!(
        echoed(&spaced, "/my%20olap/echo?x=1", ""),
        "http://test/my%20olap/echo?x=1"
    );
    let (head, _) = raw(&server, &request_line("GET", "/olapx/echo", ""));
    assert_eq!(head.status, Status::NOT_FOUND);
    let (head, _) = raw(&server, &request_line("GET", "/olap", ""));
    assert_eq!(
        head.status,
        Status::NOT_FOUND,
        "the prefix alone is the root"
    );
    let recorded = server.requests();
    assert_eq!(recorded[0].path, "/echo", "the log holds the routed path");
    assert_eq!(recorded[0].target, "/olap/echo?x=1");
    assert_eq!(
        ServerOptions::default()
            .with_path_prefix("/")
            .expect("the root")
            .path_prefix(),
        None
    );
}

#[test]
fn a_listing_states_urls_under_the_base_the_request_was_made_under() {
    let options = trusting_loopback()
        .with_forwarded_headers(ForwardedHeader::ALL)
        .with_path_prefix("/files")
        .expect("a prefix");
    let server = Server::bind_with("127.0.0.1:0", options).expect("bind");
    let root = memory_root();
    root.child_by_path("dir/a.txt")
        .expect("child")
        .write_all_bytes(b"alpha")
        .expect("write");
    server.mount("/", root).expect("mount");
    let (head, body) = raw(
        &server,
        &request_line(
            "GET",
            "/files/dir",
            "X-Forwarded-Proto: https\r\nX-Forwarded-Host: pub.example\r\n",
        ),
    );
    assert_eq!(head.status, Status::OK);
    let text = String::from_utf8(body).expect("UTF-8");
    assert!(
        text.contains("\"url\":\"https://pub.example/files/dir/a.txt\""),
        "{text}"
    );
    let (_, body) = raw(&server, &request_line("GET", "/dir", ""));
    let text = String::from_utf8(body).expect("UTF-8");
    assert!(text.contains("\"url\":\"http://test/dir/a.txt\""), "{text}");
}

#[test]
fn forwarded_header_reads_the_six_field_names_however_cased() {
    for header in ForwardedHeader::ALL {
        assert_eq!(
            ForwardedHeader::from_str(&header.as_str().to_ascii_lowercase()).expect("a name"),
            header
        );
        assert_eq!(header.to_string(), header.as_str());
    }
    assert_eq!(
        " X-FORWARDED-PREFIX "
            .parse::<ForwardedHeader>()
            .expect("a name"),
        ForwardedHeader::XForwardedPrefix
    );
    assert!(matches!(
        ForwardedHeader::from_str("X-Real-IP"),
        Err(Error::Parse {
            target: "forwarded header",
            ..
        })
    ));
    // The options keep one of each, in the order they are read.
    let options = ServerOptions::default().with_forwarded_headers([
        ForwardedHeader::XForwardedHost,
        ForwardedHeader::Forwarded,
        ForwardedHeader::XForwardedHost,
    ]);
    assert_eq!(
        options.forwarded_headers(),
        [ForwardedHeader::Forwarded, ForwardedHeader::XForwardedHost]
    );
}

#[test]
fn trusted_proxies_are_refused_by_name_where_they_are_set() {
    for network in [
        "300.1.1.1",
        "10.0.0.0/33",
        "pub.example",
        "::ffff:10.0.0.0/8",
    ] {
        let refused = ServerOptions::default().with_trusted_proxies([network]);
        assert!(
            matches!(
                refused,
                Err(Error::Parse {
                    target: "ip network",
                    ..
                })
            ),
            "{network}"
        );
    }
    let options = ServerOptions::default()
        .with_trusted_proxies(["10.0.0.5", "[fd00::]/8"])
        .expect("networks");
    assert_eq!(
        options
            .trusted_proxies()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["10.0.0.5/32", "fd00::/8"]
    );
}

#[test]
fn ip_network_reads_addresses_and_cidr_and_matches_a_mapped_v6_address() {
    let network = IpNetwork::from_str("10.0.0.0/8").expect("a network");
    assert_eq!(network.bits(), 8);
    assert!(network.contains(address("10.255.0.1")));
    assert!(network.contains(address("::ffff:10.1.2.3")));
    assert!(!network.contains(address("11.0.0.1")));
    let mapped = IpNetwork::from_str("::ffff:10.0.0.5").expect("a mapped address");
    assert_eq!(mapped.to_string(), "10.0.0.5/32");
    assert_eq!(mapped.address(), address("10.0.0.5"));
    let six = IpNetwork::from_str("[2001:db8::]/32").expect("a bracketed network");
    assert!(six.contains(address("2001:db8:1::1")));
    assert!(!six.contains(address("2001:db9::1")));
    assert_eq!(
        "::1".parse::<IpNetwork>().expect("an address").to_string(),
        "::1/128"
    );
    assert!(IpNetwork::from_str("pub.example").is_err());
}

#[test]
fn a_mapped_v6_network_counts_the_96_bits_of_the_mapping() {
    // `::ffff:10.0.0.0/104` is IPv6's spelling of `10.0.0.0/8`.
    let network = IpNetwork::from_str("::ffff:10.0.0.0/104").expect("a mapped network");
    assert_eq!(network.to_string(), "10.0.0.0/8");
    assert!(network.contains(address("10.20.30.40")));
    assert!(network.contains(address("::ffff:10.20.30.40")));
    assert!(!network.contains(address("11.0.0.1")));
    // The whole mapped space is every IPv4 address, and no IPv6 one.
    let every = IpNetwork::from_str("[::ffff:0:0]/96").expect("the mapped space");
    assert_eq!(every.to_string(), "0.0.0.0/0");
    assert!(every.contains(address("203.0.113.9")));
    assert!(!every.contains(address("2001:db8::1")));
    assert_eq!(
        IpNetwork::from_str("::ffff:10.0.0.5/128")
            .expect("one mapped address")
            .to_string(),
        "10.0.0.5/32"
    );
    // Shorter than the mapping names IPv6 space no IPv4 address maps into,
    // and longer than 128 bits is no prefix at all.
    for refused in [
        "::ffff:10.0.0.0/8",
        "::ffff:10.0.0.0/95",
        "::ffff:10.0.0.0/129",
    ] {
        match IpNetwork::from_str(refused) {
            Err(Error::Parse { target, reason, .. }) => {
                assert_eq!(target, "ip network");
                assert!(reason.contains("prefix length"), "{reason}");
            }
            other => panic!("{refused}: expected a refusal, got {other:?}"),
        }
    }
}

#[cfg(feature = "http3")]
#[test]
fn alt_svc_is_not_advertised_under_a_public_url_or_to_a_trusted_proxy() {
    let plain = Server::bind_with("127.0.0.1:0", ServerOptions::default().with_http3(true))
        .expect("bind with HTTP/3");
    let (head, _) = raw(&plain, &request_line("GET", "/nothing", ""));
    assert!(
        head.headers.get("alt-svc").is_some(),
        "advertised by default"
    );

    let public = Server::bind_with(
        "127.0.0.1:0",
        ServerOptions::default()
            .with_http3(true)
            .with_public_url(Url::from_str("https://data.example.com/").expect("a URL")),
    )
    .expect("bind with HTTP/3");
    let (head, _) = raw(&public, &request_line("GET", "/nothing", ""));
    assert_eq!(head.headers.get("alt-svc"), None);

    let proxied = Server::bind_with("127.0.0.1:0", trusting_loopback().with_http3(true))
        .expect("bind with HTTP/3");
    let (head, _) = raw(&proxied, &request_line("GET", "/nothing", ""));
    assert_eq!(head.headers.get("alt-svc"), None);
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::http_server_forwarded::{elements_of, node_address};

    #[test]
    fn forwarded_elements_split_at_commas_and_semicolons_outside_quotes() {
        let elements = elements_of(
            "for=192.0.2.60;proto=http;by=203.0.113.43, For=\"[2001:db8:cafe::17]:4711\";host=\"a,b;c\\\"d\"",
        );
        assert_eq!(
            elements,
            vec![
                vec![
                    ("for".to_owned(), "192.0.2.60".to_owned()),
                    ("proto".to_owned(), "http".to_owned()),
                    ("by".to_owned(), "203.0.113.43".to_owned()),
                ],
                vec![
                    ("for".to_owned(), "[2001:db8:cafe::17]:4711".to_owned()),
                    ("host".to_owned(), "a,b;c\"d".to_owned()),
                ],
            ]
        );
        assert!(elements_of("").is_empty());
        assert_eq!(elements_of("for=1.2.3.4;;junk;=x").len(), 1);
    }

    #[test]
    fn a_node_names_an_address_or_none() {
        let address = |text: &str| text.parse::<std::net::IpAddr>().unwrap();
        assert_eq!(node_address("192.0.2.60"), Some(address("192.0.2.60")));
        assert_eq!(node_address("192.0.2.60:4711"), Some(address("192.0.2.60")));
        assert_eq!(
            node_address("[2001:db8:cafe::17]:4711"),
            Some(address("2001:db8:cafe::17"))
        );
        assert_eq!(
            node_address("\"[2001:db8::1]\""),
            Some(address("2001:db8::1"))
        );
        assert_eq!(node_address("::ffff:10.0.0.7"), Some(address("10.0.0.7")));
        assert_eq!(node_address("unknown"), None);
        assert_eq!(node_address("_hidden"), None);
        assert_eq!(node_address("pub.example"), None);
    }
}
