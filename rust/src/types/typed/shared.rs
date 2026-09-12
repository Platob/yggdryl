//! Prebuilt shared fields, one per leaf datatype.
//!
//! A [`FieldScalar`] borrows its field, and a value that names its own datatype
//! has no field to borrow - so the crate keeps one nullable `value` field per
//! leaf datatype for the life of the program, and [`FieldScalar`] borrows
//! that. A parameter-free leaf is built once into a table indexed by
//! [`DataTypeId`]; a leaf whose identity carries a parameter - a decimal's
//! scale, a timestamp's unit and zone, a fixed width - is interned on first
//! use, because its parameters are bounded and the table is, too. Nested
//! datatypes and geospatial parameters are unbounded, so nothing is kept for
//! them and a caller pairs those under a field of its own.
//!
//! [`FieldScalar`]: super::FieldScalar

use std::collections::HashMap;
use std::sync::{LazyLock, PoisonError, RwLock};

use crate::types::{BytesLayout, BytesParameters, StringLayout, StringParameters};
use crate::{DataType, DataTypeId, Field, Scalar};

/// The name every shared field carries - the name an inferred scalar field
/// carries too, so a value typed either way is the same column.
const SHARED_NAME: &str = "value";

/// How many parameterized leaf datatypes the interned table holds.
///
/// Past this many distinct datatypes the table answers `None` rather than
/// growing, because an interned field is never freed: the bound is what makes
/// leaking one per datatype a fixed cost rather than a leak.
const INTERN_LIMIT: usize = 1 << 12;

/// One nullable field per parameter-free leaf datatype, by [`DataTypeId::as_u8`].
///
/// The parser owns which name spells which datatype, so each slot parses the
/// identifier's canonical name rather than restating that table here; a slot
/// whose name parses to another identifier - the 128-bit integer widths,
/// which no datatype answers - stays empty.
static PREBUILT: LazyLock<[Option<Field>; DataTypeId::ALL.len()]> = LazyLock::new(|| {
    DataTypeId::ALL.map(|id| {
        if id.is_parameterized() {
            return None;
        }
        DataType::from_str(id.as_str())
            .ok()
            .filter(|dtype| dtype.id() == id)
            .map(|dtype| Field::new(SHARED_NAME, dtype, true))
    })
});

/// One nullable field per plain unbounded UTF-8 layout.
///
/// Every string identifier is parameterized, so none has a slot in
/// [`PREBUILT`]; these four are what a bare string value names, and a value
/// typed by inference borrows one of them rather than interning anything.
static PLAIN_UTF8: LazyLock<[(StringParameters, Field); 4]> = LazyLock::new(|| {
    [
        StringLayout::String,
        StringLayout::LargeString,
        StringLayout::StringView,
        StringLayout::LargeStringView,
    ]
    .map(|layout| {
        let parameters = StringParameters::utf8(layout);
        (
            parameters,
            Field::new(SHARED_NAME, DataType::String(parameters), true),
        )
    })
});

/// One nullable field per plain unbounded byte layout, for the same reason.
static PLAIN_BYTES: LazyLock<[(BytesParameters, Field); 3]> = LazyLock::new(|| {
    [
        BytesLayout::Binary,
        BytesLayout::LargeBinary,
        BytesLayout::BinaryView,
    ]
    .map(|layout| {
        let parameters = BytesParameters::new(layout);
        (
            parameters,
            Field::new(SHARED_NAME, DataType::Bytes(parameters), true),
        )
    })
});

/// The interned fields of parameterized leaf datatypes, bounded by
/// [`INTERN_LIMIT`].
static INTERNED: LazyLock<RwLock<HashMap<DataType, &'static Field>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

