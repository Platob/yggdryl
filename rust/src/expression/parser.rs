//! One recursive grammar, re-entered by every nested construct.
//!
//! There is exactly one parser in this module and exactly one in the workspace
//! that reads a predicate. It is recursive descent with explicit precedence,
//! it re-enters itself for every operand - a `case` arm holds a full
//! expression, a list element holds a full expression, a cast target holds a
//! full datatype through the crate's own datatype grammar - and it refuses
//! past [`RECURSION_LIMIT`](super::RECURSION_LIMIT) with a typed error rather
//! than by overflowing a stack.
//!
//! # The shape
//!
//! ```text
//! statement  := "select" projections ["where" expr] ["order" "by" orders] ["limit" n]
//! expr       := disjunction
//! disjunction:= conjunction ("or" conjunction)*
//! conjunction:= negation ("and" negation)*
//! negation   := "not" negation | predicate
//! predicate  := additive [ comparison | "is" .. | "in" .. | "between" .. | "like" .. ]
//! additive   := product (("+" | "-") product)*
//! product    := unary (("*" | "/" | "%") unary)*
//! unary      := "-" unary | accessor
//! accessor   := atom ("." identifier | "[" key "]")*
//! atom       := literal | "(" expr ")" | column | "&holder." selector | ":" parameter
//!             | "cast" "(" expr "as" datatype ")" | "case" .. "end"
//!             | function "(" expr,* ")" | "[" expr,* "]" | "{" expr ":" expr,* "}"
//!             | "struct" "(" expr "as" identifier,* ")" | datatype text
//! ```
//!
//! # What is deliberately not here
//!
//! No subquery, no join, no aggregate, no window. Every one of those needs a
//! second relation, and this is a filter and projection tree over one. A
//! grammar that accepts them and then refuses them at bind time has told the
//! caller a lie at the point where the error message was still cheap.

use std::str::FromStr;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use super::display::{is_bare_identifier, is_reserved};
use super::selector::Selector;
use super::{Comparison, Expression, Function, Operator, RECURSION_LIMIT, Safety, Segment};
use crate::{
    DataType, Error, Float16, Float32, Float64, I256, Result, TimeUnit, Timezone, TypedValue, Value,
};

/// Which way one ordering key sorts.
#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Default,
    ::serde::Serialize,
    ::serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Smallest first.
    #[default]
    Ascending,
    /// Largest first.
    Descending,
}

/// Where nulls sit in an ordering.
#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    ::serde::Serialize,
    ::serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum NullsOrder {
    /// Nulls sort before every value.
    First,
    /// Nulls sort after every value.
    Last,
}

/// One ordering key: an expression, a direction, and where nulls sit.
#[derive(
    Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, ::serde::Serialize, ::serde::Deserialize,
)]
pub struct Order {
    expression: Expression,
    direction: Direction,
    nulls: Option<NullsOrder>,
}

impl Order {
    /// Sort ascending by an expression, leaving null placement to the reader.
    #[must_use]
    pub const fn new(expression: Expression) -> Self {
        Self {
            expression,
            direction: Direction::Ascending,
            nulls: None,
        }
    }

    /// Set the direction.
    #[must_use]
    pub const fn with_direction(mut self, direction: Direction) -> Self {
        self.direction = direction;
        self
    }

    /// Set where nulls sit.
    #[must_use]
    pub const fn with_nulls(mut self, nulls: NullsOrder) -> Self {
        self.nulls = Some(nulls);
        self
    }

    /// The expression sorted by.
    #[must_use]
    pub const fn expression(&self) -> &Expression {
        &self.expression
    }

    /// The direction sorted in.
    #[must_use]
    pub const fn direction(&self) -> Direction {
        self.direction
    }

    /// Where nulls sit, when the statement said.
    #[must_use]
    pub const fn nulls(&self) -> Option<NullsOrder> {
        self.nulls
    }
}

/// One output column: an expression and the name it is published under.
#[derive(
    Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, ::serde::Serialize, ::serde::Deserialize,
)]
pub struct Projection {
    expression: Expression,
    alias: Option<SmolStr>,
}

impl Projection {
    /// Publish an expression under the name it derives.
    #[must_use]
    pub const fn new(expression: Expression) -> Self {
        Self {
            expression,
            alias: None,
        }
    }

    /// Publish an expression under an explicit name.
    #[must_use]
    pub fn aliased(expression: Expression, alias: impl Into<SmolStr>) -> Self {
        Self {
            expression,
            alias: Some(alias.into()),
        }
    }

    /// The expression computed.
    #[must_use]
    pub const fn expression(&self) -> &Expression {
        &self.expression
    }

    /// The explicit name, when the statement gave one.
    #[must_use]
    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    /// The name this projection publishes.
    ///
    /// An aliased projection uses its alias; a bare column keeps its own name;
    /// anything else is named by its canonical text, so a statement never
    /// produces two columns that are impossible to tell apart.
    #[must_use]
    pub fn name(&self) -> SmolStr {
        if let Some(alias) = &self.alias {
            return alias.clone();
        }
        match &self.expression {
            Expression::Column(name) => name.clone(),
            Expression::Path(_, steps) => match steps.last() {
                Some(Segment::Field(name)) => name.clone(),
                _ => SmolStr::new(self.expression.to_string()),
            },
            other => SmolStr::new(other.to_string()),
        }
    }
}

/// A projection list, an optional predicate, an ordering, and a limit.
///
/// This is the whole of what a read can be asked for over one relation. It is
/// deliberately not a query language: there is no `from`, because the relation
/// is the handle the statement is given to, and no `join`, because there is
/// only ever one.
#[derive(
    Clone,
    Debug,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Default,
    ::serde::Serialize,
    ::serde::Deserialize,
)]
pub struct Statement {
    projections: Vec<Projection>,
    predicate: Option<Expression>,
    ordering: Vec<Order>,
    limit: Option<u64>,
}

