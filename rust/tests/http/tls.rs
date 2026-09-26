//! `rust/src/http/tls.rs`: the certificates a TLS connection is verified
//! against, read once for every version a client speaks.

use yggdryl::Error;
use yggdryl::http::{Client, HttpOptions};

#[test]
fn a_ca_bundle_that_cannot_be_read_is_refused() {
    let error = Client::with_options(
        &HttpOptions::default().with_ca_bundle("/nonexistent/yggdryl-bundle.pem"),
    )
    .expect_err("a refusal");
    assert!(matches!(error, Error::Io(_)), "{error:?}");
    assert!(error.to_string().contains("yggdryl-bundle.pem"), "{error}");
}

#[test]
fn a_ca_bundle_holding_no_certificate_is_refused_naming_it() {
    let path =
        std::env::temp_dir().join(format!("yggdryl-empty-bundle-{}.pem", std::process::id()));
    std::fs::write(&path, "not a certificate\n").expect("a bundle");
    let error =
        Client::with_options(&HttpOptions::default().with_ca_bundle(&path)).expect_err("a refusal");
    let _ = std::fs::remove_file(&path);
    match error {
        Error::Io(error) => {
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
            assert!(
                error.to_string().contains("holds no certificate"),
                "{error}"
            );
        }
        other => panic!("expected an io refusal, got {other:?}"),
    }
}
