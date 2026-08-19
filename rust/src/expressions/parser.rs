//! The SQL-like grammar: one recursive-descent parser with byte positions.
//!
//! The shape follows the two grammars this crate already has
//! ([`DataType::from_str`](crate::DataType::from_str) and
//! [`Field::from_str`](crate::Field::from_str)): a byte offset in every error,
//! an explicit recursion limit refused as a typed error rather than a stack
//! overflow, top-level splitting that honors quoting, and no heuristic
//! stripping of an unmatched delimiter. `CAST(x AS <type>)` hands the type half
//! straight to `DataType::from_str`, so every type the schema grammar accepts is
//! accepted here - there is no second type grammar.
//!
//! # Encapsulators
//!
//! A column called `total amount`, `select`, or `prix (€)` has to be
//! addressable, and only a delimiter pair can make it so. Three are accepted -
//! `"ansi"`, `` `hive` ``, and `[t-sql]` - each doubling its own closer to embed
//! it, and everything between the pair is part of the name: whitespace,
//! punctuation, operators, keywords, digits, and Unicode alike. One canonical
//! spelling comes back out.
//!
//! A double-quoted token is always an *identifier*, never a string. That is the
//! ANSI rule and it removes the one real ambiguity in the grammar:
//! `"venue" = 'XNAS'` compares a column to a string, while `'venue' = 'XNAS'`
//! compares two strings.

use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use super::{Accessor, ArithOp, CompareOp, Expr, Function, RECURSION_LIMIT, Statement};
use crate::{DataType, Error, Result, Value};

/// Parse one complete expression, refusing a trailing token.
///
/// # Errors
///
/// Returns [`Error::Parse`] with the byte offset of the failure and what was
/// expected there.
pub(super) fn parse_expression(text: &str) -> Result<Expr> {
    let mut parser = Parser {
        text,
        position: 0,
        depth: 0,
    };
    let expression = parser.parse_alias()?;
    parser.skip_trivia();
    if parser.position < text.len() {
        return Err(parser.error_at(
            parser.position,
            format_smolstr!(
                "expected end of expression, got {:?}",
                crate::text::elide_to(&text[parser.position..], 32)
            ),
        ));
    }
    Ok(expression)
}

/// Parse a comma-separated projection, each item with an optional alias.
///
/// The split honors quoting because it is not a split at all: the same parser
/// reads one item and then looks for a comma, so a comma inside `"a, b"` is
/// part of a name rather than a separator.
///
/// # Errors
///
/// Returns [`Error::Parse`] with the byte offset of the failure.
pub(super) fn parse_selection(text: &str) -> Result<Vec<Expr>> {
    let mut parser = Parser {
        text,
        position: 0,
        depth: 0,
    };
    let mut items = Vec::new();
    loop {
        items.push(parser.parse_alias()?);
        if parser.eat_symbol(Symbol::Comma)? {
            continue;
        }
        parser.skip_trivia();
        if parser.position < text.len() {
            return Err(parser.error_at(
                parser.position,
                format_smolstr!(
                    "expected \',\' or the end of the projection, got {:?}",
                    crate::text::elide_to(&text[parser.position..], 32)
                ),
            ));
        }
        return Ok(items);
    }
}

/// The words that are always keywords, never a bare column name.
///
/// A word outside this list stays available as an identifier even when the
/// grammar also gives it a meaning: `DATE` introduces a typed literal only when
/// a string follows it, so a column actually named `date` still reads bare.
const RESERVED: [&str; 21] = [
    "AND", "AS", "BETWEEN", "CASE", "CAST", "ELSE", "END", "ESCAPE", "FALSE", "ILIKE", "IN", "IS",
    "LIKE", "NAN", "NOT", "NULL", "OR", "THEN", "TRUE", "TRY_CAST", "WHEN",
];

/// Return whether a name is a keyword and therefore needs quoting.
#[must_use]
pub(super) fn is_reserved_word(name: &str) -> bool {
    // `INFINITY` is reserved too; it lives outside the sorted array because
    // that array is also the parser's own keyword set and infinity is read in
    // primary position only.
    RESERVED.iter().any(|word| word.eq_ignore_ascii_case(name))
        || name.eq_ignore_ascii_case("INFINITY")
}

/// One lexical token, with the span it covers.
#[derive(Clone, Debug)]
enum Token {
    /// End of input.
    End,
    /// A bare word: a keyword candidate or an unquoted identifier.
    Word(SmolStr),
    /// An encapsulated identifier, which is never a keyword.
    Quoted(SmolStr),
    /// A single-quoted string literal.
    Text(SmolStr),
    /// A numeric literal, exactly as written.
    Number(SmolStr),
    /// Punctuation.
    Symbol(Symbol),
}

/// Every punctuation token the grammar reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Symbol {
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    OpenParen,
    CloseParen,
    OpenBracket,
    CloseBracket,
    Comma,
    Dot,
    Colon,
    DoubleColon,
    Semicolon,
}

impl Symbol {
    /// The text this symbol was written as, for an error message.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Eq => "=",
            Self::NotEq => "<>",
            Self::Lt => "<",
            Self::LtEq => "<=",
            Self::Gt => ">",
            Self::GtEq => ">=",
            Self::Plus => "+",
            Self::Minus => "-",
            Self::Star => "*",
            Self::Slash => "/",
            Self::Percent => "%",
            Self::OpenParen => "(",
            Self::CloseParen => ")",
            Self::OpenBracket => "[",
            Self::CloseBracket => "]",
            Self::Comma => ",",
            Self::Dot => ".",
            Self::Colon => ":",
            Self::DoubleColon => "::",
            Self::Semicolon => ";",
        }
    }
}

/// How a `[` at the current position is to be read.
///
/// This is the one genuine collision in the grammar, and it is decided in the
/// lexer rather than guessed later: a bracket in *primary* position (the start
/// of an expression, or straight after a `.`) opens a quoted identifier, while
/// a bracket straight after a completed primary opens a subscript. Whitespace
/// never changes which one it is.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Bracket {
    /// `[my col]` names a column.
    Identifier,
    /// `a[0]` reaches inside one.
    Subscript,
}

/// The recursive-descent state: the text and where reading stopped.
struct Parser<'text> {
    text: &'text str,
    position: usize,
    depth: usize,
}

impl<'text> Parser<'text> {
    /// Build a byte-positioned parse failure.
    fn error_at(&self, position: usize, reason: SmolStr) -> Error {
        Error::Parse {
            target: "expression",
            position,
            reason,
        }
    }

