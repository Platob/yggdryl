//! [`Field`] — a **column's metadata**: its name, element [`DataTypeId`], and nullability, carried
//! in a [`Headers`] map.
//!
//! A `Field` is what a schema holds and a [`Serie`](super::Serie) reports about itself. The concrete
//! [`HeaderField`] *is* a `Headers` (the project's one metadata map): the name lives in
//! [`Headers::NAME`], the type in [`Headers::TYPE_ID`], the nullable flag in [`Headers::NULLABLE`] —
//! so a field serializes, hashes, and travels exactly like any other metadata, and arbitrary extra
//! annotations ride alongside for free.

use crate::datatype_id::DataTypeId;
use crate::headers::Headers;

/// A typed column descriptor: `name`, element type, and nullability — plus the open [`Headers`] map
/// the metadata lives in.
pub trait Field {
    /// The column name, if set.
    fn name(&self) -> Option<&str>;

    /// The element [`DataTypeId`].
    fn data_type_id(&self) -> DataTypeId;

    /// Whether the column admits nulls.
    fn nullable(&self) -> bool;

    /// The backing metadata map (the field's name/type/nullable live here, alongside any extras).
    fn headers(&self) -> &Headers;
}

/// A [`Field`] backed by a [`Headers`] map — the metadata `name` / `type_id` / `nullable` are three
/// entries in the map, so the field is a plain, serializable, hashable value.
///
/// ```
/// use yggdryl_core::typed::{Field, HeaderField};
/// use yggdryl_core::datatype_id::DataTypeId;
///
/// let field = HeaderField::new(Some("price"), DataTypeId::I64, true);
/// assert_eq!(field.name(), Some("price"));
/// assert_eq!(field.data_type_id(), DataTypeId::I64);
/// assert!(field.nullable());
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct HeaderField {
    headers: Headers,
}

impl HeaderField {
    /// A field from its `name`, element `type_id`, and `nullable` flag.
    pub fn new(name: Option<&str>, type_id: DataTypeId, nullable: bool) -> Self {
        let mut headers = Headers::new();
        if let Some(name) = name {
            headers.set_name(name);
        }
        headers.set_type_id(type_id);
        headers.set_nullable(nullable);
        HeaderField { headers }
    }

    /// A field wrapping an existing [`Headers`] map (its `type_id`/`name`/`nullable` are read from it).
    pub fn from_headers(headers: Headers) -> Self {
        HeaderField { headers }
    }

    /// The mutable metadata map — annotate the field with any extra headers.
    pub fn headers_mut(&mut self) -> &mut Headers {
        &mut self.headers
    }

    /// Consumes the field into its [`Headers`].
    pub fn into_headers(self) -> Headers {
        self.headers
    }
}

impl Field for HeaderField {
    fn name(&self) -> Option<&str> {
        self.headers.name()
    }

    fn data_type_id(&self) -> DataTypeId {
        self.headers.type_id()
    }

    fn nullable(&self) -> bool {
        self.headers.nullable()
    }

    fn headers(&self) -> &Headers {
        &self.headers
    }
}