impl DataType {
    /// The shared nullable `value` field of this datatype, when it has one.
    ///
    /// Every parameter-free leaf answers a field built once for the program,
    /// and so does a plain unbounded UTF-8 string or a plain unbounded byte
    /// column in any layout; every other parameterized leaf - a string
    /// declaring a charset, a bound or a fixed width, bounded or fixed bytes,
    /// a decimal, a timestamp, a time, a duration, an interval - answers one
    /// interned on its first ask, so a second ask for the same datatype is a
    /// lookup that allocates nothing. A nested datatype, and a geometry or
    /// geography whose coordinate reference is unbounded text, answers
    /// `None`; so does a parameterized leaf once 4096 distinct ones are held,
    /// and one whose parameters are invalid.
    ///
    /// ```
    /// use yggdryl::{DataType, Field};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let shared = DataType::Int64.shared_field().unwrap();
    /// assert_eq!(shared.name(), "value");
    /// assert_eq!(shared.dtype(), &DataType::Int64);
    /// assert!(shared.is_nullable());
    /// // The same field every time, for a bare leaf and a parameterized one.
    /// assert!(std::ptr::eq(shared, DataType::Int64.shared_field().unwrap()));
    /// let price = DataType::decimal128(10, 2)?;
    /// assert!(std::ptr::eq(
    ///     price.shared_field().unwrap(),
    ///     price.shared_field().unwrap()
    /// ));
    /// // A nested datatype has no shared field: pair it under your own.
    /// let list = DataType::list(Field::new("item", DataType::Int64, true));
    /// assert!(list.shared_field().is_none());
    /// # Ok(())
    /// # }
    /// ```
    pub fn shared_field(&self) -> Option<&'static Field> {
        let id = self.id();
        if !id.is_parameterized() {
            return PREBUILT[usize::from(id.as_u8())].as_ref();
        }
        match self {
            Self::String(parameters) => PLAIN_UTF8
                .iter()
                .find(|(plain, _)| plain == parameters)
                .map(|(_, field)| field)
                .or_else(|| interned(self)),
            Self::Bytes(parameters) => PLAIN_BYTES
                .iter()
                .find(|(plain, _)| plain == parameters)
                .map(|(_, field)| field)
                .or_else(|| interned(self)),
            Self::DateTime64 { .. }
            | Self::Time32(_)
            | Self::Time64(_)
            | Self::Duration32(_)
            | Self::Duration64(_)
            | Self::Interval(_)
            | Self::Decimal32 { .. }
            | Self::Decimal64 { .. }
            | Self::Decimal128 { .. }
            | Self::Decimal256 { .. } => interned(self),
            _ => None,
        }
    }
}

/// The interned field of one parameterized leaf datatype.
fn interned(dtype: &DataType) -> Option<&'static Field> {
    if let Some(field) = INTERNED
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(dtype)
    {
        return Some(field);
    }
    // A datatype with an invalid parameter never earns a permanent field.
    dtype.validate().ok()?;
    let mut table = INTERNED.write().unwrap_or_else(PoisonError::into_inner);
    if let Some(field) = table.get(dtype) {
        return Some(field);
    }
    if table.len() >= INTERN_LIMIT {
        return None;
    }
    let field: &'static Field = Box::leak(Box::new(Field::new(SHARED_NAME, dtype.clone(), true)));
    table.insert(dtype.clone(), field);
    Some(field)
}

impl Scalar {
    /// The shared field of the datatype this value names, when it has one.
    ///
    /// [`Self::dtype`] reads what the value is, and
    /// [`DataType::shared_field`] answers the field the crate keeps for it.
    ///
    /// ```
    /// use yggdryl::{DataType, Scalar};
    ///
    /// let shared = Scalar::from("AAPL").shared_field().unwrap();
    /// assert_eq!(shared.dtype(), &DataType::utf8());
    /// assert!(Scalar::from_sequence([Scalar::from(1)]).shared_field().is_none());
    /// ```
    pub fn shared_field(&self) -> Option<&'static Field> {
        self.dtype().ok()?.shared_field()
    }
}

#[cfg(test)]
mod tests;
