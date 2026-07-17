//! [`Buffer`] — a contiguous, growable buffer of fixed-width `T`, and the physical byte
//! storage the whole `fixed` family sits on. `U8Buffer = Buffer<u8>`, aliased [`Bytes`].

use core::fmt;
use core::marker::PhantomData;
use std::mem;

use arrow_buffer::Buffer as ArrowBuffer;

use super::{NativeType, PrimitiveType, TypedField};
use crate::io::{BufferType, IOBase, IOCursor, IOSlice, IoError};

/// The largest fixed-width primitive is 32 bytes (`u256`/`i256`); a stack scratch of this
/// size encodes one value with no allocation. Every [`NativeType`] is guarded at compile time
/// (in `fixed_native!` / `wide_int!`) to fit, so this bound can never be exceeded at runtime.
const MAX_WIDTH: usize = 32;

/// The **fixed-width** buffer sub-trait — a [`BufferType`] over a [`NativeType`], with the
/// descriptor mutualized as a default method.
pub trait FixedBuffer: BufferType {
    /// The native element type.
    type Native: NativeType;

    /// The typed data type of the elements — mutualized default.
    fn data_type(&self) -> PrimitiveType<Self::Native> {
        PrimitiveType::new()
    }
}

/// A contiguous, growable buffer of `T` values with a byte cursor — the concrete storage of
/// the `fixed` family and the implementor of [`IOBase`] / [`IOCursor`] / [`IOSlice`]. For
/// `T = u8` this is [`U8Buffer`](crate::io::fixed::U8Buffer), aliased [`Bytes`](crate::io::Bytes).
///
/// DESIGN: the physical layer is an Arrow [`Buffer`](arrow_buffer::Buffer) — `Arc`-shared and
/// immutable — never exposed in a public signature. That buys **zero-copy** reads
/// ([`pread`](IOBase::pread) copies into the caller's buffer; [`as_slice`](Buffer::as_slice)
/// hands back a typed view) and **zero-copy slices** ([`slice`](IOSlice::slice) is an `Arc`
/// bump). Writes are **copy-on-write**: an in-place write reuses the allocation, a write to a
/// shared slice copies once, so the two never alias.
///
/// Two length notions coexist: [`len`](IOBase::len) is the **byte** length (the `IOBase`
/// contract), [`count`](Buffer::count) is the **element** count — equal only when
/// `T::WIDTH == 1`. Byte-level I/O and typed access share the same storage; typed access
/// assumes the storage is element-aligned and a whole number of elements (it is, when built
/// through the typed constructors / `push` / `set`).
///
/// ```
/// use yggdryl_core::io::fixed::Buffer;
/// use yggdryl_core::io::IOBase;
///
/// let mut b = Buffer::<i32>::from_vec(vec![1, 2, 3]);
/// assert_eq!(b.count(), 3);      // 3 elements
/// assert_eq!(b.len(), 12);       // 12 bytes
/// b.push(4);
/// assert_eq!(b.get(3), Some(4));
/// assert_eq!(b.as_slice(), &[1, 2, 3, 4]);
/// ```
pub struct Buffer<T: NativeType> {
    /// The Arc-shared Arrow allocation (immutable; writes copy-on-write into a fresh one).
    bytes: ArrowBuffer,
    /// The cursor — bytes from the start; may sit past the end after a seek.
    position: u64,
    _type: PhantomData<T>,
}

impl<T: NativeType> Buffer<T> {
    /// An empty buffer with the cursor at `0`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Wraps a `Vec<T>` **without copying** — the buffer takes ownership of the allocation.
    ///
    /// DESIGN: the zero-copy move rides Arrow's `Buffer::from_vec`, which requires the physical
    /// [`ArrowNativeType`](arrow_buffer::ArrowNativeType) contract, so it is bounded on it — a
    /// constraint every Arrow-native primitive satisfies. The wide 96/128/256-bit types (which
    /// are not `ArrowNativeType`) use [`from_slice`](Buffer::from_slice) /
    /// [`from_byte_vec`](Buffer::from_byte_vec) instead.
    pub fn from_vec(values: Vec<T>) -> Self
    where
        T: arrow_buffer::ArrowNativeType,
    {
        Self {
            bytes: ArrowBuffer::from_vec(values),
            position: 0,
            _type: PhantomData,
        }
    }

