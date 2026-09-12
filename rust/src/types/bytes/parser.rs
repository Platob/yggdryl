//! The one grammar every byte spelling reads through.

use smol_str::format_smolstr;

use super::{BytesLayout, BytesParameters};
use crate::types::parser::Parser;
use crate::{DataType, Result};

impl Parser<'_> {
    /// Parse one byte layout's optional bound.
    ///
    /// The parameter list is at most one number long, and which bound it is
    /// follows from the layout - the exact width on the fixed layout, the
    /// maximum on every other - so there is one number to write and one
    /// meaning it can have.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Parse`] for a bound outside a positive `u32`,
    /// and for the fixed layout with no width.
    pub(crate) fn parse_bytes(&mut self, layout: BytesLayout) -> Result<DataType> {
        let mut parameters = BytesParameters::new(layout);
        if let Some(close) = self.consume_opening() {
            // Empty parentheses are the bare spelling with punctuation.
            if !self.consume_symbol(close) {
                let position = self.current_position();
                let label = match layout.is_fixed() {
                    true => "a byte width",
                    false => "a maximum byte length",
                };
                let value = self.parse_integer(label)?;
                let bound = u32::try_from(value).map_err(|_| {
                    self.error_at(
                        position,
                        format_smolstr!("expected {label} inside u32, got {value}"),
                    )
                })?;
                parameters = parameters
                    .try_with_bound(bound)
                    .map_err(|error| self.error_at(position, format_smolstr!("{error}")))?;
                self.expect_symbol(close)?;
            }
        }
        let position = self.current_position();
        DataType::bytes(parameters)
            .map_err(|error| self.error_at(position, format_smolstr!("{error}")))
    }
}
