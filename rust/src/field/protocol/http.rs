//! The `http:` vocabulary, on the field views that own it.
//!
//! One type reads each header key and one type writes it. The typed pairs -
//! `Content-Length`, the MIME and media projections, and `Location` - parse
//! and canonicalize here rather than at every caller.

use smol_str::SmolStr;

use super::{HttpField, HttpFieldMut};
use crate::metadata::{
    HTTP_ACCEPT_ENCODING_KEY, HTTP_ACCEPT_KEY, HTTP_ACCEPT_LANGUAGE_KEY, HTTP_ACCEPT_RANGES_KEY,
    HTTP_CACHE_CONTROL_KEY, HTTP_CONTENT_DISPOSITION_KEY, HTTP_CONTENT_ENCODING_KEY,
    HTTP_CONTENT_LANGUAGE_KEY, HTTP_CONTENT_LENGTH_KEY, HTTP_CONTENT_LOCATION_KEY,
    HTTP_CONTENT_RANGE_KEY, HTTP_CONTENT_TYPE_KEY, HTTP_ETAG_KEY, HTTP_EXPIRES_KEY,
    HTTP_LAST_MODIFIED_KEY, HTTP_LOCATION_KEY, HTTP_RANGE_KEY, HTTP_VARY_KEY, parse_content_length,
};
use crate::{Error, MediaType, MimeType, Result, Url};

impl<'field> HttpField<'field> {
    /// Returns the raw HTTP `Accept` field value.
    pub fn accept(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_ACCEPT_KEY)
    }

    /// Returns the raw HTTP `Accept-Encoding` field value.
    pub fn accept_encoding(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_ACCEPT_ENCODING_KEY)
    }

    /// Returns the raw HTTP `Accept-Language` field value.
    pub fn accept_language(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_ACCEPT_LANGUAGE_KEY)
    }

    /// Returns the raw HTTP `Accept-Ranges` field value.
    pub fn accept_ranges(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_ACCEPT_RANGES_KEY)
    }

    /// Returns the raw HTTP `Cache-Control` field value.
    pub fn cache_control(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_CACHE_CONTROL_KEY)
    }

    /// Returns the raw HTTP `Content-Disposition` field value.
    pub fn content_disposition(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_CONTENT_DISPOSITION_KEY)
    }

    /// Returns the raw HTTP `Content-Encoding` field value.
    pub fn content_encoding(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_CONTENT_ENCODING_KEY)
    }

    /// Returns the raw HTTP `Content-Language` field value.
    pub fn content_language(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_CONTENT_LANGUAGE_KEY)
    }

    /// Parses the canonical HTTP `Content-Length` field value.
    ///
    /// # Errors
    ///
    /// Every write canonicalizes this key, so an error can only originate from
    /// externally corrupted serialized state.
    pub fn content_length(&self) -> Result<Option<u64>> {
        self.as_field()
            .get_metadata(HTTP_CONTENT_LENGTH_KEY)
            .map(parse_content_length)
            .transpose()
    }

    /// Returns the raw HTTP `Content-Location` field value.
    pub fn content_location(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_CONTENT_LOCATION_KEY)
    }

    /// Returns the raw HTTP `Content-Range` field value.
    pub fn content_range(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_CONTENT_RANGE_KEY)
    }

    /// Returns the raw HTTP `Content-Type` field value, including parameters.
    pub fn content_type(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_CONTENT_TYPE_KEY)
    }

    /// Parses the base MIME type from HTTP `Content-Type`.
    ///
    /// Parameters are validated but remain available through
    /// [`Self::content_type`]. A missing header defaults to
    /// `application/octet-stream`.
    ///
    /// # Errors
    ///
    /// Returns an error when a present `Content-Type` is not valid MIME
    /// syntax.
    pub fn mime_type(&self) -> Result<MimeType> {
        self.content_type()
            .map(MimeType::from_content_type)
            .transpose()
            .map(Option::unwrap_or_default)
    }

    /// Parses HTTP `Content-Type` and `Content-Encoding` as one media value.
    ///
    /// A missing content type defaults to `application/octet-stream`.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed present MIME syntax or an unsupported
    /// content coding.
    pub fn media_type(&self) -> Result<MediaType> {
        MediaType::from_content_headers(self.content_type(), self.content_encoding())
    }

    /// Returns the raw HTTP `ETag` field value.
    pub fn etag(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_ETAG_KEY)
    }

    /// Returns the raw HTTP `Expires` field value.
    pub fn expires(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_EXPIRES_KEY)
    }

    /// Returns the raw HTTP `Last-Modified` field value.
    pub fn last_modified(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_LAST_MODIFIED_KEY)
    }

    /// Parses HTTP `Location` as an absolute URL.
    ///
    /// This is `http:location`, a different key from the namespace-free
    /// [`Field::location`](crate::Field::location) the field carries; the
    /// receiver is what says which one is meant.
    ///
    /// # Errors
    ///
    /// Raw `http:location` metadata may be relative or opaque; such a value is
    /// retained by generic access and reported as an error here.
    pub fn location(&self) -> Result<Option<Url>> {
        self.as_field()
            .get_metadata(HTTP_LOCATION_KEY)
            .map(Url::from_str)
            .transpose()
    }

    /// Returns the raw HTTP `Range` field value.
    pub fn range(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_RANGE_KEY)
    }

    /// Returns the raw HTTP `Vary` field value.
    pub fn vary(&self) -> Option<&'field str> {
        self.as_field().get_metadata(HTTP_VARY_KEY)
    }
}

