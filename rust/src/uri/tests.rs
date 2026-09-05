//! Resource identifier unit tests.

use super::{Authority, Scheme, Uri, UriPath, Url, Urn};

#[test]
fn canonical_uri_round_trip() {
    let uri = Uri::from_str("HTTPS://example.test/a%2fb?q=x#part").unwrap();
    assert_eq!(uri.to_string(), "https://example.test/a%2Fb?q=x#part");
    assert_eq!(Uri::from_str(&uri.to_string()).unwrap(), uri);
}

#[test]
fn components_validate_and_serialize_as_strings() {
    let scheme = Scheme::from_str("FILE").unwrap();
    let authority = Authority::from_str("").unwrap();
    let path = UriPath::from_str("/tmp/a.csv").unwrap();
    assert_eq!(serde_json::to_string(&scheme).unwrap(), "\"file\"");
    assert_eq!(authority.as_str(), "");
    assert_eq!(path.extension(), Some("csv"));
}

#[test]
fn specialized_values_validate() {
    assert!(Url::from_str("https://example.test").is_ok());
    assert!(Url::from_str("https:///path").is_err());
    let urn = Urn::from_str("URN:ISBN:9780131103627").unwrap();
    assert_eq!(urn.to_string(), "urn:isbn:9780131103627");
}

#[test]
fn authority_ports_are_explicit_valid_u16_values() {
    assert_eq!(
        Authority::from_str("minio:9000").unwrap().port(),
        Some(9000)
    );
    assert_eq!(
        Authority::from_str("[::1]:9000").unwrap().port(),
        Some(9000)
    );
    for invalid in ["minio:", "minio:65536", "[::1]:", "[::1]:99999"] {
        assert!(Authority::from_str(invalid).is_err(), "{invalid}");
    }
}