impl Statement {
    /// Return a deterministic hash of the canonical statement text.
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_display(self)
    }

    /// Select every column, unfiltered.
    #[must_use]
    pub const fn all() -> Self {
        Self {
            projections: Vec::new(),
            predicate: None,
            ordering: Vec::new(),
            limit: None,
        }
    }

    /// Select the given projections.
    #[must_use]
    pub fn select(projections: impl IntoIterator<Item = Projection>) -> Self {
        Self {
            projections: projections.into_iter().collect(),
            ..Self::all()
        }
    }

    /// Filter by a predicate.
    #[must_use]
    pub fn with_predicate(mut self, predicate: Expression) -> Self {
        self.predicate = Some(predicate);
        self
    }

    /// Order by the given keys.
    #[must_use]
    pub fn with_ordering(mut self, ordering: impl IntoIterator<Item = Order>) -> Self {
        self.ordering = ordering.into_iter().collect();
        self
    }

    /// Stop after this many rows.
    #[must_use]
    pub const fn with_limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    /// The projections, empty when the statement selected `*`.
    #[must_use]
    pub fn projections(&self) -> &[Projection] {
        &self.projections
    }

    /// The predicate, when the statement had a `where`.
    #[must_use]
    pub const fn predicate(&self) -> Option<&Expression> {
        self.predicate.as_ref()
    }

    /// The ordering keys, in priority order.
    #[must_use]
    pub fn ordering(&self) -> &[Order] {
        &self.ordering
    }

    /// The row limit, when the statement had one.
    #[must_use]
    pub const fn limit(&self) -> Option<u64> {
        self.limit
    }

    /// Return whether this statement selects every column unchanged.
    #[must_use]
    pub fn is_all(&self) -> bool {
        self.projections.is_empty()
    }
}

impl FromStr for Expression {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self> {
        let mut parser = Parser::new(input)?;
        let expression = parser.expression()?;
        parser.expect_end()?;
        expression.check_budget()?;
        Ok(expression)
    }
}

impl FromStr for Statement {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self> {
        let mut parser = Parser::new(input)?;
        let statement = parser.statement()?;
        parser.expect_end()?;
        if let Some(predicate) = &statement.predicate {
            predicate.check_budget()?;
        }
        for projection in &statement.projections {
            projection.expression.check_budget()?;
        }
        Ok(statement)
    }
}

// ---------------------------------------------------------------------------
// Tokens
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Token {
    /// A bare word: a keyword, a column, a function, or a datatype name.
    Word(SmolStr),
    /// A quoted name, which is always a name and never a keyword.
    Quoted(SmolStr),
    /// A numeric literal, in the text it was written with.
    Number(SmolStr),
    /// A single-quoted text literal, already unescaped.
    Text(SmolStr),
    /// One punctuation token.
    Symbol(&'static str),
}

#[derive(Clone, Debug)]
struct Spanned {
    token: Token,
    position: usize,
}

/// The multi-character symbols, longest first so `<=` never reads as `<`.
const SYMBOLS: [&str; 22] = [
    "<>", "<=", ">=", "!=", "<", ">", "(", ")", "[", "]", "{", "}", ",", ".", ":", "&", "*", "+",
    "-", "/", "%", "=",
];

fn tokenize(input: &str) -> Result<Vec<Spanned>> {
    let bytes = input.as_bytes();
    let mut tokens = Vec::new();
    let mut cursor = 0_usize;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if byte.is_ascii_whitespace() {
            cursor += 1;
            continue;
        }
        // `--` to end of line is the one comment form, because it is the one
        // every SQL dialect agrees on and it cannot start an expression.
        if byte == b'-' && bytes.get(cursor + 1) == Some(&b'-') {
            while cursor < bytes.len() && bytes[cursor] != b'\n' {
                cursor += 1;
            }
            continue;
        }
        let start = cursor;
        if byte == b'\'' {
            let (text, next) = read_delimited(input, cursor, '\'')?;
            tokens.push(Spanned {
                token: Token::Text(text),
                position: start,
            });
            cursor = next;
            continue;
        }
        if byte == b'"' || byte == b'`' {
            let quote = char::from(byte);
            let (text, next) = read_delimited(input, cursor, quote)?;
            if text.is_empty() {
                return Err(parse_error(start, "expected a name inside the quotes"));
            }
            tokens.push(Spanned {
                token: Token::Quoted(text),
                position: start,
            });
            cursor = next;
            continue;
        }
        if byte.is_ascii_digit() {
            let (text, next) = read_number(input, cursor)?;
            tokens.push(Spanned {
                token: Token::Number(text),
                position: start,
            });
            cursor = next;
            continue;
        }
        if byte.is_ascii_alphabetic() || byte == b'_' {
            let mut end = cursor;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            tokens.push(Spanned {
                token: Token::Word(SmolStr::new(&input[cursor..end])),
                position: start,
            });
            cursor = end;
            continue;
        }
        let rest = &input[cursor..];
        let Some(symbol) = SYMBOLS.into_iter().find(|symbol| rest.starts_with(symbol)) else {
            return Err(parse_error(
                start,
                format_smolstr!("expected an operator or a name, got {:?}", char::from(byte)),
            ));
        };
        tokens.push(Spanned {
            token: Token::Symbol(symbol),
            position: start,
        });
        cursor += symbol.len();
    }
    Ok(tokens)
}

