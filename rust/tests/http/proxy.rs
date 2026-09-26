//! `rust/src/http/proxy.rs`: which proxy the environment names for a
//! request, read the way curl and Python's `requests` read it.
//!
//! Every case reads a map standing in for the environment, so no test
//! changes the process's own under the suites running beside it.

use yggdryl::internals::http_proxy::environment_proxy;

fn chosen(url: &str, variables: &[(&str, &str)]) -> Option<String> {
    environment_proxy(url, variables).expect("a URL")
}

#[test]
fn each_scheme_reads_its_own_variable_then_all_proxy() {
    let variables = [
        ("http_proxy", "http://plain:3128"),
        ("https_proxy", "http://secure:3128"),
        ("all_proxy", "http://any:3128"),
    ];
    assert_eq!(
        chosen("http://api.example.com/", &variables).as_deref(),
        Some("http://plain:3128")
    );
    assert_eq!(
        chosen("https://api.example.com/", &variables).as_deref(),
        Some("http://secure:3128")
    );
    // An https URL never falls back on the http variable, only on all_proxy.
    let plain_only = [("http_proxy", "http://plain:3128")];
    assert_eq!(chosen("https://api.example.com/", &plain_only), None);
    let all_only = [("ALL_PROXY", "socks5h://any:1080")];
    assert_eq!(
        chosen("https://api.example.com/", &all_only).as_deref(),
        Some("socks5h://any:1080")
    );
    assert_eq!(chosen("http://api.example.com/", &[]), None);
}

#[test]
fn the_lower_case_variable_wins_and_an_empty_one_is_unset() {
    let both = [
        ("https_proxy", "http://lower:1"),
        ("HTTPS_PROXY", "http://upper:2"),
    ];
    assert_eq!(
        chosen("https://h.example/", &both).as_deref(),
        Some("http://lower:1")
    );
    let upper = [("HTTPS_PROXY", "http://upper:2")];
    assert_eq!(
        chosen("https://h.example/", &upper).as_deref(),
        Some("http://upper:2")
    );
    let empty = [("https_proxy", "  "), ("HTTPS_PROXY", "http://upper:2")];
    assert_eq!(
        chosen("https://h.example/", &empty).as_deref(),
        Some("http://upper:2")
    );
}

#[test]
fn no_proxy_matches_hosts_on_a_label_boundary() {
    let proxy = ("https_proxy", "http://proxy:3128");
    for (list, host, direct) in [
        ("example.com", "example.com", true),
        ("example.com", "api.example.com", true),
        (".example.com", "api.example.com", true),
        ("*.example.com", "api.example.com", true),
        ("example.com", "badexample.com", false),
        ("EXAMPLE.com", "Api.Example.COM", true),
        ("other.org, example.com", "api.example.com", true),
        ("other.org example.com", "api.example.com", true),
        ("*", "anything.at.all", true),
        ("", "api.example.com", false),
    ] {
        let variables = [proxy, ("no_proxy", list)];
        let answer = chosen(&format!("https://{host}/"), &variables);
        assert_eq!(answer.is_none(), direct, "{list:?} over {host}");
    }
}

#[test]
fn no_proxy_reads_ports_addresses_and_networks() {
    let proxy = ("http_proxy", "http://proxy:3128");
    for (list, url, direct) in [
        ("example.com:8080", "http://example.com:8080/", true),
        ("example.com:8080", "http://example.com/", false),
        ("example.com:80", "http://example.com/", true),
        ("10.0.0.0/8", "http://10.1.2.3/", true),
        ("10.0.0.0/8", "http://11.1.2.3/", false),
        ("192.168.1.7", "http://192.168.1.7/", true),
        ("192.168.1.7", "http://192.168.1.8/", false),
        ("::1", "http://[::1]:8080/", true),
        ("[::1]:8080", "http://[::1]:8080/", true),
        ("fd00::/8", "http://[fd12::1]/", true),
        ("fd00::/8", "http://10.1.2.3/", false),
    ] {
        let variables = [proxy, ("NO_PROXY", list)];
        assert_eq!(
            chosen(url, &variables).is_none(),
            direct,
            "{list:?} over {url}"
        );
    }
}

#[test]
fn under_cgi_the_upper_case_http_proxy_is_not_read() {
    // httpoxy: a server sets `HTTP_PROXY` from its request's `Proxy` header.
    let cgi = [
        ("REQUEST_METHOD", "GET"),
        ("HTTP_PROXY", "http://attacker:8080"),
    ];
    assert_eq!(chosen("http://api.example.com/", &cgi), None);
    let lower = [
        ("REQUEST_METHOD", "GET"),
        ("http_proxy", "http://mine:3128"),
    ];
    assert_eq!(
        chosen("http://api.example.com/", &lower).as_deref(),
        Some("http://mine:3128")
    );
    // Outside CGI, and for the https variable, the upper case still counts.
    assert_eq!(
        chosen(
            "http://api.example.com/",
            &[("HTTP_PROXY", "http://upper:1")]
        )
        .as_deref(),
        Some("http://upper:1")
    );
    let secure = [("REQUEST_METHOD", "GET"), ("HTTPS_PROXY", "http://upper:2")];
    assert_eq!(
        chosen("https://api.example.com/", &secure).as_deref(),
        Some("http://upper:2")
    );
}