    /// Skip whitespace and both comment forms.
    ///
    /// Neither comment can begin inside an encapsulator, because an
    /// encapsulated token is consumed whole by the scanner before trivia is
    /// ever considered again.
    fn skip_trivia(&mut self) {
        let bytes = self.text.as_bytes();
        loop {
            while self.position < bytes.len() {
                let rest = &self.text[self.position..];
                let Some(character) = rest.chars().next() else {
                    break;
                };
                if character.is_whitespace() {
                    self.position += character.len_utf8();
                } else {
                    break;
                }
            }
            if self.text[self.position..].starts_with("--") {
                let rest = &self.text[self.position..];
                self.position += rest.find('\n').map_or(rest.len(), |index| index + 1);
                continue;
            }
            if self.text[self.position..].starts_with("/*") {
                let rest = &self.text[self.position + 2..];
                self.position += rest.find("*/").map_or(rest.len() + 2, |index| index + 4);
                continue;
            }
            return;
        }
    }

    /// Read the token at the current position without consuming it.
    fn peek(&mut self, bracket: Bracket) -> Result<(Token, usize)> {
        self.skip_trivia();
        let start = self.position;
        let token = self.scan(start, bracket)?;
        self.position = start;
        Ok(token)
    }

    /// Consume the token at the current position.
    fn next(&mut self, bracket: Bracket) -> Result<Token> {
        self.skip_trivia();
        let (token, end) = self.scan(self.position, bracket)?;
        self.position = end;
        Ok(token)
    }

    /// Scan one token starting at `start`, returning it and where it ends.
    #[allow(clippy::too_many_lines)]
    fn scan(&self, start: usize, bracket: Bracket) -> Result<(Token, usize)> {
        let rest = &self.text[start..];
        let Some(character) = rest.chars().next() else {
            return Ok((Token::End, start));
        };
        // Every encapsulator is consumed whole, including its doubled closer,
        // so nothing inside one is ever judged by the grammar.
        match character {
            '"' => return self.scan_encapsulated(start, '"', '"').map(wrap_quoted),
            '`' => return self.scan_encapsulated(start, '`', '`').map(wrap_quoted),
            '[' if bracket == Bracket::Identifier => {
                return self.scan_encapsulated(start, '[', ']').map(wrap_quoted);
            }
            '\'' => {
                let (text, end) = self.scan_encapsulated(start, '\'', '\'')?;
                return Ok((Token::Text(text), end));
            }
            _ => {}
        }
        if character.is_ascii_digit() {
            return Ok(self.scan_number(start));
        }
        if character.is_alphabetic() || character == '_' {
            let mut end = start;
            for (offset, character) in rest.char_indices() {
                if character.is_alphanumeric() || character == '_' {
                    end = start + offset + character.len_utf8();
                } else {
                    break;
                }
            }
            return Ok((Token::Word(SmolStr::new(&self.text[start..end])), end));
        }
        let two = rest.get(..2).unwrap_or("");
        let symbol = match two {
            "<>" | "!=" => Some((Symbol::NotEq, 2)),
            "<=" => Some((Symbol::LtEq, 2)),
            ">=" => Some((Symbol::GtEq, 2)),
            "::" => Some((Symbol::DoubleColon, 2)),
            _ => None,
        };
        let symbol = symbol.or_else(|| {
            Some(match character {
                '=' => (Symbol::Eq, 1),
                '<' => (Symbol::Lt, 1),
                '>' => (Symbol::Gt, 1),
                '+' => (Symbol::Plus, 1),
                '-' => (Symbol::Minus, 1),
                '*' => (Symbol::Star, 1),
                '/' => (Symbol::Slash, 1),
                '%' => (Symbol::Percent, 1),
                '(' => (Symbol::OpenParen, 1),
                ')' => (Symbol::CloseParen, 1),
                '[' => (Symbol::OpenBracket, 1),
                ']' => (Symbol::CloseBracket, 1),
                ',' => (Symbol::Comma, 1),
                '.' => (Symbol::Dot, 1),
                ':' => (Symbol::Colon, 1),
                ';' => (Symbol::Semicolon, 1),
                _ => return None,
            })
        });
        match symbol {
            Some((symbol, width)) => Ok((Token::Symbol(symbol), start + width)),
            None => Err(self.error_at(
                start,
                format_smolstr!("expected an operator, name, or literal, got {character:?}"),
            )),
        }
    }

    /// Consume a delimited token, doubling the closer to embed it.
    ///
    /// An unterminated delimiter reports the byte offset of the **opener**
    /// plus what would close it - the position a caller can actually fix -
    /// rather than the offset of end-of-input.
    fn scan_encapsulated(&self, start: usize, open: char, close: char) -> Result<(SmolStr, usize)> {
        let mut value = String::new();
        let mut cursor = start + open.len_utf8();
        loop {
            let rest = &self.text[cursor..];
            let Some(character) = rest.chars().next() else {
                return Err(self.error_at(
                    start,
                    format_smolstr!("expected a closing {close:?} for the {open:?} opened here"),
                ));
            };
            cursor += character.len_utf8();
            if character != close {
                value.push(character);
                continue;
            }
            // A doubled closer is the delimiter itself, which is each
            // dialect's own escape and what keeps the token self-delimiting.
            if self.text[cursor..].starts_with(close) {
                value.push(close);
                cursor += close.len_utf8();
                continue;
            }
            return Ok((SmolStr::new(value), cursor));
        }
    }

