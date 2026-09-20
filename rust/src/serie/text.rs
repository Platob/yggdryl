//! Every column whose rows are a run of bytes: text, raw bytes, and the
//! identities stored as either.
//!
//! Arrow lays a variable-length run out as offsets into one payload buffer
//! ([`ByteSerie`]), as views into several ([`ByteViewSerie`]), or as one
//! fixed width with no offsets at all ([`FixedSerie`]). Each is one generic
//! type here and each leaf is a name for one instantiation, so
//! [`Utf8StringSerie::offsets`] lends the offsets and
//! [`Utf8StringSerie::payload`] the characters, both where they lie.
//!
//! A leaf says how the bytes are laid out; the field says what they *are* -
//! which of the crate's eighteen string leaves, which of its eleven codes,
//! a UUID, a geospatial reading. That split is why one layout can be a
//! string leaf and a byte leaf at once, and why the two are told apart by a
//! marker rather than by the buffers: [`TextRun`] and [`RawRun`] are what
//! [`BinaryStringSerie`] and [`BinarySerie`] differ by.

use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use arrow_array::builder::{FixedSizeBinaryBuilder, GenericByteBuilder, GenericByteViewBuilder};
use arrow_array::types::{
    BinaryType, BinaryViewType, ByteArrayType, ByteViewType, LargeBinaryType, LargeUtf8Type,
    StringViewType, Utf8Type,
};
use arrow_array::{
    Array, ArrayRef, FixedSizeBinaryArray, GenericByteArray, GenericByteViewArray, OffsetSizeTrait,
};
use arrow_buffer::{Buffer, NullBuffer, OffsetBuffer};

use super::{Serie, one_row};
use crate::value::SerieValue;
use crate::{Field, Result, Scalar};

/// Which family a run of bytes belongs to when its layout belongs to both.
///
/// A windows-1252 string column and a byte column are the same Arrow binary
/// buffers; what tells them apart is the field, and this is that answer made
/// a type so the two are different columns rather than one wearing two names.
pub trait ByteKind: Send + Sync + Clone + Copy + fmt::Debug + 'static {
    /// Widen a run of bytes under this marker to the serie root.
    fn into_serie(column: Serie) -> Serie {
        column
    }
}

/// The marker for a run of bytes a field reads as text.
///
/// Named for the run rather than for the bytes, because this crate already
/// has a [`TextBytes`](crate::text::TextBytes) - the payload of one text
/// line - and a reader should not have to know which of the two a name
/// means.
#[derive(Clone, Copy, Debug)]
pub struct TextRun;

/// The marker for a run of bytes a field reads as bytes.
#[derive(Clone, Copy, Debug)]
pub struct RawRun;

impl ByteKind for TextRun {}
impl ByteKind for RawRun {}

/// Which leaf of the root one byte layout under one marker widens to.
pub trait ByteLeaf<K: ByteKind>: ByteArrayType + Sized {
    /// Widen a column of this layout to the serie root.
    fn into_serie(column: ByteSerie<Self, K>) -> Serie;

    /// Narrow a serie root to a column of this layout.
    fn from_serie(serie: &Serie) -> Option<&ByteSerie<Self, K>>;
}

/// One column of variable-length runs: offsets into one payload buffer.
pub struct ByteSerie<T: ByteArrayType, K: ByteKind> {
    field: Arc<Field>,
    values: GenericByteArray<T>,
    kind: PhantomData<K>,
}

impl<T: ByteArrayType, K: ByteKind> ByteSerie<T, K> {
    /// Pair a field with the buffers that hold its rows.
    pub(crate) const fn new(field: Arc<Field>, values: GenericByteArray<T>) -> Self {
        Self {
            field,
            values,
            kind: PhantomData,
        }
    }

    /// Borrow the offsets buffer, without copying it.
    ///
    /// Row `i` occupies the payload between `offsets[i]` and `offsets[i + 1]`,
    /// which is what a reader walking runs without building one needs.
    pub fn offsets(&self) -> &OffsetBuffer<T::Offset> {
        self.values.offsets()
    }

    /// Borrow the payload buffer every run lies in, without copying it.
    pub fn payload(&self) -> &Buffer {
        self.values.values()
    }

    /// Borrow the validity bitmap, or `None` where no row is absent.
    pub fn nulls(&self) -> Option<&NullBuffer> {
        self.values.nulls()
    }