    /// Copies `values` into a new buffer (one allocation), encoding each element little-endian.
    /// Works for **every** [`NativeType`], including the wide non-Arrow-native ones.
    pub fn from_slice(values: &[T]) -> Self {
        let mut bytes = Vec::with_capacity(values.len() * T::WIDTH);
        let mut scratch = [0u8; MAX_WIDTH];
        for value in values {
            value.write_le(&mut scratch);
            bytes.extend_from_slice(&scratch[..T::WIDTH]);
        }
        Self::from_byte_vec(bytes)
    }

    /// Wraps an owned little-endian byte `Vec` **without copying** — the zero-copy hand-off used
    /// when a caller has already materialized the raw element bytes in one pass (see
    /// [`Serie::from_options`](super::Serie::from_options)).
    pub fn from_byte_vec(bytes: Vec<u8>) -> Self {
        Self {
            bytes: ArrowBuffer::from_vec(bytes),
            position: 0,
            _type: PhantomData,
        }
    }

    /// Wraps raw little-endian element `bytes` (as produced by [`as_bytes`](Buffer::as_bytes))
    /// into a new, element-aligned buffer — the inverse of `as_bytes`, used when reading a
    /// serialized column back. Copies into an aligned Arrow allocation so
    /// [`as_slice`](Buffer::as_slice) stays valid.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            bytes: ArrowBuffer::from(bytes),
            position: 0,
            _type: PhantomData,
        }
    }

    /// An empty buffer that can grow to `capacity` **elements** before its first reallocation.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            bytes: ArrowBuffer::from_vec(Vec::<u8>::with_capacity(capacity * T::WIDTH)),
            position: 0,
            _type: PhantomData,
        }
    }

    /// The number of `T` **elements** (the byte length divided by `T::WIDTH`).
    pub fn count(&self) -> usize {
        self.bytes.len() / T::WIDTH
    }

    /// The typed data type of the buffer's elements — a zero-cost `const` descriptor.
    pub const fn data_type(&self) -> PrimitiveType<T> {
        PrimitiveType::new()
    }

    /// A [`TypedField`] naming a column of this buffer's element type.
    pub fn field(&self, name: &str, nullable: bool) -> TypedField<T> {
        TypedField::new(name, nullable)
    }

    /// The raw bytes — zero-copy, borrowing the buffer.
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    /// The elements as a typed slice — **zero-copy**. Any trailing partial element (from a
    /// byte-level [`from_bytes`](Buffer::from_bytes) / [`pwrite`](IOBase::pwrite) that left a
    /// non-whole count) is ignored, so this returns exactly [`count`](Buffer::count) elements.
    ///
    /// # Panics
    /// A zero-copy `&[T]` cannot be reinterpreted over **misaligned** storage. Every buffer a
    /// safe path builds is element-aligned — the typed constructors, `push` / `set`, and
    /// [`slice`](IOSlice::slice) (which rejects a misaligned window). The only way to reach a
    /// misaligned buffer is [`from_arrow_buffer`](Buffer::from_arrow_buffer) with an externally
    /// misaligned Arrow buffer; use [`get`](Buffer::get) (which decodes each element) there.
    pub fn as_slice(&self) -> &[T] {
        // Reinterpret only whole elements (drop any trailing partial byte), so a non-multiple
        // length never panics — matching `count()`. For `T: NativeType` (an Arrow native, POD)
        // every bit pattern is a valid value, so the transmute is sound; only alignment can
        // fail, and that is asserted with a guided message.
        let bytes = &self.bytes.as_slice()[..self.count() * T::WIDTH];
        let (prefix, elements, _partial) = unsafe { bytes.align_to::<T>() };
        assert!(
            prefix.is_empty(),
            "Buffer<{}> storage is not element-aligned; rebuild it via from_vec/from_slice",
            T::NAME
        );
        elements
    }

    /// A fresh owned copy of the elements (one allocation).
    pub fn to_vec(&self) -> Vec<T> {
        self.as_slice().to_vec()
    }

    /// The element at `index`, or `None` if out of range. Decodes from the raw bytes, so it
    /// is safe regardless of alignment.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Buffer;
    ///
    /// let b = Buffer::<i32>::from_vec(vec![10, 20, 30]);
    /// assert_eq!(b.get(1), Some(20));
    /// assert_eq!(b.get(3), None);
    /// ```
    pub fn get(&self, index: usize) -> Option<T> {
        let start = index.checked_mul(T::WIDTH)?;
        let bytes = self.bytes.as_slice();
        if start + T::WIDTH > bytes.len() {
            return None;
        }
        Some(T::read_le(&bytes[start..]))
    }

    /// Overwrites the element at `index` (which must be in range), copy-on-write if shared.
    pub fn set(&mut self, index: usize, value: T) {
        let mut scratch = [0u8; MAX_WIDTH];
        value.write_le(&mut scratch);
        self.pwrite((index * T::WIDTH) as u64, &scratch[..T::WIDTH]);
    }

    /// Appends one element at the end, growing the buffer. Convenient for incremental writes;
    /// for building from a known set of values prefer [`from_vec`](Buffer::from_vec) /
    /// [`from_slice`](Buffer::from_slice), which construct in one shot rather than
    /// re-sealing the immutable buffer per element.
    pub fn push(&mut self, value: T) {
        let end = self.bytes.len() as u64;
        let mut scratch = [0u8; MAX_WIDTH];
        value.write_le(&mut scratch);
        self.pwrite(end, &scratch[..T::WIDTH]);
    }

    /// **Mutates the elements in place through copy-on-write** — the in-place primitive the column
    /// `*_assign` / `fill_null_mut` / `retain` ops build on. Applies `f` to the whole element slice:
    ///
    /// - when the buffer is **uniquely owned** (sole `Arc` holder, unoffset, element-aligned, and
    ///   our own allocation) it mutates the **existing allocation** — the **payload is never copied**
    ///   (only a couple of tiny `Arc` headers are re-created as Arrow re-wraps the reused allocation);
    /// - when it is **shared** (e.g. after a shallow [`clone`](Buffer::clone), which is an `Arc`
    ///   bump) — or externally sourced — it first copies the bytes into a fresh allocation (**one**
    ///   copy-on-write) and mutates that, leaving every other holder untouched (value semantics).
    ///
    /// So a hot loop of in-place ops on an owned column copies **no payload** per step (a bounded,
    /// size-independent header cost), and the first op after a shallow copy pays exactly one COW.
    ///
    /// # Panics
    /// Like [`as_slice`](Buffer::as_slice), a zero-copy `&mut [T]` requires **element-aligned**
    /// storage; every safe-path buffer is (asserted with a guided message).
    pub(crate) fn with_values_mut(&mut self, f: impl FnOnce(&mut [T])) {
        let byte_len = self.count() * T::WIDTH;
        // Take the allocation out to try the **no-payload-copy** owned path. `Buffer::into_mutable`
        // reuses the existing allocation — with NO payload copy — exactly when this is the sole
        // owner (`Arc::try_unwrap`: one strong ref, no weaks), unoffset, aligned, and **our own**
        // (`MutableBuffer::from_bytes` refuses an externally-sourced allocation — e.g. one imported
        // zero-copy from pyarrow via `from_arrow_buffer`, which may be foreign / read-only memory).
        // DESIGN: we deliberately delegate all three checks to Arrow rather than approximate them
        // with `strong_count`/`ptr_offset` (which miss the weak-ref and external-allocation cases and
        // would let an in-place write hit foreign memory). When `into_mutable` declines, we
        // copy-on-write into a fresh owned buffer, leaving every other holder untouched.
        let taken = mem::replace(&mut self.bytes, ArrowBuffer::from_vec(Vec::<u8>::new()));
        match taken.into_mutable() {
            Ok(mut mutable) => {
                Self::apply_aligned(&mut mutable.as_slice_mut()[..byte_len], f);
                self.bytes = mutable.into();
            }
            Err(shared) => {
                let mut vec = shared.as_slice().to_vec();
                Self::apply_aligned(&mut vec[..byte_len], f);
                self.bytes = ArrowBuffer::from_vec(vec);
            }
        }
    }

    /// Reinterprets `bytes` as a `&mut [T]` (whole elements, element-aligned) and runs `f` — the
    /// shared aligned-view step of both [`with_values_mut`](Buffer::with_values_mut) branches.
    fn apply_aligned(bytes: &mut [u8], f: impl FnOnce(&mut [T])) {
        // For `T: NativeType` (an Arrow-native POD) every bit pattern is valid, so the reinterpret
        // is sound; only alignment can fail, and that is asserted (matching `as_slice`).
        let (prefix, elements, _partial) = unsafe { bytes.align_to_mut::<T>() };
        assert!(
            prefix.is_empty(),
            "Buffer<{}> storage is not element-aligned; rebuild it via from_vec/from_slice",
            T::NAME
        );
        f(elements);
    }

    /// **Shrinks** the buffer to its first `count` elements — a zero-copy view narrowing (an `Arc`
    /// reslice reusing the allocation), the in-place [`retain`](super::Serie::retain) compaction
    /// commits with it after packing the kept elements to the front. A no-op when `count` already
    /// covers the whole buffer.
    pub(crate) fn truncate(&mut self, count: usize) {
        let new_len = count * T::WIDTH;
        if new_len < self.bytes.len() {
            // `slice_with_length` shares the `Arc`; reassigning drops the old handle, so the
            // (sole) owner keeps a length-`new_len` view over the same allocation — no copy.
            self.bytes = self.bytes.slice_with_length(0, new_len);
        }
    }
}