    /// Consume a numeric literal, keeping the digits exactly as written.
    fn scan_number(&self, start: usize) -> (Token, usize) {
        let bytes = self.text.as_bytes();
        let mut cursor = start;
        while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
            cursor += 1;
        }
        if cursor < bytes.len() && bytes[cursor] == b'.' {
            let after = cursor + 1;
            if after < bytes.len() && bytes[after].is_ascii_digit() {
                cursor = after;
                while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                    cursor += 1;
                }
            }
        }
        if cursor < bytes.len() && (bytes[cursor] | 0x20) == b'e' {
            let mut after = cursor + 1;
            if after < bytes.len() && (bytes[after] == b'+' || bytes[after] == b'-') {
                after += 1;
            }
            if after < bytes.len() && bytes[after].is_ascii_digit() {
                cursor = after;
                while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                    cursor += 1;
                }
            }
        }
        (
            Token::Number(SmolStr::new(&self.text[start..cursor])),
            cursor,
        )
    }

    /// Enter one level of recursion, refusing to pass the shared limit.
    fn descend(&mut self) -> Result<()> {
        self.depth += 1;
        if self.depth > RECURSION_LIMIT {
            return Err(self.error_at(
                self.position,
                format_smolstr!(
                    "expected nesting within the hard limit of {RECURSION_LIMIT}, got deeper"
                ),
            ));
        }
        Ok(())
    }

    /// Consume `symbol`, or leave the position alone and answer false.
    fn eat_symbol(&mut self, symbol: Symbol) -> Result<bool> {
        let saved = self.position;
        match self.next(Bracket::Subscript)? {
            Token::Symbol(found) if found == symbol => Ok(true),
            _ => {
                self.position = saved;
                Ok(false)
            }
        }
    }

    /// Consume `keyword`, or leave the position alone and answer false.
    fn eat_keyword(&mut self, keyword: &str) -> Result<bool> {
        let saved = self.position;
        match self.next(Bracket::Identifier)? {
            Token::Word(word) if word.eq_ignore_ascii_case(keyword) => Ok(true),
            _ => {
                self.position = saved;
                Ok(false)
            }
        }
    }

    /// Consume `keyword` or fail naming what was found instead.
    fn expect_keyword(&mut self, keyword: &str) -> Result<()> {
        let at = {
            self.skip_trivia();
            self.position
        };
        if self.eat_keyword(keyword)? {
            return Ok(());
        }
        let (found, _) = self.peek(Bracket::Identifier)?;
        Err(self.error_at(at, crate::text::expected_got(keyword, describe(&found))))
    }

    /// Consume `symbol` or fail naming what was found instead.
    fn expect_symbol(&mut self, symbol: Symbol) -> Result<()> {
        let at = {
            self.skip_trivia();
            self.position
        };
        if self.eat_symbol(symbol)? {
            return Ok(());
        }
        let (found, _) = self.peek(Bracket::Subscript)?;
        Err(self.error_at(
            at,
            crate::text::expected_got(format_smolstr!("{:?}", symbol.as_str()), describe(&found)),
        ))
    }

    /// `expression [AS name]` - the one place an alias may appear.
    fn parse_alias(&mut self) -> Result<Expr> {
        let expression = self.parse_or()?;
        if self.eat_keyword("AS")? {
            let name = self.parse_identifier()?;
            return Ok(expression.alias(name));
        }
        Ok(expression)
    }

    /// `or := and (OR and)*`
    fn parse_or(&mut self) -> Result<Expr> {
        self.descend()?;
        let mut operands = vec![self.parse_and()?];
        while self.eat_keyword("OR")? {
            operands.push(self.parse_and()?);
        }
        self.depth -= 1;
        Ok(Expr::any(operands))
    }

    /// `and := not (AND not)*`
    fn parse_and(&mut self) -> Result<Expr> {
        self.descend()?;
        let mut operands = vec![self.parse_not()?];
        while self.eat_keyword("AND")? {
            operands.push(self.parse_not()?);
        }
        self.depth -= 1;
        Ok(Expr::all(operands))
    }

    /// `not := NOT not | predicate`
    fn parse_not(&mut self) -> Result<Expr> {
        if self.eat_keyword("NOT")? {
            self.descend()?;
            let inner = self.parse_not()?;
            self.depth -= 1;
            return Ok(inner.not());
        }
        self.parse_predicate()
    }

    /// `predicate := additive [comparison | IS | IN | BETWEEN | LIKE]`
    fn parse_predicate(&mut self) -> Result<Expr> {
        self.descend()?;
        let left = self.parse_additive()?;
        let predicate = self.parse_predicate_tail(left)?;
        self.depth -= 1;
        Ok(predicate)
    }

    /// The tail of a predicate, which is what turns a value into a question.
    fn parse_predicate_tail(&mut self, left: Expr) -> Result<Expr> {
        let saved = self.position;
        if let Token::Symbol(symbol) = self.peek(Bracket::Subscript)?.0 {
            let op = match symbol {
                Symbol::Eq => Some(CompareOp::Eq),
                Symbol::NotEq => Some(CompareOp::NotEq),
                Symbol::Lt => Some(CompareOp::Lt),
                Symbol::LtEq => Some(CompareOp::LtEq),
                Symbol::Gt => Some(CompareOp::Gt),
                Symbol::GtEq => Some(CompareOp::GtEq),
                _ => None,
            };
            if let Some(op) = op {
                self.next(Bracket::Subscript)?;
                let right = self.parse_additive()?;
                return Ok(left.compare(op, right));
            }
        }
        if self.eat_keyword("IS")? {
            let negated = self.eat_keyword("NOT")?;
            self.expect_keyword("NULL")?;
            return Ok(if negated {
                left.is_not_null()
            } else {
                left.is_null()
            });
        }
        let negated = self.eat_keyword("NOT")?;
        if self.eat_keyword("IN")? {
            let list = self.parse_list()?;
            return Ok(if negated {
                left.is_not_in(list)
            } else {
                left.is_in(list)
            });
        }
        if self.eat_keyword("BETWEEN")? {
            let low = self.parse_additive()?;
            self.expect_keyword("AND")?;
            let high = self.parse_additive()?;
            return Ok(Expr::Between {
                expr: Arc::new(left),
                low: Arc::new(low),
                high: Arc::new(high),
                negated,
            });
        }
        let case_insensitive = {
            let saved_like = self.position;
            if self.eat_keyword("ILIKE")? {
                true
            } else if self.eat_keyword("LIKE")? {
                false
            } else {
                self.position = saved_like;
                if negated {
                    // `NOT` was consumed for a tail that never arrived, so it
                    // belongs to the enclosing `NOT` production instead.
                    self.position = saved;
                }
                return Ok(left);
            }
        };
        let pattern = self.parse_additive()?;
        let escape = if self.eat_keyword("ESCAPE")? {
            Some(self.parse_escape_character()?)
        } else {
            None
        };
        Ok(Expr::Like {
            expr: Arc::new(left),
            pattern: Arc::new(pattern),
            escape,
            negated,
            case_insensitive,
        })
    }

    /// The single character an `ESCAPE` clause names.
    fn parse_escape_character(&mut self) -> Result<char> {
        let at = {
            self.skip_trivia();
            self.position
        };
        let Token::Text(text) = self.next(Bracket::Subscript)? else {
            return Err(self.error_at(
                at,
                SmolStr::new_static("expected a one-character escape string"),
            ));
        };
        let mut characters = text.chars();
        match (characters.next(), characters.next()) {
            (Some(character), None) => Ok(character),
            _ => Err(self.error_at(
                at,
                crate::text::expected_got(
                    "a one-character escape string",
                    format_smolstr!("{text:?}"),
                ),
            )),
        }
    }

    /// `( expression, ... )` - the operand list of `IN`.
    fn parse_list(&mut self) -> Result<Vec<Expr>> {
        self.expect_symbol(Symbol::OpenParen)?;
        let mut items = Vec::new();
        if self.eat_symbol(Symbol::CloseParen)? {
            let at = self.position;
            return Err(self.error_at(
                at,
                SmolStr::new_static("expected at least one value in an IN list, got ()"),
            ));
        }
        loop {
            items.push(self.parse_or()?);
            if self.eat_symbol(Symbol::Comma)? {
                continue;
            }
            self.expect_symbol(Symbol::CloseParen)?;
            return Ok(items);
        }
    }

    /// `additive := multiplicative (('+' | '-') multiplicative)*`
    fn parse_additive(&mut self) -> Result<Expr> {
        self.descend()?;
        let mut left = self.parse_multiplicative()?;
        loop {
            let op = if self.eat_symbol(Symbol::Plus)? {
                ArithOp::Add
            } else if self.eat_symbol(Symbol::Minus)? {
                ArithOp::Sub
            } else {
                self.depth -= 1;
                return Ok(left);
            };
            let right = self.parse_multiplicative()?;
            left = left.arithmetic(op, right);
        }
    }

    /// `multiplicative := unary (('*' | '/' | '%') unary)*`
    fn parse_multiplicative(&mut self) -> Result<Expr> {
        self.descend()?;
        let mut left = self.parse_unary()?;
        loop {
            let op = if self.eat_symbol(Symbol::Star)? {
                ArithOp::Mul
            } else if self.eat_symbol(Symbol::Slash)? {
                ArithOp::Div
            } else if self.eat_symbol(Symbol::Percent)? {
                ArithOp::Mod
            } else {
                self.depth -= 1;
                return Ok(left);
            };
            let right = self.parse_unary()?;
            left = left.arithmetic(op, right);
        }
    }

    /// `unary := '-' unary | '+' unary | postfix`
    fn parse_unary(&mut self) -> Result<Expr> {
        if self.eat_symbol(Symbol::Minus)? {
            self.descend()?;
            let inner = self.parse_unary()?;
            self.depth -= 1;
            // A negated numeric literal is one literal, not a node over one:
            // it is what the caller wrote and what a bound plan should fold.
            return Ok(match inner {
                Expr::Literal(value) => match negate(&value) {
                    Some(negated) => Expr::Literal(negated),
                    None => Expr::Neg(Arc::new(Expr::Literal(value))),
                },
                other => Expr::Neg(Arc::new(other)),
            });
        }
        if self.eat_symbol(Symbol::Plus)? {
            return self.parse_unary();
        }
        self.parse_postfix()
    }

    /// `postfix := primary (accessor | '::' datatype)*`
    ///
    /// Accessors bind tighter than `::` by construction: the loop is
    /// left-associative, so `a.b[0]::int` casts the element rather than the
    /// column.
    fn parse_postfix(&mut self) -> Result<Expr> {
        let mut expression = self.parse_primary()?;
        loop {
            if self.eat_symbol(Symbol::DoubleColon)? {
                let data_type = self.parse_data_type_run()?;
                expression = expression.cast_to(data_type);
                continue;
            }
            let saved = self.position;
            if self.eat_symbol(Symbol::Dot)? {
                let name = self.parse_identifier()?;
                expression = self.push_accessor(expression, Accessor::Child(name), saved)?;
                continue;
            }
            if self.eat_symbol(Symbol::OpenBracket)? {
                let accessor = self.parse_subscript()?;
                expression = self.push_accessor(expression, accessor, saved)?;
                continue;
            }
            return Ok(expression);
        }
    }

    /// Attach one accessor, refusing a receiver that is not a column path.
    ///
    /// Reaching inside `lower(x)` would mean materializing a value to index it,
    /// and this grammar deliberately has no way to spell that, so the refusal
    /// is named at the byte where the accessor started.
    fn push_accessor(&self, expression: Expr, accessor: Accessor, at: usize) -> Result<Expr> {
        if expression.as_column().is_none() {
            return Err(self.error_at(
                at,
                SmolStr::new_static(
                    "expected a column before an accessor; only a column path may be reached into",
                ),
            ));
        }
        Ok(expression.accessor(accessor))
    }

    /// The contents of a `[...]` in subscript position.
    ///
    /// Only literals and ranges live here, never a bare name: a bare
    /// identifier inside a subscript is ambiguous between a key and a column,
    /// so it is refused naming both readings rather than guessed.
    fn parse_subscript(&mut self) -> Result<Accessor> {
        // `[:` and `[:3]` - a range whose lower bound is the start.
        if self.eat_symbol(Symbol::Colon)? {
            let end = self.parse_optional_bound()?;
            self.expect_symbol(Symbol::CloseBracket)?;
            return Ok(Accessor::Range { start: None, end });
        }
        let at = {
            self.skip_trivia();
            self.position
        };
        let token = self.next(Bracket::Subscript)?;
        let first = match token {
            Token::Number(text) => {
                let Ok(index) = text.parse::<i64>() else {
                    return Err(self.error_at(
                        at,
                        crate::text::expected_got(
                            "a whole-number index",
                            format_smolstr!("{text:?}"),
                        ),
                    ));
                };
                Value::I64(index)
            }
            Token::Symbol(Symbol::Minus) => {
                let negative_at = self.position;
                let Token::Number(text) = self.next(Bracket::Subscript)? else {
                    return Err(self.error_at(
                        negative_at,
                        SmolStr::new_static("expected a whole-number index after '-'"),
                    ));
                };
                let Ok(index) = text.parse::<i64>() else {
                    return Err(self.error_at(
                        negative_at,
                        crate::text::expected_got(
                            "a whole-number index",
                            format_smolstr!("{text:?}"),
                        ),
                    ));
                };
                Value::I64(-index)
            }
            Token::Text(text) => Value::String(text),
            // Inside a subscript there is no column position, so an
            // encapsulated token here is a string key rather than a name.
            Token::Quoted(text) => Value::String(text),
            Token::Word(word) => {
                return Err(self.error_at(
                    at,
                    format_smolstr!(
                        "expected a literal key, an index, or a range inside [], got the bare name {word:?}; write ['{word}'] for a key or [\"{word}\"] for one containing punctuation"
                    ),
                ));
            }
            other => {
                return Err(self.error_at(
                    at,
                    crate::text::expected_got(
                        "a literal key, an index, or a range",
                        describe(&other),
                    ),
                ));
            }
        };
        if self.eat_symbol(Symbol::Colon)? {
            let Value::I64(start) = first else {
                return Err(self.error_at(
                    at,
                    SmolStr::new_static("expected a whole-number lower bound before ':'"),
                ));
            };
            let end = self.parse_optional_bound()?;
            self.expect_symbol(Symbol::CloseBracket)?;
            return Ok(Accessor::Range {
                start: Some(start),
                end,
            });
        }
        self.expect_symbol(Symbol::CloseBracket)?;
        Ok(match first {
            Value::I64(index) => Accessor::Index(index),
            key => Accessor::Key(key),
        })
    }

    /// The upper half of a range, which may be absent.
    fn parse_optional_bound(&mut self) -> Result<Option<i64>> {
        let saved = self.position;
        let negative = self.eat_symbol(Symbol::Minus)?;
        let at = {
            self.skip_trivia();
            self.position
        };
        match self.next(Bracket::Subscript)? {
            Token::Number(text) => {
                let Ok(bound) = text.parse::<i64>() else {
                    return Err(self.error_at(
                        at,
                        crate::text::expected_got(
                            "a whole-number bound",
                            format_smolstr!("{text:?}"),
                        ),
                    ));
                };
                Ok(Some(if negative { -bound } else { bound }))
            }
            _ if !negative => {
                self.position = saved;
                Ok(None)
            }
            other => Err(self.error_at(
                at,
                crate::text::expected_got("a whole-number bound", describe(&other)),
            )),
        }
    }

    /// One identifier, bare or encapsulated.
    fn parse_identifier(&mut self) -> Result<SmolStr> {
        let at = {
            self.skip_trivia();
            self.position
        };
        match self.next(Bracket::Identifier)? {
            Token::Quoted(name) => Ok(name),
            Token::Word(word) if !is_reserved_word(&word) => Ok(word),
            Token::Word(word) => Err(self.error_at(
                at,
                format_smolstr!(
                    "expected a name, got the keyword {word:?}; write \"{word}\" to name a column"
                ),
            )),
            other => Err(self.error_at(at, crate::text::expected_got("a name", describe(&other)))),
        }
    }

    /// A primary: a literal, a parenthesized expression, a call, or a column.
    #[allow(clippy::too_many_lines)]
    fn parse_primary(&mut self) -> Result<Expr> {
        let at = {
            self.skip_trivia();
            self.position
        };
        let token = self.next(Bracket::Identifier)?;
        match token {
            Token::End => Err(self.error_at(
                at,
                SmolStr::new_static("expected a value, got the end of the expression"),
            )),
            Token::Number(text) => self.number_literal(&text, at),
            Token::Text(text) => Ok(Expr::Literal(Value::String(text))),
            Token::Quoted(name) => Ok(Expr::column(name)),
            Token::Symbol(Symbol::OpenParen) => {
                self.descend()?;
                let inner = self.parse_or()?;
                self.depth -= 1;
                self.expect_symbol(Symbol::CloseParen)?;
                Ok(inner)
            }
            Token::Symbol(symbol) => Err(self.error_at(
                at,
                crate::text::expected_got("a value", format_smolstr!("{:?}", symbol.as_str())),
            )),
            Token::Word(word) => self.word_primary(&word, at),
        }
    }

    /// A primary that started with a bare word.
    fn word_primary(&mut self, word: &str, at: usize) -> Result<Expr> {
        if word.eq_ignore_ascii_case("TRUE") {
            return Ok(Expr::Literal(Value::Bool(true)));
        }
        if word.eq_ignore_ascii_case("FALSE") {
            return Ok(Expr::Literal(Value::Bool(false)));
        }
        if word.eq_ignore_ascii_case("NULL") {
            return Ok(Expr::Literal(Value::Null));
        }
        if word.eq_ignore_ascii_case("NAN") {
            return Ok(Expr::Literal(Value::from(f64::NAN)));
        }
        if word.eq_ignore_ascii_case("INFINITY") {
            return Ok(Expr::Literal(Value::from(f64::INFINITY)));
        }
        if word.eq_ignore_ascii_case("CASE") {
            return self.parse_case();
        }
        // `EXTRACT(YEAR FROM x)` is ANSI's spelling of the calendar functions,
        // and the field it names is a keyword rather than an argument.
        if word.eq_ignore_ascii_case("EXTRACT") {
            let saved = self.position;
            if self.eat_symbol(Symbol::OpenParen)? {
                return self.parse_extract(at);
            }
            self.position = saved;
        }
        if word.eq_ignore_ascii_case("CAST") || word.eq_ignore_ascii_case("TRY_CAST") {
            return self.parse_cast(word.eq_ignore_ascii_case("TRY_CAST"));
        }
        // A typed literal is `DATE '...'` and friends: the keyword introduces
        // one only when a string follows, so a column named `date` reads bare.
        if let Some(temporal) = self.parse_typed_literal(word)? {
            return Ok(temporal);
        }
        if is_reserved_word(word) {
            return Err(self.error_at(
                at,
                format_smolstr!(
                    "expected a value, got the keyword {word:?}; write \"{word}\" to name a column"
                ),
            ));
        }
        // A call is a name immediately followed by `(`; anything else is a
        // column, which is what keeps `year` usable as both.
        let saved = self.position;
        if self.eat_symbol(Symbol::OpenParen)? {
            return self.parse_call(word, at, saved);
        }
        Ok(Expr::column(word))
    }

    /// `EXTRACT(<field> FROM expr)`, whose `(` has already been consumed.
    fn parse_extract(&mut self, at: usize) -> Result<Expr> {
        let field_at = {
            self.skip_trivia();
            self.position
        };
        let Token::Word(field) = self.next(Bracket::Identifier)? else {
            return Err(self.error_at(
                field_at,
                SmolStr::new_static("expected a calendar field name after EXTRACT("),
            ));
        };
        let Some(function) = Function::from_name(&field).filter(|function| function.is_calendar())
        else {
            return Err(self.error_at(
                field_at,
                crate::text::expected_got(
                    "one of YEAR, MONTH, DAY, HOUR, MINUTE, SECOND",
                    format_smolstr!("{field:?}"),
                ),
            ));
        };
        self.expect_keyword("FROM")?;
        let inner = self.parse_or()?;
        self.expect_symbol(Symbol::CloseParen)?;
        let _ = at;
        Ok(Expr::call(function, [inner]))
    }

    /// `DATE '...'`, `TIME '...'`, `TIMESTAMP '...'`, `INTERVAL '...'`, `X'..'`.
    fn parse_typed_literal(&mut self, word: &str) -> Result<Option<Expr>> {
        let saved = self.position;
        let at = {
            self.skip_trivia();
            self.position
        };
        let is_typed = ["DATE", "TIME", "TIMESTAMP", "DATETIME", "INTERVAL", "X"]
            .iter()
            .any(|keyword| keyword.eq_ignore_ascii_case(word));
        if !is_typed {
            return Ok(None);
        }
        let Token::Text(text) = self.next(Bracket::Subscript)? else {
            self.position = saved;
            return Ok(None);
        };
        let value = if word.eq_ignore_ascii_case("X") {
            parse_hex(&text).ok_or_else(|| {
                self.error_at(
                    at,
                    crate::text::expected_got(
                        "an even run of hex digits",
                        format_smolstr!("{text:?}"),
                    ),
                )
            })?
        } else {
            temporal_literal(word, &text).map_err(|reason| self.error_at(at, reason))?
        };
        Ok(Some(Expr::Literal(value)))
    }

    /// `CAST(expr AS <datatype>)`, with the type half read by the type grammar.
    fn parse_cast(&mut self, safe: bool) -> Result<Expr> {
        self.expect_symbol(Symbol::OpenParen)?;
        self.descend()?;
        let inner = self.parse_or()?;
        self.depth -= 1;
        self.expect_keyword("AS")?;
        let data_type = self.parse_data_type_until_close()?;
        self.expect_symbol(Symbol::CloseParen)?;
        Ok(Expr::Cast {
            expr: Arc::new(inner),
            data_type,
            safe,
        })
    }

    /// `CASE WHEN c THEN v [...] [ELSE v] END`.
    fn parse_case(&mut self) -> Result<Expr> {
        self.descend()?;
        let mut branches = Vec::new();
        while self.eat_keyword("WHEN")? {
            let when = self.parse_or()?;
            self.expect_keyword("THEN")?;
            let then = self.parse_or()?;
            branches.push((when, then));
        }
        if branches.is_empty() {
            let at = self.position;
            self.depth -= 1;
            return Err(self.error_at(
                at,
                SmolStr::new_static("expected at least one WHEN branch in a CASE expression"),
            ));
        }
        let otherwise = if self.eat_keyword("ELSE")? {
            Some(self.parse_or()?)
        } else {
            None
        };
        self.expect_keyword("END")?;
        self.depth -= 1;
        Ok(Expr::case(branches, otherwise))
    }

    /// A call whose `(` has already been consumed.
    fn parse_call(&mut self, name: &str, at: usize, open: usize) -> Result<Expr> {
        let Some(function) = Function::from_name(name) else {
            let vocabulary = Function::ALL
                .iter()
                .map(|function| function.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(self.error_at(
                at,
                format_smolstr!(
                    "expected one of the functions this grammar spells ({vocabulary}), got {name:?}"
                ),
            ));
        };
        // `EXTRACT(YEAR FROM x)` is the other spelling of the calendar
        // functions, and it is the one ANSI SQL standardizes.
        if function.is_calendar() {
            let saved = self.position;
            if self.eat_keyword("FROM")? {
                let inner = self.parse_or()?;
                self.expect_symbol(Symbol::CloseParen)?;
                return Ok(Expr::call(function, [inner]));
            }
            self.position = saved;
        }
        let mut args = Vec::new();
        if !self.eat_symbol(Symbol::CloseParen)? {
            loop {
                args.push(self.parse_or()?);
                if self.eat_symbol(Symbol::Comma)? {
                    continue;
                }
                self.expect_symbol(Symbol::CloseParen)?;
                break;
            }
        }
        let (least, most) = function.arity();
        if args.len() < least || args.len() > most {
            let wanted = if most == usize::MAX {
                format_smolstr!("at least {least} argument(s)")
            } else if least == most {
                format_smolstr!("{least} argument(s)")
            } else {
                format_smolstr!("{least} to {most} arguments")
            };
            return Err(self.error_at(
                open,
                crate::text::expected_got(format_smolstr!("{wanted} for {function}"), args.len()),
            ));
        }
        Ok(Expr::call(function, args))
    }

    /// Read a numeric literal into the value its spelling names.
    ///
    /// A fractional literal is an exact [`Value::Decimal`] keeping the scale as
    /// written - never an `f64`, because `0.1` has no binary expansion and a
    /// price that arrives as `0.1` must leave as `0.1`. A float is spelled with
    /// an exponent, which is also how one is written back out.
    fn number_literal(&self, text: &str, at: usize) -> Result<Expr> {
        let value = numeric_value(text).ok_or_else(|| {
            self.error_at(
                at,
                crate::text::expected_got(
                    "a number this grammar can hold",
                    format_smolstr!("{text:?}"),
                ),
            )
        })?;
        Ok(Expr::Literal(value))
    }

    /// Read a datatype spelled up to the `)` that closes a `CAST`.
    fn parse_data_type_until_close(&mut self) -> Result<DataType> {
        self.skip_trivia();
        let start = self.position;
        let end = self.scan_balanced(start, true)?;
        self.parse_data_type_text(start, end)
    }

    /// Read a datatype spelled after `::`, which ends at its own last token.
    fn parse_data_type_run(&mut self) -> Result<DataType> {
        self.skip_trivia();
        let start = self.position;
        let end = self.scan_balanced(start, false)?;
        self.parse_data_type_text(start, end)
    }

    /// Hand a slice of the expression text to the one type grammar.
    ///
    /// The byte position of a failure is reported relative to the whole
    /// expression, never to the type fragment, because that is the offset the
    /// caller can point at in what they wrote.
    fn parse_data_type_text(&mut self, start: usize, end: usize) -> Result<DataType> {
        let text = self.text[start..end].trim();
        if text.is_empty() {
            return Err(self.error_at(start, SmolStr::new_static("expected a datatype")));
        }
        let data_type = text.parse::<DataType>().map_err(|error| match error {
            Error::Parse {
                position, reason, ..
            } => self.error_at(start + position, reason),
            other => self.error_at(start, format_smolstr!("{other}")),
        })?;
        self.position = end;
        Ok(data_type)
    }

    /// Find where a datatype spelling ends.
    ///
    /// `until_close` reads to the `)` that closes the enclosing `CAST`, which
    /// is what lets a nested type carry its own parentheses; otherwise the
    /// spelling is one name plus at most one balanced parameter group, which
    /// is exactly what `::` can carry without swallowing the operator after it.
    fn scan_balanced(&self, start: usize, until_close: bool) -> Result<usize> {
        let bytes = self.text.as_bytes();
        let mut cursor = start;
        let mut depth = 0_usize;
        let mut quote: Option<u8> = None;
        while cursor < bytes.len() {
            let byte = bytes[cursor];
            if let Some(open) = quote {
                if byte == open {
                    quote = None;
                }
                cursor += 1;
                continue;
            }
            match byte {
                b'\'' | b'"' => quote = Some(byte),
                b'(' | b'<' | b'[' | b'{' => depth += 1,
                b')' | b'>' | b']' | b'}' => {
                    if depth == 0 {
                        if until_close && byte == b')' {
                            return Ok(cursor);
                        }
                        return Ok(cursor);
                    }
                    depth -= 1;
                    if depth == 0 && !until_close {
                        return Ok(cursor + 1);
                    }
                }
                b',' if depth == 0 && !until_close => return Ok(cursor),
                b' ' | b'\t' | b'\n' | b'\r' if depth == 0 && !until_close => {
                    // A datatype after `::` is one word plus at most one
                    // parameter group, so a space at depth zero ends it unless
                    // the group has not opened yet.
                    let rest = self.text[cursor..].trim_start();
                    if rest.starts_with('(') || rest.starts_with('<') {
                        cursor += 1;
                        continue;
                    }
                    return Ok(cursor);
                }
                _ if depth == 0
                    && !until_close
                    && !(byte.is_ascii_alphanumeric() || byte == b'_') =>
                {
                    return Ok(cursor);
                }
                _ => {}
            }
            cursor += 1;
        }
        if until_close {
            return Err(self.error_at(
                start,
                SmolStr::new_static("expected a closing ')' for the CAST opened before it"),
            ));
        }
        Ok(cursor)
    }
}

