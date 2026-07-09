//! [`TypedCompressionEncoder<T>`] — an element-generic compression encoder.

use crate::{CompressionEncoder, TypedEncoder};

/// A [`CompressionEncoder`] that also encodes arrays of an arbitrary element
/// type `T` through [`TypedEncoder`].
///
/// The blanket impl makes every type that is both a [`CompressionEncoder`] and a
/// [`TypedEncoder<T>`] a `TypedCompressionEncoder<T>` automatically, so codecs
/// never implement it by hand.
pub trait TypedCompressionEncoder<T>: CompressionEncoder + TypedEncoder<T> {}

impl<T, C> TypedCompressionEncoder<T> for C where C: CompressionEncoder + TypedEncoder<T> {}