    /// Borrow the Arrow array these buffers are.
    pub const fn array(&self) -> &GenericByteArray<T> {
        &self.values
    }

    /// Borrow row `index` where it lies in the payload.
    pub fn value(&self, index: usize) -> Option<&T::Native> {
        (index < self.values.len() && !self.values.is_null(index)).then(|| self.values.value(index))
    }

    /// Append one native row, without building a value for it.
    pub fn push_value(&mut self, value: Option<&T::Native>) {
        self.edit(|builder| builder.append_option(value));
    }

    /// Overwrite row `index` with one native run.
    ///
    /// A run is as long as it is, so every later offset moves: this rebuilds
    /// the offsets and the payload once, which is what a variable-length
    /// layout costs.
    ///
    /// # Errors
    ///
    /// Returns an error when `index` is past the end.
    pub fn set_value(&mut self, index: usize, value: Option<&T::Native>) -> Result<()> {
        super::require_row(self.field.name(), index, self.values.len())?;
        let held = self.values.clone();
        let mut builder = GenericByteBuilder::<T>::new();
        for row in 0..held.len() {
            if row == index {
                builder.append_option(value);
            } else if held.is_null(row) {
                builder.append_null();
            } else {
                builder.append_value(held.value(row));
            }
        }
        self.values = builder.finish();
        Ok(())
    }

    /// Run one edit against this column's own builder.
    ///
    /// Arrow hands the buffers back as a builder when nothing else holds
    /// them; when something does, the rows are copied once.
    fn edit(&mut self, change: impl FnOnce(&mut GenericByteBuilder<T>)) {
        let taken = std::mem::replace(&mut self.values, GenericByteBuilder::<T>::new().finish());
        let mut builder = match taken.into_builder() {
            Ok(builder) => builder,
            Err(shared) => {
                let mut builder = GenericByteBuilder::<T>::new();
                for row in 0..shared.len() {
                    if shared.is_null(row) {
                        builder.append_null();
                    } else {
                        builder.append_value(shared.value(row));
                    }
                }
                builder
            }
        };
        change(&mut builder);
        self.values = builder.finish();
    }
}

impl<T: ByteArrayType, K: ByteKind> SerieValue for ByteSerie<T, K>
where
    T: ByteLeaf<K>,
    T::Offset: OffsetSizeTrait,
{
    fn field(&self) -> &Field {
        &self.field
    }

    fn len(&self) -> usize {
        self.values.len()
    }

    fn null_count(&self) -> usize {
        self.values.null_count()
    }

    fn is_null(&self, index: usize) -> bool {
        index >= self.values.len() || self.values.is_null(index)
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        super::scalar_at(&self.field, &self.values, index)
    }

    fn set(&mut self, index: usize, value: Scalar) -> Result<()> {
        super::require_row(self.field.name(), index, self.values.len())?;
        let row = one_row::<GenericByteArray<T>>(&self.field, value)?;
        if row.is_null(0) {
            self.set_value(index, None)
        } else {
            let held = row.value(0);
            self.set_value(index, Some(held))
        }
    }

    fn push(&mut self, value: Scalar) -> Result<()> {
        let row = one_row::<GenericByteArray<T>>(&self.field, value)?;
        if row.is_null(0) {
            self.push_value(None);
        } else {
            self.push_value(Some(row.value(0)));
        }
        Ok(())
    }

    fn into_arrow_array(&self) -> ArrayRef {
        Arc::new(self.values.clone())
    }

    fn into_serie(self) -> Serie {
        T::into_serie(self)
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        T::from_serie(value)
    }
}

impl<T: ByteArrayType, K: ByteKind> Clone for ByteSerie<T, K> {
    fn clone(&self) -> Self {
        Self {
            field: Arc::clone(&self.field),
            values: self.values.clone(),
            kind: PhantomData,
        }
    }
}

impl<T, K> fmt::Debug for ByteSerie<T, K>
where
    T: ByteLeaf<K>,
    K: ByteKind,
    T::Offset: OffsetSizeTrait,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_leaf(self, "ByteSerie", formatter)
    }
}

