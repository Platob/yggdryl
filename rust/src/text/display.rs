//! Allocation-free helpers for canonical text representations.

use std::fmt;

use smol_str::{SmolStr, format_smolstr};

/// Maximum bytes of caller-controlled text interpolated into one error message.
///
/// An error message must never allocate proportionally to an input payload, so
/// every caller-supplied name, value, or rendered schema crosses this budget
/// before it reaches a message.
pub(crate) const ERROR_TEXT_LIMIT: usize = 64;

/// The suffix appended when bounded interpolation drops trailing text.
const ELLIPSIS: char = '\u{2026}';

/// Hash canonical display output with the stable Yggdryl FNV-1a contract.
pub(crate) fn stable_hash_display(value: &impl fmt::Display) -> u64 {
    struct StableHasher(u64);

    impl fmt::Write for StableHasher {
        fn write_str(&mut self, value: &str) -> fmt::Result {
            for byte in value.as_bytes() {
                self.0 ^= u64::from(*byte);
                self.0 = self.0.wrapping_mul(1_099_511_628_211);
            }
            Ok(())
        }
    }

    let mut hasher = StableHasher(14_695_981_039_346_656_037);
    let result = fmt::write(&mut hasher, format_args!("{value}"));
    debug_assert!(result.is_ok(), "the stable hash sink is infallible");
    hasher.0
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
}
