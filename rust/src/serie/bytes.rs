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
//! A leaf holds Arrow's buffers - the offsets or the views, the payload, the
//! validity - and never an Arrow array: a row is read off them by the leaf
//! itself, one bounds check and one slice, and `array()` hands the same
//! buffers back as the array they are, pointer bumps and no scan. Beside
//! them a leaf keeps where they live (`Backing`): the heap, or a spill
//! file's read-only mapping, which a write leaves for the heap.
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
//! builder hands views back. A shared, sliced or mapped buffer is copied
//! once, because Arrow hands back only a buffer it holds alone and a
//! mapping is never written.
//!
//! # Unsafe
//!
//! Three uses, each named where it stands, and each trusting bytes no caller
//! hands in: the buffers are private fields, they enter only through a
//! leaf's constructor - from an array the landing door proved against the
//! field, or one the leaf's own writer laid out through `array_of_rows` or
//! an Arrow builder - and a slice only narrows them.
//!
//! * `str::from_utf8_unchecked`, behind `RunNative::from_run`, in every
//!   `value`: a run read as its native - `str` or `[u8]` - with no scan, the
//!   read Arrow's own `value` makes over the same buffers. A checked read
//!   would re-scan every run it lends;
//! * `GenericByteArray::new_unchecked`, behind `runs_unchecked`, the one
//!   door an offsets run - this module's and the variant leaf's - becomes
//!   an array again through, in `array()` and in every write. A checked
//!   rebuild of a UTF-8 run scans every byte of it;
//! * `GenericByteViewArray::new_unchecked`, in the view leaf's `array()`,
//!   for the same reason over every view.
//!
//! A fixed-width leaf needs none: its rebuild checks one length.

#![allow(unsafe_code)]

use std::fmt::{self, Write as _};
use std::marker::PhantomData;
use std::ops::Range;
use std::sync::Arc;

use arrow_array::builder::{GenericByteBuilder, GenericByteViewBuilder};
use arrow_array::types::{
    BinaryType, BinaryViewType, ByteArrayType, ByteViewType, LargeBinaryType, LargeUtf8Type,
    StringViewType, Utf8Type,
};
use arrow_array::{
    Array, ArrayRef, FixedSizeBinaryArray, GenericByteArray, GenericByteViewArray, OffsetSizeTrait,
};
use arrow_buffer::{
    ArrowNativeType, Buffer, MutableBuffer, NullBuffer, OffsetBuffer, ScalarBuffer,
};
use arrow_data::{ByteView, MAX_INLINE_VIEW_LEN};
use arrow_schema::DataType as ArrowDataType;

use super::{Serie, layout, require_range, require_row, require_window};
use crate::serie::value::RunReading;
use crate::spill::Backing;
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

/// What a run reads as: the native an Arrow byte layout lends - `str` for
/// text, `[u8]` for bytes - built from a run already proven to be one.
///
/// Arrow's own conversion is sealed inside arrow-array, so the two natives
/// its byte layouts have are named here, once.
pub trait RunNative {
    /// Read `run` as this native, with no scan.
    ///
    /// # Safety
    ///
    /// `run` must be one whole value of a proven layout: valid UTF-8 where
    /// the native is `str`.
    unsafe fn from_run(run: &[u8]) -> &Self;
}

impl RunNative for str {
    unsafe fn from_run(run: &[u8]) -> &Self {
        // SAFETY: the caller's contract: `run` is one whole string of a
        // proven text layout, so valid UTF-8.
        unsafe { std::str::from_utf8_unchecked(run) }
    }
}

impl RunNative for [u8] {
    unsafe fn from_run(run: &[u8]) -> &Self {
        run
    }
}

/// Which leaf of the root one byte layout under one marker widens to.
pub trait ByteLeaf<K: ByteKind>:
    ByteArrayType<Native: RunNative> + Sized + Send + Sync + 'static
{
    /// The leaf's name, as its debug rendering spells it.
    const NAME: &'static str;

    /// Widen a column of this layout to the serie root.
    fn into_serie(column: ByteSerie<Self, K>) -> Serie;

    /// Narrow a serie root to a column of this layout.
    fn from_serie(serie: &Serie) -> Option<&ByteSerie<Self, K>>;
}

/// Which leaf of the root one view layout under one marker widens to.
pub trait ViewLeaf<K: ByteKind>:
    ByteViewType<Native: RunNative> + Sized + Send + Sync + 'static
{
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
    dtype.is_string() || dtype.kind() == DataTypeKind::Code
}

