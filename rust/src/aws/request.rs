//! Signature Version 4 on a request the HTTP client sends.
//!
//! [`Request::with_sigv4`] is the door, written here rather than in `http/`
//! so that the HTTP client knows nothing of AWS: it is the request's own
//! per-attempt hook, set to sign, and its own resend rule, set to read a
//! refused key. Who signs is the [`Session`]'s credential chain, how is
//! [`sigv4`]'s, and the retries, the redirects, the proxy and the transport
//! stay the HTTP client's.

use std::borrow::Cow;
use std::sync::Arc;
use std::time::SystemTime;

use smol_str::SmolStr;

use super::Session;
use super::sigv4::{self, EMPTY_PAYLOAD_SHA256, UNSIGNED_PAYLOAD};
use crate::http::request::host_header;
use crate::http::{Attempt, Headers, Method, Request, Status};
use crate::{Error, Result};

impl Request {
    /// Sign every attempt of this request with AWS Signature Version 4, as
    /// `session` answers, for `service` in `region`.
    ///
    /// `service` is the SigV4 signing name - `s3tables`, `execute-api`,
    /// `glue` - and `region` the region the endpoint is in; both are part of
    /// the signature's scope. Each attempt asks the session for its
    /// credential set - so a set refreshed between two attempts signs the
    /// second - and signs what is really sent: the hop's method, the host of
    /// its URL with the port the URL names, the path as it is on the wire,
    /// the query, the `content-type`, `content-md5` and `x-amz-*` headers
    /// the attempt carries, and the SHA-256 of the body. The headers it adds
    /// are `x-amz-date`, `x-amz-content-sha256`, `x-amz-security-token` for a
    /// temporary set, and `authorization`.
    ///
    /// The canonical URI follows the service: a service of the S3 family
    /// signs the path as sent, every other one signs it normalized and
    /// percent-encoded once more, as botocore does - so a path that carries
    /// `%1F` or an encoded ARN signs as the service computes it.
    ///
    /// A refusal that says the key is no longer accepted - `ExpiredToken`, a
    /// key the service does not recognize - is told to the session, and the
    /// request goes out once more, whatever its method, when the session
    /// then answers another set; the same set would be refused the same way.
    ///
    /// ```
    /// use yggdryl::aws::{Credentials, Session};
    /// use yggdryl::http::Request;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let session = Session::new()
    ///     .with_environment(false)
    ///     .with_credentials(Credentials::new("AKIDEXAMPLE", "secret"));
    /// let request = Request::get("https://s3tables.eu-west-3.amazonaws.com/iceberg/v1/config")?
    ///     .with_sigv4(&session, "s3tables", "eu-west-3");
    /// // Nothing is signed until an attempt goes out.
    /// assert!(format!("{request:?}").contains("<attempt headers>"));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Sending such a request fails, before anything goes out, with the
    /// session's own refusal when its sources failed; with a refusal naming
    /// the service when no source answered a set, rather than going unsigned;
    /// and, for a service outside the S3 family, when the body is streamed
    /// by [`Request::send_reader`], because a body that cannot be read
    /// cannot be hashed.
    #[must_use]
    pub fn with_sigv4(
        self,
        session: &Session,
        service: impl Into<SmolStr>,
        region: impl Into<String>,
    ) -> Self {
        let signing = Arc::new(Signing {
            session: session.clone(),
            service: service.into(),
            region: region.into(),
        });
        let refused = Arc::clone(&signing);
        self.with_attempt_headers(move |attempt| signing.headers(attempt, SystemTime::now()))
            .with_resend_on(move |sent, status, headers, body| {
                refused.answers_another(sent, status, headers, body, SystemTime::now())
            })
    }
}

/// Who signs a request, and the scope it is signed for.
struct Signing {
    session: Session,
    service: SmolStr,
    region: String,
}

