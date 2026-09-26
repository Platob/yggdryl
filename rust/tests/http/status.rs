//! `rust/src/http/status.rs`.

use yggdryl::Error;
use yggdryl::http::Status;

#[test]
fn a_code_outside_100_to_599_is_refused() {
    for code in [0, 1, 99, 600, 999, u16::MAX] {
        let error = Status::new(code).unwrap_err();
        assert!(
            matches!(
                error,
                Error::Parse {
                    target: "http status",
                    position: 0,
                    ..
                }
            ),
            "{code}: {error}"
        );
        assert!(error.to_string().contains(&code.to_string()));
        assert!(Status::try_from(code).is_err());
        assert!(serde_json::from_str::<Status>(&code.to_string()).is_err());
    }
    assert!(serde_json::from_str::<Status>("\"200\"").is_err());
    assert!(serde_json::from_str::<Status>("-1").is_err());
    assert!(serde_json::from_str::<Status>("200.5").is_err());
}

#[test]
fn a_status_text_must_open_with_a_three_digit_code() {
    for text in [
        "",
        "OK",
        "20",
        "2000",
        "200OK",
        "600 Too Far",
        "99 Low",
        "x200",
        "20 0",
    ] {
        let error = Status::from_str(text).unwrap_err();
        assert!(
            matches!(
                error,
                Error::Parse {
                    target: "http status",
                    ..
                }
            ),
            "{text:?}: {error}"
        );
    }
    assert_eq!(Status::from_str("200").unwrap(), Status::OK);
    assert_eq!(
        Status::from_str("404 Not Found").unwrap(),
        Status::NOT_FOUND
    );
    assert_eq!(Status::from_str("404 Whatever").unwrap(), Status::NOT_FOUND);
    assert_eq!(Status::from_str("  418 \n").unwrap().code(), 418);
    assert_eq!(
        "503 Service Unavailable".parse::<Status>().unwrap(),
        Status::SERVICE_UNAVAILABLE
    );
}

#[test]
fn every_registered_code_has_its_iana_reason_phrase() {
    let expected = [
        (100, "Continue"),
        (101, "Switching Protocols"),
        (102, "Processing"),
        (103, "Early Hints"),
        (200, "OK"),
        (201, "Created"),
        (202, "Accepted"),
        (203, "Non-Authoritative Information"),
        (204, "No Content"),
        (205, "Reset Content"),
        (206, "Partial Content"),
        (207, "Multi-Status"),
        (208, "Already Reported"),
        (226, "IM Used"),
        (300, "Multiple Choices"),
        (301, "Moved Permanently"),
        (302, "Found"),
        (303, "See Other"),
        (304, "Not Modified"),
        (305, "Use Proxy"),
        (307, "Temporary Redirect"),
        (308, "Permanent Redirect"),
        (400, "Bad Request"),
        (401, "Unauthorized"),
        (402, "Payment Required"),
        (403, "Forbidden"),
        (404, "Not Found"),
        (405, "Method Not Allowed"),
        (406, "Not Acceptable"),
        (407, "Proxy Authentication Required"),
        (408, "Request Timeout"),
        (409, "Conflict"),
        (410, "Gone"),
        (411, "Length Required"),
        (412, "Precondition Failed"),
        (413, "Content Too Large"),
        (414, "URI Too Long"),
        (415, "Unsupported Media Type"),
        (416, "Range Not Satisfiable"),
        (417, "Expectation Failed"),
        (421, "Misdirected Request"),
        (422, "Unprocessable Content"),
        (423, "Locked"),
        (424, "Failed Dependency"),
        (425, "Too Early"),
        (426, "Upgrade Required"),
        (428, "Precondition Required"),
        (429, "Too Many Requests"),
        (431, "Request Header Fields Too Large"),
        (451, "Unavailable For Legal Reasons"),
        (500, "Internal Server Error"),
        (501, "Not Implemented"),
        (502, "Bad Gateway"),
        (503, "Service Unavailable"),
        (504, "Gateway Timeout"),
        (505, "HTTP Version Not Supported"),
        (506, "Variant Also Negotiates"),
        (507, "Insufficient Storage"),
        (508, "Loop Detected"),
        (510, "Not Extended"),
        (511, "Network Authentication Required"),
    ];
    assert_eq!(Status::REGISTERED.len(), expected.len());
    for ((code, reason), (registered_code, registered_reason)) in
        expected.iter().zip(Status::REGISTERED.iter())
    {
        assert_eq!((code, reason), (registered_code, registered_reason));
        let status = Status::new(*code).unwrap();
        assert_eq!(status.code(), *code);
        assert_eq!(status.reason(), *reason);
        assert_eq!(status.to_string(), format!("{code} {reason}"));
        assert_eq!(Status::from_str(&status.to_string()).unwrap(), status);
        assert_eq!(u16::from(status), *code);
    }
    // The registry is sorted, so a code is looked up once and found once.
    assert!(
        Status::REGISTERED
            .windows(2)
            .all(|pair| pair[0].0 < pair[1].0)
    );

    // An unregistered code has no phrase and displays as its digits alone.
    for code in [199, 299, 418, 499, 599] {
        let status = Status::new(code).unwrap();
        assert_eq!(status.reason(), "");
        assert_eq!(status.to_string(), code.to_string());
        assert_eq!(Status::from_str(&status.to_string()).unwrap(), status);
    }
}

