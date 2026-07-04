//! The `serie` scalars: the dynamic [`Serie`], the statically-typed [`TypedSerie`],
//! and the buffer-backed integer series ([`Int8Serie`] … [`UInt64Serie`]).
//!
//! A serie value is a variable-length sequence of one value type — *our array* (the
//! Apache Arrow `list`). [`Serie`] is the dynamic scalar (element type erased, base
//! [`Scalar`](crate::Scalar) surface only) and [`TypedSerie<D, S>`] its
//! statically-typed form, backed by one zero-copy Arrow child array with per-element
//! scalar accessors and erasing back with [`erase`](TypedSerie::erase); every integer
//! type also has its concrete serie ([`Int8Serie`], [`Int16Serie`], [`Int32Serie`],
//! [`Int64Serie`], [`UInt8Serie`], [`UInt16Serie`], [`UInt32Serie`],
//! [`UInt64Serie`]), borrowing the raw Arrow buffers for native element access.
//! The matching [`SerieType`](yggdryl_dtype::SerieType) /
//! [`TypedSerieType`](yggdryl_dtype::TypedSerieType) data types live in
//! `yggdryl-dtype`, and the typed one's [`ScalarFactory`](crate::ScalarFactory)
//! (`TypedSerieType::scalar` / `default_scalar`) builds a [`TypedSerie`].
//!
//! ```
//! use yggdryl_scalar::{Int64Scalar, Scalar, TypedSerie};
//!
//! let numbers = TypedSerie::new(vec![Int64Scalar::new(1), Int64Scalar::new(2)]);
//! assert_eq!(numbers.len(), 2);
//! assert_eq!(numbers.get_scalar_at(0), Some(Int64Scalar::new(1)));
//! assert_eq!(
//!     TypedSerie::from_arrow(numbers.to_arrow_scalar().as_ref()).unwrap(),
//!     numbers
//! );
//! ```