impl HttpFieldMut<'_> {
    /// Sets a validated raw HTTP `Accept` field value.
    ///
    /// # Errors
    ///
    /// Returns an error when the value fails the validation its `http:` key
    /// carries, leaving the field unchanged. Every other raw setter here
    /// fails the same way.
    pub fn set_accept(&mut self, value: impl Into<String>) -> Result<()> {
        self.0.field.insert_metadata(HTTP_ACCEPT_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Accept` field value.
    pub fn remove_accept(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_ACCEPT_KEY)
    }

    /// Sets a validated raw HTTP `Accept-Encoding` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_accept_encoding(&mut self, value: impl Into<String>) -> Result<()> {
        self.0
            .field
            .insert_metadata(HTTP_ACCEPT_ENCODING_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Accept-Encoding` field value.
    pub fn remove_accept_encoding(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_ACCEPT_ENCODING_KEY)
    }

    /// Sets a validated raw HTTP `Accept-Language` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_accept_language(&mut self, value: impl Into<String>) -> Result<()> {
        self.0
            .field
            .insert_metadata(HTTP_ACCEPT_LANGUAGE_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Accept-Language` field value.
    pub fn remove_accept_language(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_ACCEPT_LANGUAGE_KEY)
    }

    /// Sets a validated raw HTTP `Accept-Ranges` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_accept_ranges(&mut self, value: impl Into<String>) -> Result<()> {
        self.0
            .field
            .insert_metadata(HTTP_ACCEPT_RANGES_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Accept-Ranges` field value.
    pub fn remove_accept_ranges(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_ACCEPT_RANGES_KEY)
    }

    /// Sets a validated raw HTTP `Cache-Control` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_cache_control(&mut self, value: impl Into<String>) -> Result<()> {
        self.0
            .field
            .insert_metadata(HTTP_CACHE_CONTROL_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Cache-Control` field value.
    pub fn remove_cache_control(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_CACHE_CONTROL_KEY)
    }

    /// Sets a validated raw HTTP `Content-Disposition` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_content_disposition(&mut self, value: impl Into<String>) -> Result<()> {
        self.0
            .field
            .insert_metadata(HTTP_CONTENT_DISPOSITION_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Content-Disposition` field value.
    pub fn remove_content_disposition(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_CONTENT_DISPOSITION_KEY)
    }

    /// Sets a validated raw HTTP `Content-Encoding` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_content_encoding(&mut self, value: impl Into<String>) -> Result<()> {
        self.0
            .field
            .insert_metadata(HTTP_CONTENT_ENCODING_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Content-Encoding` field value.
    pub fn remove_content_encoding(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_CONTENT_ENCODING_KEY)
    }

    /// Sets a validated raw HTTP `Content-Language` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_content_language(&mut self, value: impl Into<String>) -> Result<()> {
        self.0
            .field
            .insert_metadata(HTTP_CONTENT_LANGUAGE_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Content-Language` field value.
    pub fn remove_content_language(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_CONTENT_LANGUAGE_KEY)
    }

    /// Sets canonical HTTP `Content-Length` metadata.
    pub fn set_content_length(&mut self, value: u64) {
        let (_, changed) = self
            .0
            .field
            .metadata
            .insert_validated(HTTP_CONTENT_LENGTH_KEY.to_owned(), value.to_string());
        if changed {
            self.0.field.invalidate_arrow();
        }
    }

    /// Removes and parses the prior HTTP `Content-Length` value.
    ///
    /// # Errors
    ///
    /// [`HttpField::content_length`] carries when the stored value can fail to
    /// parse; this field is left unchanged when it does.
    pub fn remove_content_length(&mut self) -> Result<Option<u64>> {
        let previous = self.as_protocol().content_length()?;
        if previous.is_some() {
            self.0.field.remove_metadata(HTTP_CONTENT_LENGTH_KEY);
        }
        Ok(previous)
    }

    /// Sets a validated raw HTTP `Content-Location` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_content_location(&mut self, value: impl Into<String>) -> Result<()> {
        self.0
            .field
            .insert_metadata(HTTP_CONTENT_LOCATION_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Content-Location` field value.
    pub fn remove_content_location(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_CONTENT_LOCATION_KEY)
    }

    /// Sets a validated raw HTTP `Content-Range` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_content_range(&mut self, value: impl Into<String>) -> Result<()> {
        self.0
            .field
            .insert_metadata(HTTP_CONTENT_RANGE_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Content-Range` field value.
    pub fn remove_content_range(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_CONTENT_RANGE_KEY)
    }

    /// Sets a validated raw HTTP `Content-Type` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_content_type(&mut self, value: impl Into<String>) -> Result<()> {
        self.0.field.insert_metadata(HTTP_CONTENT_TYPE_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Content-Type` field value.
    pub fn remove_content_type(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_CONTENT_TYPE_KEY)
    }

    /// Sets the bare HTTP `Content-Type` MIME value and preserves encodings.
    pub fn set_mime_type(&mut self, value: MimeType) {
        let (_, changed) = self
            .0
            .field
            .metadata
            .insert_validated(HTTP_CONTENT_TYPE_KEY.to_owned(), value.to_string());
        if changed {
            self.0.field.invalidate_arrow();
        }
    }

    /// Removes and parses the prior HTTP `Content-Type` MIME value.
    ///
    /// Existing `Content-Encoding` metadata is deliberately preserved.
    ///
    /// # Errors
    ///
    /// Returns an error when the stored `Content-Type` is not valid MIME
    /// syntax, leaving this field unchanged.
    pub fn remove_mime_type(&mut self) -> Result<Option<MimeType>> {
        let Some(content_type) = self.as_protocol().content_type() else {
            return Ok(None);
        };
        let previous = MimeType::from_content_type(content_type)?;
        self.0.field.remove_metadata(HTTP_CONTENT_TYPE_KEY);
        Ok(Some(previous))
    }

    /// Atomically projects a media value to HTTP content headers.
    ///
    /// # Errors
    ///
    /// File encodings without registered HTTP coding tokens are rejected
    /// before either metadata key or the Arrow projection cache is changed.
    pub fn set_media_type(&mut self, value: MediaType) -> Result<()> {
        let content_type = value.base().to_string();
        let mut content_encoding = String::new();
        for encoding in value.encodings() {
            let coding = encoding
                .content_coding()
                .ok_or_else(|| Error::InvalidMetadataValue {
                    key: SmolStr::new_static(HTTP_CONTENT_ENCODING_KEY),
                    reason: SmolStr::new_static(
                        "media encoding has no registered HTTP Content-Encoding token",
                    ),
                })?;
            if !content_encoding.is_empty() {
                content_encoding.push_str(", ");
            }
            content_encoding.push_str(coding);
        }

        let mut metadata = self.0.field.metadata.clone();
        metadata.insert_validated(HTTP_CONTENT_TYPE_KEY.to_owned(), content_type);
        if content_encoding.is_empty() {
            metadata.remove(HTTP_CONTENT_ENCODING_KEY);
        } else {
            metadata.insert_validated(HTTP_CONTENT_ENCODING_KEY.to_owned(), content_encoding);
        }
        if metadata != self.0.field.metadata {
            self.0.field.metadata = metadata;
            self.0.field.invalidate_arrow();
        }
        Ok(())
    }

    /// Removes both HTTP media header keys after parsing their prior value.
    ///
    /// # Errors
    ///
    /// If either stored header is malformed, this field remains unchanged.
    pub fn remove_media_type(&mut self) -> Result<Option<MediaType>> {
        let view = self.as_protocol();
        if view.content_type().is_none() && view.content_encoding().is_none() {
            return Ok(None);
        }
        let previous = view.media_type()?;
        let mut metadata = self.0.field.metadata.clone();
        metadata.remove(HTTP_CONTENT_TYPE_KEY);
        metadata.remove(HTTP_CONTENT_ENCODING_KEY);
        self.0.field.metadata = metadata;
        self.0.field.invalidate_arrow();
        Ok(Some(previous))
    }

    /// Sets a validated raw HTTP `ETag` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_etag(&mut self, value: impl Into<String>) -> Result<()> {
        self.0.field.insert_metadata(HTTP_ETAG_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `ETag` field value.
    pub fn remove_etag(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_ETAG_KEY)
    }

    /// Sets a validated raw HTTP `Expires` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_expires(&mut self, value: impl Into<String>) -> Result<()> {
        self.0.field.insert_metadata(HTTP_EXPIRES_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Expires` field value.
    pub fn remove_expires(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_EXPIRES_KEY)
    }

    /// Sets a validated raw HTTP `Last-Modified` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_last_modified(&mut self, value: impl Into<String>) -> Result<()> {
        self.0
            .field
            .insert_metadata(HTTP_LAST_MODIFIED_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Last-Modified` field value.
    pub fn remove_last_modified(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_LAST_MODIFIED_KEY)
    }

    /// Sets typed absolute HTTP `Location` metadata.
    pub fn set_location(&mut self, value: Url) {
        let (_, changed) = self
            .0
            .field
            .metadata
            .insert_validated(HTTP_LOCATION_KEY.to_owned(), value.to_string());
        if changed {
            self.0.field.invalidate_arrow();
        }
    }

    /// Removes and parses the prior typed HTTP `Location` URL.
    ///
    /// # Errors
    ///
    /// [`HttpField::location`] carries when the stored value can fail to
    /// parse; this field is left unchanged when it does.
    pub fn remove_location(&mut self) -> Result<Option<Url>> {
        let previous = self.as_protocol().location()?;
        if previous.is_some() {
            self.0.field.remove_metadata(HTTP_LOCATION_KEY);
        }
        Ok(previous)
    }

    /// Sets a validated raw HTTP `Range` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_range(&mut self, value: impl Into<String>) -> Result<()> {
        self.0.field.insert_metadata(HTTP_RANGE_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Range` field value.
    pub fn remove_range(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_RANGE_KEY)
    }

    /// Sets a validated raw HTTP `Vary` field value.
    ///
    /// # Errors
    ///
    /// [`Self::set_accept`] carries the rule.
    pub fn set_vary(&mut self, value: impl Into<String>) -> Result<()> {
        self.0.field.insert_metadata(HTTP_VARY_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Vary` field value.
    pub fn remove_vary(&mut self) -> Option<String> {
        self.0.field.remove_metadata(HTTP_VARY_KEY)
    }
}
