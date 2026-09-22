//! Every column whose rows are a run of bytes: text, raw bytes, and the
//! identities stored as either.
//!
//! Arrow lays a variable-length run out as offsets into one payload buffer
//! ([`ByteSerie`]), as views into several ([`ByteViewSerie`]), or as one
//! fixed width with no offsets at all ([`FixedSerie`]). Each is one generic
//! type here and each leaf is a name for one instantiation, so
//! [`Utf8StringSerie::offsets`](crate::Utf8StringSerie::offsets) lends the
//! offsets and [`Utf8StringSerie::payload`](crate::Utf8StringSerie::payload)
//! the characters, both where they lie.
//!
//! A leaf says how the bytes are laid out; the field says what they *are* -
//! which of the crate's eighteen string leaves, which of its codes, a UUID,
//! a geospatial reading. That split is why one layout can be a string leaf
//! and a byte leaf at once, and why the two are told apart by a marker
//! rather than by the buffers: [`Chars`] and [`Octets`] are what
//! [`BinaryStringSerie`](crate::BinaryStringSerie) and [`BinarySerie`]
//! differ by. No byte leaf has a typed writer: codes, charsets, sizes and
//! well-known binary are all narrower than the storage, so a [`Scalar`]
//! through the field's contract is the one writer.
//!
//! A write lays its rows out once at the crate's one scalar-array boundary
//! and splices the array in: an offsets run appends into the builder Arrow
//! hands back when nothing else holds its buffers and is rebuilt once from
//! prefix, replacement and suffix otherwise; a fixed width writes its
//! payload in place under the same rule; views are rewritten, because no
//! builder hands views back.

use std::fmt::{self, Write as _};
use std::marker::PhantomData;
use std::ops::Range;
use std::sync::Arc;

use arrow_array::builder::{GenericByteBuilder, GenericByteViewBuilder};
use arrow_array::types::{
    BinaryType, BinaryViewType, ByteArrayType, ByteViewType, LargeBinaryType, LargeUtf8Type,
    StringViewType, Utf8Type,
};
use arrow_array::{Array, ArrayRef, FixedSizeBinaryArray, GenericByteArray, GenericByteViewArray};
use arrow_buffer::{ArrowNativeType, Buffer, MutableBuffer, NullBuffer, OffsetBuffer};
use arrow_schema::DataType as ArrowDataType;

use super::{Serie, layout, require_range, require_row, require_window};
use crate::value::SerieValue;
use crate::{DataType, DataTypeKind, Field, Result, Scalar};

/// The invariant a canonical row carries into a write: it lays out as the
/// field's own array, because the field's contract already rewrote it and
/// `check` already measured it.
const LAID_OUT: &str = "a canonical row lays out as its field's array: the contract rewrote it and `check` measured it";

/// The invariant a checked write carries into its builder: the offsets
/// total fits the offset type, because `check` ran `require_offset` on it.
const CHECKED: &str = "the offsets total fits the offset type: `check` ran `require_offset` on it";

/// The invariant a fixed-width write keeps: the payload holds `width` bytes
/// per row and the validity one bit per row.
const ALIGNED: &str =
    "a fixed-width payload holds width bytes per row: no public path misaligns it";

/// Which family a run of bytes belongs to when its layout belongs to both.
///
/// A windows-1252 string column and a byte column are the same Arrow binary
/// buffers; what tells them apart is the field, and this is that answer made
/// a type so the two are different columns rather than one wearing two names.
pub trait ByteKind: Send + Sync + Clone + Copy + fmt::Debug + 'static {}

/// The marker for a run of bytes a field reads as text.
#[derive(Clone, Copy, Debug)]
pub struct Chars;

/// The marker for a run of bytes a field reads as bytes.
#[derive(Clone, Copy, Debug)]
pub struct Octets;

impl ByteKind for Chars {}
impl ByteKind for Octets {}

