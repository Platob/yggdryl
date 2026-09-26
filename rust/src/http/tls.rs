//! What a TLS connection is verified against, read once for every version a
//! client speaks.
//!
//! A server's certificate chain is checked against the options' CA bundle -
//! or, when the options read the environment and name none, the bundle
//! `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE` or `CURL_CA_BUNDLE` names - and
//! against the Mozilla roots `webpki-roots` carries when no bundle is named.
//! HTTP/1.1 hands the certificates to its transport; HTTP/2 and HTTP/3 build
//! the one `rustls` configuration each version offers in ALPN from the same
//! certificates, under the ring provider the whole stack shares.

use std::path::PathBuf;

use super::HttpOptions;
use crate::{Error, Result};

/// The environment names a CA bundle is read from, in order, when the
/// options name none and read the environment.
pub(crate) const CA_BUNDLE_VARIABLES: [&str; 3] =
    ["SSL_CERT_FILE", "REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE"];

/// The certificates of the bundle the options name, or the environment does
/// when they read it; `None` when neither names one.
///
/// # Errors
///
/// [`Error::Io`] when the bundle cannot be read or holds no certificate.
pub(crate) fn bundle_certificates(
    options: &HttpOptions,
) -> Result<Option<Vec<ureq::tls::Certificate<'static>>>> {
    let path = match options.ca_bundle() {
        Some(path) => path.to_path_buf(),
        None if options.read_environment() => {
            let Some(path) = CA_BUNDLE_VARIABLES
                .iter()
                .find_map(|name| crate::auth::variable(name))
            else {
                return Ok(None);
            };
            PathBuf::from(path)
        }
        None => return Ok(None),
    };
    let pem = std::fs::read(&path).map_err(|error| {
        Error::Io(std::io::Error::new(
            error.kind(),
            format!(
                "could not read the certificate bundle {}: {error}",
                path.display()
            ),
        ))
    })?;
    let certificates: Vec<ureq::tls::Certificate<'static>> = ureq::tls::parse_pem(&pem)
        .filter_map(|item| match item {
            Ok(ureq::tls::PemItem::Certificate(certificate)) => Some(certificate),
            _ => None,
        })
        .collect();
    if certificates.is_empty() {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "the certificate bundle {} holds no certificate",
                path.display()
            ),
        )));
    }
    Ok(Some(certificates))
}

#[cfg(feature = "http2")]
pub(crate) use framed::ClientConfigs;
#[cfg(feature = "http3")]
pub(crate) use framed::ring;

#[cfg(feature = "http2")]
mod framed {
    use std::sync::Arc;

    use rustls::pki_types::CertificateDer;
    use rustls::{ClientConfig, RootCertStore};

    use super::HttpOptions;
    use crate::{Error, Result};

    /// The crypto provider every TLS connection of the crate negotiates
    /// under: the one ureq's HTTP/1.1 connections use.
    pub(crate) fn ring() -> Arc<rustls::crypto::CryptoProvider> {
        Arc::new(rustls::crypto::ring::default_provider())
    }

    /// The TLS configurations one client's framed connections open with.
    #[derive(Clone, Debug)]
    pub(crate) struct ClientConfigs {
        /// Offers `h2`, then `http/1.1`, over TLS 1.2 or 1.3.
        pub(crate) h2: Arc<ClientConfig>,
        /// Offers `h3` over TLS 1.3, the only version QUIC carries.
        #[cfg(feature = "http3")]
        pub(crate) h3: Arc<ClientConfig>,
    }

    impl ClientConfigs {
        /// The configurations `options` call for: their bundle's roots, or
        /// the Mozilla roots when they name none.
        ///
        /// # Errors
        ///
        /// As [`super::bundle_certificates`], and [`Error::Io`] when a
        /// bundle's certificate is not one a verifier can anchor on.
        pub(crate) fn for_options(options: &HttpOptions) -> Result<Self> {
            let mut roots = RootCertStore::empty();
            match super::bundle_certificates(options)? {
                Some(certificates) => {
                    for certificate in certificates {
                        roots
                            .add(CertificateDer::from(certificate.der().to_vec()))
                            .map_err(|error| {
                                Error::Io(std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    format!("a bundled certificate is no trust anchor: {error}"),
                                ))
                            })?;
                    }
                }
                None => roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned()),
            }
            let roots = Arc::new(roots);
            let refused = |error: rustls::Error| {
                Error::Io(std::io::Error::other(format!(
                    "the TLS configuration was refused: {error}"
                )))
            };
            let mut h2 = ClientConfig::builder_with_provider(ring())
                .with_safe_default_protocol_versions()
                .map_err(refused)?
                .with_root_certificates(Arc::clone(&roots))
                .with_no_client_auth();
            h2.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
            #[cfg(feature = "http3")]
            let h3 = {
                let mut h3 = ClientConfig::builder_with_provider(ring())
                    .with_protocol_versions(&[&rustls::version::TLS13])
                    .map_err(refused)?
                    .with_root_certificates(roots)
                    .with_no_client_auth();
                h3.alpn_protocols = vec![b"h3".to_vec()];
                Arc::new(h3)
            };
            Ok(Self {
                h2: Arc::new(h2),
                #[cfg(feature = "http3")]
                h3,
            })
        }
    }
}