/// Name one byte layout under one marker as a leaf of the root.
macro_rules! byte_leaf {
    ($(#[$meta:meta])* $name:ident, $arrow:ty, $marker:ty, $family:ident, $held:ident, $variant:ident) => {
        $(#[$meta])*
        pub type $name = ByteSerie<$arrow, $marker>;

        impl ByteLeaf<$marker> for $arrow {
            fn into_serie(column: ByteSerie<Self, $marker>) -> Serie {
                Serie::$family(::std::sync::Arc::new(super::$held::$variant(column)))
            }

            fn from_serie(serie: &Serie) -> Option<&ByteSerie<Self, $marker>> {
                match serie {
                    Serie::$family(family) => match family.as_ref() {
                        super::$held::$variant(column) => Some(column),
                        _ => None,
                    },
                    _ => None,
                }
            }
        }
    };
}

byte_leaf!(
    /// A column of UTF-8 text, 32-bit offsets.
    Utf8StringSerie,
    Utf8Type,
    TextRun,
    String,
    StringSerie,
    Utf8
);
byte_leaf!(
    /// A column of UTF-8 text, 64-bit offsets.
    LargeUtf8StringSerie,
    LargeUtf8Type,
    TextRun,
    String,
    StringSerie,
    LargeUtf8
);
byte_leaf!(
    /// A column of text in a charset Arrow cannot state, 32-bit offsets.
    BinaryStringSerie,
    BinaryType,
    TextRun,
    String,
    StringSerie,
    Binary
);
byte_leaf!(
    /// A column of text in a charset Arrow cannot state, 64-bit offsets.
    LargeBinaryStringSerie,
    LargeBinaryType,
    TextRun,
    String,
    StringSerie,
    LargeBinary
);
byte_leaf!(
    /// A column of byte runs, 32-bit offsets.
    BinarySerie,
    BinaryType,
    RawRun,
    Bytes,
    BytesSerie,
    Binary
);
byte_leaf!(
    /// A column of byte runs, 64-bit offsets.
    LargeBinarySerie,
    LargeBinaryType,
    RawRun,
    Bytes,
    BytesSerie,
    LargeBinary
);

// ------------------------------------------------------------------------
// Views: runs that name where they lie rather than lying in one buffer.
// ------------------------------------------------------------------------

/// Which leaf of the root one view layout under one marker widens to.
pub trait ViewLeaf<K: ByteKind>: ByteViewType + Sized {
    /// Widen a column of this layout to the serie root.
    fn into_serie(column: ByteViewSerie<Self, K>) -> Serie;

    /// Narrow a serie root to a column of this layout.
    fn from_serie(serie: &Serie) -> Option<&ByteViewSerie<Self, K>>;
}

/// One column of runs held as views into several payload buffers.
pub struct ByteViewSerie<T: ByteViewType, K: ByteKind> {
    field: Arc<Field>,
    values: GenericByteViewArray<T>,
    kind: PhantomData<K>,
}

impl<T: ByteViewType, K: ByteKind> ByteViewSerie<T, K> {
    /// Pair a field with the buffers that hold its rows.
    pub(crate) const fn new(field: Arc<Field>, values: GenericByteViewArray<T>) -> Self {
        Self {
            field,
            values,
            kind: PhantomData,
        }
    }

    /// Borrow the views buffer, one 128-bit view per row.
    pub fn views(&self) -> &[u128] {
        self.values.views()
    }

    /// Borrow the payload buffers the views name.
    pub fn payloads(&self) -> &[Buffer] {
        self.values.data_buffers()
    }

    /// Borrow the validity bitmap, or `None` where no row is absent.
    pub fn nulls(&self) -> Option<&NullBuffer> {
        self.values.nulls()
    }

    /// Borrow the Arrow array these buffers are.
    pub const fn array(&self) -> &GenericByteViewArray<T> {
        &self.values
    }

    /// Borrow row `index` where its view names it.
    pub fn value(&self, index: usize) -> Option<&T::Native> {
        (index < self.values.len() && !self.values.is_null(index)).then(|| self.values.value(index))
    }

    /// Append one native row, without building a value for it.
    ///
    /// Arrow hands no builder back for a view array, and a view word names
    /// which payload buffer holds its run, so the views are rewritten once.
    /// That is what this layout costs an append; the reads above stay one
    /// load.
    pub fn push_value(&mut self, value: Option<&T::Native>) {
        let mut builder = GenericByteViewBuilder::<T>::new();
        for row in 0..self.values.len() {
            if self.values.is_null(row) {
                builder.append_null();
            } else {
                builder.append_value(self.values.value(row));
            }
        }
        builder.append_option(value);
        self.values = builder.finish();
    }
}

impl<T: ByteViewType, K: ByteKind> SerieValue for ByteViewSerie<T, K>
where
    T: ViewLeaf<K>,
{
    fn field(&self) -> &Field {
        &self.field
    }

    fn len(&self) -> usize {
        self.values.len()
    }

    fn null_count(&self) -> usize {
        self.values.null_count()
    }

    fn is_null(&self, index: usize) -> bool {
        index >= self.values.len() || self.values.is_null(index)
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        super::scalar_at(&self.field, &self.values, index)
    }

    fn set(&mut self, index: usize, value: Scalar) -> Result<()> {
        super::require_row(self.field.name(), index, self.values.len())?;
        let row = one_row::<GenericByteViewArray<T>>(&self.field, value)?;
        let mut builder = GenericByteViewBuilder::<T>::new();
        for held in 0..self.values.len() {
            let source = if held == index { &row } else { &self.values };
            let at = if held == index { 0 } else { held };
            if source.is_null(at) {
                builder.append_null();
            } else {
                builder.append_value(source.value(at));
            }
        }
        self.values = builder.finish();
        Ok(())
    }

    fn push(&mut self, value: Scalar) -> Result<()> {
        let row = one_row::<GenericByteViewArray<T>>(&self.field, value)?;
        if row.is_null(0) {
            self.push_value(None);
        } else {
            self.push_value(Some(row.value(0)));
        }
        Ok(())
    }

    fn into_arrow_array(&self) -> ArrayRef {
        Arc::new(self.values.clone())
    }

    fn into_serie(self) -> Serie {
        T::into_serie(self)
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        T::from_serie(value)
    }
}

impl<T: ByteViewType, K: ByteKind> Clone for ByteViewSerie<T, K> {
    fn clone(&self) -> Self {
        Self {
            field: Arc::clone(&self.field),
            values: self.values.clone(),
            kind: PhantomData,
        }
    }
}

impl<T: ViewLeaf<K>, K: ByteKind> fmt::Debug for ByteViewSerie<T, K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_leaf(self, "ByteViewSerie", formatter)
    }
}