impl<T: NativeType> IOBase for Buffer<T> {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn pread(&self, offset: u64, buf: &mut [u8]) -> usize {
        let data = self.bytes.as_slice();
        if offset >= data.len() as u64 {
            return 0;
        }
        let start = offset as usize;
        let read = buf.len().min(data.len() - start);
        buf[..read].copy_from_slice(&data[start..start + read]);
        read
    }

    fn pwrite(&mut self, offset: u64, data: &[u8]) -> usize {
        if data.is_empty() {
            return 0;
        }
        let offset = offset as usize;
        let end = offset + data.len();
        // Acquire an owned `Vec` to mutate: reuse this buffer's allocation when we uniquely
        // own it (offset 0, not shared), otherwise copy-on-write so any live slice sharing the
        // allocation is left untouched. `into_vec` returns `Err(self)` — never panics — on a
        // shared or offset buffer, which is exactly the copy path.
        let current = mem::take(&mut self.bytes);
        let mut vec = match current.into_vec::<u8>() {
            Ok(owned) => owned,
            Err(shared) => shared.as_slice().to_vec(),
        };
        if end > vec.len() {
            vec.resize(end, 0); // grow, zero-filling any gap between the old end and `offset`
        }
        vec[offset..end].copy_from_slice(data);
        self.bytes = ArrowBuffer::from_vec(vec); // zero-copy back into an Arc-shared buffer
        data.len()
    }
}