/// Generates the concrete serie scalar of one integer type: `$ty`, the serie of
/// `$name` values, borrowing raw Arrow buffers (a `ScalarBuffer<$native>` element
/// buffer plus an optional `NullBuffer`). `$scalar` is the matching element
/// scalar, `$dtype` the `yggdryl-dtype` value type, `$array` the matching
/// `arrow_array` primitive array, and `$width` the element byte width used by the
/// bulk little-endian IO bridge (`from_io` / `pwrite_io`).
macro_rules! int_serie {
    ($ty:ident, $scalar:ident, $dtype:ident, $native:ty, $name:literal, $array:ident, $width:literal) => {
        #[doc = concat!("A single, possibly-null serie of `", $name, "` — *our array*, borrowing the raw Arrow")]
        #[doc = concat!("buffers ([`ScalarBuffer<", stringify!($native), ">`](crate::arrow_buffer::ScalarBuffer) elements plus an optional")]
        #[doc = "[`NullBuffer`](crate::arrow_buffer::NullBuffer)) zero-copy, optimized for native element access."]
        #[doc = ""]
        #[doc = "Where the generic [`Serie`](crate::Serie) holds an opaque Arrow array handle and"]
        #[doc = concat!("goes through the element scalars' Arrow round trip, `", stringify!($ty), "` holds the")]
        #[doc = concat!("underlying buffers themselves: [`values`](", stringify!($ty), "::values) borrows the whole")]
        #[doc = concat!("element buffer as `&[", stringify!($native), "]` without copying, [`get_at`](", stringify!($ty), "::get_at) reads")]
        #[doc = "one element null-aware as any native Rust target, and the *scalar accessor*"]
        #[doc = concat!("[`get_scalar_at`](", stringify!($ty), "::get_scalar_at) hands back an [`", stringify!($scalar), "`](crate::", stringify!($scalar), ") (the")]
        #[doc = concat!("inner null scalar for a null slot). [`from_io`](", stringify!($ty), "::from_io) /")]
        #[doc = concat!("[`pwrite_io`](", stringify!($ty), "::pwrite_io) bridge the elements to any `yggdryl-core`")]
        #[doc = "positioned-IO resource in one bulk little-endian transfer. The optimized"]
        #[doc = "[`to_arrow_scalar`](crate::Scalar::to_arrow_scalar) / [`from_arrow`](crate::Scalar::from_arrow) reassemble and"]
        #[doc = "take apart the Arrow form around the same shared buffers — reference-count"]
        #[doc = "bumps, never element copies — so the type moves across the Arrow FFI boundary"]
        #[doc = "without copying elements."]
        #[derive(Debug, Clone)]
        pub struct $ty {
            data_type: ::yggdryl_dtype::TypedSerieType<::yggdryl_dtype::$dtype>,
            values: Option<$crate::arrow_buffer::ScalarBuffer<$native>>,
            nulls: Option<$crate::arrow_buffer::NullBuffer>,
        }

        impl $ty {
            /// An array borrowing the element buffer `values` and the per-element `nulls`
            /// zero-copy. A null buffer whose length differs from the element buffer's
            /// errors with [`DataError::MismatchedNullBufferLength`](::yggdryl_dtype::DataError::MismatchedNullBufferLength).
            pub fn new(
                values: $crate::arrow_buffer::ScalarBuffer<$native>,
                nulls: Option<$crate::arrow_buffer::NullBuffer>,
            ) -> Result<Self, ::yggdryl_dtype::DataError> {
                if let Some(nulls) = &nulls {
                    if nulls.len() != values.len() {
                        return Err(::yggdryl_dtype::DataError::MismatchedNullBufferLength {
                            expected: values.len(),
                            got: nulls.len(),
                        });
                    }
                }
                Ok(Self::from_parts(values, nulls))
            }

            /// The null serie scalar.
            pub fn null() -> Self {
                Self {
                    data_type: ::yggdryl_dtype::TypedSerieType::new(::yggdryl_dtype::$dtype),
                    values: None,
                    nulls: None,
                }
            }

            // The unchecked constructor; callers guarantee `nulls` matches `values` in
            // length. An all-valid null buffer is dropped so the stored form is canonical
            // and the `nulls()` contract (`None` when every element is valid) holds on
            // every construction path.
            fn from_parts(
                values: $crate::arrow_buffer::ScalarBuffer<$native>,
                nulls: Option<$crate::arrow_buffer::NullBuffer>,
            ) -> Self {
                Self {
                    data_type: ::yggdryl_dtype::TypedSerieType::new(::yggdryl_dtype::$dtype),
                    values: Some(values),
                    nulls: nulls.filter(|nulls| nulls.null_count() > 0),
                }
            }

            /// The number of elements, `0` when null or empty
            /// ([`is_null`](crate::Scalar::is_null) distinguishes the two).
            pub fn len(&self) -> usize {
                self.values
                    .as_ref()
                    .map_or(0, $crate::arrow_buffer::ScalarBuffer::len)
            }

            /// Whether the sequence holds no elements (also `true` when null).
            pub fn is_empty(&self) -> bool {
                self.len() == 0
            }

            /// The whole element buffer as a native slice, borrowed without copying —
            /// including the (arbitrary) slots under null elements; pair with
            #[doc = concat!("[`get_at`](", stringify!($ty), "::get_at) or")]
            #[doc = concat!("[`get_scalar_at`](", stringify!($ty), "::get_scalar_at) for null-aware reads.")]
            pub fn values(&self) -> Option<&[$native]> {
                self.values.as_deref()
            }

            /// The per-element null buffer, when any element is null — `None` both for an
            /// all-valid array (an all-valid buffer is dropped at construction, so the
            /// stored form is canonical) and for the null serie.
            pub fn nulls(&self) -> Option<&$crate::arrow_buffer::NullBuffer> {
                self.nulls.as_ref()
            }

            #[doc = concat!("The elements converted out as an Arrow [`arrow_array::", stringify!($array), "`](crate::arrow_array::", stringify!($array), "),")]
            /// reassembled around the same shared buffers (a reference-count bump, not
            /// a copy) — the typed, non-optional form of the base
            /// [`Scalar::to_arrow_array`](crate::Scalar::to_arrow_array), and the
            /// explicit conversion name next to
            /// [`to_arrow_scalar`](crate::Scalar::to_arrow_scalar) (the one-element
            /// serie scalar form this array is the child of). A null serie yields an
            /// empty array (told apart from an empty serie by
            /// [`is_null`](crate::Scalar::is_null)).
            pub fn to_arrow_array(&self) -> $crate::arrow_array::$array {
                self.values.as_ref().map_or_else(
                    || $crate::arrow_array::$array::from(Vec::<$native>::new()),
                    |values| $crate::arrow_array::$array::new(values.clone(), self.nulls.clone()),
                )
            }

            /// The element at `index` read as the native Rust type `T` — the generic
            /// native accessor, answered straight from the borrowed buffers (no Arrow
            #[doc = concat!("slicing): the element becomes its [`", stringify!($scalar), "`](crate::", stringify!($scalar), ") scalar and `T` reads")]
            /// through the `as_*` contract via [`FromScalar`](crate::FromScalar).
            ///
            /// A null serie errors with [`DataError::NullValue`](::yggdryl_dtype::DataError::NullValue),
            /// an index past the end with [`DataError::OutOfBounds`](::yggdryl_dtype::DataError::OutOfBounds),
            /// and a null or non-representable element with the `as_*` contract's own errors.
            pub fn get_at<T: $crate::FromScalar>(
                &self,
                index: usize,
            ) -> Result<T, ::yggdryl_dtype::DataError> {
                let values = self.values.as_ref().ok_or(::yggdryl_dtype::DataError::NullValue)?;
                if index >= values.len() {
                    return Err(::yggdryl_dtype::DataError::OutOfBounds {
                        index,
                        len: values.len(),
                    });
                }
                let scalar = if self
                    .nulls
                    .as_ref()
                    .is_none_or(|nulls| nulls.is_valid(index))
                {
                    $crate::$scalar::new(values[index])
                } else {
                    $crate::$scalar::null()
                };
                T::from_scalar(&scalar)
            }

            /// A serie read out of a `yggdryl-core` positioned-IO resource: the whole
            /// byte size read in one bulk
            /// [`pread_byte_array`](yggdryl_core::RawIOBase::pread_byte_array) and split
            #[doc = concat!("into little-endian `", stringify!($native), "` elements, all valid (the byte")]
            /// layer carries no nulls). A byte size that is not a whole number of
            /// elements errors with [`DataError::InvalidByteLength`](::yggdryl_dtype::DataError::InvalidByteLength).
            pub fn from_io(
                io: &(impl ::yggdryl_core::RawIOBase + ?Sized),
            ) -> Result<Self, ::yggdryl_dtype::DataError> {
                let size = io.byte_size();
                if !size.is_multiple_of($width) {
                    return Err(::yggdryl_dtype::DataError::InvalidByteLength {
                        expected: size.div_ceil($width) * $width,
                        got: size,
                    });
                }
                // One bulk read instead of a per-element pread loop: a single bounds
                // check and allocation, then a pure in-memory little-endian split.
                let bytes = io.pread_byte_array(0, ::yggdryl_core::Whence::Start, size)?;
                let values = bytes
                    .chunks_exact($width)
                    .map(|chunk| {
                        <$native>::from_le_bytes(
                            chunk.try_into().expect("chunks_exact yields exact widths"),
                        )
                    })
                    .collect::<Vec<_>>();
                Ok(Self::from(values))
            }

            /// Write every element buffer slot into a `yggdryl-core` positioned-IO
            /// resource in one bulk
            /// [`pwrite_byte_array`](yggdryl_core::RawIOBase::pwrite_byte_array), element
            #[doc = concat!("`index` landing at `position + index * ", stringify!($width), "` relative to `whence` — the raw")]
            #[doc = concat!("slots under null elements included, like [`values`](", stringify!($ty), "::values).")]
            /// A null serie errors with [`DataError::NullValue`](::yggdryl_dtype::DataError::NullValue).
            pub fn pwrite_io(
                &self,
                io: &mut (impl ::yggdryl_core::RawIOBase + ?Sized),
                position: usize,
                whence: ::yggdryl_core::Whence,
            ) -> Result<(), ::yggdryl_dtype::DataError> {
                let values = self.values.as_ref().ok_or(::yggdryl_dtype::DataError::NullValue)?;
                // One bulk write instead of a per-element pwrite loop: serialize every
                // slot little-endian into one buffer, then a single positioned write.
                // Elements land at the same offsets from the once-resolved start; a
                // `Whence::End` start is now resolved once, where the old loop
                // re-resolved a growing end before every element.
                let bytes = values
                    .iter()
                    .flat_map(|value| value.to_le_bytes())
                    .collect::<Vec<u8>>();
                io.pwrite_byte_array(position, whence, &bytes)?;
                Ok(())
            }

            /// The element at `index` as a scalar (a null element is the inner null
            /// scalar), or `None` when the serie is null or `index` is out of bounds.
            pub fn get_scalar_at(&self, index: usize) -> Option<$crate::$scalar> {
                let values = self.values.as_ref()?;
                if index >= values.len() {
                    return None;
                }
                Some(
                    if self
                        .nulls
                        .as_ref()
                        .is_none_or(|nulls| nulls.is_valid(index))
                    {
                        $crate::$scalar::new(values[index])
                    } else {
                        $crate::$scalar::null()
                    },
                )
            }
        }

        impl Default for $ty {
            /// The default serie scalar: the empty serie.
            fn default() -> Self {
                Self::from_parts($crate::arrow_buffer::ScalarBuffer::from(Vec::new()), None)
            }
        }

        impl PartialEq for $ty {
            // Compared logically, like Arrow arrays: null is distinct from every
            // present serie (empty included), and two present series compare by
            // element values and per-element nullness — an all-valid null buffer
            // equals no null buffer at all.
            fn eq(&self, other: &Self) -> bool {
                match (self.values.is_none(), other.values.is_none()) {
                    (true, true) => true,
                    (false, false) => self.to_arrow_array() == other.to_arrow_array(),
                    _ => false,
                }
            }
        }

        impl Eq for $ty {}

        impl ::core::convert::From<$crate::arrow_buffer::ScalarBuffer<$native>> for $ty {
            /// An all-valid array borrowing the element buffer zero-copy.
            fn from(values: $crate::arrow_buffer::ScalarBuffer<$native>) -> Self {
                Self::from_parts(values, None)
            }
        }

        impl ::core::convert::From<$crate::arrow_array::$array> for $ty {
            /// An array taking over the Arrow array's buffers, shared zero-copy.
            fn from(values: $crate::arrow_array::$array) -> Self {
                let (_, values, nulls) = values.into_parts();
                Self::from_parts(values, nulls)
            }
        }

        impl ::core::convert::From<Vec<$native>> for $ty {
            /// An array over the native values, moved into the element buffer.
            fn from(values: Vec<$native>) -> Self {
                Self::from_parts($crate::arrow_buffer::ScalarBuffer::from(values), None)
            }
        }

        impl ::core::convert::From<Vec<Option<$native>>> for $ty {
            /// An array over the native values with per-element nulls.
            fn from(values: Vec<Option<$native>>) -> Self {
                Self::from($crate::arrow_array::$array::from(values))
            }
        }

        impl $crate::Scalar for $ty {
            type DataType = ::yggdryl_dtype::TypedSerieType<::yggdryl_dtype::$dtype>;
            #[doc = concat!("The raw element buffer — like [`values`](", stringify!($ty), "::values), it includes")]
            /// the slots under null elements.
            type Value = [$native];

            fn data_type(&self) -> &::yggdryl_dtype::TypedSerieType<::yggdryl_dtype::$dtype> {
                &self.data_type
            }

            fn is_null(&self) -> bool {
                self.values.is_none()
            }

            fn value(&self) -> Option<&[$native]> {
                self.values.as_deref()
            }

            fn to_arrow_scalar(&self) -> $crate::arrow_array::ArrayRef {
                let Some(values) = &self.values else {
                    return $crate::arrow_array::new_null_array(
                        &::yggdryl_dtype::DataType::to_arrow(&self.data_type),
                        1,
                    );
                };
                // The buffers are reassembled into the one-element serie —
                // reference-count bumps, not copies.
                let elements = $crate::arrow_array::$array::new(values.clone(), self.nulls.clone());
                let array = $crate::arrow_array::ListArray::try_new(
                    ::yggdryl_dtype::Serie::item_field(&self.data_type),
                    $crate::arrow_buffer::OffsetBuffer::from_lengths([values.len()]),
                    ::std::sync::Arc::new(elements),
                    None,
                )
                .expect(concat!("a one-element serie of ", $name, " elements is valid"));
                ::std::sync::Arc::new(array)
            }

            // The base trait's array form: the element array (not the one-element
            // list wrapper the default would give). Delegates to the inherent,
            // typed `to_arrow_array` above — which shadows this for direct calls.
            fn to_arrow_array(&self) -> $crate::arrow_array::ArrayRef {
                ::std::sync::Arc::new($ty::to_arrow_array(self))
            }

            // The dynamic serie view: this concrete serie becomes the item serie of
            // a dynamic handle — a reference-count bump, not a copy.
            fn as_serie(&self) -> Result<$crate::Serie, ::yggdryl_dtype::DataError> {
                Ok($crate::Serie::from_parts(
                    self.data_type.erase(),
                    self.values
                        .as_ref()
                        .map(|_| $crate::AnySerie::from(self.clone())),
                ))
            }

            fn from_arrow(
                array: &dyn $crate::arrow_array::Array,
            ) -> Result<Self, ::yggdryl_dtype::DataError> {
                let length = $crate::arrow_array::Array::len(array);
                if length != 1 {
                    return Err(::yggdryl_dtype::DataError::InvalidScalarLength { got: length });
                }
                // Validates the serie-of-element layout, then takes the buffers apart
                // and shares them.
                <::yggdryl_dtype::TypedSerieType<::yggdryl_dtype::$dtype> as ::yggdryl_dtype::DataType>::from_arrow(
                    $crate::arrow_array::Array::data_type(array),
                )?;
                let array = array
                    .as_any()
                    .downcast_ref::<$crate::arrow_array::ListArray>()
                    .expect("a value with a serie data type is a serie array");
                if $crate::arrow_array::Array::is_null(array, 0) {
                    return Ok(Self::null());
                }
                let elements = array.value(0);
                let elements = elements
                    .as_any()
                    .downcast_ref::<$crate::arrow_array::$array>()
                    .expect(concat!("a validated serie of ", $name, " has ", $name, " elements"));
                Ok(Self::from_parts(
                    elements.values().clone(),
                    $crate::arrow_array::Array::nulls(elements).cloned(),
                ))
            }
        }

        impl
            $crate::TypedScalar<
                ::yggdryl_dtype::TypedSerieType<::yggdryl_dtype::$dtype>,
                [$native],
                $crate::arrow_array::ListArray,
                $crate::arrow_array::$array,
            > for $ty
        {
        }
    };
}
pub(crate) use int_serie;

mod any_serie;
mod int16_serie;
mod int32_serie;
mod int64_serie;
mod int8_serie;
#[allow(clippy::module_inception)] // the dynamic-scalar module shares the family's bare name
mod serie;
pub(crate) mod struct_serie;
mod typed_serie;
mod typed_struct_serie;
mod uint16_serie;
mod uint32_serie;
mod uint64_serie;
mod uint8_serie;

pub use any_serie::AnySerie;
pub use int16_serie::Int16Serie;
pub use int32_serie::Int32Serie;
pub use int64_serie::Int64Serie;
pub use int8_serie::Int8Serie;
pub use serie::Serie;
pub use struct_serie::StructSerie;
pub use typed_serie::TypedSerie;
pub use typed_struct_serie::TypedStructSerie;
pub use uint16_serie::UInt16Serie;
pub use uint32_serie::UInt32Serie;
pub use uint64_serie::UInt64Serie;
pub use uint8_serie::UInt8Serie;