/// Name one view layout under one marker as a leaf of the root.
macro_rules! view_leaf {
    ($(#[$meta:meta])* $name:ident, $arrow:ty, $marker:ty, $family:ident, $held:ident, $variant:ident) => {
        $(#[$meta])*
        pub type $name = ByteViewSerie<$arrow, $marker>;

        impl ViewLeaf<$marker> for $arrow {
            fn into_serie(column: ByteViewSerie<Self, $marker>) -> Serie {
                Serie::$family(::std::sync::Arc::new(super::$held::$variant(column)))
            }

            fn from_serie(serie: &Serie) -> Option<&ByteViewSerie<Self, $marker>> {
                match serie {
                    Serie::$family(family) => match family.as_ref() {
                        super::$held::$variant(column) => Some(column),
                        _ => None,
                    },
                    _ => None,
                }
            }
        }
    };
}

view_leaf!(
    /// A column of UTF-8 text held as views.
    Utf8ViewStringSerie,
    StringViewType,
    TextRun,
    String,
    StringSerie,
    Utf8View
);
view_leaf!(
    /// A column of text in another charset, held as views.
    BinaryViewStringSerie,
    BinaryViewType,
    TextRun,
    String,
    StringSerie,
    BinaryView
);
view_leaf!(
    /// A column of byte runs held as views.
    BinaryViewSerie,
    BinaryViewType,
    RawRun,
    Bytes,
    BytesSerie,
    BinaryView
);

// ------------------------------------------------------------------------
// Fixed width: no offsets, because every run is the same length.
// ------------------------------------------------------------------------

/// One column of fixed-width byte runs: one payload buffer and no offsets.
pub struct FixedSerie<K: ByteKind> {
    field: Arc<Field>,
    values: FixedSizeBinaryArray,
    kind: PhantomData<K>,
}

impl<K: ByteKind> FixedSerie<K> {
    /// Pair a field with the buffer that holds its rows.
    pub(crate) const fn new(field: Arc<Field>, values: FixedSizeBinaryArray) -> Self {
        Self {
            field,
            values,
            kind: PhantomData,
        }
    }

    /// Return the width every row occupies.
    pub fn width(&self) -> i32 {
        self.values.value_length()
    }

    /// Borrow the payload buffer every run lies in, without copying it.
    pub fn payload(&self) -> &Buffer {
        self.values.values()
    }

    /// Borrow the validity bitmap, or `None` where no row is absent.
    pub fn nulls(&self) -> Option<&NullBuffer> {
        self.values.nulls()
    }

    /// Borrow the Arrow array this buffer is.
    pub const fn array(&self) -> &FixedSizeBinaryArray {
        &self.values
    }

    /// Borrow row `index` where it lies in the payload.
    pub fn value(&self, index: usize) -> Option<&[u8]> {
        (index < self.values.len() && !self.values.is_null(index)).then(|| self.values.value(index))
    }

    /// Append one native row, without building a value for it.
    ///
    /// Arrow hands no builder back for a fixed-width array, so the payload is
    /// rewritten once; the reads above stay one load at a known offset.
    ///
    /// # Errors
    ///
    /// Returns an error when the run is not this column's width.
    pub fn push_value(&mut self, value: Option<&[u8]>) -> Result<()> {
        let mut builder =
            FixedSizeBinaryBuilder::with_capacity(self.values.len() + 1, self.width());
        for row in 0..self.values.len() {
            if self.values.is_null(row) {
                builder.append_null();
            } else {
                append_fixed(&mut builder, Some(self.values.value(row)))?;
            }
        }
        append_fixed(&mut builder, value)?;
        self.values = builder.finish();
        Ok(())
    }
}

/// Append one fixed-width run, naming a width that is not the column's.
fn append_fixed(builder: &mut FixedSizeBinaryBuilder, value: Option<&[u8]>) -> Result<()> {
    match value {
        Some(bytes) => builder
            .append_value(bytes)
            .map_err(|error| crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$"),
                reason: smol_str::format_smolstr!("{error}"),
            }),
        None => {
            builder.append_null();
            Ok(())
        }
    }
}

impl<K: ByteKind> SerieValue for FixedSerie<K>
where
    Self: FixedLeaf,
{
    fn field(&self) -> &Field {
        &self.field
    }

    fn len(&self) -> usize {
        self.values.len()
    }

    fn null_count(&self) -> usize {
        self.values.null_count()
    }

    fn is_null(&self, index: usize) -> bool {
        index >= self.values.len() || self.values.is_null(index)
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        super::scalar_at(&self.field, &self.values, index)
    }

    fn set(&mut self, index: usize, value: Scalar) -> Result<()> {
        super::require_row(self.field.name(), index, self.values.len())?;
        let row = one_row::<FixedSizeBinaryArray>(&self.field, value)?;
        let mut builder = FixedSizeBinaryBuilder::with_capacity(self.values.len(), self.width());
        for held in 0..self.values.len() {
            let source = if held == index { &row } else { &self.values };
            let at = if held == index { 0 } else { held };
            if source.is_null(at) {
                builder.append_null();
            } else {
                append_fixed(&mut builder, Some(source.value(at)))?;
            }
        }
        self.values = builder.finish();
        Ok(())
    }

    fn push(&mut self, value: Scalar) -> Result<()> {
        let row = one_row::<FixedSizeBinaryArray>(&self.field, value)?;
        if row.is_null(0) {
            self.push_value(None)
        } else {
            let held = row.value(0).to_vec();
            self.push_value(Some(&held))
        }
    }

    fn into_arrow_array(&self) -> ArrayRef {
        Arc::new(self.values.clone())
    }

    fn into_serie(self) -> Serie {
        <Self as FixedLeaf>::into_serie(self)
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        <Self as FixedLeaf>::from_serie(value)
    }
}

/// Which leaf of the root one fixed-width column widens to.
pub trait FixedLeaf: Sized {
    /// Widen this column to the serie root.
    fn into_serie(column: Self) -> Serie;

    /// Narrow a serie root to this column.
    fn from_serie(serie: &Serie) -> Option<&Self>;
}

impl FixedLeaf for FixedSerie<TextRun> {
    fn into_serie(column: Self) -> Serie {
        Serie::String(Arc::new(super::StringSerie::Fixed(column)))
    }

    fn from_serie(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::String(family) => match family.as_ref() {
                super::StringSerie::Fixed(column) => Some(column),
                _ => None,
            },
            _ => None,
        }
    }
}

