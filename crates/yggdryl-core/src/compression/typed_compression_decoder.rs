//! [`TypedCompressionDecoder<T>`] — an element-generic compression decoder.

use crate::{CompressionDecoder, TypedDecoder};

/// A [`CompressionDecoder`] that also decodes into arrays of an arbitrary element
/// type `T` through [`TypedDecoder`].
///
/// The blanket impl makes every type that is both a [`CompressionDecoder`] and a
/// [`TypedDecoder<T>`] a `TypedCompressionDecoder<T>` automatically, so codecs
/// never implement it by hand.
pub trait TypedCompressionDecoder<T>: CompressionDecoder + TypedDecoder<T> {}

impl<T, C> TypedCompressionDecoder<T> for C where C: CompressionDecoder + TypedDecoder<T> {}