/// Read a delimited run, treating a doubled delimiter as one literal character.
///
/// The doubling rule is the SQL one and it is the only escape: a backslash in
/// a literal is a backslash, which is what a Windows path and a glob both
/// need it to be.
fn read_delimited(input: &str, start: usize, delimiter: char) -> Result<(SmolStr, usize)> {
    let width = delimiter.len_utf8();
    let mut text = String::new();
    let mut cursor = start + width;
    loop {
        let Some(character) = input[cursor..].chars().next() else {
            return Err(parse_error(
                start,
                format_smolstr!("expected a closing {delimiter:?}"),
            ));
        };
        let step = character.len_utf8();
        if character != delimiter {
            text.push(character);
            cursor += step;
            continue;
        }
        if input[cursor + step..].starts_with(delimiter) {
            text.push(delimiter);
            cursor += step * 2;
            continue;
        }
        return Ok((SmolStr::new(text), cursor + step));
    }
}

/// Read one numeric literal, integer or floating, in the text it was written.
fn read_number(input: &str, start: usize) -> Result<(SmolStr, usize)> {
    let bytes = input.as_bytes();
    let mut cursor = start;
    while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
        cursor += 1;
    }
    if cursor < bytes.len() && bytes[cursor] == b'.' {
        // A `.` followed by a digit is a fraction; a `.` followed by a name is
        // a path step off an integer-looking column, which the grammar has no
        // way to spell, so only the digit case consumes it.
        if bytes.get(cursor + 1).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
            while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                cursor += 1;
            }
        }
    }
    if cursor < bytes.len() && (bytes[cursor] == b'e' || bytes[cursor] == b'E') {
        let mut lookahead = cursor + 1;
        if lookahead < bytes.len() && (bytes[lookahead] == b'+' || bytes[lookahead] == b'-') {
            lookahead += 1;
        }
        if lookahead < bytes.len() && bytes[lookahead].is_ascii_digit() {
            cursor = lookahead;
            while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                cursor += 1;
            }
        }
    }
    Ok((SmolStr::new(&input[start..cursor]), cursor))
}

fn parse_error(position: usize, reason: impl Into<SmolStr>) -> Error {
    Error::Parse {
        target: "expression",
        position,
        reason: reason.into(),
    }
}

// ---------------------------------------------------------------------------
// The parser
// ---------------------------------------------------------------------------

struct Parser<'input> {
    input: &'input str,
    tokens: Vec<Spanned>,
    cursor: usize,
    depth: usize,
}

impl<'input> Parser<'input> {
    fn new(input: &'input str) -> Result<Self> {
        Ok(Self {
            input,
            tokens: tokenize(input)?,
            cursor: 0,
            depth: 0,
        })
    }

