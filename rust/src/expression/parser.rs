//! One recursive grammar, re-entered by every nested construct.
//!
//! There is exactly one parser in this module and exactly one in the workspace
//! that reads a path, a term, a filter or a selector. It is recursive descent
//! with explicit precedence, it re-enters itself for every operand - a `case`
//! arm holds a full term, a serie element holds a full term, a cast target
//! holds a full datatype through the crate's own datatype grammar - and it
//! refuses past [`RECURSION_LIMIT`](super::RECURSION_LIMIT) with a typed error
//! rather than by overflowing a stack.
//!
//! # The shape
//!
//! ```text
//! expression := "select" selector | "where" filter
//! selector   := "*" | projection ("," projection)*
//! projection := term ["as" identifier] [datatype ["null" | "not" "null"]]
//! filter     := term
//! term       := disjunction
//! disjunction:= conjunction ("or" conjunction)*
//! conjunction:= negation ("and" negation)*
//! negation   := "not" negation | predicate
//! predicate  := additive [ comparison | "is" .. | "in" .. | "between" .. | "like" .. ]
//! additive   := product (("+" | "-") product)*
//! product    := unary (("*" | "/" | "%") unary)*
//! unary      := "-" unary | accessor
//! accessor   := atom ("." identifier | "[" segment "]")*
//! segment    := integer | "'key'" | [integer] ":" [integer] | term
//! atom       := literal | "(" term ")" | column | ":" parameter
//!             | "cast" "(" term "as" datatype ")" | "case" .. "end"
//!             | function "(" term,* ")" | "[" term,* "]" | "{" term ":" term,* "}"
//!             | "struct" "(" term "as" identifier,* ")" | datatype text
//! path       := ["."] identifier ("." identifier | "[" segment "]")* ["as" identifier]
//! ```
//!
//! A plan reads its sections around these, each optional and in order -
//! `create`, a write verb, `select`, `from`, the joins, `where`, `order by`,
//! `limit`/`offset` - and a join is
//!
//! ```text
//! join       := [ "inner" | "left" ["outer"] | "right" ["outer"]
//!               | "full" ["outer"] | "outer" | "semi" | "anti" ] "join" source
//!               ( "on" term ("and" term)* | "using" "(" identifier ("," identifier)* ")" )
//! source     := target | "(" plan ")"
//! ```
//!
//! where each `on` conjunct is an equality, its left term over the rows so
//! far and its right term over the source.
//!
//! # What is deliberately not here
//!
//! No subquery inside a term, no join inside a term, no aggregate, no window,
//! no ordering. Every one of those needs a second relation or the whole of
//! one, and a term is a projection and filter tree over rows that stream; a
//! second relation enters a plan only as its `from` or a `join`. A grammar
//! that accepts them and then refuses them at bind time has told the caller a
//! lie at the point where the error message was still cheap.

use std::str::FromStr;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use super::display::{is_bare_identifier, is_reserved};
use super::join::{JoinKey, JoinKeys};
use super::path::{FieldPath, FieldSegment};
use super::plan::{Location, Ordering, Plan, Source, Target, Verb, Write};
use super::selector::{Projection, Selector};
use super::{
    Comparison, Expression, Filter, Function, Literal, Operator, RECURSION_LIMIT, Safety, Term,
    UserRef,
};
use crate::boolean::{BOOLEAN_SPELLINGS, boolean_from_text};
use crate::floating::{FLOAT_SPELLINGS, float_from_text};
use crate::integer::{INTEGER_SPELLINGS, integer_from_text};
use crate::{DataType, Error, JoinKind, Result, Scalar, Url};

impl FromStr for Term {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self> {
        let mut parser = Parser::new(input)?;
        let term = parser.term()?;
        parser.expect_end()?;
        term.check_budget()?;
        Ok(term)
    }
}

/// Parse one filter, with or without its `where` keyword in front.
pub(crate) fn parse_filter(input: &str) -> Result<Filter> {
    let mut parser = Parser::new(input)?;
    let _ = parser.eat_word("where");
    let term = parser.term()?;
    parser.expect_end()?;
    term.check_budget()?;
    Ok(Filter::new(term))
}

/// Parse one selector, with or without its `select` keyword in front.
pub(crate) fn parse_selector(input: &str) -> Result<Selector> {
    let mut parser = Parser::new(input)?;
    let _ = parser.eat_word("select");
    let selector = parser.selector()?;
    parser.expect_end()?;
    selector.check_budget()?;
    Ok(selector)
}

/// Parse one projection: a term, its alias, its declared type.
pub(crate) fn parse_projection(input: &str) -> Result<Projection> {
    let mut parser = Parser::new(input)?;
    let projection = parser.projection()?;
    parser.expect_end()?;
    projection.term().check_budget()?;
    Ok(projection)
}

/// Parse one `order by` key: a term, then `asc` or `desc`, then `nulls
/// first` or `nulls last`.
pub(crate) fn parse_ordering(input: &str) -> Result<Ordering> {
    let mut parser = Parser::new(input)?;
    let ordering = parser.ordering()?;
    parser.expect_end()?;
    ordering.term().check_budget()?;
    Ok(ordering)
}

/// Parse the keys of an `order by` clause without its keywords: one key or
/// several separated by `,`, each a term with its optional direction and
/// nulls placement - `venue, price desc nulls first`.
pub(crate) fn parse_orderings(input: &str) -> Result<Vec<Ordering>> {
    let mut parser = Parser::new(input)?;
    let mut keys = Vec::new();
    loop {
        let ordering = parser.ordering()?;
        ordering.term().check_budget()?;
        keys.push(ordering);
        if !parser.eat_symbol(",") {
            break;
        }
    }
    parser.expect_end()?;
    Ok(keys)
}

/// Parse the keys of a join: one or several separated by `,`, each a term
/// over both sides or an equality of a left term and a right term - `id,
/// venue = market`.
pub(crate) fn parse_join_keys(input: &str) -> Result<JoinKeys> {
    let mut parser = Parser::new(input)?;
    let mut keys = Vec::new();
    loop {
        let term = parser.term()?;
        term.check_budget()?;
        keys.push(JoinKey::from_term(term)?);
        if !parser.eat_symbol(",") {
            break;
        }
    }
    parser.expect_end()?;
    Ok(JoinKeys::new(keys))
}

