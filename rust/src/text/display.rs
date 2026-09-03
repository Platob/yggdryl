//! Allocation-free helpers for canonical text representations.

use std::fmt;
use std::hash::{Hash, Hasher};

use smol_str::{SmolStr, format_smolstr};

/// Maximum bytes of caller-controlled text interpolated into one error message.
///
/// An error message must never allocate proportionally to an input payload, so
/// every caller-supplied name, value, or rendered schema crosses this budget
/// before it reaches a message.
pub(crate) const ERROR_TEXT_LIMIT: usize = 64;

/// The suffix appended when bounded interpolation drops trailing text.
const ELLIPSIS: char = '\u{2026}';

/// The FNV-1a offset basis every stable hash starts from.
const FNV_OFFSET_BASIS: u64 = 14_695_981_039_346_656_037;

/// Fold `bytes` into an FNV-1a state.
const fn fnv1a_fold(mut state: u64, bytes: &[u8]) -> u64 {
    let mut index = 0;
    while index < bytes.len() {
        state ^= bytes[index] as u64;
        state = state.wrapping_mul(1_099_511_628_211);
        index += 1;
    }
    state
}

/// Hash raw bytes with the stable Yggdryl FNV-1a contract.
///
/// This is the same 64-bit state every core value's `stable_hash` folds its
/// canonical `Display` rendering through, exposed for a caller that already
/// holds the bytes - hashing a `&str` here equals hashing its rendering
/// there, so the two spellings can never disagree. It is the deterministic
/// value the line projection's `hash` column carries (reinterpreted as
/// `i64`, bit pattern preserved), so it is stable across runs, platforms,
/// and releases by contract.
pub const fn stable_hash_bytes(bytes: &[u8]) -> u64 {
    fnv1a_fold(FNV_OFFSET_BASIS, bytes)
}

/// Hash a sequence of byte chunks as one value, with the same contract.
///
/// The fold is associative over the chunk boundary, so the chunks of a value
/// and the contiguous value hash **identically**. That is what lets a message
/// spliced from two spans - the record with the row header removed from the middle
/// of a line - hash the same as the equivalent joined string, without ever
/// building the join. A hash that depended on where the row header sat in the line
/// would be a silent correctness bug.
///
/// ```
/// use yggdryl::text::{stable_hash_bytes, stable_hash_chunks};
///
/// assert_eq!(
///     stable_hash_chunks([b"fill ".as_slice(), b"100".as_slice()]),
///     stable_hash_bytes(b"fill 100"),
/// );
/// // An empty chunk contributes nothing, wherever it sits.
/// assert_eq!(
///     stable_hash_chunks([b"".as_slice(), b"fill 100".as_slice(), b"".as_slice()]),
///     stable_hash_bytes(b"fill 100"),
/// );
/// ```
pub fn stable_hash_chunks<'chunk>(chunks: impl IntoIterator<Item = &'chunk [u8]>) -> u64 {
    chunks.into_iter().fold(FNV_OFFSET_BASIS, fnv1a_fold)
}

/// Hash canonical display output with the stable Yggdryl FNV-1a contract.
pub(crate) fn stable_hash_display(value: &impl fmt::Display) -> u64 {
    struct StableHasher(u64);

    impl fmt::Write for StableHasher {
        fn write_str(&mut self, value: &str) -> fmt::Result {
            self.0 = fnv1a_fold(self.0, value.as_bytes());
            Ok(())
        }
    }

    let mut hasher = StableHasher(FNV_OFFSET_BASIS);
    let result = fmt::write(&mut hasher, format_args!("{value}"));
    debug_assert!(result.is_ok(), "the stable hash sink is infallible");
    hasher.0
}

/// Hash a native structural [`Hash`] implementation with the stable FNV sink.
pub(crate) fn stable_hash_of(value: &impl Hash) -> u64 {
    let mut hasher = StableHash::default();
    value.hash(&mut hasher);
    hasher.finish()
}

struct StableHash(u64);

impl Default for StableHash {
    fn default() -> Self {
        Self(FNV_OFFSET_BASIS)
    }
}