impl Signing {
    /// The headers that sign `attempt` at `now`.
    fn headers(&self, attempt: &Attempt<'_>, now: SystemTime) -> Result<Headers> {
        let payload_hash = match attempt.body() {
            Some(body) if !body.is_empty() => Cow::Owned(sigv4::sha256_hex(body)),
            _ if attempt.is_streamed() => {
                if !sigv4::is_s3_family(&self.service) {
                    return Err(refusal(format!(
                        "SigV4 for `{}` signs the body, and a streamed one cannot be read: \
                         send it as bytes",
                        self.service
                    )));
                }
                Cow::Borrowed(UNSIGNED_PAYLOAD)
            }
            _ => Cow::Borrowed(EMPTY_PAYLOAD_SHA256),
        };
        let Some(signer) = self.session.signer(&self.service, &self.region, now)? else {
            return Err(refusal(format!(
                "SigV4 for `{}` asked, no credential source answered",
                self.service
            )));
        };
        let url = attempt.url();
        let path = url.path_text(false)?;
        let query: Vec<(String, String)> = url
            .parameters(true)?
            .iter()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect();
        let signed: Vec<(String, String)> = attempt
            .headers()
            .iter()
            .filter(|(name, _)| is_signed(name))
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect();
        let mut headers = Headers::new();
        for (name, value) in signer.sign(
            attempt.method().as_str(),
            &host_header(url),
            if path.is_empty() { "/" } else { &path },
            &query,
            &signed,
            &payload_hash,
            now,
        ) {
            headers.insert(&name, &value)?;
        }
        Ok(headers)
    }

    /// Whether the refusal `sent` was answered with says its key is no
    /// longer accepted and the session now answers another one.
    fn answers_another(
        &self,
        sent: &Attempt<'_>,
        status: Status,
        headers: &Headers,
        body: &[u8],
        now: SystemTime,
    ) -> Result<bool> {
        if !matches!(status.code(), 400 | 403) {
            return Ok(false);
        }
        let Some(signed) = sent
            .headers()
            .get("authorization")
            .and_then(sigv4::signed_access_key)
        else {
            return Ok(false);
        };
        let endpoint = host_header(sent.url());
        match super::error_code(headers, body) {
            Some(code) => {
                self.session
                    .answers_another(signed, Some(&code), &endpoint, &self.region, now)
            }
            // A `HEAD` is refused with no body to name a code in: a
            // temporary set refused that way is read again, with nothing
            // held against its key.
            None if sent.method() == Method::Head
                && body.is_empty()
                && sent.headers().get("x-amz-security-token").is_some() =>
            {
                self.session
                    .answers_another(signed, None, &endpoint, &self.region, now)
            }
            None => Ok(false),
        }
    }
}

/// Whether a header the attempt carries is signed beside the signer's own:
/// what the service reads of the request, and nothing a proxy rewrites.
fn is_signed(name: &str) -> bool {
    name.eq_ignore_ascii_case("content-type")
        || name.eq_ignore_ascii_case("content-md5")
        || name
            .get(..6)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("x-amz-"))
}

/// A refusal to sign.
fn refusal(message: String) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        message,
    ))
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/aws/request.rs` and the `aws_session` benchmark reach
    //! and a caller cannot: the signing of one attempt at a stated instant.
    use std::time::SystemTime;

    use crate::Result;
    use crate::Url;
    use crate::aws::Session;
    use crate::http::{Attempt, Headers, Method};

    /// The headers `with_sigv4` adds to one attempt of `method` at `url`
    /// carrying `headers` and `body`, signed at `now`.
    #[allow(clippy::too_many_arguments)]
    pub fn signed_headers(
        session: &Session,
        service: &str,
        region: &str,
        method: Method,
        url: &Url,
        headers: &Headers,
        body: Option<&[u8]>,
        now: SystemTime,
    ) -> Result<Headers> {
        super::Signing {
            session: session.clone(),
            service: service.into(),
            region: region.to_owned(),
        }
        .headers(&Attempt::new(1, method, url, headers, body, false), now)
    }
}
