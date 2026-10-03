//! The SOAP 1.1 HTTP binding of a provider: the routes a [`Service`] answers
//! on an [`http::Server`](crate::http::Server).
//!
//! XML for Analysis is SOAP 1.1 over HTTP `POST`, and the connection side of
//! that - framing, keep-alive, `Expect: 100-continue`, timeouts and bounds,
//! HTTP/2 and HTTP/3, the exchange trace - is the HTTP server's. What is
//! XMLA's is here: a `POST` is answered by [`Service::handle`] as the
//! response is sent, never held whole, so an Execute over a large table
//! streams; a `GET` answers a short description of the endpoint, so a
//! browser or a probe learns what it reached, and a `HEAD` its head; any
//! other method is the server's `405`.
//!
//! ```no_run
//! use std::sync::Arc;
//! use yggdryl::FolderCatalog;
//! use yggdryl::holder::Holder;
//! use yggdryl::http::{Server, ServerOptions};
//! use yggdryl::xmla::{Service, ServiceOptions};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let service = Service::new(ServiceOptions::new())
//!     .with_catalog(FolderCatalog::bound("market", Holder::folder("/data/market")?));
//! let server = Server::bind_with("127.0.0.1:8080", ServerOptions::default())?;
//! let endpoint = Arc::new(service).route(&server, "/xmla")?;
//! println!("serving XMLA at {endpoint}");
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;

use crate::http::{Body, Method, Request, Response, Server, Status};
use crate::soap::{self, Fault, FaultCode};
use crate::{Format, MimeType, Result, Url};

use super::response::{ACTOR, XmlaError, fault, write_fault};
use super::service::{Service, code};

/// The content-negotiation flags every SOAP answer carries: MS-SSAS 2.1.2's
/// `NEGO,REQ_SX,REQ_XPRESS,RESP_SX,RESP_XPRESS`, all zero - plain text XML
/// both ways, which a client that offered the binary or the compressed
/// encodings reads as the server declining them.
const NEGOTIATION: (&str, &str) = ("X-Transport-Caps-Negotiation-Flags", "0,0,0,0,0");

impl Service {
    /// Answer XML for Analysis at `path` on `server`: a `POST` carrying a
    /// SOAP 1.1 Discover or Execute, answered as it is sent, and a `GET` (a
    /// `HEAD` its head) describing the endpoint under the URL the request
    /// was made at - the one a proxy in front presents, when the server
    /// trusts it; answers the endpoint's URL on the server's own socket
    /// ([`Server::public_url_of`] is the one clients reach).
    ///
    /// Every answer to a `POST` is `200` under `text/xml; charset=utf-8`
    /// and `X-Transport-Caps-Negotiation-Flags: 0,0,0,0,0`, a fault
    /// included - the way the reference providers answer and XMLA clients
    /// read one - and a body that is no SOAP message, or not XML at all, is
    /// a `Client` fault naming what it was. A `POST` to `path` with a
    /// trailing slash is served by the same route, never redirected - a
    /// redirected client may not repeat its body - while a `GET` of it is the
    /// server's `308` to the path without it. Routing the same `path` again
    /// replaces the service it answers with.
    ///
    /// # Errors
    ///
    /// A `path` the server cannot route: one carrying a query, a fragment or
    /// a control byte.
    pub fn route(self: Arc<Self>, server: &Server, path: &str) -> Result<Url> {
        // A path the server would not route is refused here rather than
        // routed nowhere.
        let path = crate::http::server::normalize_path(path)?;
        let path = path.as_str();
        let endpoint = server.url_of(path)?;
        let provider = format!(
            "{} {}",
            self.options().provider_name,
            self.options().provider_version
        );
        server.route(Some(Method::Get), path, move |request| {
            // The endpoint as this request reached it, its query left off.
            let mut reached = request.url().clone();
            reached.set_query(None)?;
            Ok(Response::new(Status::OK).with_text(&format!(
                "{provider}: an XML for Analysis 1.1 provider at {reached}. POST a SOAP 1.1 Discover or Execute here.\n"
            )))
        });
        server.route(Some(Method::Post), path, move |request| {
            Arc::clone(&self).answer_http(request)
        });
        Ok(endpoint)
    }

    /// The answer to one `POST`: the message handed to [`Self::handle`] as
    /// the response is written, or the fault a body that is no XML earns.
    fn answer_http(self: Arc<Self>, request: &Request) -> Result<Response> {
        let soap_answer = Response::new(Status::OK)
            .with_header("content-type", soap::CONTENT_TYPE)?
            .with_header(NEGOTIATION.0, NEGOTIATION.1)?;
        if let Some(content_type) = request.headers().get("content-type") {
            // The media type, read as RFC 9110 reads it: a case-insensitive
            // `type/subtype`, `application/xmla+xml` and any `+xml` suffix an
            // XML document's.
            let base = content_type.split(';').next().unwrap_or("").trim();
            let is_xml = MimeType::from_content_type(base)
                .is_ok_and(|mime| mime == MimeType::XMLA || mime.format() == Some(Format::Xml));
            if !is_xml {
                let refused = refusal(format!(
                    "expected an XML content type for a SOAP message, got {base}"
                ))?;
                return Ok(soap_answer.with_body(refused));
            }
        }
        // The service is the one door the body goes through: it parses the
        // message and answers it, or writes the fault the parse earns.
        let message = match request.body() {
            Body::Empty => Arc::from([]),
            Body::Bytes(bytes) => Arc::clone(bytes),
        };
        Ok(soap_answer.with_writer(move |writer| {
            self.handle(&message, writer)?;
            Ok(())
        }))
    }
}

/// A `Client` fault carrying the XMLA `Error`, for a body that is no XMLA
/// request, as the bytes of its SOAP message.
fn refusal(description: String) -> Result<Vec<u8>> {
    let refused = fault(
        FaultCode::Client,
        XmlaError::new(code::BAD_REQUEST, description.clone()),
    )
    .unwrap_or_else(|_| Fault::client(description).with_actor(ACTOR));
    write_fault(Vec::new(), &[], &refused)
}