/// Which leaf of the root one byte layout under one marker widens to.
pub trait ByteLeaf<K: ByteKind>: ByteArrayType + Sized + Send + Sync + 'static {
    /// The leaf's name, as its debug rendering spells it.
    const NAME: &'static str;

    /// Widen a column of this layout to the serie root.
    fn into_serie(column: ByteSerie<Self, K>) -> Serie;

    /// Narrow a serie root to a column of this layout.
    fn from_serie(serie: &Serie) -> Option<&ByteSerie<Self, K>>;
}

/// Which leaf of the root one view layout under one marker widens to.
pub trait ViewLeaf<K: ByteKind>: ByteViewType + Sized + Send + Sync + 'static {
    /// The leaf's name, as its debug rendering spells it.
    const NAME: &'static str;

    /// Widen a column of this layout to the serie root.
    fn into_serie(column: ByteViewSerie<Self, K>) -> Serie;

    /// Narrow a serie root to a column of this layout.
    fn from_serie(serie: &Serie) -> Option<&ByteViewSerie<Self, K>>;
}

/// Which leaf of the root one fixed-width column widens to.
pub trait FixedLeaf: Sized {
    /// The leaf's name, as its debug rendering spells it.
    const NAME: &'static str;

    /// Widen this column to the serie root.
    fn into_serie(column: Self) -> Serie;

    /// Narrow a serie root to this column.
    fn from_serie(serie: &Serie) -> Option<&Self>;
}

/// Whether a field reads its bytes as text.
///
/// A string leaf and a registered code both do; everything else stored in
/// bytes - a UUID, a geospatial reading, a plain byte column - does not.
pub(crate) fn is_text(dtype: &DataType) -> bool {
    matches!(dtype, DataType::String(_)) || dtype.kind() == DataTypeKind::Code
}

/// Lay canonical `rows` out as the array `field` projects to, once.
///
/// The crate's one scalar-array boundary, so a byte leaf never grows a
/// second writer: the rows went through the field's contract, so the only
/// refusal left is the layout's own bound on one write, which `check`
/// surfaces by name.
fn lay_out<A: Array + Clone + 'static>(field: &Field, rows: &[Scalar]) -> Result<A> {
    let borrowed: Vec<&Scalar> = rows.iter().collect();
    let array = crate::arrow::value::array_from_values(field, &borrowed)?;
    Ok(array.as_any().downcast_ref::<A>().cloned().expect(LAID_OUT))
}

// ------------------------------------------------------------------------
// One offsets run: what the byte leaf and the variant leaf's two runs
// share, so the run is written in one place.
// ------------------------------------------------------------------------

/// The bytes `run` reaches between its first and last offset.
///
/// A sliced run's offsets start past zero and its payload buffer runs past
/// its end, so neither the buffer's length nor the last offset alone says
/// how many bytes the rows hold.
pub(crate) fn run_bytes<T: ByteArrayType>(run: &GenericByteArray<T>) -> usize {
    let offsets = run.offsets();
    (offsets[run.len()] - offsets[0]).as_usize()
}

/// Refuse a write of `replacement` bytes over rows `range` of `run` whose
/// offsets total would pass the offset type, naming the column.
///
/// # Errors
///
/// [`layout::require_offset`] carries the rule.
pub(crate) fn require_run_fits<T: ByteArrayType>(
    name: &str,
    run: &GenericByteArray<T>,
    range: &Range<usize>,
    replacement: usize,
) -> Result<()> {
    let offsets = run.offsets();
    let replaced = (offsets[range.end] - offsets[range.start]).as_usize();
    let total = (run_bytes(run) - replaced).saturating_add(replacement);
    layout::require_offset::<T::Offset>(name, total)
}

