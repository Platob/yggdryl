//! The string columns: every byte layout of [`bytes`](super::bytes), read
//! as text.
//!
//! A string leaf shares its buffers with the byte leaf of the same layout
//! and differs from it by the [`Chars`] marker alone: the field says which
//! of the crate's eighteen string leaves - or which registered code - the
//! bytes are, and that reading is what [`crate::SerieValue::scalar`]
//! answers. The seven names here are the seven layouts a string field can
//! project to.

use super::bytes::{Chars, byte_leaf, fixed_leaf, view_leaf};
use arrow_array::types::{
    BinaryType, BinaryViewType, LargeBinaryType, LargeUtf8Type, StringViewType, Utf8Type,
};

byte_leaf!(
    /// A column of UTF-8 text, 32-bit offsets.
    Utf8StringSerie, Utf8Type, Chars, String, StringSerie, Utf8
);
byte_leaf!(
    /// A column of UTF-8 text, 64-bit offsets.
    LargeUtf8StringSerie, LargeUtf8Type, Chars, String, StringSerie, LargeUtf8
);
byte_leaf!(
    /// A column of text in a charset Arrow cannot state, 32-bit offsets.
    BinaryStringSerie, BinaryType, Chars, String, StringSerie, Binary
);
byte_leaf!(
    /// A column of text in a charset Arrow cannot state, 64-bit offsets.
    LargeBinaryStringSerie, LargeBinaryType, Chars, String, StringSerie, LargeBinary
);
view_leaf!(
    /// A column of UTF-8 text held as views.
    Utf8ViewStringSerie, StringViewType, Chars, String, StringSerie, Utf8View
);
view_leaf!(
    /// A column of text in another charset, held as views.
    BinaryViewStringSerie, BinaryViewType, Chars, String, StringSerie, BinaryView
);
fixed_leaf!(
    /// A column of fixed-width text: a padded code, or an enum member.
    FixedStringSerie, Chars, String, StringSerie
);