/// Recovered CP1252 scalars can contain unassigned characters; unlike a
/// scalar, a column must be able to write every accepted value as bytes.
fn require_encodable(field: &Field, start: usize, rows: &[Scalar]) -> Result<()> {
    if field.dtype().charset() != Some(crate::Charset::Cp1252) {
        return Ok(());
    }
    for (index, row) in rows.iter().enumerate() {
        if let Some(text) = row.as_str() {
            crate::cp1252::require_encodable(text).map_err(|refusal| {
                crate::Error::InvalidRecord {
                    path: smol_str::format_smolstr!("{}[{}]", field.name(), start + index),
                    reason: smol_str::format_smolstr!("{refusal}"),
                }
            })?;
        }
    }
    Ok(())
}

/// Lay canonical `rows` out as the array `field` projects to, once.
///
/// The crate's one scalar-array boundary, so a byte leaf never grows a
/// second writer: the rows went through the field's contract, so the only
/// refusal left is the layout's own bound on one write, which `check`
/// surfaces by name.
fn lay_out<A: Array + Clone + 'static>(field: &Field, rows: &[Scalar]) -> Result<A> {
    let borrowed: Vec<&Scalar> = rows.iter().collect();
    let array = crate::serie::value::array_of_rows(field, &borrowed)?;
    Ok(array.as_any().downcast_ref::<A>().cloned().expect(LAID_OUT))
}

// ------------------------------------------------------------------------
// One offsets run: what the byte leaf and the variant leaf's two runs
// share, so the run is written in one place.
// ------------------------------------------------------------------------

/// The bytes the run `offsets` delimit reaches between its first and last
/// offset.
///
/// A sliced run's offsets start past zero and its payload buffer runs past
/// its end, so neither the buffer's length nor the last offset alone says
/// how many bytes the rows hold.
pub(crate) fn run_bytes<O: OffsetSizeTrait>(offsets: &OffsetBuffer<O>) -> usize {
    (offsets[offsets.len() - 1] - offsets[0]).as_usize()
}

/// Refuse a write of `replacement` bytes over rows `range` of the run
/// `offsets` delimit whose offsets total would pass the offset type, naming
/// the column.
///
/// # Errors
///
/// [`layout::require_offset`] carries the rule.
pub(crate) fn require_run_fits<O: OffsetSizeTrait>(
    name: &str,
    offsets: &OffsetBuffer<O>,
    range: &Range<usize>,
    replacement: usize,
) -> Result<()> {
    let replaced = (offsets[range.end] - offsets[range.start]).as_usize();
    let total = (run_bytes(offsets) - replaced).saturating_add(replacement);
    layout::require_offset::<O>(name, total)
}

