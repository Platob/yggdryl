//! The **fixed-size byte** element types — [`FixedBinary`] and [`FixedUtf8`] — each element a fixed
//! byte width (the parameterized length), packed at that stride with **no offsets** buffer.
//!
//! They share the [`VarType`](crate::typed::VarType) bytes↔value descriptor with the variable-length
//! [`Binary`](crate::typed::Binary) / [`Utf8`](crate::typed::Utf8), but the carrier is
//! [`FixedSizeSerie`] (a single data buffer at a fixed stride) rather than the offsets+data
//! [`VarSerie`](crate::typed::VarSerie). A shorter value is **zero-padded** to the width, a longer
//! one truncated; the width lives in the [`Field`](crate::typed::Field) metadata.

use core::marker::PhantomData;

use crate::datatype_id::DataTypeId;
use crate::io::memory::{Heap, IOBase};
use crate::typed::{HeaderField, Scalar, Serie, VarType};

/// Fixed-size **binary** — each element is exactly the column's byte width (`Vec<u8>`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FixedBinary;

impl VarType for FixedBinary {
    type Owned = Vec<u8>;
    const DATA_TYPE_ID: DataTypeId = DataTypeId::FixedBinary;
    fn to_owned(bytes: &[u8]) -> Option<Vec<u8>> {
        Some(bytes.to_vec())
    }
    fn owned_bytes(value: &Vec<u8>) -> &[u8] {
        value
    }
}

/// Fixed-size **UTF-8** — each element is exactly the column's byte width (`String`), zero-padded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FixedUtf8;

impl VarType for FixedUtf8 {
    type Owned = String;
    const DATA_TYPE_ID: DataTypeId = DataTypeId::FixedUtf8;
    fn to_owned(bytes: &[u8]) -> Option<String> {
        core::str::from_utf8(bytes).ok().map(str::to_string)
    }
    fn owned_bytes(value: &String) -> &[u8] {
        value.as_bytes()
    }
}

/// A **fixed-size column** over one data buffer `D` (default [`Heap`]) at a fixed byte `width` per
/// element, plus an optional validity buffer. Element `i` is `data[i * width..(i + 1) * width]`; a
/// shorter pushed value is zero-padded, a longer one truncated.
pub struct FixedSizeSerie<T: VarType, D: IOBase = Heap> {
    data: D,
    validity: Option<D>,
    len: usize,
    width: usize,
    name: Option<Box<str>>,
    _type: PhantomData<T>,
}

impl<T: VarType> FixedSizeSerie<T, Heap> {
    /// An empty non-nullable column of the given fixed element `width` (bytes).
    pub fn new(width: usize) -> Self {
        FixedSizeSerie {
            data: Heap::new(),
            validity: None,
            len: 0,
            width,
            name: None,
            _type: PhantomData,
        }
    }

    /// A non-nullable column of fixed `width` holding `values` (each zero-padded / truncated).
    pub fn from_values(width: usize, values: &[T::Owned]) -> Self {
        let mut column = Self::new(width);
        for value in values {
            column.push(value);
        }
        column
    }

    /// A column of fixed `width` from options.
    pub fn from_options(width: usize, values: &[Option<T::Owned>]) -> Self {
        let mut column = Self::new(width);
        column.ensure_validity();
        for value in values {
            match value {
                Some(value) => column.push(value),
                None => column.push_null(),
            }
        }
        column
    }

    /// Sets the column **name** (reported by [`field`](FixedSizeSerie::field)).
    pub fn with_name(mut self, name: &str) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Appends a value's **raw bytes**, zero-padded (or truncated) to the fixed width.
    pub fn push_bytes(&mut self, bytes: &[u8]) {
        let mut slot = vec![0u8; self.width];
        let take = bytes.len().min(self.width);
        slot[..take].copy_from_slice(&bytes[..take]);
        self.data
            .pwrite_byte_array(self.len as u64 * self.width as u64, &slot);
        if let Some(validity) = self.validity.as_mut() {
            validity
                .pwrite_bit(self.len as u64, true)
                .expect("bit write into a heap");
        }
        self.len += 1;
    }

    /// Appends a **non-null** value.
    pub fn push(&mut self, value: &T::Owned) {
        self.push_bytes(T::owned_bytes(value));
    }

    /// Appends a **null** — a zero-filled slot, validity bit clear.
    pub fn push_null(&mut self) {
        self.ensure_validity();
        self.data
            .pwrite_byte_array(self.len as u64 * self.width as u64, &vec![0u8; self.width]);
        self.validity
            .as_mut()
            .expect("validity ensured")
            .pwrite_bit(self.len as u64, false)
            .expect("bit write into a heap");
        self.len += 1;
    }

    fn ensure_validity(&mut self) {
        if self.validity.is_none() {
            let mut validity = Heap::new();
            for index in 0..self.len as u64 {
                validity
                    .pwrite_bit(index, true)
                    .expect("bit write into a heap");
            }
            self.validity = Some(validity);
        }
    }
}

impl<T: VarType, D: IOBase> FixedSizeSerie<T, D> {
    /// Wraps an existing `data` buffer (+ optional validity) as a `len`-element fixed-`width` column.
    pub fn from_parts(data: D, validity: Option<D>, len: usize, width: usize) -> Self {
        FixedSizeSerie {
            data,
            validity,
            len,
            width,
            name: None,
            _type: PhantomData,
        }
    }

    /// The fixed byte **width** of one element.
    pub fn width(&self) -> usize {
        self.width
    }

    fn valid(&self, index: usize) -> bool {
        index < self.len
            && self
                .validity
                .as_ref()
                .is_none_or(|bits| bits.pread_bit(index as u64).unwrap_or(false))
    }

    /// The **raw** `width` bytes of the element at `index`, ignoring validity.
    pub fn bytes_at(&self, index: usize) -> Option<Vec<u8>> {
        (index < self.len).then(|| {
            self.data
                .pread_vec(index as u64 * self.width as u64, self.width)
        })
    }

    /// The backing data buffer.
    pub fn data(&self) -> &D {
        &self.data
    }

    /// The validity bit buffer, when nullable.
    pub fn validity(&self) -> Option<&D> {
        self.validity.as_ref()
    }

    /// Every element as its owned value, ignoring validity (a byte slot that does not decode — an
    /// invalid-UTF-8 `FixedUtf8` — is skipped). Mirrors [`VarSerie::values`](crate::typed::VarSerie).
    pub fn values(&self) -> Vec<T::Owned> {
        (0..self.len)
            .filter_map(|index| self.bytes_at(index).and_then(|bytes| T::to_owned(&bytes)))
            .collect()
    }

    /// The column's [`Field`](crate::typed::Field) — `name`, `type_id`, `nullable`, and the fixed
    /// byte `width`.
    pub fn field(&self) -> HeaderField {
        HeaderField::fixed_size(
            self.name.as_deref(),
            T::DATA_TYPE_ID,
            self.width as u32,
            self.validity.is_some(),
        )
    }
}

impl<T: VarType, D: IOBase> Scalar for FixedSizeSerie<T, D> {
    type Value = T::Owned;

    fn data_type_id(&self) -> DataTypeId {
        T::DATA_TYPE_ID
    }

    fn len(&self) -> usize {
        self.len
    }

    fn is_valid(&self, index: usize) -> bool {
        self.valid(index)
    }

    fn get(&self, index: usize) -> Option<T::Owned> {
        if self.valid(index) {
            self.bytes_at(index).and_then(|bytes| T::to_owned(&bytes))
        } else {
            None
        }
    }
}

impl<T: VarType, D: IOBase> Serie for FixedSizeSerie<T, D> {}