/// Parse one expression: a plan, or plans separated by `;`.
///
/// A plan spelling only its `select` section is the selector; only its
/// `where` section, the filter. Anything else is the plan itself.
pub(crate) fn parse_expression(input: &str) -> Result<Expression> {
    let mut parser = Parser::new(input)?;
    let mut steps = Vec::new();
    loop {
        if parser.peek().is_none() && !steps.is_empty() {
            break;
        }
        let plan = parser.plan()?;
        plan.check_budget()?;
        steps.push(plan.into_expression());
        if !parser.eat_symbol(";") {
            break;
        }
    }
    parser.expect_end()?;
    Ok(Expression::sequence(steps))
}

/// Parse one plan and nothing else.
pub(crate) fn parse_plan(input: &str) -> Result<Plan> {
    let mut parser = Parser::new(input)?;
    let plan = parser.plan()?;
    parser.expect_end()?;
    plan.check_budget()?;
    Ok(plan)
}

/// Parse one write verb, in any spelling the grammar reads, with or without
/// the word that introduces its target.
pub(crate) fn parse_verb(input: &str) -> Result<Verb> {
    let mut parser = Parser::new(input)?;
    let verb = parser.verb()?.ok_or_else(|| {
        parse_error(
            parser.position(),
            format_smolstr!(
                "expected a write verb - insert, append, overwrite, replace, upsert, merge or \
                 delete - got {}",
                parser.describe()
            ),
        )
    })?;
    parser.expect_end()?;
    Ok(verb)
}

/// Parse one comparison: a symbol, or `is [not] distinct from`.
pub(crate) fn parse_comparison(input: &str) -> Result<Comparison> {
    let mut parser = Parser::new(input)?;
    let comparison = parser.comparison()?;
    parser.expect_end()?;
    Ok(comparison)
}

/// Parse one target: a URL, or a catalog path, with its properties.
pub(crate) fn parse_target(input: &str) -> Result<Target> {
    let mut parser = Parser::new(input)?;
    let target = parser.target()?;
    parser.expect_end()?;
    Ok(target)
}

/// Parse one field path: steps and an alias, and nothing else.
///
/// The empty text is the root. A leading dot is optional, a name after a dot
/// may be any word, and both quote styles are read for an alias - the widths
/// a hand-written path needs at intake, rendered back one way.
pub(crate) fn parse_field_path(input: &str) -> Result<FieldPath> {
    field_path(input).map_err(|error| match error {
        Error::Parse {
            position, reason, ..
        } => Error::Parse {
            target: super::path::TARGET,
            position,
            reason,
        },
        other => other,
    })
}

