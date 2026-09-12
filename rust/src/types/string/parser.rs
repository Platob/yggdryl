//! The one grammar every string spelling reads through.

use smol_str::format_smolstr;

use super::{StringLayout, StringParameters};
use crate::types::parser::Parser;
use crate::{Charset, DataType, Result};

impl Parser<'_> {
    /// Parse one string's optional charset and optional byte bound.
    ///
    /// The parameter list is positional and at most two long: a word is the
    /// charset, a number is the bound. Which bound it is follows from the
    /// layout - the exact width on a fixed string, the maximum on every
    /// other - so there is one number to write and one meaning it can have.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Parse`] for a charset beside a `utf8` or
    /// `ascii` spelling, since the name already declares one; for a charset
    /// no name answers to; and for a bound outside a positive `u32`.
    pub(crate) fn parse_string(&mut self, layout: StringLayout, keyword: &str) -> Result<DataType> {
        // The `utf8` and `ascii` spellings put the charset in the name, so
        // naming another one beside them would be two answers to one
        // question. The keyword arrives folded and the layout's names are
        // not, so they meet on the fold rather than on the underscores.
        let named = if crate::types::parser::folds_equal(keyword, layout.as_utf8_str()) {
            Some(Charset::Utf8)
        } else if crate::types::parser::folds_equal(keyword, layout.as_ascii_str()) {
            Some(Charset::Ascii)
        } else {
            None
        };
        let mut parameters = StringParameters::new(layout, named.unwrap_or(Charset::Utf8));
        let mut bound = None;

        if let Some(close) = self.consume_opening() {
            // Empty parentheses are the bare spelling with punctuation.
            if !self.consume_symbol(close) {
                if self.peek_integer().is_none() {
                    let position = self.current_position();
                    let name = self.parse_text("a charset")?;
                    if named.is_some() {
                        return Err(self.error_at(
                            position,
                            format_smolstr!(
                                "expected no charset on {keyword}, got {name:?}; {} is the spelling that takes one",
                                layout.as_str()
                            ),
                        ));
                    }
                    let charset = Charset::from_str(&name)
                        .map_err(|error| self.error_at(position, format_smolstr!("{error}")))?;
                    parameters = parameters.with_charset(charset);
                    if self.consume_separator() {
                        bound = Some(self.parse_bound(layout)?);
                    }
                } else {
                    bound = Some(self.parse_bound(layout)?);
                }
                self.expect_symbol(close)?;
            }
        }

        let position = self.current_position();
        if let Some(bound) = bound {
            parameters = parameters
                .try_with_bound(bound)
                .map_err(|error| self.error_at(position, format_smolstr!("{error}")))?;
        }
        DataType::string(parameters)
            .map_err(|error| self.error_at(position, format_smolstr!("{error}")))
    }

    /// Read the one number a string's parameter list carries.
    fn parse_bound(&mut self, layout: StringLayout) -> Result<u32> {
        let position = self.current_position();
        let label = match layout.is_fixed() {
            true => "a byte width",
            false => "a maximum byte length",
        };
        let value = self.parse_integer(label)?;
        u32::try_from(value).map_err(|_| {
            self.error_at(
                position,
                format_smolstr!("expected {label} inside u32, got {value}"),
            )
        })
    }
}
