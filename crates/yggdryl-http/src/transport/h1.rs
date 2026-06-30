//! HTTP/1.1 transport via [`ureq`] v3.
//!
//! `H1Transport` wraps a `ureq::Agent` (which manages connection pooling and
//! TLS internally) and translates between the public `HttpRequest`/`HttpResponse`
//! model and the ureq v3 API. All retry, cookie and redirect logic lives in
//! `session.rs`; the transport only sends one request and returns the raw
//! response. HTTP 4xx/5xx statuses are returned as `Ok(response)`, not
//! errors — the session layer decides what to do with them.

use std::io::Read;
use std::sync::Arc;

use crate::error::HttpError;

use super::{RawResponse, SendConfig, Transport};

/// HTTP/1.1 transport backed by `ureq` v3.
pub(crate) struct H1Transport {
    agent: ureq::Agent,
}

impl H1Transport {
    /// Creates a transport with default settings.
    ///
    /// `http_status_as_error` is set to `false` so 4xx/5xx responses come back
    /// as `Ok(response)` — the session layer handles status-based logic.
    pub(crate) fn new() -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .new_agent();
        H1Transport { agent }
    }

    /// Creates a transport that trusts the given PEM CA certificate bundle
    /// in addition to the platform roots.
    ///
    /// Each PEM-encoded certificate in `pem_bytes` is added as a trusted root.
    #[allow(dead_code)]
    pub(crate) fn with_ca_pem(pem_bytes: &[u8]) -> Result<Self, HttpError> {
        let certs: Vec<ureq::tls::Certificate<'static>> = ureq::tls::parse_pem(pem_bytes)
            .filter_map(|item| match item {
                Ok(ureq::tls::PemItem::Certificate(c)) => Some(c),
                _ => None,
            })
            .collect();

        if certs.is_empty() {
            return Err(HttpError::Tls(
                "no certificates found in the PEM bundle".to_string(),
            ));
        }

        let tls_config = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::Specific(Arc::new(certs)))
            .build();

        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .tls_config(tls_config)
            .build()
            .new_agent();

        Ok(H1Transport { agent })
    }
}

impl Transport for H1Transport {
    fn send(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<&[u8]>,
        config: &SendConfig,
    ) -> Result<RawResponse, HttpError> {
        crate::log_event!(debug, "H1 send method={method} url={url}");

        if let Some(data) = body.filter(|b| !b.is_empty()) {
            let mut builder = ureq::http::Request::builder().method(method).uri(url);
            for (k, v) in headers {
                builder = builder.header(k.as_str(), v.as_str());
            }
            let request = builder
                .body(data.to_vec())
                .map_err(|e| HttpError::Transport(e.to_string()))?;
            let request = apply_timeout(request, &self.agent, config);
            let resp = self.agent.run(request).map_err(map_ureq_err)?;
            raw_response_from(resp)
        } else {
            let mut builder = ureq::http::Request::builder().method(method).uri(url);
            for (k, v) in headers {
                builder = builder.header(k.as_str(), v.as_str());
            }
            let request = builder
                .body(())
                .map_err(|e| HttpError::Transport(e.to_string()))?;
            let request = apply_timeout(request, &self.agent, config);
            let resp = self.agent.run(request).map_err(map_ureq_err)?;
            raw_response_from(resp)
        }
    }

    fn send_streaming(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body_reader: Box<dyn Read + Send + 'static>,
        body_len: Option<u64>,
        config: &SendConfig,
    ) -> Result<RawResponse, HttpError> {
        crate::log_event!(
            debug,
            "H1 streaming send method={method} url={url} len={body_len:?}"
        );

        let mut builder = ureq::http::Request::builder().method(method).uri(url);
        for (k, v) in headers {
            builder = builder.header(k.as_str(), v.as_str());
        }
        if let Some(len) = body_len {
            builder = builder.header("content-length", len.to_string());
        }

        let send_body = ureq::SendBody::from_owned_reader(body_reader);
        let request = builder
            .body(send_body)
            .map_err(|e| HttpError::Transport(e.to_string()))?;
        let request = apply_timeout(request, &self.agent, config);
        let resp = self.agent.run(request).map_err(map_ureq_err)?;
        raw_response_from(resp)
    }
}

/// Applies a per-request timeout to `request` via `agent.configure_request`.
fn apply_timeout<B: ureq::AsSendBody>(
    request: ureq::http::Request<B>,
    agent: &ureq::Agent,
    config: &SendConfig,
) -> ureq::http::Request<B> {
    if let Some(timeout) = config.timeout {
        agent
            .configure_request(request)
            .timeout_global(Some(timeout))
            .build()
    } else {
        request
    }
}

/// Converts `ureq::Error` to `HttpError`.
fn map_ureq_err(e: ureq::Error) -> HttpError {
    match e {
        ureq::Error::Timeout(_) => HttpError::Timeout,
        ureq::Error::Io(io_err)
            if io_err.kind() == std::io::ErrorKind::TimedOut
                || io_err.kind() == std::io::ErrorKind::WouldBlock =>
        {
            HttpError::Timeout
        }
        ureq::Error::HostNotFound => HttpError::Transport("hostname not found".to_string()),
        other => HttpError::Transport(other.to_string()),
    }
}

/// Extracts status, headers, and body from a ureq v3 response.
fn raw_response_from(resp: ureq::http::Response<ureq::Body>) -> Result<RawResponse, HttpError> {
    let status = resp.status().as_u16();

    let headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_ascii_lowercase(),
                value.to_str().unwrap_or("").to_string(),
            )
        })
        .collect();

    let content_length = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse::<u64>().ok());

    let (_, body) = resp.into_parts();
    let body_reader: Box<dyn Read + Send + 'static> = Box::new(body.into_reader());

    Ok(RawResponse {
        status,
        headers,
        version: crate::HttpVersion::Http1_1,
        content_length,
        body: body_reader,
    })
}
