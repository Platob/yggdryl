//! HTTP/3 transport via [`quinn`] + [`h3`] + [`h3-quinn`].
//!
//! `H3Transport` connects over QUIC (TLS 1.3 with ALPN `h3`) and uses the
//! `h3` crate for HTTP/3 framing. A process-global QUIC `Endpoint` is shared
//! across all instances so each request avoids allocating a new UDP socket.
//! Connection establishment is per-request for now; pooling is a future
//! optimization.

use std::io::{Cursor, Read};
use std::net::ToSocketAddrs;
use std::sync::{Arc, OnceLock};

use bytes::{Buf, Bytes};
use http::Uri;

use crate::error::HttpError;
use crate::HttpVersion;

use super::{RawResponse, SendConfig, Transport};

/// Returns the process-global QUIC client endpoint.
fn quic_endpoint() -> &'static quinn::Endpoint {
    static EP: OnceLock<quinn::Endpoint> = OnceLock::new();
    EP.get_or_init(|| {
        quinn::Endpoint::client("0.0.0.0:0".parse().unwrap())
            .expect("failed to create QUIC client endpoint")
    })
}

/// HTTP/3 transport.
pub(crate) struct H3Transport {
    quinn_config: quinn::ClientConfig,
}

impl H3Transport {
    pub(crate) fn new() -> Self {
        let roots = webpki_roots::TLS_SERVER_ROOTS
            .iter()
            .cloned()
            .collect::<rustls::RootCertStore>();
        let mut tls = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        tls.alpn_protocols = vec![b"h3".to_vec()];
        let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls)
            .expect("H3 TLS config must be valid with ring provider");
        H3Transport {
            quinn_config: quinn::ClientConfig::new(Arc::new(crypto)),
        }
    }
}

impl Transport for H3Transport {
    fn send(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<&[u8]>,
        config: &SendConfig,
    ) -> Result<RawResponse, HttpError> {
        crate::log_event!(debug, "H3 send method={method} url={url}");
        let quinn_config = self.quinn_config.clone();
        let method = method.to_string();
        let url = url.to_string();
        let headers: Vec<(String, String)> = headers.to_vec();
        let body_bytes: Bytes = body.map(|b| Bytes::copy_from_slice(b)).unwrap_or_default();
        let timeout = config.timeout;

        super::runtime().block_on(async move {
            send_h3(quinn_config, &method, &url, &headers, body_bytes, timeout).await
        })
    }
}

async fn send_h3(
    quinn_config: quinn::ClientConfig,
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body_bytes: Bytes,
    timeout: Option<std::time::Duration>,
) -> Result<RawResponse, HttpError> {
    let uri: Uri = url
        .parse()
        .map_err(|e| HttpError::InvalidUrl(format!("{e}")))?;
    let host = uri
        .host()
        .ok_or_else(|| HttpError::InvalidUrl("URL has no host".to_string()))?;
    let port = uri.port_u16().unwrap_or(443); // H3 is always HTTPS

    let addr = format!("{host}:{port}");
    let sock_addr = addr
        .to_socket_addrs()
        .map_err(|e| HttpError::Transport(format!("DNS resolve {addr}: {e}")))?
        .next()
        .ok_or_else(|| HttpError::Transport(format!("no addresses for {addr}")))?;

    let endpoint = quic_endpoint();
    let quic_conn = endpoint
        .connect_with(quinn_config, sock_addr, host)
        .map_err(|e| HttpError::Transport(format!("QUIC connect {addr}: {e}")))?
        .await
        .map_err(|e| HttpError::Transport(format!("QUIC connect {addr}: {e}")))?;

    let h3_quic_conn = h3_quinn::Connection::new(quic_conn);
    let (mut driver, mut send_request) = h3::client::new(h3_quic_conn)
        .await
        .map_err(|e| HttpError::Transport(format!("H3 handshake: {e}")))?;

    tokio::spawn(async move {
        crate::log_event!(debug, "H3 connection closed: {}", driver.wait_idle().await);
    });

    let http_method = http::Method::from_bytes(method.as_bytes())
        .map_err(|e| HttpError::Transport(format!("invalid HTTP method: {e}")))?;

    let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
    let uri_ref: Uri = path_and_query
        .parse()
        .map_err(|e| HttpError::InvalidUrl(format!("{e}")))?;

    let mut builder = http::Request::builder()
        .method(http_method)
        .uri(uri_ref)
        .header(http::header::HOST, host);

    for (k, v) in headers {
        builder = builder.header(k.as_str(), v.as_str());
    }

    let req = builder
        .body(())
        .map_err(|e| HttpError::Transport(format!("request build: {e}")))?;

    let fut = async {
        let mut request_stream = send_request
            .send_request(req)
            .await
            .map_err(|e| HttpError::Transport(format!("send request: {e}")))?;

        if !body_bytes.is_empty() {
            request_stream
                .send_data(body_bytes)
                .await
                .map_err(|e| HttpError::Transport(format!("send body: {e}")))?;
        }

        request_stream
            .finish()
            .await
            .map_err(|e| HttpError::Transport(format!("finish request: {e}")))?;

        let response = request_stream
            .recv_response()
            .await
            .map_err(|e| HttpError::Transport(format!("recv response: {e}")))?;

        let status = response.status().as_u16();
        let resp_headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_ascii_lowercase(),
                    v.to_str().unwrap_or("").to_string(),
                )
            })
            .collect();

        let content_length: Option<u64> = resp_headers
            .iter()
            .find(|(k, _)| k == "content-length")
            .and_then(|(_, v)| v.parse::<u64>().ok());

        let mut body_buf: Vec<u8> = Vec::new();
        while let Some(chunk) = request_stream
            .recv_data()
            .await
            .map_err(|e| HttpError::Transport(format!("recv body: {e}")))?
        {
            body_buf.extend_from_slice(chunk.chunk());
        }

        let body_reader: Box<dyn Read + Send + 'static> = Box::new(Cursor::new(body_buf));

        Ok::<RawResponse, HttpError>(RawResponse {
            status,
            headers: resp_headers,
            version: HttpVersion::Http3,
            content_length,
            body: body_reader,
        })
    };

    if let Some(dur) = timeout {
        tokio::time::timeout(dur, fut)
            .await
            .map_err(|_| HttpError::Timeout)?
    } else {
        fut.await
    }
}
