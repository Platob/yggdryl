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
#[cfg(feature = "internals")]
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

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/text/display.rs` pins and a caller cannot reach.
    //!
    //! Bounded interpolation is what keeps an error message from growing with
    //! the payload it names: a caller reads the sentence, never the budget or
    //! the eliding that built it. Every door here forwards to the real item
    //! and hands back a public value, so `text::display` stays exactly as
    //! private as it was.

    use std::fmt;

    /// The byte budget caller text crosses before it reaches a message.
    pub const ERROR_TEXT_LIMIT: usize = super::ERROR_TEXT_LIMIT;

    /// Borrow caller text for bounded interpolation: `Display` renders it
    /// unquoted, `Debug` quoted and escaped.
    pub fn elide_to(value: &str, limit: usize) -> impl fmt::Debug + fmt::Display {
        super::elide_to(value, limit)
    }

    /// Render any [`fmt::Display`] value through an explicit byte budget.
    pub fn elide_display_to<T: fmt::Display>(value: &T, limit: usize) -> impl fmt::Display {
        super::elide_display_to(value, limit)
    }

    /// Build the canonical `expected ..., got ...` failure sentence.
    pub fn expected_got(expected: impl fmt::Display, actual: impl fmt::Display) -> String {
        super::expected_got(expected, actual).to_string()
    }
}
