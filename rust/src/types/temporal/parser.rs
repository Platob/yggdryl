//! Temporal datatype grammar.

use smol_str::format_smolstr;

use crate::types::parser::{Parser, Token, TokenKind, is_closing_or_separator, precision_to_unit};
use crate::{DataType, Error, Result, TimeUnit, Timezone};

impl Parser<'_> {
    pub(crate) fn parse_datetime64(&mut self, keyword: &str, depth: usize) -> Result<DataType> {
        self.check_depth(depth)?;
        let mut unit = TimeUnit::Microsecond;
        let mut timezone = if keyword == "timestampltz" || keyword == "timestampwithtimezone" {
            Timezone::UTC
        } else {
            Timezone::NAIVE
        };

        if let Some(close) = self.consume_opening() {
            if self.peek_symbol() != Some(close) {
                if let Some(precision) = self.peek_integer() {
                    let precision_start = self.current_position();
                    self.index += 1;
                    unit = precision_to_unit(precision, precision_start)?;
                } else if self.peek_word_is("none") {
                    self.index += 1;
                    timezone = Timezone::NAIVE;
                } else if self.peek_word_is("some") {
                    self.index += 1;
                    let inner_close = self
                        .consume_opening()
                        .ok_or_else(|| self.error_here("expected Some(timezone)"))?;
                    timezone = self.parse_timezone()?;
                    self.expect_symbol(inner_close)?;
                } else {
                    let (parsed, unit_start) =
                        self.parse_time_unit_span(Some(close), "datetime64 unit")?;
                    unit = parsed;
                    if !unit.is_arrow_time() {
                        return Err(self.error_at(
                            unit_start,
                            "datetime64 requires a temporal resolution unit",
                        ));
                    }
                }

                if self.consume_separator() {
                    self.consume_label("timezone");
                    if self.peek_word_is("none") {
                        self.index += 1;
                        timezone = Timezone::NAIVE;
                    } else if self.peek_word_is("some") {
                        self.index += 1;
                        let inner_close = self
                            .consume_opening()
                            .ok_or_else(|| self.error_here("expected Some(timezone)"))?;
                        timezone = self.parse_timezone()?;
                        self.expect_symbol(inner_close)?;
                    } else {
                        timezone = self.parse_timezone()?;
                    }
                }
            }
            self.expect_symbol(close)?;
        }

        if self.consume_word("with") {
            self.expect_word("time")?;
            self.expect_word("zone")?;
            timezone = Timezone::UTC;
        } else if self.consume_word("without") {
            self.expect_word("time")?;
            self.expect_word("zone")?;
            timezone = Timezone::NAIVE;
        }

        DataType::datetime64(unit, timezone)
    }

    pub(crate) fn parse_sql_time(&mut self, depth: usize) -> Result<DataType> {
        self.check_depth(depth)?;
        if self.peek_opening().is_none() {
            return DataType::time(TimeUnit::Microsecond);
        }
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected time unit"))?;
        let (unit, unit_start) = if let Some(precision) = self.peek_integer() {
            let start = self.current_position();
            self.index += 1;
            (precision_to_unit(precision, start)?, start)
        } else {
            self.parse_time_unit_span(Some(close), "time unit")?
        };
        self.expect_symbol(close)?;
        DataType::time(unit).map_err(|error| self.error_at(unit_start, format_smolstr!("{error}")))
    }

    pub(crate) fn parse_required_time_unit(&mut self, depth: usize) -> Result<(TimeUnit, usize)> {
        self.check_depth(depth)?;
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected a temporal unit parameter"))?;
        let (unit, unit_start) = self.parse_time_unit_span(Some(close), "temporal unit")?;
        if !unit.is_arrow_time() {
            return Err(self.error_at(unit_start, "expected a temporal resolution"));
        }
        self.expect_symbol(close)?;
        Ok((unit, unit_start))
    }

    pub(crate) fn parse_interval_unit(&mut self, depth: usize) -> Result<TimeUnit> {
        self.check_depth(depth)?;
        let (unit, unit_start, sql_style) = if self
            .tokens
            .get(self.index)
            .is_none_or(|_| self.is_time_unit_boundary(self.index, None))
        {
            (TimeUnit::MonthDayNano, self.current_position(), false)
        } else if let Some(close) = self.consume_opening() {
            let (unit, unit_start) = self.parse_time_unit_span(Some(close), "interval unit")?;
            self.expect_symbol(close)?;
            (unit, unit_start, false)
        } else {
            let (unit, unit_start) = self.parse_time_unit_span(None, "interval unit")?;
            (unit, unit_start, true)
        };
        // In SQL, bare `INTERVAL DAY` names the day-time interval family;
        // parenthesized `interval(day)` remains the scalar Date32 unit and is
        // rejected below rather than contextually reinterpreted.
        let unit = if sql_style && unit == TimeUnit::Day {
            TimeUnit::DayTime
        } else {
            unit
        };
        if unit.is_interval() {
            Ok(unit)
        } else {
            Err(self.error_at(unit_start, "interval requires an interval layout"))
        }
    }

    pub(crate) fn parse_time_unit_span(
        &mut self,
        close: Option<char>,
        label: &str,
    ) -> Result<(TimeUnit, usize)> {
        let first = self.index;
        let mut end = first;
        while self.tokens.get(end).is_some() {
            if self.is_time_unit_boundary(end, close) {
                break;
            }
            end += 1;
        }
        if first == end {
            return Err(self.error_here(format_smolstr!("expected {label}")));
        }

        let first_token = &self.tokens[first];
        let last_token = &self.tokens[end - 1];
        let (value, source_start, quoted_token) = match &first_token.kind {
            TokenKind::Quoted(value) if end == first + 1 => {
                (value.as_str(), first_token.start + 1, Some(first_token))
            }
            _ => (
                &self.source[first_token.start..last_token.end],
                first_token.start,
                None,
            ),
        };
        let parsed = TimeUnit::from_str(value).map_err(|error| match error {
            Error::Parse {
                position, reason, ..
            } => {
                let position = quoted_token.map_or_else(
                    || source_start.saturating_add(position),
                    |token| self.quoted_source_position(token, position),
                );
                self.error_at(position, reason)
            }
            error => self.error_at(source_start, format_smolstr!("{error}")),
        });
        self.index = end;
        parsed.map(|unit| (unit, source_start))
    }

    pub(crate) fn quoted_source_position(&self, token: &Token, decoded_position: usize) -> usize {
        let Some(quote) = self.source[token.start..].chars().next() else {
            return token.start;
        };
        let mut source_position = token.start.saturating_add(quote.len_utf8());
        let body_end = token.end.saturating_sub(quote.len_utf8());
        let mut decoded_offset = 0_usize;

        while source_position < body_end {
            if decoded_position <= decoded_offset {
                return source_position;
            }
            let logical_start = source_position;
            let Some(character) = self.source[source_position..].chars().next() else {
                return logical_start;
            };
            source_position = source_position.saturating_add(character.len_utf8());

            let decoded_len =
                if character == quote && self.source[source_position..].starts_with(quote) {
                    source_position = source_position.saturating_add(quote.len_utf8());
                    quote.len_utf8()
                } else if character == '\\' {
                    let Some(escaped) = self.source[source_position..].chars().next() else {
                        return logical_start;
                    };
                    source_position = source_position.saturating_add(escaped.len_utf8());
                    if escaped == 'u' {
                        let digits_end = source_position.saturating_add(4);
                        let decoded_len = self
                            .source
                            .get(source_position..digits_end)
                            .and_then(|digits| u32::from_str_radix(digits, 16).ok())
                            .and_then(char::from_u32)
                            .map_or(1, char::len_utf8);
                        source_position = digits_end.min(body_end);
                        decoded_len
                    } else {
                        1
                    }
                } else {
                    character.len_utf8()
                };

            if decoded_position < decoded_offset.saturating_add(decoded_len) {
                return logical_start;
            }
            decoded_offset = decoded_offset.saturating_add(decoded_len);
        }
        body_end
    }

    pub(crate) fn is_time_unit_boundary(&self, index: usize, close: Option<char>) -> bool {
        match self.tokens.get(index).map(|token| &token.kind) {
            Some(TokenKind::Symbol(symbol)) => {
                close == Some(*symbol)
                    || matches!(*symbol, ',' | ';')
                    || (close.is_none()
                        && (is_closing_or_separator(*symbol)
                            || matches!(*symbol, '?' | '!')
                            || (*symbol == '['
                                && self
                                    .tokens
                                    .get(index + 1)
                                    .is_some_and(|next| next.kind == TokenKind::Symbol(']')))))
            }
            Some(TokenKind::Word(value)) if close.is_none() => {
                ["not", "required", "null", "nullable"]
                    .iter()
                    .any(|boundary| value.eq_ignore_ascii_case(boundary))
            }
            _ => false,
        }
    }
}
