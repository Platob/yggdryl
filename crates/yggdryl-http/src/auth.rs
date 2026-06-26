//! HTTP authentication helpers — the `Authorization` header builders.
//!
//! Basic and Bearer auth are both nothing more than an `Authorization` header,
//! so the per-request ([`with_basic_auth`](crate::HttpRequest::with_basic_auth))
//! and per-session ([`with_basic_auth`](crate::HttpSession::with_basic_auth))
//! helpers funnel through the two free functions here. Keeping the tiny,
//! dependency-free Base64 encoder and the header formatting in one place means
//! all credential logic lives here and nowhere else.

/// The standard Base64 alphabet (RFC 4648 §4).
const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encodes `input` as standard, padded Base64 (RFC 4648 §4).
///
/// A dependency-free encoder — Basic auth is the crate's only Base64 user, so a
/// full codec crate would be overkill against the project's dependency-light rule.
fn base64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        out.push(BASE64_ALPHABET[(b0 >> 2) as usize] as char);
        out.push(BASE64_ALPHABET[(((b0 & 0b11) << 4) | (b1 >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 {
            BASE64_ALPHABET[(((b1 & 0b1111) << 2) | (b2 >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            BASE64_ALPHABET[(b2 & 0b11_1111) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// The `Authorization` value for HTTP Basic auth: `Basic <base64(user:pass)>`
/// (RFC 7617).
pub(crate) fn basic_auth_header(username: &str, password: &str) -> String {
    let mut credentials = String::with_capacity(username.len() + 1 + password.len());
    credentials.push_str(username);
    credentials.push(':');
    credentials.push_str(password);
    format!("Basic {}", base64_encode(credentials.as_bytes()))
}

/// The `Authorization` value for HTTP Bearer auth: `Bearer <token>` (RFC 6750).
pub(crate) fn bearer_auth_header(token: &str) -> String {
    format!("Bearer {token}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648_vectors() {
        // The canonical RFC 4648 §10 test vectors exercise every padding case.
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn basic_auth_encodes_credentials() {
        // `Aladdin:open sesame` is RFC 7617's worked example.
        assert_eq!(
            basic_auth_header("Aladdin", "open sesame"),
            "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );
        // An empty password still carries the separating colon.
        assert_eq!(basic_auth_header("user", ""), "Basic dXNlcjo=");
    }

    #[test]
    fn bearer_auth_prefixes_the_token() {
        assert_eq!(bearer_auth_header("abc.def.ghi"), "Bearer abc.def.ghi");
    }
}