/// The bytes one canonical row stores in an offsets layout.
///
/// A row that reaches a write went through the field's contract, so its own
/// value says what the layout writes for it: a string the bytes of its
/// charset's encoding (padded to its width where it has one), bytes and a
/// geospatial reading their payload, a code and every rendered text
/// datatype the characters of its one spelling. An absent row stores
/// nothing.
pub(crate) fn stored_bytes(row: &Scalar) -> usize {
    match row {
        Scalar::Null => 0,
        Scalar::String(text) => text.encoded_len(),
        Scalar::Bytes(bytes) => bytes.as_bytes().len(),
        Scalar::Geometry(value) => value.as_bytes().len(),
        Scalar::Geography(value) => value.as_bytes().len(),
        Scalar::Timezone(zone) => zone.as_str().len(),
        Scalar::MimeType(mime) => mime.as_str().len(),
        code if code.is_code() => code.as_str().map_or(0, str::len),
        Scalar::Version(version) => rendered_bytes(version),
        Scalar::Url(url) => rendered_bytes(url),
        Scalar::Urn(urn) => rendered_bytes(urn),
        Scalar::MediaType(media) => rendered_bytes(media),
        // No other value canonicalizes under a byte layout.
        _ => 0,
    }
}

/// The bytes one rendered spelling takes, counted and never built.
fn rendered_bytes(value: &impl fmt::Display) -> usize {
    let mut counted = Counted(0);
    // A counting sink never refuses a byte.
    let _ = write!(counted, "{value}");
    counted.0
}

/// A formatting sink that counts the bytes written and keeps none.
struct Counted(usize);

impl fmt::Write for Counted {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.0 += text.len();
        Ok(())
    }
}

/// Take `run` as a builder to write into, with room for `rows` more rows
/// of `bytes` more bytes.
///
/// Arrow hands the buffers back as a builder when nothing else holds them
/// and their pointer was never advanced, which is what makes an append in
/// place; a foreign, shared or sliced run is copied once, and every later
/// edit is in place. A byte layout carries no parameter, so `finish`
/// restores exactly the datatype `into_builder` took.
///
/// `into_builder` is asked only of a run whose first offset is zero: it
/// cuts the payload from zero whichever offset the rows start at, and hands
/// a sliced run back inconsistent with its own offsets on the way out
/// (arrow-array 59.2, `GenericByteArray::into_builder`), so a run that
/// starts past zero is copied without being asked.
fn run_builder<T: ByteArrayType>(
    run: GenericByteArray<T>,
    rows: usize,
    bytes: usize,
) -> GenericByteBuilder<T> {
    let shared = if run.offsets()[0] == T::Offset::usize_as(0) {
        match run.into_builder() {
            Ok(builder) => return builder,
            Err(shared) => shared,
        }
    } else {
        run
    };
    let mut builder =
        GenericByteBuilder::<T>::with_capacity(shared.len() + rows, run_bytes(&shared) + bytes);
    // The same offsets the run already holds cannot overflow.
    builder.append_array(&shared).expect(CHECKED);
    builder
}

/// `run` with rows `range` replaced by `replacement`, whose offsets total
/// was checked.
///
/// An append writes into the builder the run hands back; anything else is
/// one rebuild through a builder - prefix, replacement, suffix, one pass -
/// because a run is as long as it is and every later offset moves.
pub(crate) fn splice_run<T: ByteArrayType>(
    run: GenericByteArray<T>,
    range: Range<usize>,
    replacement: &GenericByteArray<T>,
) -> GenericByteArray<T> {
    let len = run.len();
    if range.start == len {
        let mut builder = run_builder(run, replacement.len(), run_bytes(replacement));
        builder.append_array(replacement).expect(CHECKED);
        return builder.finish();
    }
    let prefix = run.slice(0, range.start);
    let suffix = run.slice(range.end, len - range.end);
    let mut builder = GenericByteBuilder::<T>::with_capacity(
        prefix.len() + replacement.len() + suffix.len(),
        run_bytes(&prefix) + run_bytes(replacement) + run_bytes(&suffix),
    );
    for piece in [&prefix, replacement, &suffix] {
        builder.append_array(piece).expect(CHECKED);
    }
    builder.finish()
}

