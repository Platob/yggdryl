//! The [`Method`] enum.

use std::fmt;

use crate::error::HttpError;

/// An HTTP request method.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Method {
    /// `GET` — the default.
    #[default]
    Get,
    /// `POST`
    Post,
    /// `PUT`
    Put,
    /// `PATCH`
    Patch,
    /// `DELETE`
    Delete,
    /// `HEAD`
    Head,
    /// `OPTIONS`
    Options,
}

impl Method {
    /// Parses a method name (case-insensitive); an unknown method is an
    /// [`HttpError::InvalidHeader`].
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Method, HttpError> {
        let method = match value.trim().to_ascii_uppercase().as_str() {
            "GET" => Method::Get,
            "POST" => Method::Post,
            "PUT" => Method::Put,
            "PATCH" => Method::Patch,
            "DELETE" => Method::Delete,
            "HEAD" => Method::Head,
            "OPTIONS" => Method::Options,
            other => {
                return Err(HttpError::InvalidHeader(format!(
                    "unknown method {other:?}"
                )))
            }
        };
        Ok(method)
    }

    /// The canonical upper-case name (`"GET"`, `"POST"`, …).
    pub fn as_str(&self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Patch => "PATCH",
            Method::Delete => "DELETE",
            Method::Head => "HEAD",
            Method::Options => "OPTIONS",
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