impl<T: NativeType> IOCursor for Buffer<T> {
    fn position(&self) -> u64 {
        self.position
    }

    fn set_position(&mut self, position: u64) {
        self.position = position;
    }
}

impl<T: NativeType> IOSlice for Buffer<T> {
    fn slice(&self, offset: u64, len: u64) -> Result<Self, IoError> {
        let available = self.len();
        if offset > available || len > available - offset {
            return Err(IoError::SliceOutOfBounds {
                offset,
                len,
                available,
            });
        }
        // A typed window must start and span whole elements, so its bytes stay element-aligned
        // and typed access ([`as_slice`](Buffer::as_slice) / Arrow) never sees a misaligned or
        // partial element. A byte buffer (`T::WIDTH == 1`) always passes, so `Bytes` slicing is
        // unchanged.
        let width = T::WIDTH as u64;
        if !offset.is_multiple_of(width) || !len.is_multiple_of(width) {
            return Err(IoError::SliceMisaligned {
                offset,
                len,
                width: T::WIDTH,
            });
        }
        Ok(Self {
            // Zero-copy: `slice_with_length` shares the Arc, bumping the refcount only.
            bytes: self.bytes.slice_with_length(offset as usize, len as usize),
            position: 0,
            _type: PhantomData,
        })
    }
}

impl<T: NativeType> Default for Buffer<T> {
    fn default() -> Self {
        Self {
            bytes: ArrowBuffer::default(),
            position: 0,
            _type: PhantomData,
        }
    }
}

impl<T: NativeType> Clone for Buffer<T> {
    fn clone(&self) -> Self {
        Self {
            // Cheap: an Arc bump, not a payload copy.
            bytes: self.bytes.clone(),
            position: self.position,
            _type: PhantomData,
        }
    }
}

// Value comparison by content (the raw bytes): equal iff the stored bytes are equal. The
// cursor is transient I/O state, not part of the value.
impl<T: NativeType> PartialEq for Buffer<T> {
    fn eq(&self, other: &Self) -> bool {
        self.bytes.as_slice() == other.bytes.as_slice()
    }
}

