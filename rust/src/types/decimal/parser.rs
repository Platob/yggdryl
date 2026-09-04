//! Decimal parameter grammar.

use crate::Result;
use crate::types::parser::Parser;

impl Parser<'_> {
    pub(crate) fn parse_decimal_parameters(&mut self, default_precision: u8) -> Result<(u8, i8)> {
        let Some(close) = self.consume_opening() else {
            return Ok((default_precision, 0));
        };
        self.consume_label("precision");
        let position = self.current_position();
        let precision_value = self.parse_integer("decimal precision")?;
        let precision = u8::try_from(precision_value)
            .map_err(|_| self.error_at(position, "decimal precision must fit in u8"))?;
        let mut scale = 0_i8;
        if self.consume_separator() {
            self.consume_label("scale");
            let position = self.current_position();
            let scale_value = self.parse_integer("decimal scale")?;
            scale = i8::try_from(scale_value)
                .map_err(|_| self.error_at(position, "decimal scale must fit in i8"))?;
        }
        self.expect_symbol(close)?;
        Ok((precision, scale))
    }
}