/// Wrap an encapsulated token as the identifier it always is.
fn wrap_quoted(scanned: (SmolStr, usize)) -> (Token, usize) {
    (Token::Quoted(scanned.0), scanned.1)
}

/// Describe a token for the `expected X, got Y` half of a message.
fn describe(token: &Token) -> SmolStr {
    match token {
        Token::End => SmolStr::new_static("the end of the expression"),
        Token::Word(word) | Token::Quoted(word) => format_smolstr!("{word:?}"),
        Token::Text(text) => format_smolstr!("the string {text:?}"),
        Token::Number(text) => format_smolstr!("the number {text}"),
        Token::Symbol(symbol) => format_smolstr!("{:?}", symbol.as_str()),
    }
}

/// Read a numeric literal into the value its spelling names.
fn numeric_value(text: &str) -> Option<Value> {
    if text.contains(['e', 'E']) {
        return text.parse::<f64>().ok().map(Value::from);
    }
    if let Some((whole, fraction)) = text.split_once('.') {
        let digits = fraction.len();
        let scale = i8::try_from(digits).ok()?;
        let mut unscaled = whole.parse::<i128>().ok()?;
        unscaled = unscaled.checked_mul(10_i128.checked_pow(u32::try_from(digits).ok()?)?)?;
        unscaled = unscaled.checked_add(fraction.parse::<i128>().ok()?)?;
        return Some(Value::Decimal(unscaled, scale));
    }
    if let Ok(value) = text.parse::<i64>() {
        return Some(Value::I64(value));
    }
    if let Ok(value) = text.parse::<i128>() {
        return Some(Value::I128(value));
    }
    text.parse::<u128>().ok().map(Value::U128)
}

