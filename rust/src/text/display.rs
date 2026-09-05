//! Allocation-free helpers for canonical text representations.

use std::fmt;
use std::hash::{Hash, Hasher};

use smol_str::{SmolStr, format_smolstr};

use crate::xxhash::Xxh3;

/// Maximum bytes of caller-controlled text interpolated into one error message.
///
/// An error message must never allocate proportionally to an input payload, so
/// every caller-supplied name, value, or rendered schema crosses this budget
/// before it reaches a message.
pub(crate) const ERROR_TEXT_LIMIT: usize = 64;

/// The suffix appended when bounded interpolation drops trailing text.
const ELLIPSIS: char = '\u{2026}';

/// Hash canonical display output with the stable Yggdryl XXH3-64 contract.
///
/// The rendering is streamed through the hasher rather than assembled, so a
/// value whose canonical text is large costs no copy of it.
pub(crate) fn stable_hash_display(value: &impl fmt::Display) -> u64 {
    struct StableHasher(Xxh3);

    impl fmt::Write for StableHasher {
        fn write_str(&mut self, value: &str) -> fmt::Result {
            self.0.write_bytes(value.as_bytes());
            Ok(())
        }
    }

    let mut hasher = StableHasher(Xxh3::new());
    let result = fmt::write(&mut hasher, format_args!("{value}"));
    debug_assert!(result.is_ok(), "the stable hash sink is infallible");
    hasher.0.as_u64()
}

/// Hash a native structural [`Hash`] implementation with the stable sink.
pub(crate) fn stable_hash_of(value: &impl Hash) -> u64 {
    let mut hasher = StableHash::default();
    value.hash(&mut hasher);
    hasher.finish()
}

/// XXH3-64 behind explicit little-endian integer writes.
///
/// [`Hasher`]'s default `write_u8` through `write_usize` bodies use
/// native-endian bytes, so handing a bare [`Xxh3`] to a [`Hash`]
/// implementation would make a stored hash disagree between a big-endian and a
/// little-endian machine. Overriding them is the whole reason this stays a
/// named type rather than an inline call.
#[derive(Default)]
struct StableHash(Xxh3);

impl Hasher for StableHash {
    fn finish(&self) -> u64 {
        self.0.as_u64()
    }

    fn write(&mut self, bytes: &[u8]) {
        self.0.write_bytes(bytes);
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
        use super::stable_hash_display;
        use crate::xxhash::xxh3;

        // A str's Display output is its bytes, so hashing the rendering here
        // and hashing the bytes with `xxhash::xxh3` are the same function
        // reached two ways. That is the whole reason there is no second
        // byte-oriented spelling beside this one.
        for text in ["", "x", "fill 100 @ 187.23", "é—both\nlines"] {
            assert_eq!(xxh3(text.as_bytes()), stable_hash_display(&text));
        }
        // The published XXH3-64 vectors pin the contract.
        assert_eq!(stable_hash_display(&""), 0x2d06_8005_38d3_94c2);
        assert_eq!(stable_hash_display(&"abc"), 0x78af_5f94_892f_3950);
    }

    #[test]
    fn the_structural_sink_writes_little_endian_integers() {
        use std::hash::Hasher as _;

        use super::StableHash;
        use crate::xxhash::xxh3;

        // Every `write_*` override is pinned against the explicit
        // little-endian bytes, so a big-endian target answers the same value a
        // little-endian one stored.
        let mut sink = StableHash::default();
        sink.write_u32(0x0102_0304);
        sink.write_i64(-2);
        sink.write_usize(7);
        let mut expected = Vec::new();
        expected.extend_from_slice(&0x0102_0304_u32.to_le_bytes());
        expected.extend_from_slice(&(-2_i64).to_le_bytes());
        expected.extend_from_slice(&7_u64.to_le_bytes());
        assert_eq!(sink.finish(), xxh3(&expected));
    }
}