impl FixedLeaf for FixedSerie<RawRun> {
    fn into_serie(column: Self) -> Serie {
        Serie::Bytes(Arc::new(super::BytesSerie::Fixed(column)))
    }

    fn from_serie(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Bytes(family) => match family.as_ref() {
                super::BytesSerie::Fixed(column) => Some(column),
                _ => None,
            },
            _ => None,
        }
    }
}

impl<K: ByteKind> Clone for FixedSerie<K> {
    fn clone(&self) -> Self {
        Self {
            field: Arc::clone(&self.field),
            values: self.values.clone(),
            kind: PhantomData,
        }
    }
}

impl<K: ByteKind> fmt::Debug for FixedSerie<K>
where
    Self: FixedLeaf,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_leaf(self, "FixedSerie", formatter)
    }
}

/// A column of fixed-width text, a padded code or an enum member.
pub type FixedStringSerie = FixedSerie<TextRun>;

/// A column of fixed-width bytes: a UUID, or any identity stored as one.
pub type FixedBytesSerie = FixedSerie<RawRun>;

// ------------------------------------------------------------------------
// The identity every byte column has. Written out rather than macroed,
// because each of these carries two type parameters and a bound the one-
// parameter macro cannot spell.
// ------------------------------------------------------------------------

/// Emit the identity one generic byte column has.
macro_rules! byte_identity {
    ($name:ident, $bound:ident, $param:ident) => {
        impl<T, K> PartialEq for $name<T, K>
        where
            T: $bound<K>,
            K: ByteKind,
            Self: SerieValue,
        {
            fn eq(&self, other: &Self) -> bool {
                super::compare_leaves(self, other) == ::std::cmp::Ordering::Equal
            }
        }

        impl<T, K> Eq for $name<T, K>
        where
            T: $bound<K>,
            K: ByteKind,
            Self: SerieValue,
        {
        }

        impl<T, K> PartialOrd for $name<T, K>
        where
            T: $bound<K>,
            K: ByteKind,
            Self: SerieValue,
        {
            fn partial_cmp(&self, other: &Self) -> Option<::std::cmp::Ordering> {
                Some(::std::cmp::Ord::cmp(self, other))
            }
        }

        impl<T, K> Ord for $name<T, K>
        where
            T: $bound<K>,
            K: ByteKind,
            Self: SerieValue,
        {
            fn cmp(&self, other: &Self) -> ::std::cmp::Ordering {
                super::compare_leaves(self, other)
            }
        }

        impl<T, K> ::std::hash::Hash for $name<T, K>
        where
            T: $bound<K>,
            K: ByteKind,
            Self: SerieValue,
        {
            fn hash<H: ::std::hash::Hasher>(&self, state: &mut H) {
                super::hash_leaf(self, state);
            }
        }

        impl<T, K> fmt::Display for $name<T, K>
        where
            T: $bound<K>,
            K: ByteKind,
            Self: SerieValue,
        {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                super::display_leaf(self, formatter)
            }
        }

        const _: () = {
            let _ = ::std::marker::PhantomData::<$param>;
        };
    };
}

byte_identity!(ByteSerie, ByteLeaf, u8);
byte_identity!(ByteViewSerie, ViewLeaf, u8);

impl<K: ByteKind> PartialEq for FixedSerie<K>
where
    Self: FixedLeaf,
{
    fn eq(&self, other: &Self) -> bool {
        super::compare_leaves(self, other) == std::cmp::Ordering::Equal
    }
}

impl<K: ByteKind> Eq for FixedSerie<K> where Self: FixedLeaf {}

impl<K: ByteKind> PartialOrd for FixedSerie<K>
where
    Self: FixedLeaf,
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(std::cmp::Ord::cmp(self, other))
    }
}

impl<K: ByteKind> Ord for FixedSerie<K>
where
    Self: FixedLeaf,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        super::compare_leaves(self, other)
    }
}

impl<K: ByteKind> std::hash::Hash for FixedSerie<K>
where
    Self: FixedLeaf,
{
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        super::hash_leaf(self, state);
    }
}

impl<K: ByteKind> fmt::Display for FixedSerie<K>
where
    Self: FixedLeaf,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::display_leaf(self, formatter)
    }
}
