//! Geospatial datatype grammar.

use smol_str::format_smolstr;

use crate::types::parser::Parser;
use crate::{DataType, EdgeAlgorithm, Result};

impl Parser<'_> {
    pub(crate) fn parse_geospatial(&mut self, geography: bool) -> Result<DataType> {
        let build = |crs: Option<&str>, algorithm: Option<EdgeAlgorithm>| {
            if geography {
                DataType::geography(crs, algorithm)
            } else {
                DataType::geometry(crs)
            }
        };
        let Some(close) = self.consume_opening() else {
            return build(None, None).map_err(|error| self.error_here(format_smolstr!("{error}")));
        };
        // Empty parentheses are the bare spelling with punctuation.
        if self.consume_symbol(close) {
            return build(None, None).map_err(|error| self.error_here(format_smolstr!("{error}")));
        }
        let crs_position = self.current_position();
        let crs = self.parse_text("a coordinate reference system")?;
        let mut algorithm = None;
        if self.consume_separator() {
            let algorithm_position = self.current_position();
            let name = self.parse_text("an edge algorithm")?;
            if !geography {
                return Err(self.error_at(
                    algorithm_position,
                    format_smolstr!(
                        "expected no edge algorithm for geometry, got {name:?}; geography is the type whose edges take one"
                    ),
                ));
            }
            algorithm =
                Some(EdgeAlgorithm::from_str(&name).map_err(|error| {
                    self.error_at(algorithm_position, format_smolstr!("{error}"))
                })?);
        }
        self.expect_symbol(close)?;
        build(Some(&crs), algorithm)
            .map_err(|error| self.error_at(crs_position, format_smolstr!("{error}")))
    }
}