/// Negate a numeric literal in place, so `-1` is one value rather than a node.
fn negate(value: &Value) -> Option<Value> {
    Some(match value {
        Value::I8(inner) => Value::I8(inner.checked_neg()?),
        Value::I16(inner) => Value::I16(inner.checked_neg()?),
        Value::I32(inner) => Value::I32(inner.checked_neg()?),
        Value::I64(inner) => Value::I64(inner.checked_neg()?),
        Value::I128(inner) => Value::I128(inner.checked_neg()?),
        Value::Decimal(unscaled, scale) => Value::Decimal(unscaled.checked_neg()?, *scale),
        Value::F32(inner) => Value::from(-inner.as_f32()),
        Value::F64(inner) => Value::from(-inner.as_f64()),
        _ => return None,
    })
}

/// Read `X'..'` into the bytes it spells.
fn parse_hex(text: &str) -> Option<Value> {
    if text.len() % 2 != 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(text.len() / 2);
    let raw = text.as_bytes();
    for pair in raw.chunks_exact(2) {
        let text = std::str::from_utf8(pair).ok()?;
        bytes.push(u8::from_str_radix(text, 16).ok()?);
    }
    Some(Value::Bytes(Arc::from(bytes)))
}

/// Read a typed temporal literal through the one ISO reader.
fn temporal_literal(keyword: &str, text: &str) -> std::result::Result<Value, SmolStr> {
    let described = |what: &str| format_smolstr!("expected an ISO-8601 {what}, got {text:?}");
    if keyword.eq_ignore_ascii_case("DATE") {
        return crate::generic::iso::parse_date(text)
            .map(Value::Date)
            .map_err(|_| described("date"));
    }
    if keyword.eq_ignore_ascii_case("TIME") {
        return crate::generic::iso::parse_time(text)
            .map(|(count, unit)| Value::Time(count, unit))
            .map_err(|_| described("time"));
    }
    if keyword.eq_ignore_ascii_case("INTERVAL") {
        return crate::generic::iso::parse_duration(text)
            .map(|(count, unit)| Value::Duration(count, unit))
            .map_err(|_| described("duration"));
    }
    // A zoned reading and a naive one are different kinds, not two displays of
    // one, so the text decides which: an offset or a `Z` makes an instant.
    if let Ok((count, unit, zone)) = crate::generic::iso::parse_timestamp(text) {
        return Ok(Value::Timestamp(count, unit, zone));
    }
    crate::generic::iso::parse_datetime(text)
        .map(|(count, unit)| Value::DateTime(count, unit))
        .map_err(|_| described("timestamp"))
}

