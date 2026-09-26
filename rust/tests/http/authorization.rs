//! `rust/src/http/authorization.rs`: the credential a request carries.

use yggdryl::Url;
use yggdryl::http::Authorization;

#[test]
fn basic_is_the_base64_of_user_colon_password_under_authorization() {
    let basic = Authorization::basic("aladdin", "open sesame");
    assert_eq!(basic.header_name(), "Authorization");
    // RFC 7617 section 2's own example.
    assert_eq!(basic.header_value(), "Basic YWxhZGRpbjpvcGVuIHNlc2FtZQ==");
    assert_eq!(
        Authorization::basic("user", "").header_value(),
        "Basic dXNlcjo="
    );
    assert_eq!(
        Authorization::basic("user", "pa:ss").header_value(),
        "Basic dXNlcjpwYTpzcw==",
        "a colon in the password rides along"
    );
}

#[test]
fn bearer_is_the_token_under_authorization() {
    let bearer = Authorization::bearer("mF_9.B5f-4.1JqM");
    assert_eq!(bearer.header_name(), "Authorization");
    assert_eq!(bearer.header_value(), "Bearer mF_9.B5f-4.1JqM");
}

#[test]
fn a_header_credential_is_sent_verbatim_under_its_own_name() {
    let key = Authorization::header("X-Api-Key", "k-123");
    assert_eq!(key.header_name(), "X-Api-Key");
    assert_eq!(key.header_value(), "k-123");
    let token = Authorization::header("Authorization", "Token abc");
    assert_eq!(token.header_name(), "Authorization");
    assert_eq!(token.header_value(), "Token abc");
}

#[test]
fn debug_prints_redacted_where_the_secret_would_be() {
    let basic = format!("{:?}", Authorization::basic("aladdin", "open sesame"));
    assert_eq!(
        basic,
        "Basic { username: \"aladdin\", password: <redacted> }"
    );
    let bearer = format!("{:?}", Authorization::bearer("mF_9.B5f-4.1JqM"));
    assert_eq!(bearer, "Bearer(<redacted>)");
    assert!(!bearer.contains("mF_9"), "{bearer}");
    let header = format!("{:?}", Authorization::header("X-Api-Key", "k-123"));
    assert_eq!(header, "Header { name: \"X-Api-Key\", value: <redacted> }");
    assert!(!header.contains("k-123"), "{header}");
}

#[test]
fn equality_reads_the_scheme_and_the_credential() {
    assert_eq!(
        Authorization::basic("u", "p"),
        Authorization::basic("u", "p")
    );
    assert_ne!(
        Authorization::basic("u", "p"),
        Authorization::basic("u", "q")
    );
    assert_ne!(
        Authorization::bearer("t"),
        Authorization::header("Authorization", "Bearer t")
    );
    let cloned = Authorization::bearer("t").clone();
    assert_eq!(cloned, Authorization::bearer("t"));
}

#[test]
fn a_urls_user_information_is_a_basic_credential_percent_decoded() {
    let url = Url::from_str("https://al%20addin:open%20sesame@api.example.com/v1").expect("a URL");
    assert_eq!(
        Authorization::from_url(&url),
        Some(Authorization::basic("al addin", "open sesame"))
    );
    let no_password = Url::from_str("https://aladdin@api.example.com/v1").expect("a URL");
    assert_eq!(
        Authorization::from_url(&no_password),
        Some(Authorization::basic("aladdin", ""))
    );
    let anonymous = Url::from_str("https://api.example.com/v1").expect("a URL");
    assert_eq!(Authorization::from_url(&anonymous), None);
}