impl Hasher for StableHash {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        self.0 = fnv1a_fold(self.0, bytes);
    }

    fn write_u8(&mut self, value: u8) {
        self.write(&value.to_le_bytes());
    }

    fn write_u16(&mut self, value: u16) {
        self.write(&value.to_le_bytes());
    }

    fn write_u32(&mut self, value: u32) {
        self.write(&value.to_le_bytes());
    }

    fn write_u64(&mut self, value: u64) {
        self.write(&value.to_le_bytes());
    }

    fn write_u128(&mut self, value: u128) {
        self.write(&value.to_le_bytes());
    }

    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }

    fn write_i8(&mut self, value: i8) {
        self.write(&value.to_le_bytes());
    }

    fn write_i16(&mut self, value: i16) {
        self.write(&value.to_le_bytes());
    }

    fn write_i32(&mut self, value: i32) {
        self.write(&value.to_le_bytes());
    }

    fn write_i64(&mut self, value: i64) {
        self.write(&value.to_le_bytes());
    }

    fn write_i128(&mut self, value: i128) {
        self.write(&value.to_le_bytes());
    }

    fn write_isize(&mut self, value: isize) {
        self.write_i64(value as i64);
    }
}

/// Borrow caller text for bounded interpolation into an error message.
///
/// `Display` renders the text unquoted; `Debug` renders it quoted and escaped,
/// which is what a caller-supplied identifier requires so an empty string, a
/// trailing space, or a control character stays visible. Both stop at the last
/// character boundary at or before `limit` and append an ellipsis.
pub(crate) const fn elide_to(value: &str, limit: usize) -> Elided<'_> {
    Elided { value, limit }
}

/// Caller text bounded to a byte budget for error interpolation.
#[derive(Clone, Copy)]
pub(crate) struct Elided<'a> {
    value: &'a str,
    limit: usize,
}

/// Returns the retained prefix and whether anything was dropped.
fn truncate_at_boundary(value: &str, limit: usize) -> (&str, bool) {
    if value.len() <= limit {
        return (value, false);
    }
    let mut end = limit;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    (&value[..end], true)
}

impl fmt::Display for Elided<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (retained, elided) = truncate_at_boundary(self.value, self.limit);
        formatter.write_str(retained)?;
        if elided {
            formatter.write_char_ellipsis()?;
        }
        Ok(())
    }
}

impl fmt::Debug for Elided<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (retained, elided) = truncate_at_boundary(self.value, self.limit);
        if elided {
            // Escape the retained prefix, then reopen the quote for the
            // ellipsis so the rendered value stays one balanced literal.
            let escaped = format!("{retained:?}");
            let trimmed = escaped.strip_suffix('"').unwrap_or(&escaped);
            formatter.write_str(trimmed)?;
            write!(formatter, "{ELLIPSIS}\"")
        } else {
            write!(formatter, "{retained:?}")
        }
    }
}

/// Extension used by [`Elided`] so the ellipsis has one spelling.
trait WriteEllipsis {
    fn write_char_ellipsis(&mut self) -> fmt::Result;
}

impl WriteEllipsis for fmt::Formatter<'_> {
    fn write_char_ellipsis(&mut self) -> fmt::Result {
        use fmt::Write as _;
        self.write_char(ELLIPSIS)
    }
}

/// Render any [`fmt::Display`] value through the error-text byte budget.
///
/// Use this for nested [`crate::DataType`], [`crate::Field`], and
/// `arrow_schema::Schema` values, whose canonical text grows with the caller's
/// schema and must not be copied whole into a message.
pub(crate) const fn elide_display<T: fmt::Display>(value: &T) -> ElidedDisplay<'_, T> {
    ElidedDisplay {
        value,
        limit: ERROR_TEXT_LIMIT,
    }
}

/// Render any [`fmt::Display`] value through an explicit byte budget.
#[cfg(test)]
pub(crate) const fn elide_display_to<T: fmt::Display>(
    value: &T,
    limit: usize,
) -> ElidedDisplay<'_, T> {
    ElidedDisplay { value, limit }
}

/// A [`fmt::Display`] value bounded to a byte budget for error interpolation.
#[derive(Clone, Copy)]
pub(crate) struct ElidedDisplay<'a, T: fmt::Display> {
    value: &'a T,
    limit: usize,
}