// ------------------------------------------------------------------------
// Offsets into one payload buffer.
// ------------------------------------------------------------------------

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

    /// Borrow row `index` where it lies in the payload: `None` when the row
    /// is absent or past the end.
    pub fn value(&self, index: usize) -> Option<&T::Native> {
        (index < self.values.len() && self.values.is_valid(index)).then(|| self.values.value(index))
    }

    /// Refuse what a write could not do: an offsets total past the offset
    /// type.
    ///
    /// The rows are measured, never laid out: a canonical row already says
    /// what its layout stores ([`stored_bytes`]), so a replacement past the
    /// offset type is refused by name before a byte of it is built.
    pub(crate) fn check(&self, range: &Range<usize>, rows: &[Scalar]) -> Result<()> {
        let replacement = rows.iter().fold(0_usize, |total, row| {
            total.saturating_add(stored_bytes(row))
        });
        require_run_fits(self.field.name(), &self.values, range, replacement)
    }

    /// Write canonical `rows` over a checked `range`.
    pub(crate) fn write(&mut self, range: Range<usize>, rows: Vec<Scalar>) {
        let replacement: GenericByteArray<T> = lay_out(&self.field, &rows).expect(LAID_OUT);
        self.write_array(range, &replacement);
    }

    /// Append `other`'s buffers, whose field agrees with this one's,
    /// answering whether the offsets reach the total; `false` leaves this
    /// column as it was, and the root then reads the rows and refuses by
    /// name.
    pub(crate) fn append(&mut self, other: &Self) -> bool {
        let len = self.values.len();
        let fits = require_run_fits(
            self.field.name(),
            &self.values,
            &(len..len),
            run_bytes(&other.values),
        )
        .is_ok();
        if fits {
            self.write_array(len..len, &other.values);
        }
        fits
    }

    /// Replace rows `range` by `replacement`, which lays out as this column.
    fn write_array(&mut self, range: Range<usize>, replacement: &GenericByteArray<T>) {
        let taken = std::mem::replace(&mut self.values, GenericByteArray::<T>::new_null(0));
        self.values = splice_run(taken, range, replacement);
    }
}

impl<T: ByteLeaf<K>, K: ByteKind> SerieValue for ByteSerie<T, K> {
    fn field(&self) -> &Field {
        &self.field
    }

    fn field_ref(&self) -> &Arc<Field> {
        &self.field
    }

    fn len(&self) -> usize {
        self.values.len()
    }

    fn null_count(&self) -> usize {
        self.values.null_count()
    }

    fn is_null(&self, index: usize) -> Result<bool> {
        require_row(self.field.name(), index, self.values.len())?;
        Ok(self.values.is_null(index))
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        require_row(self.field.name(), index, self.values.len())?;
        Ok(crate::arrow::value::value_from_array(
            self.field.dtype(),
            &self.values,
            index,
        )?)
    }

    fn slice(&self, offset: usize, length: usize) -> Result<Self> {
        require_window(self.field.name(), offset, length, self.values.len())?;
        Ok(Self::new(
            Arc::clone(&self.field),
            self.values.slice(offset, length),
        ))
    }

    fn splice(&mut self, range: Range<usize>, rows: Vec<Scalar>) -> Result<()> {
        require_range(self.field.name(), &range, self.values.len())?;
        let canonical = rows
            .into_iter()
            .map(|row| self.field.scalar(row))
            .collect::<Result<Vec<Scalar>>>()?;
        self.check(&range, &canonical)?;
        self.write(range, canonical);
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
        Self::new(Arc::clone(&self.field), self.values.clone())
    }
}

impl<T: ByteLeaf<K>, K: ByteKind> fmt::Debug for ByteSerie<T, K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_column(self, T::NAME, formatter)
    }
}

serie_leaf!(ByteSerie<T: ByteLeaf<K>, K: ByteKind>);

