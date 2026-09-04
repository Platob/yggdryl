//! Nested datatype grammar.

use smol_str::format_smolstr;

use crate::types::parser::{ListKind, Parser, normalized};
use crate::{DataType, Result, UnionMode};

impl Parser<'_> {
    pub(crate) fn parse_list(&mut self, kind: ListKind, depth: usize) -> Result<DataType> {
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected a list child in (), [], {}, or <>"))?;
        let field = self.parse_field_or_type("item", true, depth)?;
        self.expect_symbol(close)?;
        Ok(match kind {
            ListKind::List => DataType::list(field),
            ListKind::ListView => DataType::list_view(field),
            ListKind::LargeList => DataType::large_list(field),
            ListKind::LargeListView => DataType::large_list_view(field),
        })
    }

    pub(crate) fn parse_fixed_size_list(&mut self, depth: usize) -> Result<DataType> {
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected fixed-size-list parameters"))?;
        let field = self.parse_field_or_type("item", true, depth)?;
        self.expect_separator("expected a list length after the child")?;
        self.consume_label("length");
        let length = self.parse_i32("list length")?;
        self.expect_symbol(close)?;
        DataType::fixed_size_list(field, length)
    }

    pub(crate) fn parse_struct(&mut self, depth: usize) -> Result<DataType> {
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected struct fields"))?;
        let collection_close = if self.peek_symbol() == Some('[') {
            self.index += 1;
            Some(']')
        } else {
            None
        };
        let body_close = collection_close.unwrap_or(close);
        let mut fields = Vec::new();
        while self.peek_symbol() != Some(body_close) {
            fields.push(self.parse_named_field(depth)?);
            if self.peek_symbol() == Some(body_close) {
                break;
            }
            self.expect_separator("expected ',' between struct fields")?;
        }
        self.expect_symbol(body_close)?;
        if collection_close.is_some() {
            self.expect_symbol(close)?;
        }
        DataType::from_fields(fields)
    }

    pub(crate) fn parse_dictionary(&mut self, depth: usize) -> Result<DataType> {
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected dictionary key and value types"))?;
        self.consume_label("key");
        let key = self.parse_type(depth)?;
        self.expect_separator("expected dictionary value type")?;
        self.consume_label("value");
        let value = self.parse_type(depth)?;
        self.expect_symbol(close)?;
        DataType::dictionary(key, value)
    }

    pub(crate) fn parse_map(&mut self, depth: usize) -> Result<DataType> {
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected map parameters"))?;

        if self.peek_word_is("field") {
            let entries = self.parse_explicit_field(depth, Some("entries"))?;
            let mut keys_sorted = false;
            if self.consume_separator() {
                self.consume_label("keys_sorted");
                keys_sorted = self.parse_bool("keys_sorted")?;
            }
            self.expect_symbol(close)?;
            return DataType::map(entries, keys_sorted);
        }

        self.consume_label("key");
        let key = self.parse_type(depth)?;
        self.expect_separator("expected map value type")?;
        self.consume_label("value");
        let value = self.parse_type(depth)?;
        let mut keys_sorted = false;
        if self.consume_separator() {
            self.consume_label("keys_sorted");
            keys_sorted = self.parse_bool("keys_sorted")?;
        }
        self.expect_symbol(close)?;
        DataType::map_of(key, value, keys_sorted)
    }

    pub(crate) fn parse_run_end(&mut self, depth: usize) -> Result<DataType> {
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected run-end and value fields"))?;
        let run_ends = self.parse_field_or_type("run_ends", false, depth)?;
        self.expect_separator("expected encoded values field")?;
        let values = self.parse_field_or_type("values", true, depth)?;
        self.expect_symbol(close)?;
        DataType::run_end_encoded(run_ends, values)
    }

    /// Parse the optional `('crs')` / `('crs', 'algorithm')` parameters.
    ///
    /// Bare `geometry` and `geography` fill the defaults, so the parameters
    /// appear exactly when they say something. A geometry given an edge
    /// algorithm is refused by name at the algorithm's own position -
    /// straight planar lines need none - and an unknown algorithm reports the
    /// accepted vocabulary.
    pub(crate) fn parse_union(&mut self, keyword: &str, depth: usize) -> Result<DataType> {
        let is_variant = keyword == "variant";
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected union or variant members"))?;
        let mut mode = if keyword == "denseunion" || is_variant {
            UnionMode::Dense
        } else {
            UnionMode::Sparse
        };
        if self.peek_union_mode(close) {
            let position = self.current_position();
            mode = self.parse_union_mode()?;
            if is_variant && mode == UnionMode::Sparse {
                return Err(self.error_at(position, "variant layout must be dense"));
            }
            if self.peek_symbol() != Some(close) {
                self.expect_separator("expected ',' after union mode")?;
            }
        }

        let collection_close = if self.peek_symbol() == Some('[') {
            self.index += 1;
            Some(']')
        } else {
            None
        };
        let body_close = collection_close.unwrap_or(close);
        let mut fields = Vec::new();
        let mut next_id = 0_i16;

        while self.peek_symbol() != Some(body_close) {
            let member_close = if self.peek_symbol() == Some('(') {
                self.index += 1;
                Some(')')
            } else {
                None
            };

            let type_id = if let Some(value) = self.peek_integer() {
                let position = self.current_position();
                self.index += 1;
                let id = i8::try_from(value)
                    .map_err(|_| self.error_at(position, "union type id must fit in i8"))?;
                if is_variant && i16::from(id) != next_id {
                    return Err(
                        self.error_at(position, "variant type ids must be sequential from zero")
                    );
                }
                if !self.consume_symbol('=')
                    && !self.consume_symbol(':')
                    && !self.consume_symbol(',')
                {
                    return Err(self.error_here("expected '=', ':', or ',' after union type id"));
                }
                id
            } else {
                i8::try_from(next_id).map_err(|_| {
                    self.error_here(if is_variant {
                        "a variant cannot contain more than 128 members"
                    } else {
                        "a union cannot contain more than 128 members"
                    })
                })?
            };
            next_id = i16::from(type_id) + 1;
            let field =
                self.parse_field_or_type(&format_smolstr!("member_{type_id}"), true, depth)?;
            if let Some(member_close) = member_close {
                self.expect_symbol(member_close)?;
            }
            fields.push((type_id, field));

            if self.peek_symbol() == Some(body_close) {
                break;
            }
            self.expect_separator("expected ',' between union members")?;
        }
        self.expect_symbol(body_close)?;

        if collection_close.is_some() {
            if self.consume_separator() {
                let position = self.current_position();
                mode = self.parse_union_mode()?;
                if is_variant && mode == UnionMode::Sparse {
                    return Err(self.error_at(position, "variant layout must be dense"));
                }
            }
            self.expect_symbol(close)?;
        }
        if is_variant {
            DataType::dense_union(fields.into_iter().map(|(_, field)| field))
        } else {
            DataType::union(fields, mode)
        }
    }

    pub(crate) fn parse_union_mode(&mut self) -> Result<UnionMode> {
        let value = self.parse_text("union mode")?;
        match normalized(&value).as_str() {
            "dense" => Ok(UnionMode::Dense),
            "sparse" => Ok(UnionMode::Sparse),
            _ => Err(self.error_here("union mode must be dense or sparse")),
        }
    }
}