impl<T: fmt::Display> fmt::Display for ElidedDisplay<'_, T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        /// Counts bytes and drops writes past the budget instead of allocating
        /// the complete rendering first.
        struct BoundedSink<'sink, 'target> {
            target: &'sink mut fmt::Formatter<'target>,
            remaining: usize,
            elided: bool,
        }

        impl fmt::Write for BoundedSink<'_, '_> {
            fn write_str(&mut self, value: &str) -> fmt::Result {
                if self.remaining == 0 {
                    self.elided |= !value.is_empty();
                    return Ok(());
                }
                let (retained, elided) = truncate_at_boundary(value, self.remaining);
                self.remaining -= retained.len();
                self.elided |= elided;
                self.target.write_str(retained)
            }
        }

        let mut sink = BoundedSink {
            target: formatter,
            remaining: self.limit,
            elided: false,
        };
        fmt::write(&mut sink, format_args!("{}", self.value))?;
        let elided = sink.elided;
        if elided {
            use fmt::Write as _;
            formatter.write_char(ELLIPSIS)?;
        }
        Ok(())
    }
}

/// Build the canonical failure sentence required by the error contract.
///
/// Both sides render through the same formatter so a reader can diff them by
/// eye.
pub(crate) fn expected_got(expected: impl fmt::Display, actual: impl fmt::Display) -> SmolStr {
    format_smolstr!("expected {expected}, got {actual}")
}

#[cfg(test)]
mod tests {
    use super::{ERROR_TEXT_LIMIT, elide_display_to, elide_to, expected_got};

    #[test]
    fn short_text_is_unchanged() {
        assert_eq!(elide_to("id", ERROR_TEXT_LIMIT).to_string(), "id");
        assert_eq!(format!("{:?}", elide_to("id", ERROR_TEXT_LIMIT)), "\"id\"");
    }

    #[test]
    fn debug_keeps_empty_and_whitespace_visible() {
        let quoted = |value: &str| format!("{:?}", elide_to(value, ERROR_TEXT_LIMIT));
        assert_eq!(quoted(""), "\"\"");
        assert_eq!(quoted("a "), "\"a \"");
        assert_eq!(quoted("a\tb"), "\"a\\tb\"");
    }

    #[test]
    fn long_text_is_bounded_and_marked() {
        let long = "x".repeat(ERROR_TEXT_LIMIT * 4);
        let rendered = elide_to(&long, ERROR_TEXT_LIMIT).to_string();
        assert!(rendered.len() <= ERROR_TEXT_LIMIT + 4, "{}", rendered.len());
        assert!(rendered.ends_with('\u{2026}'), "{rendered}");
    }

    #[test]
    fn truncation_never_splits_a_character() {
        // Each `é` is two bytes, so a 5-byte budget must stop at 4.
        let text = "ééé";
        let rendered = elide_to(text, 5).to_string();
        assert_eq!(rendered, "éé\u{2026}");
    }

    #[test]
    fn debug_truncation_stays_a_balanced_literal() {
        let rendered = format!("{:?}", elide_to("abcdefgh", 3));
        assert_eq!(rendered, "\"abc\u{2026}\"");
    }

    #[test]
    fn display_values_are_bounded_without_rendering_the_whole_value() {
        struct Wide(usize);
        impl std::fmt::Display for Wide {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                for index in 0..self.0 {
                    write!(formatter, "field_{index},")?;
                }
                Ok(())
            }
        }

        let rendered = elide_display_to(&Wide(10_000), 32).to_string();
        assert!(rendered.len() <= 36, "{}", rendered.len());
        assert!(rendered.ends_with('\u{2026}'), "{rendered}");
        assert!(rendered.starts_with("field_0,"), "{rendered}");
    }

    #[test]
    fn expected_got_uses_the_contract_sentence() {
        assert_eq!(expected_got("int64", "utf8"), "expected int64, got utf8");
    }

    #[test]
    fn byte_and_display_hashing_agree() {
        use super::{stable_hash_bytes, stable_hash_display};

        // A str's Display output is its bytes, so the two entry points are the
        // same function reached two ways.
        for text in ["", "x", "fill 100 @ 187.23", "é—both\nlines"] {
            assert_eq!(
                stable_hash_bytes(text.as_bytes()),
                stable_hash_display(&text)
            );
        }
        // The classic FNV-1a test vector pins the offset basis and prime.
        assert_eq!(stable_hash_bytes(b""), 14_695_981_039_346_656_037);
        assert_eq!(stable_hash_bytes(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}