    fn position(&self) -> usize {
        self.tokens
            .get(self.cursor)
            .map_or(self.input.len(), |spanned| spanned.position)
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.cursor).map(|spanned| &spanned.token)
    }

    fn peek_at(&self, ahead: usize) -> Option<&Token> {
        self.tokens
            .get(self.cursor + ahead)
            .map(|spanned| &spanned.token)
    }

    fn advance(&mut self) -> Option<Token> {
        let token = self
            .tokens
            .get(self.cursor)
            .map(|spanned| spanned.token.clone());
        if token.is_some() {
            self.cursor += 1;
        }
        token
    }

    fn at_symbol(&self, symbol: &str) -> bool {
        matches!(self.peek(), Some(Token::Symbol(held)) if *held == symbol)
    }

    fn eat_symbol(&mut self, symbol: &str) -> bool {
        if self.at_symbol(symbol) {
            self.cursor += 1;
            return true;
        }
        false
    }

    fn expect_symbol(&mut self, symbol: &str) -> Result<()> {
        if self.eat_symbol(symbol) {
            return Ok(());
        }
        Err(parse_error(
            self.position(),
            format_smolstr!("expected {symbol:?}, got {}", self.describe()),
        ))
    }

    fn at_word(&self, word: &str) -> bool {
        matches!(self.peek(), Some(Token::Word(held)) if held.eq_ignore_ascii_case(word))
    }

    fn word_at(&self, ahead: usize, word: &str) -> bool {
        matches!(self.peek_at(ahead), Some(Token::Word(held)) if held.eq_ignore_ascii_case(word))
    }

    fn eat_word(&mut self, word: &str) -> bool {
        if self.at_word(word) {
            self.cursor += 1;
            return true;
        }
        false
    }

    fn expect_word(&mut self, word: &str) -> Result<()> {
        if self.eat_word(word) {
            return Ok(());
        }
        Err(parse_error(
            self.position(),
            format_smolstr!("expected {word:?}, got {}", self.describe()),
        ))
    }

    fn describe(&self) -> SmolStr {
        match self.peek() {
            None => SmolStr::new_static("the end of the expression"),
            Some(Token::Word(word)) => format_smolstr!("{word:?}"),
            Some(Token::Quoted(name)) => format_smolstr!("the quoted name {name:?}"),
            Some(Token::Number(text)) => format_smolstr!("the number {text}"),
            Some(Token::Text(text)) => format_smolstr!("the text {text:?}"),
            Some(Token::Symbol(symbol)) => format_smolstr!("{symbol:?}"),
        }
    }

    fn expect_end(&self) -> Result<()> {
        if self.cursor == self.tokens.len() {
            return Ok(());
        }
        Err(parse_error(
            self.position(),
            format_smolstr!(
                "expected the end of the expression, got {}",
                self.describe()
            ),
        ))
    }

    /// Enter one level of nesting, refusing past the shared hard limit.
    fn enter(&mut self) -> Result<()> {
        self.depth += 1;
        if self.depth > RECURSION_LIMIT {
            return Err(parse_error(
                self.position(),
                format_smolstr!("expected nesting within the hard limit of {RECURSION_LIMIT}"),
            ));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    // -- statement ----------------------------------------------------------

    fn statement(&mut self) -> Result<Statement> {
        self.expect_word("select")?;
        let mut projections = Vec::new();
        if self.eat_symbol("*") {
            // `select *` is the empty projection list: every column, unchanged.
        } else {
            loop {
                let expression = self.expression()?;
                let alias = if self.eat_word("as") {
                    Some(self.identifier()?)
                } else {
                    None
                };
                projections.push(match alias {
                    Some(alias) => Projection::aliased(expression, alias),
                    None => Projection::new(expression),
                });
                if !self.eat_symbol(",") {
                    break;
                }
            }
        }
        let predicate = if self.eat_word("where") {
            Some(self.expression()?)
        } else {
            None
        };
        let mut ordering = Vec::new();
        if self.eat_word("order") {
            self.expect_word("by")?;
            loop {
                let expression = self.expression()?;
                let direction = if self.eat_word("desc") {
                    Direction::Descending
                } else {
                    let _ = self.eat_word("asc");
                    Direction::Ascending
                };
                let nulls = if self.eat_word("nulls") {
                    if self.eat_word("first") {
                        Some(NullsOrder::First)
                    } else {
                        self.expect_word("last")?;
                        Some(NullsOrder::Last)
                    }
                } else {
                    None
                };
                let mut order = Order::new(expression).with_direction(direction);
                if let Some(nulls) = nulls {
                    order = order.with_nulls(nulls);
                }
                ordering.push(order);
                if !self.eat_symbol(",") {
                    break;
                }
            }
        }
        let limit = if self.eat_word("limit") {
            let position = self.position();
            let Some(Token::Number(text)) = self.advance() else {
                return Err(parse_error(position, "expected a whole number of rows"));
            };
            Some(text.parse::<u64>().map_err(|_| {
                parse_error(
                    position,
                    format_smolstr!("expected a whole number of rows, got {text}"),
                )
            })?)
        } else {
            None
        };
        Ok(Statement {
            projections,
            predicate,
            ordering,
            limit,
        })
    }

    // -- expression ---------------------------------------------------------

    fn expression(&mut self) -> Result<Expression> {
        self.enter()?;
        let parsed = self.disjunction();
        self.leave();
        parsed
    }

    fn disjunction(&mut self) -> Result<Expression> {
        let mut operands = vec![self.conjunction()?];
        while self.eat_word("or") {
            operands.push(self.conjunction()?);
        }
        Ok(if operands.len() == 1 {
            operands.swap_remove(0)
        } else {
            Expression::any(operands)
        })
    }

    fn conjunction(&mut self) -> Result<Expression> {
        let mut operands = vec![self.negation()?];
        while self.eat_word("and") {
            operands.push(self.negation()?);
        }
        Ok(if operands.len() == 1 {
            operands.swap_remove(0)
        } else {
            Expression::all(operands)
        })
    }

    fn negation(&mut self) -> Result<Expression> {
        if self.eat_word("not") {
            self.enter()?;
            let inner = self.negation();
            self.leave();
            return Ok(inner?.not());
        }
        self.predicate()
    }

    #[allow(clippy::too_many_lines)]
    fn predicate(&mut self) -> Result<Expression> {
        let left = self.additive()?;
        if let Some(comparison) = self.comparison_symbol() {
            let right = self.additive()?;
            return Ok(left.compare(comparison, right));
        }
        if self.at_word("is") {
            self.cursor += 1;
            let negated = self.eat_word("not");
            if self.eat_word("null") {
                return Ok(if negated {
                    left.is_not_null()
                } else {
                    left.is_null()
                });
            }
            self.expect_word("distinct")?;
            self.expect_word("from")?;
            let right = self.additive()?;
            let comparison = if negated {
                Comparison::IsNotDistinctFrom
            } else {
                Comparison::IsDistinctFrom
            };
            return Ok(left.compare(comparison, right));
        }
        let negated = self.at_word("not")
            && (self.word_at(1, "in")
                || self.word_at(1, "between")
                || self.word_at(1, "like")
                || self.word_at(1, "ilike")
                || self.word_at(1, "glob"));
        if negated {
            self.cursor += 1;
        }
        let built = if self.eat_word("in") {
            let position = self.position();
            self.expect_symbol("(")?;
            let mut list = Vec::new();
            if self.at_symbol(")") {
                return Err(parse_error(
                    position,
                    "expected at least one value in the `in` list",
                ));
            }
            loop {
                list.push(self.expression()?);
                if !self.eat_symbol(",") {
                    break;
                }
            }
            self.expect_symbol(")")?;
            left.is_in(list)
        } else if self.eat_word("between") {
            let low = self.additive()?;
            self.expect_word("and")?;
            let high = self.additive()?;
            left.between(low, high)
        } else if self.at_word("like") || self.at_word("ilike") {
            let case_insensitive = self.at_word("ilike");
            self.cursor += 1;
            let pattern = self.additive()?;
            let escape = if self.eat_word("escape") {
                let position = self.position();
                let Some(Token::Text(text)) = self.advance() else {
                    return Err(parse_error(position, "expected a one-character escape"));
                };
                let mut characters = text.chars();
                match (characters.next(), characters.next()) {
                    (Some(character), None) => Some(character),
                    _ => {
                        return Err(parse_error(
                            position,
                            format_smolstr!("expected exactly one escape character, got {text:?}"),
                        ));
                    }
                }
            } else {
                None
            };
            Expression::Like {
                value: Box::new(left),
                pattern: Box::new(pattern),
                case_insensitive,
                escape,
            }
        } else if self.eat_word("glob") {
            let pattern = self.additive()?;
            left.glob(pattern)
        } else {
            if negated {
                return Err(parse_error(
                    self.position(),
                    format_smolstr!(
                        "expected `in`, `between`, `like`, `ilike`, or `glob` after `not`, got {}",
                        self.describe()
                    ),
                ));
            }
            return Ok(left);
        };
        Ok(if negated { built.not() } else { built })
    }

    fn comparison_symbol(&mut self) -> Option<Comparison> {
        let comparison = match self.peek() {
            Some(Token::Symbol("=")) => Comparison::Eq,
            Some(Token::Symbol("<>" | "!=")) => Comparison::NotEq,
            Some(Token::Symbol("<")) => Comparison::Lt,
            Some(Token::Symbol("<=")) => Comparison::LtEq,
            Some(Token::Symbol(">")) => Comparison::Gt,
            Some(Token::Symbol(">=")) => Comparison::GtEq,
            _ => return None,
        };
        self.cursor += 1;
        Some(comparison)
    }

    fn additive(&mut self) -> Result<Expression> {
        let mut left = self.product()?;
        loop {
            let operator = if self.at_symbol("+") {
                Operator::Add
            } else if self.at_symbol("-") {
                Operator::Sub
            } else {
                return Ok(left);
            };
            self.cursor += 1;
            let right = self.product()?;
            left = left.arithmetic(operator, right);
        }
    }

    fn product(&mut self) -> Result<Expression> {
        let mut left = self.unary()?;
        loop {
            let operator = if self.at_symbol("*") {
                Operator::Mul
            } else if self.at_symbol("/") {
                Operator::Div
            } else if self.at_symbol("%") {
                Operator::Rem
            } else {
                return Ok(left);
            };
            self.cursor += 1;
            let right = self.unary()?;
            left = left.arithmetic(operator, right);
        }
    }

    fn unary(&mut self) -> Result<Expression> {
        if self.eat_symbol("-") {
            self.enter()?;
            let inner = self.unary();
            self.leave();
            return Ok(inner?.neg());
        }
        // A leading `+` is a no-op every dialect accepts and none stores.
        if self.eat_symbol("+") {
            return self.unary();
        }
        self.accessor()
    }

    fn accessor(&mut self) -> Result<Expression> {
        let mut base = self.atom()?;
        loop {
            if self.eat_symbol(".") {
                let name = self.identifier()?;
                base = base.child(name);
                continue;
            }
            if self.at_symbol("[") {
                // A `[` after a value is a path step; a `[` that starts a value
                // was already consumed by `atom`.
                self.cursor += 1;
                let segment = self.segment()?;
                self.expect_symbol("]")?;
                base = base.path([segment]);
                continue;
            }
            return Ok(base);
        }
    }

    /// Read one path step: an integer position, or a constant key.
    fn segment(&mut self) -> Result<Segment> {
        let position = self.position();
        let negative = self.eat_symbol("-");
        if let Some(Token::Number(text)) = self.peek().cloned() {
            if !text.contains(['.', 'e', 'E']) {
                self.cursor += 1;
                let magnitude = text.parse::<i64>().map_err(|_| {
                    parse_error(
                        position,
                        format_smolstr!(
                            "expected a list position that fits in 64 bits, got {text}"
                        ),
                    )
                })?;
                return Ok(Segment::Index(if negative {
                    -magnitude
                } else {
                    magnitude
                }));
            }
        }
        if negative {
            return Err(parse_error(
                position,
                "expected a whole list position after `-`",
            ));
        }
        let key = self.expression()?;
        let Expression::Literal(held) = key else {
            return Err(parse_error(
                position,
                "expected a constant key; use get(container, key) for a computed one",
            ));
        };
        Ok(Segment::Key(held))
    }

    fn identifier(&mut self) -> Result<SmolStr> {
        let position = self.position();
        match self.peek().cloned() {
            Some(Token::Quoted(name)) => {
                self.cursor += 1;
                Ok(name)
            }
            Some(Token::Word(name)) if !is_reserved(&name) => {
                self.cursor += 1;
                Ok(name)
            }
            Some(Token::Word(name)) => Err(parse_error(
                position,
                format_smolstr!("expected a name, got the reserved word {name:?}"),
            )),
            _ => Err(parse_error(
                position,
                format_smolstr!("expected a name, got {}", self.describe()),
            )),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn atom(&mut self) -> Result<Expression> {
        let position = self.position();
        if self.eat_symbol("(") {
            let inner = self.expression()?;
            self.expect_symbol(")")?;
            return Ok(inner);
        }
        if self.eat_symbol("[") {
            let mut items = Vec::new();
            if !self.at_symbol("]") {
                loop {
                    items.push(self.expression()?);
                    if !self.eat_symbol(",") {
                        break;
                    }
                }
            }
            self.expect_symbol("]")?;
            return Ok(Expression::List(Arc::from(items)));
        }
        if self.eat_symbol("{") {
            let mut entries = Vec::new();
            if !self.at_symbol("}") {
                loop {
                    let key = self.expression()?;
                    self.expect_symbol(":")?;
                    let value = self.expression()?;
                    entries.push((key, value));
                    if !self.eat_symbol(",") {
                        break;
                    }
                }
            }
            self.expect_symbol("}")?;
            return Ok(Expression::Map(Arc::from(entries)));
        }
        if self.eat_symbol("&") {
            return self.attribute(position);
        }
        if self.eat_symbol(":") {
            return Ok(Expression::parameter(self.identifier()?));
        }
        match self.peek().cloned() {
            Some(Token::Number(text)) => {
                self.cursor += 1;
                return number_literal(&text, position);
            }
            Some(Token::Text(text)) => {
                self.cursor += 1;
                return Ok(Expression::literal(Value::String(text)));
            }
            Some(Token::Quoted(name)) => {
                self.cursor += 1;
                return Ok(Expression::column(name));
            }
            _ => {}
        }
        let Some(Token::Word(word)) = self.peek().cloned() else {
            return Err(parse_error(
                position,
                format_smolstr!("expected a value or a name, got {}", self.describe()),
            ));
        };
        let lowered = word.to_ascii_lowercase();
        match lowered.as_str() {
            "null" => {
                self.cursor += 1;
                return Ok(Expression::literal(Value::Null));
            }
            "true" => {
                self.cursor += 1;
                return Ok(Expression::literal(Value::Bool(true)));
            }
            "false" => {
                self.cursor += 1;
                return Ok(Expression::literal(Value::Bool(false)));
            }
            "cast" | "try_cast" => {
                self.cursor += 1;
                return self.cast(&lowered);
            }
            "case" => {
                self.cursor += 1;
                return self.case();
            }
            "struct" if matches!(self.peek_at(1), Some(Token::Symbol("("))) => {
                self.cursor += 1;
                return self.structure();
            }
            _ => {}
        }
        if is_reserved(&lowered) {
            return Err(parse_error(
                position,
                format_smolstr!("expected a value or a name, got the reserved word {word:?}"),
            ));
        }
        // A word followed by `(` is a function call when the name is one, and
        // a parameterized datatype when a literal follows the closing paren.
        if matches!(self.peek_at(1), Some(Token::Symbol("("))) {
            if let Some(function) = Function::from_name(&lowered) {
                self.cursor += 1;
                let arguments = self.arguments()?;
                let (least, most) = function.arity();
                if arguments.len() < least || arguments.len() > most {
                    return Err(parse_error(
                        position,
                        format_smolstr!(
                            "expected {} to take {}, got {} argument(s)",
                            function.as_str(),
                            arity_text(least, most),
                            arguments.len()
                        ),
                    ));
                }
                return Ok(Expression::call(function, arguments));
            }
            if let Some(literal) = self.typed_literal(position)? {
                return Ok(literal);
            }
            return Err(parse_error(
                position,
                format_smolstr!(
                    "expected one of the functions {}, got {word:?}",
                    Function::vocabulary()
                ),
            ));
        }
        if matches!(self.peek_at(1), Some(Token::Text(_) | Token::Word(_))) {
            if let Some(literal) = self.typed_literal(position)? {
                return Ok(literal);
            }
        }
        self.cursor += 1;
        Ok(Expression::column(word))
    }

    /// Read `&holder.<selector>`, the one attribute spelling.
    fn attribute(&mut self, position: usize) -> Result<Expression> {
        let holder = self.identifier()?;
        if !holder.eq_ignore_ascii_case("holder") {
            return Err(parse_error(
                position,
                format_smolstr!("expected `&holder.<attribute>`, got `&{holder}`"),
            ));
        }
        self.expect_symbol(".")?;
        let name_position = self.position();
        let name = self.identifier()?;
        if name.eq_ignore_ascii_case("partition") {
            self.expect_symbol("[")?;
            let key_position = self.position();
            let Some(Token::Text(column)) = self.advance() else {
                return Err(parse_error(
                    key_position,
                    "expected a quoted partition column name",
                ));
            };
            self.expect_symbol("]")?;
            return Ok(Expression::attribute(Selector::Partition(column)));
        }
        let selector = Selector::from_name(&name)
            .ok_or_else(|| super::selector::unknown(&name, name_position))?;
        Ok(Expression::attribute(selector))
    }

    fn arguments(&mut self) -> Result<Vec<Expression>> {
        self.expect_symbol("(")?;
        let mut arguments = Vec::new();
        if !self.at_symbol(")") {
            loop {
                arguments.push(self.expression()?);
                if !self.eat_symbol(",") {
                    break;
                }
            }
        }
        self.expect_symbol(")")?;
        Ok(arguments)
    }

    fn cast(&mut self, keyword: &str) -> Result<Expression> {
        self.expect_symbol("(")?;
        let inner = self.expression()?;
        self.expect_word("as")?;
        let data_type = self.data_type()?;
        self.expect_symbol(")")?;
        Ok(Expression::Cast(
            Box::new(inner),
            data_type,
            if keyword == "try_cast" {
                Safety::Safe
            } else {
                Safety::Strict
            },
        ))
    }

    fn case(&mut self) -> Result<Expression> {
        let position = self.position();
        let mut branches = Vec::new();
        while self.eat_word("when") {
            let when = self.expression()?;
            self.expect_word("then")?;
            let then = self.expression()?;
            branches.push((when, then));
        }
        if branches.is_empty() {
            return Err(parse_error(position, "expected at least one `when` branch"));
        }
        let otherwise = if self.eat_word("else") {
            Some(self.expression()?)
        } else {
            None
        };
        self.expect_word("end")?;
        Ok(Expression::case(branches, otherwise))
    }

    fn structure(&mut self) -> Result<Expression> {
        self.expect_symbol("(")?;
        let mut children = Vec::new();
        if !self.at_symbol(")") {
            loop {
                let value = self.expression()?;
                self.expect_word("as")?;
                let name = self.identifier()?;
                children.push((name, value));
                if !self.eat_symbol(",") {
                    break;
                }
            }
        }
        self.expect_symbol(")")?;
        Ok(Expression::Struct(Arc::from(children)))
    }

    /// Read one datatype through the crate's own datatype grammar.
    ///
    /// The datatype text is taken verbatim from the input rather than rebuilt
    /// from tokens, so there is exactly one datatype parser in the crate and
    /// this module never learns what a datatype looks like.
    fn data_type(&mut self) -> Result<DataType> {
        let position = self.position();
        let Some(Token::Word(_)) = self.peek() else {
            return Err(parse_error(
                position,
                format_smolstr!("expected a datatype, got {}", self.describe()),
            ));
        };
        self.cursor += 1;
        let end = if self.at_symbol("(") {
            self.skip_balanced()?
        } else {
            self.token_end(self.cursor - 1)
        };
        let text = self.input[position..end].trim();
        DataType::from_str(text).map_err(|error| match error {
            Error::Parse { reason, .. } => parse_error(position, reason),
            other => parse_error(position, format_smolstr!("{other}")),
        })
    }

    /// Consume a balanced `(...)` run and answer the byte just past it.
    fn skip_balanced(&mut self) -> Result<usize> {
        let opened = self.position();
        let mut depth = 0_usize;
        loop {
            match self.peek() {
                Some(Token::Symbol("(")) => depth += 1,
                Some(Token::Symbol(")")) => {
                    depth -= 1;
                    if depth == 0 {
                        let end = self.token_end(self.cursor);
                        self.cursor += 1;
                        return Ok(end);
                    }
                }
                None => {
                    return Err(parse_error(opened, "expected a closing \")\""));
                }
                _ => {}
            }
            self.cursor += 1;
        }
    }

    /// The byte just past the token at `index`.
    fn token_end(&self, index: usize) -> usize {
        self.tokens
            .get(index + 1)
            .map_or(self.input.len(), |next| next.position)
    }

    /// Read `<datatype> '<text>'` or `<datatype> null`, if that is what is here.
    ///
    /// Answers `None` without consuming anything when the word is not a
    /// datatype, so the caller can fall back to reading it as a column.
    fn typed_literal(&mut self, position: usize) -> Result<Option<Expression>> {
        let restore = self.cursor;
        let Ok(data_type) = self.data_type() else {
            self.cursor = restore;
            return Ok(None);
        };
        match self.peek().cloned() {
            Some(Token::Text(text)) => {
                self.cursor += 1;
                let value = value_from_text(&data_type, &text, position)?;
                Ok(Some(Expression::Literal(
                    TypedValue::from_parts(data_type, value)
                        .map_err(|error| parse_error(position, format_smolstr!("{error}")))?,
                )))
            }
            Some(Token::Word(word)) if word.eq_ignore_ascii_case("null") => {
                self.cursor += 1;
                Ok(Some(Expression::Literal(
                    TypedValue::from_parts(data_type, Value::Null)
                        .map_err(|error| parse_error(position, format_smolstr!("{error}")))?,
                )))
            }
            _ => {
                self.cursor = restore;
                Ok(None)
            }
        }
    }
}

/// The inclusive arity of a function, in the words an error message uses.
fn arity_text(least: usize, most: usize) -> SmolStr {
    if most == usize::MAX {
        return format_smolstr!("at least {least} argument(s)");
    }
    if least == most {
        return format_smolstr!("exactly {least} argument(s)");
    }
    format_smolstr!("{least} to {most} arguments")
}

/// Read one numeric literal: `int64` when whole, `float64` when not.
fn number_literal(text: &str, position: usize) -> Result<Expression> {
    if text.contains(['.', 'e', 'E']) {
        let held = text.parse::<f64>().map_err(|_| {
            parse_error(
                position,
                format_smolstr!("expected a 64-bit floating-point number, got {text}"),
            )
        })?;
        return Ok(Expression::literal(held));
    }
    let held = text.parse::<i64>().map_err(|_| {
        parse_error(
            position,
            format_smolstr!("expected a whole number that fits in 64 bits, got {text}"),
        )
    })?;
    Ok(Expression::literal(held))
}

/// Read one value out of the text half of a typed literal.
///
/// The text forms are the crate's own: ISO 8601 for every temporal, an exact
/// decimal string for a decimal, lowercase hex for binary. Nothing here is a
/// second value parser - each family delegates to the one the codecs use.
pub(crate) fn value_from_text(data_type: &DataType, text: &str, position: usize) -> Result<Value> {
    use crate::generic::iso;
    use DataType as D;

    let fail = |expected: &str| {
        parse_error(
            position,
            format_smolstr!("expected {expected}, got {text:?}"),
        )
    };
    let integer = |text: &str| -> Result<Value> {
        text.parse::<i128>()
            .map(Value::I128)
            .map_err(|_| fail("a whole number"))
    };
    let value = match data_type {
        D::Null => Value::Null,
        D::Boolean => match text {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => return Err(fail("`true` or `false`")),
        },
        D::Int8 | D::Int16 | D::Int32 | D::Int64 => integer(text)?,
        // A date is an ISO date, never a raw count of days: the count is a
        // physical detail and the literal is what a person wrote.
        D::Date32 | D::Date64 => Value::date32(iso::parse_date(text)?),
        D::UInt8 | D::UInt16 | D::UInt32 | D::UInt64 => text
            .parse::<u128>()
            .map(Value::U128)
            .map_err(|_| fail("a whole number that is not negative"))?,
        D::Float16 => Value::F16(Float16::from_f16(half::f16::from_f64(
            float_from_text(text).ok_or_else(|| fail("a floating-point number"))?,
        ))),
        D::Float32 => Value::F32(Float32::from_f32(
            float_from_text(text).ok_or_else(|| fail("a floating-point number"))? as f32,
        )),
        D::Float64 => Value::F64(Float64::from_f64(
            float_from_text(text).ok_or_else(|| fail("a floating-point number"))?,
        )),
        D::Decimal32 { scale, .. } | D::Decimal64 { scale, .. } | D::Decimal128 { scale, .. } => {
            Value::d128(
                decimal_from_text(text, *scale).ok_or_else(|| {
                    fail("an exact decimal that fits the declared precision and scale")
                })?,
                *scale,
            )
        }
        D::Decimal256 { scale, .. } => Value::d256(
            I256::from_i128(decimal_from_text(text, *scale).ok_or_else(|| {
                fail("an exact decimal that fits the declared precision and scale")
            })?),
            *scale,
        ),
        D::Utf8 | D::LargeUtf8 | D::Utf8View => Value::String(SmolStr::new(text)),
        D::Binary | D::LargeBinary | D::BinaryView | D::FixedSizeBinary(_) => {
            Value::Bytes(Arc::from(
                bytes_from_hex(text).ok_or_else(|| fail("an even-length run of hex digits"))?,
            ))
        }
        D::Time32(_) | D::Time64(_) => {
            let (count, unit) = iso::parse_time(text)?;
            parsed_time_value(count, unit)?
        }
        D::Timestamp(_, Some(_)) => {
            let (count, unit, zone) = iso::parse_timestamp(text)?;
            Value::datetime64(count, unit, zone)?
        }
        D::Timestamp(_, None) => {
            let (count, unit) = iso::parse_datetime(text)?;
            Value::datetime64(count, unit, Timezone::NAIVE)?
        }
        D::Duration32(_) | D::Duration64(_) => {
            let (count, unit) = iso::parse_duration(text)?;
            parsed_duration_value(count, unit)?
        }
        other => {
            return Err(parse_error(
                position,
                format_smolstr!(
                    "expected a datatype with a text literal form, got {other}; \
                     build a nested constant with [], {{}}, or struct()"
                ),
            ));
        }
    };
    // The one conversion in the module puts the parsed value in exactly the
    // declared type - the same call `cast` makes, so a written literal and a
    // cast value can never end up shaped differently.
    super::eval::convert(data_type, &value, super::Safety::Strict)
        .map_err(|error| parse_error(position, format_smolstr!("{error}")))
}

fn parsed_time_value(count: i64, unit: TimeUnit) -> Result<Value> {
    match unit {
        TimeUnit::Second | TimeUnit::Millisecond => Value::time32(
            i32::try_from(count).map_err(|_| Error::InvalidRecord {
                path: "$".into(),
                reason: "time32 count exceeds 32 bits".into(),
            })?,
            unit,
            Timezone::NAIVE,
        ),
        TimeUnit::Microsecond | TimeUnit::Nanosecond => Value::time64(count, unit, Timezone::NAIVE),
        _ => Err(Error::InvalidRecord {
            path: "$".into(),
            reason: "time requires a fixed-length clock unit".into(),
        }),
    }
}

fn parsed_duration_value(count: i64, unit: TimeUnit) -> Result<Value> {
    match unit {
        TimeUnit::Second | TimeUnit::Millisecond | TimeUnit::Microsecond | TimeUnit::Nanosecond => {
            Value::duration64(count, unit)
        }
        _ => Err(Error::InvalidRecord {
            path: "$".into(),
            reason: "duration requires a fixed-length clock unit".into(),
        }),
    }
}

/// Read a float, accepting the three names the finite grammar cannot spell.
fn float_from_text(text: &str) -> Option<f64> {
    match text.to_ascii_lowercase().as_str() {
        "nan" => Some(f64::NAN),
        "inf" | "+inf" | "infinity" => Some(f64::INFINITY),
        "-inf" | "-infinity" => Some(f64::NEG_INFINITY),
        _ => text.parse::<f64>().ok(),
    }
}

/// Read an exact decimal at a declared scale, refusing a digit that would drop.
fn decimal_from_text(text: &str, scale: i8) -> Option<i128> {
    let (sign, digits) = match text.strip_prefix('-') {
        Some(rest) => (-1_i128, rest),
        None => (1_i128, text.strip_prefix('+').unwrap_or(text)),
    };
    let (whole, fraction) = match digits.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (digits, ""),
    };
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    if !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let mut unscaled = whole.parse::<i128>().unwrap_or_default();
    for byte in fraction.bytes() {
        unscaled = unscaled
            .checked_mul(10)?
            .checked_add(i128::from(byte - b'0'))?;
    }
    let written = i32::try_from(fraction.len()).ok()?;
    let declared = i32::from(scale);
    match declared.checked_sub(written)? {
        // The literal has fewer places than the column: pad with zeros.
        shift if shift > 0 => {
            for _ in 0..shift {
                unscaled = unscaled.checked_mul(10)?;
            }
        }
        // The literal has more: only exact trailing zeros may be dropped.
        shift if shift < 0 => {
            for _ in 0..-shift {
                if unscaled % 10 != 0 {
                    return None;
                }
                unscaled /= 10;
            }
        }
        _ => {}
    }
    Some(sign * unscaled)
}

/// Read lowercase or uppercase hex into bytes.
fn bytes_from_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(text.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let high = char::from(pair[0]).to_digit(16)?;
        let low = char::from(pair[1]).to_digit(16)?;
        decoded.push(u8::try_from(high * 16 + low).ok()?);
    }
    Some(decoded)
}

/// Return whether a name needs quoting to survive a round trip.
///
/// Exposed for the bindings, which build expressions from caller-supplied
/// column names and must not have to guess this rule.
#[must_use]
pub fn needs_quoting(name: &str) -> bool {
    !is_bare_identifier(name)
}