/// Name one byte layout under one marker as a leaf of the root.
macro_rules! byte_leaf {
    ($(#[$meta:meta])* $name:ident, $arrow:ty, $marker:ty, $family:ident, $held:ident, $variant:ident) => {
        $(#[$meta])*
        pub type $name = $crate::serie::bytes::ByteSerie<$arrow, $marker>;

        impl $crate::serie::bytes::ByteLeaf<$marker> for $arrow {
            const NAME: &'static str = stringify!($name);

            fn into_serie(column: $crate::serie::bytes::ByteSerie<Self, $marker>) -> $crate::Serie {
                $crate::Serie::$family(::std::sync::Arc::new($crate::serie::$held::$variant(column)))
            }

            fn from_serie(
                serie: &$crate::Serie,
            ) -> Option<&$crate::serie::bytes::ByteSerie<Self, $marker>> {
                match serie {
                    $crate::Serie::$family(family) => match family.as_ref() {
                        $crate::serie::$held::$variant(column) => Some(column),
                        _ => None,
                    },
                    _ => None,
                }
            }
        }
    };
}

pub(crate) use byte_leaf;

byte_leaf!(
    /// A column of byte runs, 32-bit offsets.
    BinarySerie, BinaryType, Octets, Bytes, BytesSerie, Binary
);
byte_leaf!(
    /// A column of byte runs, 64-bit offsets.
    LargeBinarySerie, LargeBinaryType, Octets, Bytes, BytesSerie, LargeBinary
);

// ------------------------------------------------------------------------
// Views: runs that name where they lie rather than lying in one buffer.
// ------------------------------------------------------------------------

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

    /// Borrow row `index` where its view names it: `None` when the row is
    /// absent or past the end.
    pub fn value(&self, index: usize) -> Option<&T::Native> {
        (index < self.values.len() && self.values.is_valid(index)).then(|| self.values.value(index))
    }

    /// Refuse what a write could not do: nothing, for views.
    pub(crate) fn check(&self, range: &Range<usize>, rows: &[Scalar]) -> Result<()> {
        let _ = (range, rows);
        Ok(())
    }

    /// Write canonical `rows` over a checked `range`: the views rewritten.
    pub(crate) fn write(&mut self, range: Range<usize>, rows: Vec<Scalar>) {
        let replacement: GenericByteViewArray<T> = lay_out(&self.field, &rows).expect(LAID_OUT);
        self.rewrite(range, &replacement);
    }

    /// Append `other`'s buffers, whose field agrees with this one's; views
    /// name their blocks, so no total is out of reach.
    pub(crate) fn append(&mut self, other: &Self) -> bool {
        let len = self.values.len();
        self.rewrite(len..len, &other.values);
        true
    }

    /// Replace rows `range` by `replacement`: every run packed again into
    /// fresh blocks.
    ///
    /// No builder hands views back, and a view names the buffer its run
    /// lies in, so an edit is one pass over every row. Packing the runs
    /// again rather than carrying the old buffers along is what keeps a
    /// column edited many times from holding every byte it ever held.
    fn rewrite(&mut self, range: Range<usize>, replacement: &GenericByteViewArray<T>) {
        let len = self.values.len();
        let mut builder =
            GenericByteViewBuilder::<T>::with_capacity(len - range.len() + replacement.len());
        let mut pack = |array: &GenericByteViewArray<T>, rows: Range<usize>| {
            for row in rows {
                builder.append_option(array.is_valid(row).then(|| array.value(row)));
            }
        };
        pack(&self.values, 0..range.start);
        pack(replacement, 0..replacement.len());
        pack(&self.values, range.end..len);
        self.values = builder.finish();
    }
}

impl<T: ViewLeaf<K>, K: ByteKind> SerieValue for ByteViewSerie<T, K> {
    fn field(&self) -> &Field {
        &self.field
    }

    fn field_ref(&self) -> &Arc<Field> {
        &self.field
    }

    fn len(&self) -> usize {
        self.values.len()
    }

    fn null_count(&self) -> usize {
        self.values.null_count()
    }

    fn is_null(&self, index: usize) -> Result<bool> {
        require_row(self.field.name(), index, self.values.len())?;
        Ok(self.values.is_null(index))
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        require_row(self.field.name(), index, self.values.len())?;
        Ok(crate::arrow::value::value_from_array(
            self.field.dtype(),
            &self.values,
            index,
        )?)
    }

    fn slice(&self, offset: usize, length: usize) -> Result<Self> {
        require_window(self.field.name(), offset, length, self.values.len())?;
        Ok(Self::new(
            Arc::clone(&self.field),
            self.values.slice(offset, length),
        ))
    }

    fn splice(&mut self, range: Range<usize>, rows: Vec<Scalar>) -> Result<()> {
        require_range(self.field.name(), &range, self.values.len())?;
        let canonical = rows
            .into_iter()
            .map(|row| self.field.scalar(row))
            .collect::<Result<Vec<Scalar>>>()?;
        self.check(&range, &canonical)?;
        self.write(range, canonical);
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
        Self::new(Arc::clone(&self.field), self.values.clone())
    }
}

impl<T: ViewLeaf<K>, K: ByteKind> fmt::Debug for ByteViewSerie<T, K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_column(self, T::NAME, formatter)
    }
}

