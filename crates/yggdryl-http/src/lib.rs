//! # yggdryl-http
//!
//! A blocking, requests-like HTTP client that streams response bodies through
//! the `yggdryl-core` byte-IO abstraction.
//!
//! ## Quick start
//!
//! ```no_run
//! use yggdryl_http::{HttpSession, HttpRequest, Method};
//!
//! let session = HttpSession::new();
//!
//! // Simple GET
//! let resp = session.get("https://example.com")?;
//! assert!(resp.ok());
//! println!("status: {}", resp.status);
//! let body = resp.text()?;
//!
//! // Builder-style request
//! let resp = session.request(
//!     HttpRequest::post("https://api.example.com/data")?
//!         .with_header("content-type", "application/json")
//!         .with_body(b"{\"hello\":\"world\"}")
//! )?;
//! resp.raise_for_status()?;
//! # Ok::<_, yggdryl_http::HttpError>(())
//! ```
//!
//! ## Features
//!
//! | Feature | Effect |
//! |---------|--------|
//! | `compression` | Auto-decompress `Content-Encoding` bodies (on by default) |
//! | `http2` | Enable HTTP/2 via hyper + tokio |
//! | `http3` | Enable HTTP/3 via quinn + h3 |
//! | `media` | Expose `mime_type()` on responses |
//! | `serde` | `Serialize`/`Deserialize` for data types |
//! | `log` | Structured logging via the `log` facade |

/// Emits a `log` event when the `log` feature is on; expands to nothing
/// otherwise. Avoids a direct `log::` dependency in the default build.
macro_rules! log_event {
    ($level:ident, $($arg:tt)+) => {{
        #[cfg(feature = "log")]
        log::$level!($($arg)+);
    }};
}
pub(crate) use log_event;

mod body;
mod cookie;
mod error;
mod method;
mod request;
mod response;
mod retry;
mod session;
mod stream;
pub(crate) mod transport;
mod version;

pub use body::RequestBody;
pub use cookie::{Cookie, CookieJar};
pub use error::HttpError;
pub use method::Method;
pub use request::HttpRequest;
pub use response::HttpResponse;
pub use retry::RetryConfig;
pub use session::{HttpSession, SessionConfig};
pub use stream::HttpStream;
pub use version::HttpVersion;