/// The Arrow array an offsets run's parts are, rebuilt with no scan.
///
/// The one door an offsets run of this module or of the variant leaf
/// becomes an array again through, so the trust it takes is stated once.
///
/// # Safety
///
/// The parts must be ones [`GenericByteArray::try_new`] accepts: every
/// consecutive pair of offsets a slice of `data` - over valid UTF-8, on
/// character boundaries, for text - and `nulls` one bit per row. The parts
/// of an array the landing door proved or a builder laid out are, and so
/// are those parts sliced alike.
pub(crate) unsafe fn runs_unchecked<T: ByteArrayType>(
    offsets: OffsetBuffer<T::Offset>,
    data: Buffer,
    nulls: Option<NullBuffer>,
) -> GenericByteArray<T> {
    // SAFETY: the caller's contract is `new_unchecked`'s own, stated above.
    unsafe { GenericByteArray::new_unchecked(offsets, data, nulls) }
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
        crate::string_scalars!(text) => row
            .string_parameters()
            .map_or(0, |leaf| leaf.encoded_len(text.as_str())),
        crate::bytes_scalars!(bytes) => bytes.as_bytes().len(),
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
    let mut builder = GenericByteBuilder::<T>::with_capacity(
        shared.len() + rows,
        run_bytes(shared.offsets()) + bytes,
    );
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
        let mut builder = run_builder(run, replacement.len(), run_bytes(replacement.offsets()));
        builder.append_array(replacement).expect(CHECKED);
        return builder.finish();
    }
    let prefix = run.slice(0, range.start);
    let suffix = run.slice(range.end, len - range.end);
    let mut builder = GenericByteBuilder::<T>::with_capacity(
        prefix.len() + replacement.len() + suffix.len(),
        run_bytes(prefix.offsets())
            + run_bytes(replacement.offsets())
            + run_bytes(suffix.offsets()),
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
///
/// The leaf holds the buffers themselves, never an Arrow array: a read is
/// one slice of the payload and [`Self::array`] hands the same buffers back
/// as the array they are.
pub struct ByteSerie<T: ByteArrayType, K: ByteKind> {
    field: Arc<Field>,
    /// One more offset than rows: row `i` occupies the payload between
    /// `offsets[i]` and `offsets[i + 1]`.
    offsets: OffsetBuffer<T::Offset>,
    /// The payload every run lies in.
    data: Buffer,
    nulls: Option<NullBuffer>,
    /// How a run reads as the field's value, resolved from the field once
    /// where the column landed.
    reading: RunReading<T::Native>,
    kind: PhantomData<K>,
    /// Where the buffers live: carried by a slice and a clone, the heap's
    /// again after a write.
    backing: Backing,
}

impl<T: ByteArrayType, K: ByteKind> ByteSerie<T, K>
where
    T::Native: RunNative,
{
    /// Pair a field with the buffers that hold its rows and the reading its
    /// datatype resolved to, taking the array apart.
    pub(crate) fn new(
        field: Arc<Field>,
        values: GenericByteArray<T>,
        reading: RunReading<T::Native>,
    ) -> Self {
        let (offsets, data, nulls) = values.into_parts();
        Self {
            field,
            offsets,
            data,
            nulls,
            reading,
            kind: PhantomData,
            backing: Backing::Heap,
        }
    }

    /// Borrow the offsets buffer, without copying it.
    ///
    /// Row `i` occupies the payload between `offsets[i]` and `offsets[i + 1]`,
    /// which is what a reader walking runs without building one needs.
    pub fn offsets(&self) -> &OffsetBuffer<T::Offset> {
        &self.offsets
    }

    /// Borrow the payload buffer every run lies in, without copying it.
    pub fn payload(&self) -> &Buffer {
        &self.data
    }

    /// Borrow the validity bitmap, or `None` where no row is absent.
    pub fn nulls(&self) -> Option<&NullBuffer> {
        self.nulls.as_ref()
    }

    /// Return the Arrow array these buffers are, sharing them: pointer bumps,
    /// no copy and no scan.
    pub fn array(&self) -> GenericByteArray<T> {
        // SAFETY: the buffers entered through `new` from an array the landing
        // door proved or a builder laid out, and were since only sliced
        // (`slice`) or replaced whole by a write's spliced array; the fields
        // are private, so no other path reaches them.
        unsafe { runs_unchecked(self.offsets.clone(), self.data.clone(), self.nulls.clone()) }
    }

    /// Borrow row `index` where it lies in the payload: `None` when the row
    /// is absent or past the end.
    ///
    /// One bounds check and one slice of the payload: no array is built and
    /// the run is not scanned.
    pub fn value(&self, index: usize) -> Option<&T::Native> {
        if index >= self.row_count() || self.is_absent(index) {
            return None;
        }
        let run = &self.data[self.offsets[index].as_usize()..self.offsets[index + 1].as_usize()];
        // SAFETY: the run between two consecutive offsets of this leaf is one
        // whole value of the layout - a whole UTF-8 string for text - because
        // the buffers came from an array the landing door proved or a builder
        // laid out, and a slice keeps every pair of offsets it keeps; this is
        // the read Arrow's own `value` makes over the same buffers.
        Some(unsafe { T::Native::from_run(run) })
    }

    /// State where the buffers live: what a spill sets over the buffers it
    /// mapped.
    pub(crate) fn set_backing(&mut self, backing: Backing) {
        self.backing = backing;
    }

    /// The rows the offsets delimit.
    fn row_count(&self) -> usize {
        self.offsets.len() - 1
    }

    /// Whether row `index`, which is in range, is absent.
    fn is_absent(&self, index: usize) -> bool {
        self.nulls
            .as_ref()
            .is_some_and(|nulls| nulls.is_null(index))
    }

    /// Refuse what a write could not do: an offsets total past the offset
    /// type.
    ///
    /// The rows are measured, never laid out: a canonical row already says
    /// what its layout stores ([`stored_bytes`]), so a replacement past the
    /// offset type is refused by name before a byte of it is built.
    pub(crate) fn check(&self, range: &Range<usize>, rows: &[Scalar]) -> Result<()> {
        require_encodable(&self.field, range.start, rows)?;
        let replacement = rows.iter().fold(0_usize, |total, row| {
            total.saturating_add(stored_bytes(row))
        });
        require_run_fits(self.field.name(), &self.offsets, range, replacement)
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
        let len = self.row_count();
        let fits = require_run_fits(
            self.field.name(),
            &self.offsets,
            &(len..len),
            run_bytes(&other.offsets),
        )
        .is_ok();
        if fits {
            self.write_array(len..len, &other.array());
        }
        fits
    }

    /// Replace rows `range` by `replacement`, which lays out as this column.
    ///
    /// The buffers are taken out of the leaf first, so the run the splice
    /// takes apart is their one holder and Arrow hands them back as a
    /// builder where nothing else holds them; what comes back is the heap's.
    fn write_array(&mut self, range: Range<usize>, replacement: &GenericByteArray<T>) {
        let offsets = std::mem::replace(&mut self.offsets, OffsetBuffer::new_empty());
        let data = std::mem::take(&mut self.data);
        // SAFETY: the leaf's own buffers, moved out whole, under the argument
        // `array` makes for them.
        let taken = unsafe { runs_unchecked(offsets, data, self.nulls.take()) };
        let (offsets, data, nulls) = splice_run(taken, range, replacement).into_parts();
        self.offsets = offsets;
        self.data = data;
        self.nulls = nulls;
        self.backing = Backing::Heap;
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
        self.row_count()
    }

    fn null_count(&self) -> usize {
        self.nulls.as_ref().map_or(0, NullBuffer::null_count)
    }

    fn is_null(&self, index: usize) -> Result<bool> {
        require_row(self.field.name(), index, self.row_count())?;
        Ok(self.is_absent(index))
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        require_row(self.field.name(), index, self.row_count())?;
        match self.value(index) {
            Some(cell) => Ok((self.reading)(self.field.dtype(), cell)?),
            None => Ok(Scalar::Null),
        }
    }

    fn slice(&self, offset: usize, length: usize) -> Result<Self> {
        require_window(self.field.name(), offset, length, self.row_count())?;
        Ok(Self {
            field: Arc::clone(&self.field),
            offsets: self.offsets.slice(offset, length),
            data: self.data.clone(),
            nulls: self.nulls.as_ref().map(|nulls| nulls.slice(offset, length)),
            reading: self.reading,
            kind: PhantomData,
            backing: self.backing,
        })
    }

    fn splice(&mut self, range: Range<usize>, rows: Vec<Scalar>) -> Result<()> {
        require_range(self.field.name(), &range, self.row_count())?;
        let canonical = rows
            .into_iter()
            .map(|row| self.field.scalar(row))
            .collect::<Result<Vec<Scalar>>>()?;
        self.check(&range, &canonical)?;
        self.write(range, canonical);
        Ok(())
    }

    fn into_arrow_array(&self) -> ArrayRef {
        Arc::new(self.array())
    }

    fn memory_size(&self) -> usize {
        // The typed array is pointer bumps, so nothing is boxed to count it.
        crate::arrow::sliced_size(&self.array())
    }

    fn resident_size(&self) -> usize {
        if self.backing.is_mapped() {
            0
        } else {
            self.memory_size()
        }
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
            offsets: self.offsets.clone(),
            data: self.data.clone(),
            nulls: self.nulls.clone(),
            reading: self.reading,
            kind: PhantomData,
            backing: self.backing,
        }
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
    ($(#[$meta:meta])* $name:ident, $arrow:ty, $marker:ty) => {
        $(#[$meta])*
        pub type $name = $crate::serie::bytes::ByteSerie<$arrow, $marker>;

        impl $crate::serie::bytes::ByteLeaf<$marker> for $arrow {
            const NAME: &'static str = stringify!($name);

            fn into_serie(column: $crate::serie::bytes::ByteSerie<Self, $marker>) -> $crate::Serie {
                $crate::serie::Leaf::root(column)
            }

            fn from_serie(
                serie: &$crate::Serie,
            ) -> Option<&$crate::serie::bytes::ByteSerie<Self, $marker>> {
                $crate::serie::Leaf::narrow(serie)
            }
        }
    };
}

pub(crate) use byte_leaf;

byte_leaf!(
    /// A column of byte runs, 32-bit offsets.
    BinarySerie, BinaryType, Octets
);
byte_leaf!(
    /// A column of byte runs, 64-bit offsets.
    LargeBinarySerie, LargeBinaryType, Octets
);

// ------------------------------------------------------------------------
// Views: runs that name where they lie rather than lying in one buffer.
// ------------------------------------------------------------------------

/// The bytes one view occupies in the views buffer.
const VIEW_BYTES: usize = std::mem::size_of::<u128>();

/// Where a short run starts inside its view: past the four-byte length.
const INLINE_AT: usize = std::mem::size_of::<u32>();

/// One column of runs held as views into several payload buffers.
///
/// The leaf holds the buffers themselves, never an Arrow array: a read
/// decodes the row's view and slices where it points, and [`Self::array`]
/// hands the same buffers back as the array they are.
pub struct ByteViewSerie<T: ByteViewType, K: ByteKind> {
    field: Arc<Field>,
    /// One view per row: its length, then the run itself where it is twelve
    /// bytes or fewer, else its prefix, a payload index and an offset.
    views: ScalarBuffer<u128>,
    /// The payloads the views of the longer runs name.
    buffers: Arc<[Buffer]>,
    nulls: Option<NullBuffer>,
    /// How a run reads as the field's value, resolved from the field once
    /// where the column landed.
    reading: RunReading<T::Native>,
    kind: PhantomData<K>,
    /// Where the buffers live: carried by a slice and a clone, the heap's
    /// again after a write.
    backing: Backing,
}

impl<T: ByteViewType, K: ByteKind> ByteViewSerie<T, K>
where
    T::Native: RunNative,
{
    /// Pair a field with the buffers that hold its rows and the reading its
    /// datatype resolved to, taking the array apart.
    pub(crate) fn new(
        field: Arc<Field>,
        values: GenericByteViewArray<T>,
        reading: RunReading<T::Native>,
    ) -> Self {
        let (views, buffers, nulls) = values.into_parts();
        Self {
            field,
            views,
            buffers,
            nulls,
            reading,
            kind: PhantomData,
            backing: Backing::Heap,
        }
    }

    /// Borrow the views buffer, one 128-bit view per row.
    pub fn views(&self) -> &[u128] {
        &self.views
    }

    /// Borrow the payload buffers the views name.
    pub fn payloads(&self) -> &[Buffer] {
        &self.buffers
    }

    /// Borrow the validity bitmap, or `None` where no row is absent.
    pub fn nulls(&self) -> Option<&NullBuffer> {
        self.nulls.as_ref()
    }

    /// Return the Arrow array these buffers are, sharing them: pointer bumps,
    /// no copy and no scan.
    pub fn array(&self) -> GenericByteViewArray<T> {
        // SAFETY: the views and payloads entered through `new` from an array
        // the landing door proved or a builder laid out, and were since only
        // sliced (`slice`) or replaced whole by a builder's (`rewrite`); the
        // fields are private, so no other path reaches them.
        unsafe {
            GenericByteViewArray::new_unchecked(
                self.views.clone(),
                Arc::clone(&self.buffers),
                self.nulls.clone(),
            )
        }
    }

    /// Borrow row `index` where its view names it: `None` when the row is
    /// absent or past the end.
    ///
    /// One bounds check, one view decoded and one slice: a run of twelve
    /// bytes or fewer lies in the view itself, a longer one in the payload
    /// the view names. No array is built and the run is not scanned.
    pub fn value(&self, index: usize) -> Option<&T::Native> {
        if index >= self.views.len() || self.is_absent(index) {
            return None;
        }
        let view = ByteView::from(self.views[index]);
        let length = view.length as usize;
        let run = if view.length <= MAX_INLINE_VIEW_LEN {
            let at = index * VIEW_BYTES + INLINE_AT;
            &self.views.inner()[at..at + length]
        } else {
            let offset = view.offset as usize;
            &self.buffers[view.buffer_index as usize][offset..offset + length]
        };
        // SAFETY: the run a view of this leaf names is one whole value of the
        // layout - a whole UTF-8 string for text - because the views came from
        // an array the landing door proved or a builder laid out, and a slice
        // keeps the views it keeps whole; this is the read Arrow's own
        // `value` makes over the same buffers.
        Some(unsafe { T::Native::from_run(run) })
    }

    /// State where the buffers live: what a spill sets over the buffers it
    /// mapped.
    pub(crate) fn set_backing(&mut self, backing: Backing) {
        self.backing = backing;
    }

    /// Whether row `index`, which is in range, is absent.
    fn is_absent(&self, index: usize) -> bool {
        self.nulls
            .as_ref()
            .is_some_and(|nulls| nulls.is_null(index))
    }

    /// Refuse recovered text whose charset cannot encode it.
    pub(crate) fn check(&self, range: &Range<usize>, rows: &[Scalar]) -> Result<()> {
        require_encodable(&self.field, range.start, rows)
    }

    /// Write canonical `rows` over a checked `range`: the views rewritten.
    pub(crate) fn write(&mut self, range: Range<usize>, rows: Vec<Scalar>) {
        let replacement: GenericByteViewArray<T> = lay_out(&self.field, &rows).expect(LAID_OUT);
        self.rewrite(range, &replacement);
    }

    /// Append `other`'s buffers, whose field agrees with this one's; views
    /// name their blocks, so no total is out of reach.
    pub(crate) fn append(&mut self, other: &Self) -> bool {
        let len = self.views.len();
        self.rewrite(len..len, &other.array());
        true
    }

    /// Replace rows `range` by `replacement`: every run packed again into
    /// fresh blocks on the heap.
    ///
    /// No builder hands views back, and a view names the buffer its run
    /// lies in, so an edit is one pass over every row. Packing the runs
    /// again rather than carrying the old buffers along is what keeps a
    /// column edited many times from holding every byte it ever held.
    fn rewrite(&mut self, range: Range<usize>, replacement: &GenericByteViewArray<T>) {
        let len = self.views.len();
        let mut builder =
            GenericByteViewBuilder::<T>::with_capacity(len - range.len() + replacement.len());
        for row in 0..range.start {
            builder.append_option(self.value(row));
        }
        for row in 0..replacement.len() {
            builder.append_option(replacement.is_valid(row).then(|| replacement.value(row)));
        }
        for row in range.end..len {
            builder.append_option(self.value(row));
        }
        let (views, buffers, nulls) = builder.finish().into_parts();
        self.views = views;
        self.buffers = buffers;
        self.nulls = nulls;
        self.backing = Backing::Heap;
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
        self.views.len()
    }

    fn null_count(&self) -> usize {
        self.nulls.as_ref().map_or(0, NullBuffer::null_count)
    }

    fn is_null(&self, index: usize) -> Result<bool> {
        require_row(self.field.name(), index, self.views.len())?;
        Ok(self.is_absent(index))
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        require_row(self.field.name(), index, self.views.len())?;
        match self.value(index) {
            Some(cell) => Ok((self.reading)(self.field.dtype(), cell)?),
            None => Ok(Scalar::Null),
        }
    }

    fn slice(&self, offset: usize, length: usize) -> Result<Self> {
        require_window(self.field.name(), offset, length, self.views.len())?;
        Ok(Self {
            field: Arc::clone(&self.field),
            views: self.views.slice(offset, length),
            buffers: Arc::clone(&self.buffers),
            nulls: self.nulls.as_ref().map(|nulls| nulls.slice(offset, length)),
            reading: self.reading,
            kind: PhantomData,
            backing: self.backing,
        })
    }

    fn splice(&mut self, range: Range<usize>, rows: Vec<Scalar>) -> Result<()> {
        require_range(self.field.name(), &range, self.views.len())?;
        let canonical = rows
            .into_iter()
            .map(|row| self.field.scalar(row))
            .collect::<Result<Vec<Scalar>>>()?;
        self.check(&range, &canonical)?;
        self.write(range, canonical);
        Ok(())
    }

    fn into_arrow_array(&self) -> ArrayRef {
        Arc::new(self.array())
    }

    fn memory_size(&self) -> usize {
        // The typed array is pointer bumps, so nothing is boxed to count it.
        crate::arrow::sliced_size(&self.array())
    }

    fn resident_size(&self) -> usize {
        if self.backing.is_mapped() {
            0
        } else {
            self.memory_size()
        }
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
            views: self.views.clone(),
            buffers: Arc::clone(&self.buffers),
            nulls: self.nulls.clone(),
            reading: self.reading,
            kind: PhantomData,
            backing: self.backing,
        }
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
    ($(#[$meta:meta])* $name:ident, $arrow:ty, $marker:ty) => {
        $(#[$meta])*
        pub type $name = $crate::serie::bytes::ByteViewSerie<$arrow, $marker>;

        impl $crate::serie::bytes::ViewLeaf<$marker> for $arrow {
            const NAME: &'static str = stringify!($name);

            fn into_serie(
                column: $crate::serie::bytes::ByteViewSerie<Self, $marker>,
            ) -> $crate::Serie {
                $crate::serie::Leaf::root(column)
            }

            fn from_serie(
                serie: &$crate::Serie,
            ) -> Option<&$crate::serie::bytes::ByteViewSerie<Self, $marker>> {
                $crate::serie::Leaf::narrow(serie)
            }
        }
    };
}

pub(crate) use view_leaf;

view_leaf!(
    /// A column of byte runs held as views.
    BinaryViewSerie, BinaryViewType, Octets
);

// ------------------------------------------------------------------------
// Fixed width: no offsets, because every run is the same length.
// ------------------------------------------------------------------------

/// One column of fixed-width byte runs: one payload buffer and no offsets.
///
/// The leaf holds the buffer itself, never an Arrow array: row `i` is the
/// `width` bytes at `i * width`, and [`Self::array`] hands the same buffer
/// back as the array it is.
pub struct FixedSerie<K: ByteKind> {
    field: Arc<Field>,
    /// The bytes every row occupies, an absent one included.
    width: i32,
    /// `width` bytes per row, from row zero.
    data: Buffer,
    nulls: Option<NullBuffer>,
    /// The row count, held because a width of zero leaves no byte to divide
    /// it out of.
    len: usize,
    /// How a slot reads as the field's value, resolved from the field once
    /// where the column landed.
    reading: RunReading<[u8]>,
    kind: PhantomData<K>,
    /// Where the buffer lives: carried by a slice and a clone, the heap's
    /// again after a write.
    backing: Backing,
}

impl<K: ByteKind> FixedSerie<K> {
    /// Pair a field with the buffer that holds its rows and the reading its
    /// datatype resolved to, taking the array apart.
    pub(crate) fn new(
        field: Arc<Field>,
        values: FixedSizeBinaryArray,
        reading: RunReading<[u8]>,
    ) -> Self {
        let len = values.len();
        let (width, data, nulls) = values.into_parts();
        Self {
            field,
            width,
            data,
            nulls,
            len,
            reading,
            kind: PhantomData,
            backing: Backing::Heap,
        }
    }

    /// Return the width every row occupies.
    pub fn width(&self) -> i32 {
        self.width
    }

    /// Borrow the payload buffer every run lies in, without copying it.
    pub fn payload(&self) -> &Buffer {
        &self.data
    }

    /// Borrow the validity bitmap, or `None` where no row is absent.
    pub fn nulls(&self) -> Option<&NullBuffer> {
        self.nulls.as_ref()
    }

    /// Return the Arrow array this buffer is, sharing it: pointer bumps and
    /// one length checked.
    pub fn array(&self) -> FixedSizeBinaryArray {
        FixedSizeBinaryArray::try_new_with_len(
            self.width,
            self.data.clone(),
            self.nulls.clone(),
            self.len,
        )
        .expect(ALIGNED)
    }

    /// Borrow row `index` where it lies in the payload: `None` when the row
    /// is absent or past the end.
    ///
    /// One bounds check and one slice of the payload: no array is built.
    pub fn value(&self, index: usize) -> Option<&[u8]> {
        if index >= self.len || self.is_absent(index) {
            return None;
        }
        let width = self.width.as_usize();
        Some(&self.data[index * width..(index + 1) * width])
    }

    /// State where the buffer lives: what a spill sets over the buffer it
    /// mapped.
    pub(crate) fn set_backing(&mut self, backing: Backing) {
        self.backing = backing;
    }

    /// Whether row `index`, which is in range, is absent.
    fn is_absent(&self, index: usize) -> bool {
        self.nulls
            .as_ref()
            .is_some_and(|nulls| nulls.is_null(index))
    }

    /// Refuse what a write could not do: a fixed-width total past `i32`.
    pub(crate) fn check(&self, range: &Range<usize>, rows: &[Scalar]) -> Result<()> {
        require_encodable(&self.field, range.start, rows)?;
        let total = (self.len - range.len() + rows.len()).saturating_mul(self.width.as_usize());
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
        let total = (self.len + other.len).saturating_mul(self.width.as_usize());
        let fits = layout::require_offset::<i32>(self.field.name(), total).is_ok();
        if fits {
            self.write_array(self.len..self.len, &other.array());
        }
        fits
    }

    /// Replace rows `range` by `replacement`, which is this column's width.
    ///
    /// Every row occupies `width` bytes at `index * width`, an absent one
    /// included, so an append and a same-count overwrite - `set` among them,
    /// one `memcpy` - write the payload the column holds when nothing else
    /// holds it and its pointer was never advanced; a foreign, shared,
    /// sliced or mapped payload, or a count that changes, is copied once
    /// from prefix, replacement and suffix, and every later edit is in
    /// place. What comes back is the heap's.
    fn write_array(&mut self, range: Range<usize>, replacement: &FixedSizeBinaryArray) {
        let len = self.len;
        let width = self.width.as_usize();
        let present: Vec<bool> = (0..replacement.len())
            .map(|row| replacement.is_valid(row))
            .collect();
        let in_place = range.start == len || range.len() == replacement.len();
        // Taken out whole, so the leaf is the payload's one holder when
        // nothing else holds it.
        let payload = match std::mem::take(&mut self.data).into_mutable() {
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
        self.nulls = layout::splice_nulls(self.nulls.take(), len, range.clone(), &present);
        self.data = payload;
        self.len = len - range.len() + replacement.len();
        self.backing = Backing::Heap;
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
        self.len
    }

    fn null_count(&self) -> usize {
        self.nulls.as_ref().map_or(0, NullBuffer::null_count)
    }

    fn is_null(&self, index: usize) -> Result<bool> {
        require_row(self.field.name(), index, self.len)?;
        Ok(self.is_absent(index))
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        require_row(self.field.name(), index, self.len)?;
        match self.value(index) {
            Some(cell) => Ok((self.reading)(self.field.dtype(), cell)?),
            None => Ok(Scalar::Null),
        }
    }

    fn slice(&self, offset: usize, length: usize) -> Result<Self> {
        require_window(self.field.name(), offset, length, self.len)?;
        let width = self.width.as_usize();
        Ok(Self {
            field: Arc::clone(&self.field),
            width: self.width,
            data: self.data.slice_with_length(offset * width, length * width),
            nulls: self.nulls.as_ref().map(|nulls| nulls.slice(offset, length)),
            len: length,
            reading: self.reading,
            kind: PhantomData,
            backing: self.backing,
        })
    }

    fn splice(&mut self, range: Range<usize>, rows: Vec<Scalar>) -> Result<()> {
        require_range(self.field.name(), &range, self.len)?;
        let canonical = rows
            .into_iter()
            .map(|row| self.field.scalar(row))
            .collect::<Result<Vec<Scalar>>>()?;
        self.check(&range, &canonical)?;
        self.write(range, canonical);
        Ok(())
    }

    fn into_arrow_array(&self) -> ArrayRef {
        Arc::new(self.array())
    }

    fn memory_size(&self) -> usize {
        // The typed array is pointer bumps, so nothing is boxed to count it.
        crate::arrow::sliced_size(&self.array())
    }

    fn resident_size(&self) -> usize {
        if self.backing.is_mapped() {
            0
        } else {
            self.memory_size()
        }
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
        Self {
            field: Arc::clone(&self.field),
            width: self.width,
            data: self.data.clone(),
            nulls: self.nulls.clone(),
            len: self.len,
            reading: self.reading,
            kind: PhantomData,
            backing: self.backing,
        }
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
    ($(#[$meta:meta])* $name:ident, $marker:ty) => {
        $(#[$meta])*
        pub type $name = $crate::serie::bytes::FixedSerie<$marker>;

        impl $crate::serie::bytes::FixedLeaf for $crate::serie::bytes::FixedSerie<$marker> {
            const NAME: &'static str = stringify!($name);

            fn into_serie(column: Self) -> $crate::Serie {
                $crate::serie::Leaf::root(column)
            }

            fn from_serie(serie: &$crate::Serie) -> Option<&Self> {
                $crate::serie::Leaf::narrow(serie)
            }
        }
    };
}

pub(crate) use fixed_leaf;

fixed_leaf!(
    /// A column of fixed-width bytes: a UUID, or any identity stored as one.
    FixedBytesSerie, Octets
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
    proof: &super::arrow::Proof,
    _budget: &mut crate::budget::MaterializationBudget,
    _resolved: Option<&super::arrow::Resolved>,
) -> crate::arrow::Result<Option<Serie>> {
    use super::arrow::held;
    use super::string::{
        BinaryStringSerie, BinaryViewStringSerie, FixedStringSerie, LargeBinaryStringSerie,
        LargeUtf8StringSerie, Utf8StringSerie, Utf8ViewStringSerie,
    };
    use crate::serie::value::{binary_reading, text_reading};

    let _ = (parent, proof);
    let text = is_text(field.dtype());
    // The layout is the array's; the reading is the field's, resolved here
    // once so a cell read is one run borrowed and one constructor.
    let dtype = field.dtype();
    Ok(Some(match array.data_type() {
        ArrowDataType::Utf8 => {
            let runs = held::<GenericByteArray<Utf8Type>>(&array)?;
            Utf8StringSerie::new(Arc::clone(&field), runs, text_reading(dtype)?).into_serie()
        }
        ArrowDataType::LargeUtf8 => {
            let runs = held::<GenericByteArray<LargeUtf8Type>>(&array)?;
            LargeUtf8StringSerie::new(Arc::clone(&field), runs, text_reading(dtype)?).into_serie()
        }
        ArrowDataType::Utf8View => {
            let runs = held::<GenericByteViewArray<StringViewType>>(&array)?;
            Utf8ViewStringSerie::new(Arc::clone(&field), runs, text_reading(dtype)?).into_serie()
        }
        ArrowDataType::Binary => {
            let runs = held::<GenericByteArray<BinaryType>>(&array)?;
            let reading = binary_reading(dtype)?;
            if text {
                BinaryStringSerie::new(Arc::clone(&field), runs, reading).into_serie()
            } else {
                BinarySerie::new(Arc::clone(&field), runs, reading).into_serie()
            }
        }
        ArrowDataType::LargeBinary => {
            let runs = held::<GenericByteArray<LargeBinaryType>>(&array)?;
            let reading = binary_reading(dtype)?;
            if text {
                LargeBinaryStringSerie::new(Arc::clone(&field), runs, reading).into_serie()
            } else {
                LargeBinarySerie::new(Arc::clone(&field), runs, reading).into_serie()
            }
        }
        ArrowDataType::BinaryView => {
            let runs = held::<GenericByteViewArray<BinaryViewType>>(&array)?;
            let reading = binary_reading(dtype)?;
            if text {
                BinaryViewStringSerie::new(Arc::clone(&field), runs, reading).into_serie()
            } else {
                BinaryViewSerie::new(Arc::clone(&field), runs, reading).into_serie()
            }
        }
        ArrowDataType::FixedSizeBinary(_) => {
            let runs = held::<FixedSizeBinaryArray>(&array)?;
            let reading = binary_reading(dtype)?;
            if text {
                FixedStringSerie::new(Arc::clone(&field), runs, reading).into_serie()
            } else {
                FixedBytesSerie::new(Arc::clone(&field), runs, reading).into_serie()
            }
        }
        _ => return Ok(None),
    }))
}