fn field_path(input: &str) -> Result<FieldPath> {
    let mut parser = Parser::new(input)?;
    if parser.peek().is_none() {
        return Ok(FieldPath::root());
    }
    let mut segments: Vec<FieldSegment> = Vec::new();
    // A leading dot is optional, so the first step may be a bare name.
    if parser.eat_symbol(".") {
        segments.push(FieldSegment::Field(parser.step_name()?));
    } else if parser.at_symbol("[") {
        parser.cursor += 1;
        segments.push(parser.segment()?);
        parser.expect_symbol("]")?;
    } else if let Some(Token::Number(text)) = parser.peek().cloned() {
        // A bare decimal is a name and not a position, exactly as it is one
        // layer down: a text line's entry keyed `55` is reached by the path
        // `55`, and a term reads the same number as a literal.
        parser.cursor += 1;
        segments.push(FieldSegment::Field(text));
    } else {
        segments.push(FieldSegment::Field(parser.identifier()?));
    }
    loop {
        if parser.eat_symbol(".") {
            segments.push(FieldSegment::Field(parser.step_name()?));
            continue;
        }
        if parser.at_symbol("[") {
            parser.cursor += 1;
            segments.push(parser.segment()?);
            parser.expect_symbol("]")?;
            continue;
        }
        break;
    }
    let mut path = FieldPath::new(segments);
    if parser.eat_word("as") {
        let alias = parser.alias_name()?;
        path.set_alias(Some(&alias))?;
    }
    parser.expect_end()?;
    Ok(path)
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
    /// An unquoted location after a word that takes one - `from`, `into`,
    /// `to`, a write verb, `create` (with or without `table` or `view`),
    /// `join`: raw text opening as a URL or a path, running to the first
    /// whitespace, `,`, `;` or `)`.
    Location(SmolStr),
    /// One punctuation token.
    Symbol(&'static str),
}

#[derive(Clone, Debug)]
struct Spanned {
    token: Token,
    position: usize,
}

/// The multi-character symbols, longest first so `<=` never reads as `<`.
///
/// No term takes `&`. It is tokenized so a bracketed location part reads
/// the raw text between its brackets - `[R&D]` - which the whole input must
/// tokenize to reach; anywhere else it is refused as an unexpected symbol.
const SYMBOLS: [&str; 23] = [
    "<>", "<=", ">=", "!=", "<", ">", "(", ")", "[", "]", "{", "}", ",", ".", ":", ";", "&", "*",
    "+", "-", "/", "%", "=",
];

fn tokenize(input: &str) -> Result<Vec<Spanned>> {
    let bytes = input.as_bytes();
    // A token is a few characters of the grammar, so this holds the run
    // without growing.
    let mut tokens = Vec::with_capacity(input.len() / 4 + 8);
    let mut cursor = 0_usize;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if byte.is_ascii_whitespace() {
            cursor += 1;
            continue;
        }
        // `--` to end of line is the one comment form, because it is the one
        // every SQL dialect agrees on and it cannot start a term.
        if byte == b'-' && bytes.get(cursor + 1) == Some(&b'-') {
            while cursor < bytes.len() && bytes[cursor] != b'\n' {
                cursor += 1;
            }
            continue;
        }
        let start = cursor;
        if let Some(end) = location_end(input, cursor, &tokens) {
            tokens.push(Spanned {
                token: Token::Location(SmolStr::new(&input[cursor..end])),
                position: start,
            });
            cursor = end;
            continue;
        }
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

/// `text` with its ASCII letters lowered, inline for the words the grammar
/// spells.
fn folded(text: &str) -> SmolStr {
    text.chars().map(|held| held.to_ascii_lowercase()).collect()
}

/// Whether an unquoted location may follow the tokens read so far: after
/// a word that introduces a target, so `price / size` after a column stays
/// a division. `table` and `view` take one only straight after `create`,
/// because either may name a column anywhere else.
fn takes_location(tokens: &[Spanned]) -> bool {
    let word = |back: usize| match tokens.len().checked_sub(back).map(|at| &tokens[at].token) {
        Some(Token::Word(word)) => Some(folded(word)),
        _ => None,
    };
    let Some(previous) = word(1) else {
        return false;
    };
    match previous.as_str() {
        "from" | "into" | "to" | "insert" | "append" | "overwrite" | "replace" | "upsert"
        | "merge" | "delete" | "create" | "join" => true,
        "table" | "view" => word(2).as_deref() == Some("create"),
        _ => false,
    }
}

/// Where the unquoted location starting at `start` ends, when one starts
/// there: after a word that takes one, raw text opening with `<scheme>://`,
/// `/`, `./`, `../`, `~/` or a drive letter, running to the first
/// whitespace, `,`, `;` or `)`.
fn location_end(input: &str, start: usize, tokens: &[Spanned]) -> Option<usize> {
    if !takes_location(tokens) {
        return None;
    }
    let rest = &input[start..];
    let length = rest
        .find(|held: char| held.is_whitespace() || matches!(held, ',' | ';' | ')'))
        .unwrap_or(rest.len());
    let text = &rest[..length];
    let bytes = text.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    let scheme = text.find("://").is_some_and(|at| {
        let scheme = &text[..at];
        scheme
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
            && scheme
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
    });
    let opens = text.starts_with('/')
        || text.starts_with("./")
        || text.starts_with("../")
        || text.starts_with("~/")
        || drive
        || scheme;
    opens.then_some(start + length)
}

/// Read a delimited run, treating a doubled delimiter as one literal character.
///
/// The doubling rule is the SQL one and it is the only escape: a backslash in
/// a literal is a backslash, which is what a Windows path and a glob both
/// need it to be.
fn read_delimited(input: &str, start: usize, delimiter: char) -> Result<(SmolStr, usize)> {
    let width = delimiter.len_utf8();
    let opened = start + width;
    // A run doubling no delimiter inside it - nearly every literal - is
    // read off the input in one piece; one that does is walked below.
    let Some(at) = input[opened..].find(delimiter) else {
        return Err(parse_error(
            start,
            format_smolstr!("expected a closing {delimiter:?}"),
        ));
    };
    let close = opened + at;
    if !input[close + width..].starts_with(delimiter) {
        return Ok((SmolStr::new(&input[opened..close]), close + width));
    }
    let mut text = String::new();
    let mut cursor = opened;
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

/// Return whether a word opens a plan section, so it cannot be a location.
fn is_section_word(word: &str) -> bool {
    matches!(
        folded(word).as_str(),
        "select"
            | "from"
            | "where"
            | "with"
            | "by"
            | "on"
            | "order"
            | "limit"
            | "offset"
            | "create"
            | "insert"
            | "upsert"
            | "merge"
            | "delete"
            | "append"
            | "overwrite"
            | "replace"
    )
}

/// The operands of a conjunction, nested ones included, in order: what a
/// join's `on` splits into, one key per equality.
fn flatten_conjunction(term: Term, out: &mut Vec<Term>) {
    match term {
        Term::And(operands) => {
            for operand in operands.iter() {
                flatten_conjunction(operand.clone(), out);
            }
        }
        other => out.push(other),
    }
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
            Some(Token::Location(text)) => format_smolstr!("the location {text}"),
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

    // -- selector -----------------------------------------------------------

    fn selector(&mut self) -> Result<Selector> {
        let mut projections = Vec::new();
        if self.eat_symbol("*") {
            // `select *` is every column, unchanged; `* exclude (a, b)` the
            // same with names left out; and a comma after either appends
            // projections to what the `*` keeps.
            let mut names = Vec::new();
            if self.eat_word("exclude") || self.eat_word("except") {
                self.expect_symbol("(")?;
                loop {
                    names.push(self.identifier()?);
                    if !self.eat_symbol(",") {
                        break;
                    }
                }
                self.expect_symbol(")")?;
            }
            if self.eat_symbol(",") {
                self.projections(&mut projections)?;
            }
            return Ok(Selector::starred(names, projections));
        }
        if self.peek().is_none() {
            return Err(parse_error(
                self.position(),
                "expected a projection or `*`, got the end of the expression",
            ));
        }
        self.projections(&mut projections)?;
        Ok(Selector::new(projections))
    }

    /// Read a comma-separated run of projections, at least one.
    fn projections(&mut self, projections: &mut Vec<Projection>) -> Result<()> {
        loop {
            projections.push(self.projection()?);
            if !self.eat_symbol(",") {
                return Ok(());
            }
        }
    }

    /// Read one projection: a term, then its alias and its declared type in
    /// either order, the way both `select ... as name` and a `create table`
    /// column definition write them.
    fn projection(&mut self) -> Result<Projection> {
        let mut projection = Projection::new(self.term()?);
        loop {
            if self.eat_word("as") {
                projection = projection.with_alias(self.identifier()?);
                continue;
            }
            if self.at_word("with") && self.peek_at(1) == Some(&Token::Symbol("(")) {
                let position = self.position();
                self.cursor += 1;
                let entries = self.properties()?;
                let metadata = crate::Metadata::try_from(
                    entries
                        .into_iter()
                        .collect::<std::collections::BTreeMap<_, _>>(),
                )
                .map_err(|error| parse_error(position, format_smolstr!("{error}")))?;
                projection = projection.with_metadata(metadata);
                continue;
            }
            if projection.dtype().is_none()
                && let Some(dtype) = self.declared_dtype()?
            {
                projection = projection.with_dtype(dtype);
                if self.eat_word("null") {
                    projection = projection.with_nullable(true);
                } else if self.at_word("not") {
                    self.cursor += 1;
                    self.expect_word("null")?;
                    projection = projection.with_nullable(false);
                }
                continue;
            }
            return Ok(projection);
        }
    }

    /// Read a datatype where a projection may declare one, without consuming
    /// anything that is not one.
    fn declared_dtype(&mut self) -> Result<Option<DataType>> {
        let Some(Token::Word(word)) = self.peek() else {
            return Ok(None);
        };
        if is_reserved(word) {
            return Ok(None);
        }
        let restore = self.cursor;
        match self.dtype() {
            Ok(dtype) => Ok(Some(dtype)),
            Err(_) => {
                self.cursor = restore;
                Ok(None)
            }
        }
    }

    // -- plan -------------------------------------------------------------

    /// Read one plan: its sections in order, each optional, at least one.
    fn plan(&mut self) -> Result<Plan> {
        let start = self.position();
        let mut plan = Plan::new();
        let mut any = false;
        if self.eat_word("create") {
            any = true;
            let _ = self.eat_word("table") || self.eat_word("view");
            let target = if self.at_symbol("(") {
                None
            } else {
                Some(self.target()?)
            };
            let schema = if self.eat_symbol("(") {
                let schema = self.selector()?;
                self.expect_symbol(")")?;
                schema
            } else {
                Selector::all()
            };
            plan = plan.create(target, schema);
            if self.at_word("with") && self.peek_at(1) == Some(&Token::Symbol("(")) {
                let position = self.position();
                self.cursor += 1;
                let entries = self.properties()?;
                let metadata = crate::Metadata::try_from(
                    entries
                        .into_iter()
                        .collect::<std::collections::BTreeMap<_, _>>(),
                )
                .map_err(|error| parse_error(position, format_smolstr!("{error}")))?;
                plan.set_root_metadata(metadata);
            }
        }
        if let Some(verb) = self.verb()? {
            any = true;
            let mut write = Write::new(verb);
            if self.at_location() {
                write = write.into(self.target()?);
            }
            if verb == Verb::Upsert && (self.eat_word("by") || self.eat_word("on")) {
                self.expect_symbol("(")?;
                let keys = self.selector()?;
                self.expect_symbol(")")?;
                write = write.by(keys);
            }
            plan = plan.write(write);
        }
        if self.eat_word("select") {
            any = true;
            plan.set_selector(self.selector()?);
        }
        if self.eat_word("from") {
            any = true;
            plan = plan.read_from(self.source()?);
        }
        while let Some(how) = self.join_kind() {
            any = true;
            let source = self.source()?;
            let keys = self.join_keys()?;
            plan = plan.join(how, source, keys)?;
        }
        if self.eat_word("where") {
            any = true;
            plan.set_filter(Filter::new(self.term()?));
        }
        if self.at_word("order") {
            self.cursor += 1;
            self.expect_word("by")?;
            any = true;
            let mut keys = Vec::new();
            loop {
                keys.push(self.ordering()?);
                if !self.eat_symbol(",") {
                    break;
                }
            }
            plan = plan.order_by(keys);
        }
        // `limit` and `offset` are read in either order, since engines
        // differ on which comes first; the canonical text puts `limit` first.
        loop {
            if plan.row_limit().is_none() && self.eat_word("limit") {
                any = true;
                plan = plan.limit(Some(self.count()?));
            } else if plan.row_offset().is_none() && self.eat_word("offset") {
                any = true;
                plan = plan.offset(Some(self.count()?));
            } else {
                break;
            }
        }
        if !any {
            return Err(super::unknown_clause(self.input[start..].trim()));
        }
        Ok(plan)
    }

    /// Read the words that open a join, answering the kind they name;
    /// nothing, and nothing consumed, when no join opens here.
    ///
    /// A kind word counts only before `join` - or `outer join` - so a word
    /// that happens to spell a kind is never taken for one.
    fn join_kind(&mut self) -> Option<JoinKind> {
        if self.eat_word("join") {
            return Some(JoinKind::Inner);
        }
        let Some(Token::Word(word)) = self.peek() else {
            return None;
        };
        let how = match folded(word).as_str() {
            "inner" => JoinKind::Inner,
            "left" => JoinKind::Left,
            "right" => JoinKind::Right,
            "full" | "outer" => JoinKind::Full,
            "semi" => JoinKind::Semi,
            "anti" => JoinKind::Anti,
            _ => return None,
        };
        // `outer` is a kind of its own and the optional word after `left`,
        // `right` and `full`; it never follows another kind.
        let outer = matches!(how, JoinKind::Left | JoinKind::Right)
            || (how == JoinKind::Full && !self.at_word("outer"));
        let words = if outer && self.word_at(1, "outer") {
            2
        } else {
            1
        };
        if !self.word_at(words, "join") {
            return None;
        }
        self.cursor += words + 1;
        Some(how)
    }

    /// Read a join's keys: `on` equalities joined by `and`, or `using` and
    /// the names both sides share.
    fn join_keys(&mut self) -> Result<JoinKeys> {
        if self.eat_word("using") {
            self.expect_symbol("(")?;
            let mut keys = Vec::new();
            loop {
                keys.push(JoinKey::using(Term::column(self.identifier()?)));
                if !self.eat_symbol(",") {
                    break;
                }
            }
            self.expect_symbol(")")?;
            return Ok(JoinKeys::new(keys));
        }
        if !self.eat_word("on") {
            return Err(parse_error(
                self.position(),
                format_smolstr!(
                    "expected `on` or `using` and the keys a join matches on, got {}",
                    self.describe()
                ),
            ));
        }
        let position = self.position();
        let mut conjuncts = Vec::new();
        flatten_conjunction(self.term()?, &mut conjuncts);
        conjuncts
            .into_iter()
            .map(|conjunct| match conjunct {
                Term::Compare(_, Comparison::Eq, _) => JoinKey::from_term(conjunct),
                other => Err(parse_error(
                    position,
                    format_smolstr!(
                        "expected a join's `on` to be equalities joined by `and`, each a left \
                         term = a right term, got `{other}`"
                    ),
                )),
            })
            .collect::<Result<Vec<_>>>()
            .map(JoinKeys::new)
    }

    /// Read a write verb in any spelling this grammar reads, answering its
    /// canonical one; nothing when no verb is here.
    fn verb(&mut self) -> Result<Option<Verb>> {
        // The word after the verb - `into`, `to`, `from` - introduces a
        // target and is optional, since a write with no target writes to the
        // handle the plan is given to.
        if self.eat_word("insert") {
            if self.eat_word("overwrite") {
                let _ = self.eat_word("into");
                return Ok(Some(Verb::Overwrite));
            }
            let _ = self.eat_word("into");
            return Ok(Some(Verb::Insert));
        }
        if self.eat_word("append") {
            let _ = self.eat_word("into") || self.eat_word("to");
            return Ok(Some(Verb::Insert));
        }
        if self.eat_word("overwrite") || self.eat_word("replace") {
            let _ = self.eat_word("into");
            return Ok(Some(Verb::Overwrite));
        }
        if self.eat_word("upsert") || self.eat_word("merge") {
            let _ = self.eat_word("into");
            return Ok(Some(Verb::Upsert));
        }
        if self.eat_word("delete") {
            let _ = self.eat_word("from");
            return Ok(Some(Verb::Delete));
        }
        Ok(None)
    }

    /// Return whether a location starts here: a quoted URL, a name, a quoted
    /// name, a bracketed name, or a number.
    fn at_location(&self) -> bool {
        match self.peek() {
            Some(Token::Text(_) | Token::Quoted(_) | Token::Number(_) | Token::Location(_)) => true,
            Some(Token::Symbol("[")) => true,
            Some(Token::Word(word)) => !is_section_word(word),
            _ => false,
        }
    }

    /// Read one target: a location and its `with (...)` properties.
    fn target(&mut self) -> Result<Target> {
        let mut target = Target::new(self.location()?);
        if self.at_word("with") && self.peek_at(1) == Some(&Token::Symbol("(")) {
            self.cursor += 1;
            target = target.with_properties(self.properties()?);
        }
        Ok(target)
    }

    /// Read one location: a quoted URL, or a dotted catalog path.
    fn location(&mut self) -> Result<Location> {
        let position = self.position();
        if let Some(Token::Text(text)) = self.peek().cloned() {
            self.cursor += 1;
            return Url::from_str(&text)
                .map(Location::Url)
                .map_err(|error| parse_error(position, format_smolstr!("{error}")));
        }
        if let Some(Token::Location(text)) = self.peek().cloned() {
            self.cursor += 1;
            return Url::from_location(&text)
                .map(Location::Url)
                .map_err(|error| parse_error(position, format_smolstr!("{error}")));
        }
        let mut parts = vec![self.part()?];
        while self.eat_symbol(".") {
            parts.push(self.part()?);
        }
        Ok(Location::Parts(parts))
    }

    /// Read one part of a catalog path, quoted any way an engine quotes it.
    fn part(&mut self) -> Result<SmolStr> {
        let position = self.position();
        match self.peek().cloned() {
            Some(Token::Word(word)) if !is_section_word(&word) => {
                self.cursor += 1;
                Ok(word)
            }
            Some(Token::Quoted(name) | Token::Number(name)) => {
                self.cursor += 1;
                Ok(name)
            }
            Some(Token::Symbol("[")) => {
                // A bracketed name is read from the text itself, so whatever
                // the brackets hold - spaces, dots, dashes - is one part.
                let open = self.cursor;
                let close = (open + 1..self.tokens.len())
                    .find(|index| matches!(self.tokens[*index].token, Token::Symbol("]")))
                    .ok_or_else(|| parse_error(position, "expected a closing \"]\""))?;
                let text = self.input[self.token_end(open)..self.tokens[close].position].trim();
                if text.is_empty() {
                    return Err(parse_error(position, "expected a name inside the brackets"));
                }
                self.cursor = close + 1;
                Ok(SmolStr::new(text))
            }
            _ => Err(parse_error(
                position,
                format_smolstr!(
                    "expected a location - a quoted URL or a catalog path - got {}",
                    self.describe()
                ),
            )),
        }
    }

    /// Read one source: a target, or a plan in parentheses.
    fn source(&mut self) -> Result<Source> {
        if self.eat_symbol("(") {
            self.enter()?;
            let plan = self.plan();
            self.leave();
            let plan = plan?;
            self.expect_symbol(")")?;
            return Ok(Source::Plan(Box::new(plan)));
        }
        Ok(Source::Target(self.target()?))
    }

    /// Read `(name = 'value', ...)`, the `with` keyword already consumed.
    fn properties(&mut self) -> Result<Vec<(String, String)>> {
        self.expect_symbol("(")?;
        let mut properties = Vec::new();
        loop {
            let position = self.position();
            let mut name = match self.peek().cloned() {
                Some(Token::Word(word) | Token::Quoted(word) | Token::Number(word)) => {
                    self.cursor += 1;
                    word.to_string()
                }
                _ => {
                    return Err(parse_error(
                        position,
                        format_smolstr!("expected a property name, got {}", self.describe()),
                    ));
                }
            };
            while self.eat_symbol(".") {
                name.push('.');
                name.push_str(&self.step_name()?);
            }
            self.expect_symbol("=")?;
            let position = self.position();
            let value = match self.peek().cloned() {
                Some(Token::Text(text) | Token::Number(text) | Token::Word(text)) => {
                    self.cursor += 1;
                    text.to_string()
                }
                _ => {
                    return Err(parse_error(
                        position,
                        format_smolstr!("expected a property value, got {}", self.describe()),
                    ));
                }
            };
            properties.push((name, value));
            if !self.eat_symbol(",") {
                break;
            }
        }
        self.expect_symbol(")")?;
        Ok(properties)
    }

    /// Read one `order by` key.
    fn ordering(&mut self) -> Result<Ordering> {
        let term = self.term()?;
        let mut key = if self.eat_word("desc") {
            Ordering::desc(term)
        } else {
            let _ = self.eat_word("asc");
            Ordering::asc(term)
        };
        if self.eat_word("nulls") {
            if self.eat_word("first") {
                key = key.nulls_first(true);
            } else {
                self.expect_word("last")?;
            }
        }
        Ok(key)
    }

    /// Read one whole number, for a `limit` or an `offset`.
    fn count(&mut self) -> Result<u64> {
        let position = self.position();
        match self.peek().cloned() {
            Some(Token::Number(text)) => {
                self.cursor += 1;
                text.parse().map_err(|_| {
                    parse_error(
                        position,
                        format_smolstr!("expected a row count, got {text}"),
                    )
                })
            }
            _ => Err(parse_error(
                position,
                format_smolstr!("expected a row count, got {}", self.describe()),
            )),
        }
    }

    // -- term ---------------------------------------------------------------

    fn term(&mut self) -> Result<Term> {
        self.enter()?;
        let parsed = self.disjunction();
        self.leave();
        parsed
    }

    fn disjunction(&mut self) -> Result<Term> {
        let mut operands = vec![self.conjunction()?];
        while self.eat_word("or") {
            operands.push(self.conjunction()?);
        }
        Ok(if operands.len() == 1 {
            operands.swap_remove(0)
        } else {
            Term::any(operands)
        })
    }

    fn conjunction(&mut self) -> Result<Term> {
        let mut operands = vec![self.negation()?];
        while self.eat_word("and") {
            operands.push(self.negation()?);
        }
        Ok(if operands.len() == 1 {
            operands.swap_remove(0)
        } else {
            Term::all(operands)
        })
    }

    fn negation(&mut self) -> Result<Term> {
        if self.eat_word("not") {
            self.enter()?;
            let inner = self.negation();
            self.leave();
            return Ok(inner?.not());
        }
        self.predicate()
    }

    #[allow(clippy::too_many_lines)]
    fn predicate(&mut self) -> Result<Term> {
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
            let comparison = self.distinct_from(negated)?;
            let right = self.additive()?;
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
                list.push(self.term()?);
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
            Term::Like {
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

    /// Read one comparison where one is expected: a symbol, or `is [not]
    /// distinct from`.
    fn comparison(&mut self) -> Result<Comparison> {
        if let Some(comparison) = self.comparison_symbol() {
            return Ok(comparison);
        }
        if self.eat_word("is") {
            let negated = self.eat_word("not");
            return self.distinct_from(negated);
        }
        Err(parse_error(
            self.position(),
            format_smolstr!(
                "expected a comparison - =, <>, !=, <, <=, >, >= or is [not] distinct from - \
                 got {}",
                self.describe()
            ),
        ))
    }

    /// The rest of `is [not] distinct from`, its `is` and `not` already read.
    fn distinct_from(&mut self, negated: bool) -> Result<Comparison> {
        self.expect_word("distinct")?;
        self.expect_word("from")?;
        Ok(if negated {
            Comparison::IsNotDistinctFrom
        } else {
            Comparison::IsDistinctFrom
        })
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

    fn additive(&mut self) -> Result<Term> {
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

    fn product(&mut self) -> Result<Term> {
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

    fn unary(&mut self) -> Result<Term> {
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

    fn accessor(&mut self) -> Result<Term> {
        let mut base = self.atom()?;
        loop {
            if self.eat_symbol(".") {
                let name = self.step_name()?;
                base = base.child(name);
                continue;
            }
            if self.at_symbol("[") {
                // A `[` after a value is a path step; a `[` that starts a value
                // was already consumed by `atom`.
                let position = self.position();
                self.cursor += 1;
                let segment = self.segment()?;
                self.expect_symbol("]")?;
                if !matches!(base, Term::Path(_)) && segment.as_predicate().is_some() {
                    return Err(parse_error(
                        position,
                        "expected a column path before a predicate segment, got a computed \
                         value; a predicate keeps the elements of the serie a column holds",
                    ));
                }
                base = base.path([segment])?;
                continue;
            }
            return Ok(base);
        }
    }

    /// Read one path step: an integer position, a run of positions, a text
    /// key, or - anything else - a predicate over the elements.
    fn segment(&mut self) -> Result<FieldSegment> {
        // A run: `[:]`, `[:3]`, `[1:]`, `[1:3]`, either bound negative.
        if self.eat_symbol(":") {
            let end = self.optional_position()?;
            return Ok(FieldSegment::Range { start: None, end });
        }
        let opened = self.cursor;
        if let Some(start) = self.optional_position()? {
            if self.eat_symbol(":") {
                let end = self.optional_position()?;
                return Ok(FieldSegment::Range {
                    start: Some(start),
                    end,
                });
            }
            if self.at_symbol("]") {
                return Ok(FieldSegment::Index(start));
            }
            // A whole number that opens a longer term is that term's first
            // operand, so the term is read from where the bracket opened.
            self.cursor = opened;
        }
        let position = self.position();
        let term = self.term()?;
        Ok(match term {
            // A text constant names a map entry or a struct child; every
            // other term - a bare boolean column included - is a predicate.
            Term::Literal(held) if held.value().as_str().is_some() => FieldSegment::Key(held),
            // A bare constant that is no boolean can never keep an element,
            // so it is refused here, where the position is still known.
            Term::Literal(held)
                if !held.is_null() && !matches!(held.dtype(), DataType::Boolean) =>
            {
                return Err(parse_error(
                    position,
                    format_smolstr!(
                        "expected a whole serie position, a text key, or a predicate, got {held}"
                    ),
                ));
            }
            predicate => FieldSegment::filter(predicate),
        })
    }

    /// Read one optionally negative whole position, when one is here.
    fn optional_position(&mut self) -> Result<Option<i64>> {
        let position = self.position();
        let negative = self.at_symbol("-")
            && matches!(self.peek_at(1), Some(Token::Number(text)) if !text.contains(['.', 'e', 'E']));
        if negative {
            self.cursor += 1;
        }
        let Some(Token::Number(text)) = self.peek().cloned() else {
            return Ok(None);
        };
        // A number that is not whole is no position; it is read as the
        // term it opens, and refused there when it opens none.
        if text.contains(['.', 'e', 'E']) {
            return Ok(None);
        }
        self.cursor += 1;
        let magnitude = text.parse::<i64>().map_err(|_| {
            parse_error(
                position,
                format_smolstr!("expected a serie position that fits in 64 bits, got {text}"),
            )
        })?;
        Ok(Some(if negative { -magnitude } else { magnitude }))
    }

    /// Read one name where the grammar admits only a name: an alias, a struct
    /// child in a constructor, a column.
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

    /// Read one name after a dot, where any word is a name.
    ///
    /// A step is unambiguous: nothing but a name can follow a dot, so a child
    /// called `as` or `select` is reachable without quotes.
    fn step_name(&mut self) -> Result<SmolStr> {
        let position = self.position();
        match self.peek().cloned() {
            Some(Token::Quoted(name) | Token::Word(name) | Token::Number(name)) => {
                self.cursor += 1;
                Ok(name)
            }
            _ => Err(parse_error(
                position,
                format_smolstr!("expected a segment name, got {}", self.describe()),
            )),
        }
    }

    /// Read one alias, bare or quoted either way.
    ///
    /// Wider at intake than a name is, because an alias is written by hand
    /// and both quote styles are spellings people reach for. It still renders
    /// back one way.
    fn alias_name(&mut self) -> Result<SmolStr> {
        if let Some(Token::Text(text)) = self.peek().cloned() {
            self.cursor += 1;
            return Ok(text);
        }
        self.identifier()
    }

    #[allow(clippy::too_many_lines)]
    fn atom(&mut self) -> Result<Term> {
        let position = self.position();
        if self.eat_symbol("(") {
            let inner = self.term()?;
            self.expect_symbol(")")?;
            return Ok(inner);
        }
        if self.eat_symbol("[") {
            let mut items = Vec::new();
            if !self.at_symbol("]") {
                loop {
                    items.push(self.term()?);
                    if !self.eat_symbol(",") {
                        break;
                    }
                }
            }
            self.expect_symbol("]")?;
            return Ok(Term::Serie(Arc::from(items)));
        }
        if self.eat_symbol("{") {
            let mut entries = Vec::new();
            if !self.at_symbol("}") {
                loop {
                    let key = self.term()?;
                    self.expect_symbol(":")?;
                    let value = self.term()?;
                    entries.push((key, value));
                    if !self.eat_symbol(",") {
                        break;
                    }
                }
            }
            self.expect_symbol("}")?;
            return Ok(Term::Map(Arc::from(entries)));
        }
        if self.eat_symbol(":") {
            return Ok(Term::parameter(self.identifier()?));
        }
        match self.peek().cloned() {
            Some(Token::Number(text)) => {
                self.cursor += 1;
                return number_literal(&text, position);
            }
            Some(Token::Text(text)) => {
                self.cursor += 1;
                return Ok(Term::literal(Scalar::from(text)));
            }
            Some(Token::Quoted(name)) => {
                self.cursor += 1;
                return Ok(Term::column(name));
            }
            _ => {}
        }
        let Some(Token::Word(word)) = self.peek().cloned() else {
            return Err(parse_error(
                position,
                format_smolstr!("expected a value or a name, got {}", self.describe()),
            ));
        };
        // Folded into an inline string: a word of the grammar is short, and
        // the fold allocates nothing for it.
        let lowered = folded(&word);
        match lowered.as_str() {
            "null" => {
                self.cursor += 1;
                return Ok(Term::literal(Scalar::Null));
            }
            "true" => {
                self.cursor += 1;
                return Ok(Term::literal(Scalar::from(true)));
            }
            "false" => {
                self.cursor += 1;
                return Ok(Term::literal(Scalar::from(false)));
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
        // `namespace.name(` is a user-defined function: registered outside
        // the grammar, so the name is read here and resolved where it binds.
        let user_name = match (self.peek_at(1), self.peek_at(2), self.peek_at(3)) {
            (Some(Token::Symbol(".")), Some(Token::Word(name)), Some(Token::Symbol("("))) => {
                Some(name.clone())
            }
            _ => None,
        };
        if let Some(name) = user_name {
            let reference = UserRef::new(&word, &name)
                .map_err(|error| parse_error(position, format_smolstr!("{error}")))?;
            self.cursor += 3;
            let arguments = self.arguments()?;
            return Ok(Term::call(Function::User(reference), arguments));
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
                return Ok(Term::call(function, arguments));
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
        if matches!(self.peek_at(1), Some(Token::Text(_) | Token::Word(_)))
            && let Some(literal) = self.typed_literal(position)?
        {
            return Ok(literal);
        }
        self.cursor += 1;
        Ok(Term::column(word))
    }

    fn arguments(&mut self) -> Result<Vec<Term>> {
        self.expect_symbol("(")?;
        let mut arguments = Vec::new();
        if !self.at_symbol(")") {
            loop {
                arguments.push(self.term()?);
                if !self.eat_symbol(",") {
                    break;
                }
            }
        }
        self.expect_symbol(")")?;
        Ok(arguments)
    }

    fn cast(&mut self, keyword: &str) -> Result<Term> {
        self.expect_symbol("(")?;
        let inner = self.term()?;
        self.expect_word("as")?;
        let dtype = self.dtype()?;
        self.expect_symbol(")")?;
        Ok(Term::Cast(
            Box::new(inner),
            dtype,
            if keyword == "try_cast" {
                Safety::Safe
            } else {
                Safety::Strict
            },
        ))
    }

    fn case(&mut self) -> Result<Term> {
        let position = self.position();
        let mut branches = Vec::new();
        while self.eat_word("when") {
            let when = self.term()?;
            self.expect_word("then")?;
            let then = self.term()?;
            branches.push((when, then));
        }
        if branches.is_empty() {
            return Err(parse_error(position, "expected at least one `when` branch"));
        }
        let otherwise = if self.eat_word("else") {
            Some(self.term()?)
        } else {
            None
        };
        self.expect_word("end")?;
        Ok(Term::case(branches, otherwise))
    }

    fn structure(&mut self) -> Result<Term> {
        self.expect_symbol("(")?;
        let mut children = Vec::new();
        if !self.at_symbol(")") {
            loop {
                let value = self.term()?;
                self.expect_word("as")?;
                let name = self.identifier()?;
                children.push((name, value));
                if !self.eat_symbol(",") {
                    break;
                }
            }
        }
        self.expect_symbol(")")?;
        Ok(Term::Struct(Arc::from(children)))
    }

    /// Read one datatype through the crate's own datatype grammar.
    ///
    /// The datatype text is taken verbatim from the input rather than rebuilt
    /// from tokens, so there is exactly one datatype parser in the crate and
    /// this module never learns what a datatype looks like.
    fn dtype(&mut self) -> Result<DataType> {
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
        } else if self.at_symbol("<") {
            self.skip_angled()?
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

    /// Consume a balanced `<...>` run, the nested datatype spelling, and
    /// answer the byte just past it.
    fn skip_angled(&mut self) -> Result<usize> {
        let opened = self.position();
        let mut depth = 0_usize;
        loop {
            match self.peek() {
                Some(Token::Symbol("<")) => depth += 1,
                Some(Token::Symbol(">")) => {
                    depth -= 1;
                    if depth == 0 {
                        let end = self.token_end(self.cursor);
                        self.cursor += 1;
                        return Ok(end);
                    }
                }
                // `>=` and `<>` never occur inside a datatype, so either one
                // is the end of the run this parser was asked to skip.
                Some(Token::Symbol(">=" | "<>" | "<=")) | None => {
                    return Err(parse_error(opened, "expected a closing \">\""));
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
    fn typed_literal(&mut self, position: usize) -> Result<Option<Term>> {
        let restore = self.cursor;
        let Ok(dtype) = self.dtype() else {
            self.cursor = restore;
            return Ok(None);
        };
        match self.peek().cloned() {
            Some(Token::Text(text)) => {
                self.cursor += 1;
                let value = value_from_text(&dtype, &text, position)?;
                Ok(Some(Term::Literal(Literal::new(dtype, value).map_err(
                    |error| parse_error(position, format_smolstr!("{error}")),
                )?)))
            }
            Some(Token::Word(word)) if word.eq_ignore_ascii_case("null") => {
                self.cursor += 1;
                Ok(Some(Term::Literal(
                    Literal::new(dtype, Scalar::Null)
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
fn number_literal(text: &str, position: usize) -> Result<Term> {
    if text.contains(['.', 'e', 'E']) {
        let held = text.parse::<f64>().map_err(|_| {
            parse_error(
                position,
                format_smolstr!("expected a 64-bit floating-point number, got {text}"),
            )
        })?;
        return Ok(Term::literal(held));
    }
    let held = text.parse::<i64>().map_err(|_| {
        parse_error(
            position,
            format_smolstr!("expected a whole number that fits in 64 bits, got {text}"),
        )
    })?;
    Ok(Term::literal(held))
}

/// Read one value out of the text half of a typed literal.
///
/// The text forms are the crate's own: ISO 8601 for every temporal, an exact
/// decimal string for a decimal, lowercase hex for binary. Nothing here is a
/// second value parser - each family delegates to the reader its root file
/// owns, the one a column of text is cast through.
pub(crate) fn value_from_text(dtype: &DataType, text: &str, position: usize) -> Result<Scalar> {
    use DataType as D;

    let fail = |expected: &str| {
        parse_error(
            position,
            format_smolstr!("expected {expected}, got {text:?}"),
        )
    };
    let value = match dtype {
        D::Null => Scalar::Null,
        D::Boolean => boolean_from_text(text).ok_or_else(|| fail(BOOLEAN_SPELLINGS))?,
        // Every width reads the one integer spelling; the closing conversion
        // narrows it to the declared width and refuses a magnitude or a sign
        // that width cannot hold.
        D::Int8 | D::Int16 | D::Int32 | D::Int64 | D::UInt8 | D::UInt16 | D::UInt32 | D::UInt64 => {
            integer_from_text(text).ok_or_else(|| fail(INTEGER_SPELLINGS))?
        }
        // A temporal literal is its classic spelling, never a raw count: the
        // count is a physical detail and the literal is what a person wrote.
        // The reading is the crate's one text reading, so a literal and a
        // cast of the same text land on the same value.
        D::Date32
        | D::Date64
        | D::Time32(_)
        | D::Time64(_)
        | D::DateTime64 { .. }
        | D::Duration32(_)
        | D::Duration64(_) => Scalar::from_temporal_text(dtype, text)?,
        // Every width reads the one float spelling; the closing conversion
        // rounds it to the declared width.
        D::Float16 | D::Float32 | D::Float64 => {
            float_from_text(text).ok_or_else(|| fail(FLOAT_SPELLINGS))?
        }
        // Every width and both fixed leaves read the one decimal spelling at
        // the declared scale, a digit past it refused; the closing conversion
        // holds the coefficient to the declared precision.
        D::Decimal32 { .. }
        | D::Decimal64 { .. }
        | D::Decimal128 { .. }
        | D::Decimal256 { .. }
        | D::Decimal
        | D::BigDecimal => Scalar::from_decimal_text(dtype, text)
            .map_err(|_| fail("an exact decimal that fits the declared precision and scale"))?,
        crate::string_dtypes!() | D::Uuid | D::Version => Scalar::from(SmolStr::new(text)),
        code if code.is_code() => Scalar::from(SmolStr::new(text)),
        // An enum literal is a spelling its leaf's value door reads.
        held if held.is_enum() => Scalar::from(SmolStr::new(text)),
        crate::bytes_dtypes!() => Scalar::from(
            crate::bytes::bytes_from_hex(text)
                .ok_or_else(|| fail("an even-length run of hex digits"))?,
        ),
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
    super::eval::convert(dtype, &value, super::Safety::Strict)
        .map_err(|error| parse_error(position, format_smolstr!("{error}")))
}

/// Return whether a name needs quoting to survive a round trip.
///
/// Exposed for the bindings, which build terms from caller-supplied column
/// names and must not have to guess this rule.
#[must_use]
pub fn needs_quoting(name: &str) -> bool {
    !is_bare_identifier(name)
}