/// Parse one statement, or a `;`-separated chain of them.
///
/// A chain is itself a statement, so a chain of chains is a chain - which is
/// what makes "compose freely" cost nothing to say and nothing at run time.
///
/// # Errors
///
/// Returns [`Error::Parse`] with the byte offset of the failure.
pub(super) fn parse_statement(text: &str) -> Result<Statement> {
    let mut parser = Parser {
        text,
        position: 0,
        depth: 0,
    };
    let mut steps = Vec::new();
    loop {
        steps.push(parser.parse_one_statement()?);
        // A trailing `;` ends the last statement rather than opening another.
        let separated = parser.eat_symbol(Symbol::Semicolon)?;
        parser.skip_trivia();
        if !separated && parser.position < text.len() {
            return Err(parser.error_at(
                parser.position,
                format_smolstr!(
                    "expected ';' between statements, got {:?}",
                    crate::text::elide_to(&text[parser.position..], 32)
                ),
            ));
        }
        if parser.position >= text.len() {
            return Ok(Statement::chain(steps));
        }
        if !matches!(
            parser.peek(Bracket::Identifier)?.0,
            Token::Word(_) | Token::End
        ) {
            return Err(parser.error_at(
                parser.position,
                format_smolstr!(
                    "expected ';' or the end of the statement, got {:?}",
                    crate::text::elide_to(&text[parser.position..], 32)
                ),
            ));
        }
    }
}

