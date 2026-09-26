//! `rust/src/http/method.rs`.

use yggdryl::Error;
use yggdryl::http::Method;

#[test]
fn an_unknown_method_is_refused_by_name_with_the_vocabulary() {
    for text in ["", "FETCH", "G ET", "get post", "🙂"] {
        let error = Method::from_str(text).unwrap_err();
        assert!(
            matches!(
                error,
                Error::Parse {
                    target: "http method",
                    position: 0,
                    ..
                }
            ),
            "{text:?}: {error}"
        );
        assert!(
            error
                .to_string()
                .contains("GET, HEAD, POST, PUT, PATCH, DELETE, OPTIONS, TRACE, CONNECT")
        );
    }
    assert!(serde_json::from_str::<Method>("\"FETCH\"").is_err());
    assert!(serde_json::from_str::<Method>("1").is_err());
}

#[test]
fn every_method_has_one_upper_case_spelling_read_case_insensitively() {
    let table = [
        (Method::Get, "GET"),
        (Method::Head, "HEAD"),
        (Method::Post, "POST"),
        (Method::Put, "PUT"),
        (Method::Patch, "PATCH"),
        (Method::Delete, "DELETE"),
        (Method::Options, "OPTIONS"),
        (Method::Trace, "TRACE"),
        (Method::Connect, "CONNECT"),
    ];
    assert_eq!(Method::ALL.len(), table.len());
    for (method, canonical) in table {
        assert!(Method::ALL.contains(&method));
        assert_eq!(method.as_str(), canonical);
        assert_eq!(method.as_ref(), canonical);
        assert_eq!(method.to_string(), canonical);
        assert_eq!(Method::from_str(canonical).unwrap(), method);
        assert_eq!(Method::from_str(&canonical.to_lowercase()).unwrap(), method);
        assert_eq!(
            Method::from_str(&format!(" {canonical}\t")).unwrap(),
            method
        );
        assert_eq!(canonical.parse::<Method>().unwrap(), method);
        assert_eq!(
            serde_json::to_string(&method).unwrap(),
            format!("\"{canonical}\"")
        );
        assert_eq!(
            serde_json::from_str::<Method>(&format!("\"{canonical}\"")).unwrap(),
            method
        );
    }
}

#[test]
fn safety_idempotence_and_a_body_follow_rfc_9110_section_9() {
    let table = [
        // method, safe, idempotent, has a body
        (Method::Get, true, true, false),
        (Method::Head, true, true, false),
        (Method::Post, false, false, true),
        (Method::Put, false, true, true),
        (Method::Patch, false, false, true),
        (Method::Delete, false, true, false),
        (Method::Options, true, true, false),
        (Method::Trace, true, true, false),
        (Method::Connect, false, false, false),
    ];
    for (method, safe, idempotent, body) in table {
        assert_eq!(method.is_safe(), safe, "{method}");
        assert_eq!(method.is_idempotent(), idempotent, "{method}");
        assert_eq!(method.has_request_body(), body, "{method}");
        // Every safe method is idempotent.
        assert!(!method.is_safe() || method.is_idempotent(), "{method}");
    }
}