serie_leaf!(ByteViewSerie<T: ViewLeaf<K>, K: ByteKind>);

/// Name one view layout under one marker as a leaf of the root.
macro_rules! view_leaf {
    ($(#[$meta:meta])* $name:ident, $arrow:ty, $marker:ty, $family:ident, $held:ident, $variant:ident) => {
        $(#[$meta])*
        pub type $name = $crate::serie::bytes::ByteViewSerie<$arrow, $marker>;

        impl $crate::serie::bytes::ViewLeaf<$marker> for $arrow {
            const NAME: &'static str = stringify!($name);

            fn into_serie(
                column: $crate::serie::bytes::ByteViewSerie<Self, $marker>,
            ) -> $crate::Serie {
                $crate::Serie::$family(::std::sync::Arc::new($crate::serie::$held::$variant(column)))
            }

            fn from_serie(
                serie: &$crate::Serie,
            ) -> Option<&$crate::serie::bytes::ByteViewSerie<Self, $marker>> {
                match serie {
                    $crate::Serie::$family(family) => match family.as_ref() {
                        $crate::serie::$held::$variant(column) => Some(column),
                        _ => None,
                    },
                    _ => None,
                }
            }
        }
    };
}

pub(crate) use view_leaf;

view_leaf!(
    /// A column of byte runs held as views.
    BinaryViewSerie, BinaryViewType, Octets, Bytes, BytesSerie, BinaryView
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

    /// Borrow row `index` where it lies in the payload: `None` when the row
    /// is absent or past the end.
    pub fn value(&self, index: usize) -> Option<&[u8]> {
        (index < self.values.len() && self.values.is_valid(index)).then(|| self.values.value(index))
    }

    /// Refuse what a write could not do: a fixed-width total past `i32`.
    pub(crate) fn check(&self, range: &Range<usize>, rows: &[Scalar]) -> Result<()> {
        let total =
            (self.values.len() - range.len() + rows.len()).saturating_mul(self.values.value_size());
        layout::require_offset::<i32>(self.field.name(), total)
    }

    /// Write canonical `rows` over a checked `range`.
    pub(crate) fn write(&mut self, range: Range<usize>, rows: Vec<Scalar>) {
        let replacement: FixedSizeBinaryArray = lay_out(&self.field, &rows).expect(LAID_OUT);
        self.write_array(range, &replacement);
    }

    /// Append `other`'s buffers, whose field agrees with this one's,
    /// answering whether the payload total stays within `i32`; `false`
    /// leaves this column as it was.
    pub(crate) fn append(&mut self, other: &Self) -> bool {
        let len = self.values.len();
        let total = (len + other.values.len()).saturating_mul(self.values.value_size());
        let fits = layout::require_offset::<i32>(self.field.name(), total).is_ok();
        if fits {
            self.write_array(len..len, &other.values);
        }
        fits
    }

    /// Replace rows `range` by `replacement`, which is this column's width.
    ///
    /// Every row occupies `width` bytes at `index * width`, an absent one
    /// included, so an append and a same-count overwrite - `set` among them,
    /// one `memcpy` - write the payload the column holds when nothing else
    /// holds it and its pointer was never advanced; a foreign, shared or
    /// sliced payload, or a count that changes, is copied once from prefix,
    /// replacement and suffix, and every later edit is in place.
    fn write_array(&mut self, range: Range<usize>, replacement: &FixedSizeBinaryArray) {
        let len = self.values.len();
        let width = self.values.value_size();
        let present: Vec<bool> = (0..replacement.len())
            .map(|row| replacement.is_valid(row))
            .collect();
        let taken = std::mem::replace(&mut self.values, FixedSizeBinaryArray::new_null(0, 0));
        let (width_declared, payload, nulls) = taken.into_parts();
        let in_place = range.start == len || range.len() == replacement.len();
        let payload = match payload.into_mutable() {
            Ok(mut owned) if in_place => {
                if range.start == len {
                    owned.extend_from_slice(replacement.value_data());
                } else {
                    owned.as_slice_mut()[range.start * width..range.end * width]
                        .copy_from_slice(replacement.value_data());
                }
                Buffer::from(owned)
            }
            Ok(owned) => rebuilt_payload(&Buffer::from(owned), width, range.clone(), replacement),
            Err(shared) => rebuilt_payload(&shared, width, range.clone(), replacement),
        };
        let nulls = layout::splice_nulls(nulls, len, range.clone(), &present);
        self.values = FixedSizeBinaryArray::try_new_with_len(
            width_declared,
            payload,
            nulls,
            len - range.len() + replacement.len(),
        )
        .expect(ALIGNED);
    }
}

/// The fixed-width payload `held` is with rows `range` replaced by
/// `replacement`'s bytes: one copy from prefix, replacement and suffix.
fn rebuilt_payload(
    held: &Buffer,
    width: usize,
    range: Range<usize>,
    replacement: &FixedSizeBinaryArray,
) -> Buffer {
    let bytes = held.as_slice();
    let mut rebuilt = MutableBuffer::with_capacity(
        bytes.len() - range.len() * width + replacement.value_data().len(),
    );
    rebuilt.extend_from_slice(&bytes[..range.start * width]);
    rebuilt.extend_from_slice(replacement.value_data());
    rebuilt.extend_from_slice(&bytes[range.end * width..]);
    Buffer::from(rebuilt)
}

impl<K: ByteKind> SerieValue for FixedSerie<K>
where
    Self: FixedLeaf,
{
    fn field(&self) -> &Field {
        &self.field
    }

    fn field_ref(&self) -> &Arc<Field> {
        &self.field
    }

    fn len(&self) -> usize {
        self.values.len()
    }

    fn null_count(&self) -> usize {
        self.values.null_count()
    }

    fn is_null(&self, index: usize) -> Result<bool> {
        require_row(self.field.name(), index, self.values.len())?;
        Ok(self.values.is_null(index))
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        require_row(self.field.name(), index, self.values.len())?;
        Ok(crate::arrow::value::value_from_array(
            self.field.dtype(),
            &self.values,
            index,
        )?)
    }

    fn slice(&self, offset: usize, length: usize) -> Result<Self> {
        require_window(self.field.name(), offset, length, self.values.len())?;
        Ok(Self::new(
            Arc::clone(&self.field),
            self.values.slice(offset, length),
        ))
    }

    fn splice(&mut self, range: Range<usize>, rows: Vec<Scalar>) -> Result<()> {
        require_range(self.field.name(), &range, self.values.len())?;
        let canonical = rows
            .into_iter()
            .map(|row| self.field.scalar(row))
            .collect::<Result<Vec<Scalar>>>()?;
        self.check(&range, &canonical)?;
        self.write(range, canonical);
        Ok(())
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

impl<K: ByteKind> Clone for FixedSerie<K> {
    fn clone(&self) -> Self {
        Self::new(Arc::clone(&self.field), self.values.clone())
    }
}

impl<K: ByteKind> fmt::Debug for FixedSerie<K>
where
    Self: FixedLeaf,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_column(self, <Self as FixedLeaf>::NAME, formatter)
    }
}

serie_leaf!(FixedSerie<K: ByteKind>);

/// Name the fixed-width layout under one marker as a leaf of the root.
macro_rules! fixed_leaf {
    ($(#[$meta:meta])* $name:ident, $marker:ty, $family:ident, $held:ident) => {
        $(#[$meta])*
        pub type $name = $crate::serie::bytes::FixedSerie<$marker>;

        impl $crate::serie::bytes::FixedLeaf for $crate::serie::bytes::FixedSerie<$marker> {
            const NAME: &'static str = stringify!($name);

            fn into_serie(column: Self) -> $crate::Serie {
                $crate::Serie::$family(::std::sync::Arc::new($crate::serie::$held::Fixed(column)))
            }

            fn from_serie(serie: &$crate::Serie) -> Option<&Self> {
                match serie {
                    $crate::Serie::$family(family) => match family.as_ref() {
                        $crate::serie::$held::Fixed(column) => Some(column),
                        _ => None,
                    },
                    _ => None,
                }
            }
        }
    };
}

pub(crate) use fixed_leaf;

fixed_leaf!(
    /// A column of fixed-width bytes: a UUID, or any identity stored as one.
    FixedBytesSerie, Octets, Bytes, BytesSerie
);

/// Build the column `field` types out of a byte-run array, or answer `None`
/// for a layout that is not one.
///
/// The field's family - text or bytes - picks the marker; the Arrow
/// datatype picks the layout. The door has already proven the projection,
/// the absence and the values, so nothing is read here.
pub(crate) fn column_of(
    field: Arc<Field>,
    array: ArrayRef,
    parent: Option<&NullBuffer>,
    proven: bool,
) -> crate::arrow::Result<Option<Serie>> {
    use super::arrow::held;
    use super::string::{
        BinaryStringSerie, BinaryViewStringSerie, FixedStringSerie, LargeBinaryStringSerie,
        LargeUtf8StringSerie, Utf8StringSerie, Utf8ViewStringSerie,
    };

    let _ = (parent, proven);
    let text = is_text(field.dtype());
    Ok(Some(match array.data_type() {
        ArrowDataType::Utf8 => {
            Utf8StringSerie::new(field, held::<GenericByteArray<Utf8Type>>(&array)?).into_serie()
        }
        ArrowDataType::LargeUtf8 => {
            LargeUtf8StringSerie::new(field, held::<GenericByteArray<LargeUtf8Type>>(&array)?)
                .into_serie()
        }
        ArrowDataType::Utf8View => {
            Utf8ViewStringSerie::new(field, held::<GenericByteViewArray<StringViewType>>(&array)?)
                .into_serie()
        }
        ArrowDataType::Binary => {
            let runs = held::<GenericByteArray<BinaryType>>(&array)?;
            if text {
                BinaryStringSerie::new(field, runs).into_serie()
            } else {
                BinarySerie::new(field, runs).into_serie()
            }
        }
        ArrowDataType::LargeBinary => {
            let runs = held::<GenericByteArray<LargeBinaryType>>(&array)?;
            if text {
                LargeBinaryStringSerie::new(field, runs).into_serie()
            } else {
                LargeBinarySerie::new(field, runs).into_serie()
            }
        }
        ArrowDataType::BinaryView => {
            let runs = held::<GenericByteViewArray<BinaryViewType>>(&array)?;
            if text {
                BinaryViewStringSerie::new(field, runs).into_serie()
            } else {
                BinaryViewSerie::new(field, runs).into_serie()
            }
        }
        ArrowDataType::FixedSizeBinary(_) => {
            let runs = held::<FixedSizeBinaryArray>(&array)?;
            if text {
                FixedStringSerie::new(field, runs).into_serie()
            } else {
                FixedBytesSerie::new(field, runs).into_serie()
            }
        }
        _ => return Ok(None),
    }))
}