impl Parser<'_> {
    /// One statement, without its chain separator.
    fn parse_one_statement(&mut self) -> Result<Statement> {
        let at = {
            self.skip_trivia();
            self.position
        };
        let Token::Word(verb) = self.next(Bracket::Identifier)? else {
            return Err(self.error_at(
                at,
                SmolStr::new_static(
                    "expected a statement verb: SELECT, INSERT, UPDATE, DELETE, or ALTER",
                ),
            ));
        };
        if verb.eq_ignore_ascii_case("SELECT") {
            return self.parse_select();
        }
        if verb.eq_ignore_ascii_case("DELETE") {
            return self.parse_delete();
        }
        if verb.eq_ignore_ascii_case("UPDATE") {
            return self.parse_update();
        }
        if verb.eq_ignore_ascii_case("INSERT") {
            return self.parse_insert();
        }
        if verb.eq_ignore_ascii_case("ALTER") {
            return self.parse_alter();
        }
        Err(self.error_at(
            at,
            crate::text::expected_got(
                "one of SELECT, INSERT, UPDATE, DELETE, ALTER",
                format_smolstr!("{verb:?}"),
            ),
        ))
    }

    /// The target a statement names, which the handle already answered.
    ///
    /// A handle *is* the `FROM` clause, so the target is accepted and checked
    /// for shape rather than resolved: there is no catalog here to resolve it
    /// against, and pretending otherwise would be a second addressing model.
    fn parse_target(&mut self) -> Result<Option<SmolStr>> {
        let saved = self.position;
        if self.eat_symbol(Symbol::Dot)? {
            return Ok(None);
        }
        match self.next(Bracket::Identifier)? {
            Token::Quoted(name) => Ok(Some(name)),
            Token::Word(word) if !is_reserved_word(&word) => Ok(Some(word)),
            _ => {
                self.position = saved;
                Ok(None)
            }
        }
    }

    /// `SELECT <projection> [FROM .] [WHERE <expr>]`
    fn parse_select(&mut self) -> Result<Statement> {
        let selection = if self.eat_symbol(Symbol::Star)? {
            super::Selection::everything()
        } else {
            let mut items = vec![self.parse_alias()?];
            while self.eat_symbol(Symbol::Comma)? {
                items.push(self.parse_alias()?);
            }
            super::Selection::from_exprs(items)
        };
        if self.eat_keyword("FROM")? {
            self.parse_target()?;
        }
        let filter = self.parse_where()?;
        Ok(Statement::Select { selection, filter })
    }

    /// `DELETE [FROM .] [WHERE <expr>]`
    fn parse_delete(&mut self) -> Result<Statement> {
        if self.eat_keyword("FROM")? {
            self.parse_target()?;
        }
        Ok(Statement::Delete {
            filter: self.parse_where()?,
        })
    }

    /// `UPDATE . SET c = e [, ...] [WHERE <expr>]`
    fn parse_update(&mut self) -> Result<Statement> {
        self.parse_target()?;
        self.expect_keyword("SET")?;
        let mut assignments = Vec::new();
        loop {
            let column = self.parse_identifier()?;
            self.expect_symbol(Symbol::Eq)?;
            assignments.push((column, self.parse_or()?));
            if !self.eat_symbol(Symbol::Comma)? {
                break;
            }
        }
        Ok(Statement::Update {
            assignments: Arc::from(assignments),
            filter: self.parse_where()?,
        })
    }

    /// `INSERT INTO . [(a, b)] VALUES (…), (…)`
    fn parse_insert(&mut self) -> Result<Statement> {
        self.expect_keyword("INTO")?;
        self.parse_target()?;
        let mut columns = Vec::new();
        // A parenthesized column list is optional; without one the values are
        // positional, which is what a row already is.
        let saved = self.position;
        if self.eat_symbol(Symbol::OpenParen)? {
            let mut named = Vec::new();
            loop {
                match self.parse_identifier() {
                    Ok(name) => named.push(name),
                    Err(_) => {
                        self.position = saved;
                        named.clear();
                        break;
                    }
                }
                if self.eat_symbol(Symbol::Comma)? {
                    continue;
                }
                if self.eat_symbol(Symbol::CloseParen)? {
                    columns = named;
                } else {
                    self.position = saved;
                }
                break;
            }
        }
        self.expect_keyword("VALUES")?;
        let mut rows = Vec::new();
        loop {
            self.expect_symbol(Symbol::OpenParen)?;
            let mut values = Vec::new();
            if !self.eat_symbol(Symbol::CloseParen)? {
                loop {
                    values.push(self.parse_or()?);
                    if self.eat_symbol(Symbol::Comma)? {
                        continue;
                    }
                    self.expect_symbol(Symbol::CloseParen)?;
                    break;
                }
            }
            rows.push(values);
            if !self.eat_symbol(Symbol::Comma)? {
                break;
            }
        }
        Ok(Statement::Insert {
            columns: Arc::from(columns),
            rows: rows.into_iter().map(Arc::from).collect(),
        })
    }

    /// `ALTER [TABLE .] ADD|DROP|RENAME|ALTER COLUMN …`
    fn parse_alter(&mut self) -> Result<Statement> {
        // `TABLE` is optional, and the target after it is the handle itself.
        self.eat_keyword("TABLE")?;
        self.parse_target()?;
        let at = {
            self.skip_trivia();
            self.position
        };
        let Token::Word(action) = self.next(Bracket::Identifier)? else {
            return Err(self.error_at(
                at,
                SmolStr::new_static("expected ADD, DROP, RENAME, or ALTER after ALTER TABLE"),
            ));
        };
        // MySQL's `CHANGE`/`MODIFY` and Postgres's `ALTER` all name the same
        // four actions; only the canonical spellings are emitted back out.
        if action.eq_ignore_ascii_case("ADD") {
            self.eat_keyword("COLUMN")?;
            let name = self.parse_identifier()?;
            let data_type = self.parse_data_type_run()?;
            let mut default = None;
            let mut computed = None;
            if self.eat_keyword("DEFAULT")? {
                default = Some(self.parse_or()?);
            }
            if self.eat_keyword("AS")? {
                computed = Some(self.parse_or()?);
            }
            return Ok(Statement::AddColumn {
                name,
                data_type,
                default,
                computed,
            });
        }
        if action.eq_ignore_ascii_case("DROP") {
            self.eat_keyword("COLUMN")?;
            return Ok(Statement::DropColumn {
                name: self.parse_identifier()?,
            });
        }
        if action.eq_ignore_ascii_case("RENAME") {
            self.eat_keyword("COLUMN")?;
            let from = self.parse_identifier()?;
            self.expect_keyword("TO")?;
            return Ok(Statement::RenameColumn {
                from,
                to: self.parse_identifier()?,
            });
        }
        if action.eq_ignore_ascii_case("ALTER")
            || action.eq_ignore_ascii_case("MODIFY")
            || action.eq_ignore_ascii_case("CHANGE")
        {
            self.eat_keyword("COLUMN")?;
            let name = self.parse_identifier()?;
            self.eat_keyword("TYPE")?;
            return Ok(Statement::AlterColumnType {
                name,
                data_type: self.parse_data_type_run()?,
            });
        }
        Err(self.error_at(
            at,
            crate::text::expected_got(
                "one of ADD, DROP, RENAME, ALTER",
                format_smolstr!("{action:?}"),
            ),
        ))
    }

    /// `[WHERE <expr>]`
    fn parse_where(&mut self) -> Result<Option<Expr>> {
        if self.eat_keyword("WHERE")? {
            return Ok(Some(self.parse_or()?));
        }
        Ok(None)
    }
}