#[test]
fn the_named_constants_are_their_codes() {
    let table = [
        (Status::CONTINUE, 100),
        (Status::OK, 200),
        (Status::CREATED, 201),
        (Status::ACCEPTED, 202),
        (Status::NO_CONTENT, 204),
        (Status::PARTIAL_CONTENT, 206),
        (Status::MOVED_PERMANENTLY, 301),
        (Status::FOUND, 302),
        (Status::SEE_OTHER, 303),
        (Status::NOT_MODIFIED, 304),
        (Status::TEMPORARY_REDIRECT, 307),
        (Status::PERMANENT_REDIRECT, 308),
        (Status::BAD_REQUEST, 400),
        (Status::UNAUTHORIZED, 401),
        (Status::FORBIDDEN, 403),
        (Status::NOT_FOUND, 404),
        (Status::METHOD_NOT_ALLOWED, 405),
        (Status::REQUEST_TIMEOUT, 408),
        (Status::CONFLICT, 409),
        (Status::PRECONDITION_FAILED, 412),
        (Status::RANGE_NOT_SATISFIABLE, 416),
        (Status::TOO_EARLY, 425),
        (Status::TOO_MANY_REQUESTS, 429),
        (Status::INTERNAL_SERVER_ERROR, 500),
        (Status::NOT_IMPLEMENTED, 501),
        (Status::BAD_GATEWAY, 502),
        (Status::SERVICE_UNAVAILABLE, 503),
        (Status::GATEWAY_TIMEOUT, 504),
    ];
    for (status, code) in table {
        assert_eq!(status.code(), code);
        assert_eq!(Status::new(code).unwrap(), status);
    }
}

#[test]
fn the_class_predicates_are_the_hundreds_digit_and_retryable_is_seven_codes() {
    for code in 100..=599_u16 {
        let status = Status::new(code).unwrap();
        assert_eq!(status.is_informational(), (100..200).contains(&code));
        assert_eq!(status.is_success(), (200..300).contains(&code));
        assert_eq!(status.is_redirect(), (300..400).contains(&code));
        assert_eq!(status.is_client_error(), (400..500).contains(&code));
        assert_eq!(status.is_server_error(), (500..600).contains(&code));
        assert_eq!(
            status.is_retryable(),
            [408, 425, 429, 500, 502, 503, 504].contains(&code),
            "{code}"
        );
    }
}

#[test]
fn serde_carries_the_integer_and_order_is_the_code() {
    assert_eq!(serde_json::to_string(&Status::OK).unwrap(), "200");
    assert_eq!(
        serde_json::from_str::<Status>("404").unwrap(),
        Status::NOT_FOUND
    );
    let document: Vec<Status> = serde_json::from_str("[200, 301, 503]").unwrap();
    assert_eq!(
        document,
        [
            Status::OK,
            Status::MOVED_PERMANENTLY,
            Status::SERVICE_UNAVAILABLE
        ]
    );
    assert!(Status::OK < Status::NOT_FOUND);
    assert_eq!(Status::OK.clone(), Status::OK);
    let mut sorted = vec![Status::NOT_FOUND, Status::OK, Status::CONTINUE];
    sorted.sort();
    assert_eq!(sorted, [Status::CONTINUE, Status::OK, Status::NOT_FOUND]);
}
