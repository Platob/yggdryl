//! [`VarSerie`] — a **variable-length typed column**: an `i32` **offsets** buffer + a **data**
//! buffer (element `i` is `data[offsets[i]..offsets[i + 1]]`), plus an optional validity bit buffer.
//!
//! This is the Arrow variable-length layout over the [`IOBase`] contract: the offsets and data are
//! each an `IOBase` source, so a `Binary` / `Utf8` column is in-heap, memory-mapped, or on device
//! memory with no change to its surface. It implements the same [`Scalar`] / [`Serie`] traits the
//! fixed families do — its `Value` is the type's owned form (`Vec<u8>` / `String`).

use core::marker::PhantomData;

use crate::datatype_id::DataTypeId;
use crate::io::memory::{Heap, IOBase};
use crate::typed::{HeaderField, Scalar, Serie, VarType};

/// A **variable-length column** over an `i32` offsets buffer + a data buffer (default [`Heap`]),
/// plus an optional validity buffer. Element `i` occupies `data[offsets[i]..offsets[i + 1]]`.
pub struct VarSerie<T: VarType, D: IOBase = Heap> {
    /// `len + 1` little-endian `i32` offsets; `offsets[0] == 0`.
    offsets: D,
    /// The concatenated element bytes.
    data: D,
    validity: Option<D>,
    len: usize,
    name: Option<Box<str>>,
    _type: PhantomData<T>,
}

impl<T: VarType> VarSerie<T, Heap> {
    /// An empty non-nullable column.
    pub fn new() -> Self {
        let mut offsets = Heap::new();
        offsets
            .pwrite_i32(0, 0)
            .expect("offset[0] into a fresh heap");
        VarSerie {
            offsets,
            data: Heap::new(),
            validity: None,
            len: 0,
            name: None,
            _type: PhantomData,
        }
    }

    /// A non-nullable column holding `values`.
    pub fn from_values(values: &[T::Owned]) -> Self {
        let mut column = Self::new();
        for value in values {
            column.push(value);
        }
        column
    }

    /// A column from options — pushing a null (an empty span) where a value is absent.
    pub fn from_options(values: &[Option<T::Owned>]) -> Self {
        let mut column = Self::new();
        column.ensure_validity();
        for value in values {
            match value {
                Some(value) => column.push(value),
                None => column.push_null(),
            }
        }
        column
    }

    /// Sets the column **name** (reported by [`field`](VarSerie::field)).
    pub fn with_name(mut self, name: &str) -> Self {
        self.name = Some(name.into());
        self
    }

    /// The byte end of the current content — `offsets[len]`.
    fn end_offset(&self) -> i32 {
        self.offsets.pread_i32(self.len as u64 * 4).unwrap_or(0)
    }

    /// Appends the **raw bytes** of a non-null element (the type-agnostic front door).
    pub fn push_bytes(&mut self, bytes: &[u8]) {
        let start = self.end_offset();
        self.data.pwrite_byte_array(start as u64, bytes);
        let end = start + bytes.len() as i32;
        self.offsets
            .pwrite_i32((self.len as u64 + 1) * 4, end)
            .expect("offset write into a heap");
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

    /// Appends a **null** — a zero-length span, validity bit clear.
    pub fn push_null(&mut self) {
        self.ensure_validity();
        let start = self.end_offset();
        self.offsets
            .pwrite_i32((self.len as u64 + 1) * 4, start) // empty span: offset[len+1] == offset[len]
            .expect("offset write into a heap");
        self.validity
            .as_mut()
            .expect("validity ensured")
            .pwrite_bit(self.len as u64, false)
            .expect("bit write into a heap");
        self.len += 1;
    }

    /// Appends an option — [`push`](VarSerie::push) / [`push_null`](VarSerie::push_null).
    pub fn push_option(&mut self, value: Option<&T::Owned>) {
        match value {
            Some(value) => self.push(value),
            None => self.push_null(),
        }
    }

    /// Ensures a validity buffer exists, back-filling every existing element as valid.
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

impl<T: VarType> Default for VarSerie<T, Heap> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: VarType, D: IOBase> VarSerie<T, D> {
    /// Wraps existing offsets + data (+ optional validity) as a `len`-element column — the zero-copy
    /// "view any [`IOBase`] pair as a variable-length column" front door.
    pub fn from_parts(offsets: D, data: D, validity: Option<D>, len: usize) -> Self {
        VarSerie {
            offsets,
            data,
            validity,
            len,
            name: None,
            _type: PhantomData,
        }
    }

    /// Whether the element at `index` is valid (non-null).
    fn valid(&self, index: usize) -> bool {
        index < self.len
            && self
                .validity
                .as_ref()
                .is_none_or(|bits| bits.pread_bit(index as u64).unwrap_or(false))
    }

    /// The **raw bytes** of the element at `index`, ignoring validity — `None` when out of range.
    pub fn bytes_at(&self, index: usize) -> Option<Vec<u8>> {
        if index >= self.len {
            return None;
        }
        let start = self.offsets.pread_i32(index as u64 * 4).ok()?.max(0) as u64;
        let end = self.offsets.pread_i32((index as u64 + 1) * 4).ok()?.max(0) as u64;
        Some(
            self.data
                .pread_vec(start, end.saturating_sub(start) as usize),
        )
    }

    /// The backing offsets buffer.
    pub fn offsets(&self) -> &D {
        &self.offsets
    }

    /// The backing data buffer.
    pub fn data(&self) -> &D {
        &self.data
    }

    /// The validity bit buffer, when the column is nullable.
    pub fn validity(&self) -> Option<&D> {
        self.validity.as_ref()
    }

    /// Every element as its owned value, ignoring validity (invalid-byte elements are skipped).
    pub fn values(&self) -> Vec<T::Owned> {
        (0..self.len)
            .filter_map(|index| self.bytes_at(index).and_then(|bytes| T::to_owned(&bytes)))
            .collect()
    }

    /// The column's [`Field`](crate::typed::Field) metadata — `name`, `type_id`, `nullable`.
    pub fn field(&self) -> HeaderField {
        HeaderField::new(
            self.name.as_deref(),
            T::DATA_TYPE_ID,
            self.validity.is_some(),
        )
    }
}

impl<T: VarType, D: IOBase> Scalar for VarSerie<T, D> {
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

impl<T: VarType, D: IOBase> Serie for VarSerie<T, D> {}