impl<T: NativeType> Eq for Buffer<T> {}

impl<T: NativeType> fmt::Debug for Buffer<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never dump the whole payload — show the shape, not megabytes of bytes.
        f.debug_struct("Buffer")
            .field("type", &T::NAME)
            .field("count", &self.count())
            .field("position", &self.position)
            .finish()
    }
}

// The trait-hierarchy impls: `Buffer<T>` is the fixed implementation of `BufferType`. Bodies
// delegate to the inherent methods (Rust resolves inherent before trait methods, so `self.get`
// here calls the inherent `get`, never the trait one — no recursion).
impl<T: NativeType> BufferType for Buffer<T> {
    type Elem = T;

    fn count(&self) -> usize {
        self.count()
    }

    fn as_bytes(&self) -> &[u8] {
        self.as_bytes()
    }

    fn get(&self, index: usize) -> Option<T> {
        self.get(index)
    }
}

impl<T: NativeType> FixedBuffer for Buffer<T> {
    type Native = T;
}

/// The backing Arrow buffer for **any** `NativeType` (feature `arrow`) — the wide non-Arrow-native
/// integers need it too, so it lives outside the `ArrowNative`-only interop block.
#[cfg(feature = "arrow")]
impl<T: NativeType> Buffer<T> {
    /// The backing Arrow [`Buffer`](arrow_buffer::Buffer) — **zero-copy** (an `Arc` bump), for every
    /// `NativeType`. Crate-internal (the erased [`AnySerie`](crate::io::AnySerie) reads it).
    pub(crate) fn arrow_bytes(&self) -> arrow_buffer::Buffer {
        self.bytes.clone()
    }
}

/// Zero-copy interop with the Arrow ecosystem (feature `arrow`): a [`Buffer`] shares its
/// `Arc`-backed allocation with an Arrow buffer / `PrimitiveArray`, so conversion is a
/// refcount bump, never a payload copy.
#[cfg(feature = "arrow")]
impl<T: super::ArrowNative> Buffer<T> {
    /// The backing bytes as an Arrow value buffer, **element-aligned** — zero-copy (an `Arc`
    /// bump) when already aligned, else realigned with one copy. A byte-level
    /// [`slice`](IOSlice::slice) at an offset that is not a multiple of `align_of::<T>()`
    /// produces a misaligned buffer; Arrow's `ScalarBuffer` requires alignment, so we fix it
    /// up here instead of panicking.
    pub(crate) fn arrow_values(&self) -> arrow_buffer::Buffer {
        if self.bytes.as_ptr().align_offset(core::mem::align_of::<T>()) == 0 {
            self.bytes.clone()
        } else {
            arrow_buffer::Buffer::from(self.bytes.as_slice())
        }
    }

    /// The backing Arrow [`Buffer`](arrow_buffer::Buffer) — **zero-copy** (an `Arc` bump).
    pub fn to_arrow_buffer(&self) -> arrow_buffer::Buffer {
        self.bytes.clone()
    }

    /// Wraps an Arrow [`Buffer`](arrow_buffer::Buffer) as raw element bytes — **zero-copy**.
    pub fn from_arrow_buffer(buffer: arrow_buffer::Buffer) -> Self {
        Self {
            bytes: buffer,
            position: 0,
            _type: PhantomData,
        }
    }

    /// This buffer as an Arrow [`PrimitiveArray`](arrow_array::PrimitiveArray) (no nulls) —
    /// **zero-copy**, sharing the value allocation.
    pub fn to_arrow_array(&self) -> arrow_array::PrimitiveArray<T::Arrow> {
        let values = arrow_buffer::ScalarBuffer::<T>::new(self.arrow_values(), 0, self.count());
        arrow_array::PrimitiveArray::<T::Arrow>::new(values, None)
    }

    /// Builds a buffer from an Arrow [`PrimitiveArray`](arrow_array::PrimitiveArray)'s values —
    /// **zero-copy** (the array's nulls, if any, are dropped; use [`Serie`](super::Serie) to
    /// keep them).
    pub fn from_arrow_array(array: &arrow_array::PrimitiveArray<T::Arrow>) -> Self {
        Self::from_arrow_buffer(array.values().inner().clone())
    }
}
