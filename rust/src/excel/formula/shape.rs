//! A formula's shape: its text as verbatim runs and references relative to
//! the cell holding it, independent of that cell.
//!
//! Two cells whose formulas differ only by where they stand - `=A1*B1` in
//! `C1` and `=A2*B2` in `C2` - hold one shape, so a sheet parses each shape
//! once and every cell of a filled column, or every dependent of a shared
//! formula, holds the same [`Arc`](std::sync::Arc). Only the references are
//! modelled: every other character is a run of the text as it was written,
//! so a user's spacing and literal spelling survive every edit, and a
//! rename rewrites the prefixes of the references that name the sheet and
//! nothing else.

use std::fmt;
use std::sync::OnceLock;

use smallvec::SmallVec;
use smol_str::SmolStr;

use crate::excel::cell::CellRef;

use super::functions::Function;
use super::lexer::{Kind, Lexeme, Prefix, RawCoord, RawReference, RawSheet, RawTarget};
use super::parser::{self, Expr};
use super::reference::{Coord, Reference, SheetSpec, Target, same_sheet};

/// One piece of a shape, in text order.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Token {
    /// A run of the text exactly as written.
    Text(SmolStr),
    /// A cell reference relative to the host, or a defined name.
    Reference(Reference),
    /// A function's name: the prefix the file stores it under, and the name
    /// a user types. The `(` after it is text.
    Function { prefix: Prefix, name: SmolStr },
    /// `_xlfn.SINGLE(`: implicit intersection, which entry spells `@`, or
    /// `@(` when what it applies to is not one operand.
    Single { wrapped: bool },
    /// The `)` closing a [`Token::Single`]: nothing in entry, or `)` when
    /// wrapped.
    SingleEnd { wrapped: bool },
}

/// Why a formula is carried and never computed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Held {
    /// A reference into another workbook: `[1]Sheet1!A1`.
    External,
    /// A structured reference into a table: `Table1[Column]`.
    Structured,
    /// A spilled range: `A1#`.
    Spill,
    /// A function that only a dynamic array states: `_xlfn.ANCHORARRAY`.
    DynamicArray,
    /// An unknown or prefix-only function this engine cannot compute.
    UnknownFunction,
    /// Text the grammar does not read.
    Unrecognized,
}

impl Held {
    /// The reason as a refusal or a status names it.
    #[cfg(feature = "internals")]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::External => "a reference into another workbook",
            Self::Structured => "a structured reference into a table",
            Self::Spill => "a spilled range reference",
            Self::DynamicArray => "a dynamic array's own function",
            Self::UnknownFunction => "a function this engine does not implement",
            Self::Unrecognized => "text the formula grammar does not read",
        }
    }
}

/// A formula's shape.
#[derive(Debug)]
pub(crate) struct Shape {
    pub(crate) tokens: Box<[Token]>,
    /// The sheets its references name, each once, in text order: the
    /// filter a rename or a removal reads before rewriting anything.
    pub(crate) sheets: SmallVec<[SmolStr; 1]>,
    /// Whether it calls a volatile function.
    pub(crate) volatile: bool,
    /// Why it is never computed, the first reason in text order.
    pub(crate) held: Option<Held>,
    /// One arena per shared shape. It is not part of shape identity.
    compiled: OnceLock<std::result::Result<Expr, Held>>,
}

impl PartialEq for Shape {
    fn eq(&self, other: &Self) -> bool {
        self.tokens == other.tokens
    }
}

impl Eq for Shape {}

impl std::hash::Hash for Shape {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.tokens.hash(state);
    }
}

impl Shape {
    /// Parse only when computation asks. File intake itself is total.
    pub(crate) fn compiled(&self) -> &std::result::Result<Expr, Held> {
        self.compiled.get_or_init(|| parser::compile(&self.tokens))
    }

    /// Whether the lazy arena exists, exposed only to the mirrored test.
    #[cfg(feature = "internals")]
    pub(crate) fn is_cached(&self) -> bool {
        self.compiled.get().is_some()
    }

    /// The first reason this shape cannot be computed.
    pub(crate) fn held_reason(&self) -> Option<Held> {
        self.held
            .or_else(|| self.compiled().as_ref().err().copied())
    }

    /// Seed a validated entry's tree when canonical file spelling has the
    /// same grammar. A rewritten shape always gets a fresh OnceLock.
    pub(crate) fn install(&mut self, expression: Expr) {
        let _ = self.compiled.set(Ok(expression));
    }

    /// The shape of `tokens`: the facts every reader asks read once.
    pub(crate) fn of(tokens: Vec<Token>) -> Self {
        let mut sheets: SmallVec<[SmolStr; 1]> = SmallVec::new();
        let mut volatile = false;
        let mut held = None;
        let mut note = |name: &SmolStr| {
            if !sheets.iter().any(|held| same_sheet(held, name)) {
                sheets.push(name.clone());
            }
        };
        for token in &tokens {
            match token {
                Token::Reference(reference) => match &reference.sheet {
                    SheetSpec::Named { name, .. } => note(name),
                    SheetSpec::Span { first, last, .. } => {
                        note(first);
                        note(last);
                    }
                    SheetSpec::External(_) => {
                        held.get_or_insert(Held::External);
                    }
                    SheetSpec::Own | SheetSpec::Invalid => {}
                },
                Token::Function { name, .. } => match Function::lookup(name) {
                    Some(function) => {
                        let info = function.info();
                        volatile |= info.volatile;
                        if info.signature.is_none() {
                            let reason = if function == Function::Anchorarray {
                                Held::DynamicArray
                            } else {
                                Held::UnknownFunction
                            };
                            held.get_or_insert(reason);
                        }
                    }
                    None => {
                        held.get_or_insert(Held::UnknownFunction);
                    }
                },
                Token::Text(_) | Token::Single { .. } | Token::SingleEnd { .. } => {}
            }
        }
        Self {
            tokens: tokens.into_boxed_slice(),
            sheets,
            volatile,
            held,
            compiled: OnceLock::new(),
        }
    }

    /// The shape of the lexed `lexemes` at `host`, as a file spells them:
    /// every run kept as written. Answers the first reason it is held for,
    /// beyond what [`Self::of`] reads off the tokens.
    pub(crate) fn from_lexemes(lexemes: &[Lexeme<'_>], host: CellRef) -> Self {
        let mut builder = Builder::default();
        let singles = singles(lexemes);
        let mut held = None;
        for (at, lexeme) in lexemes.iter().enumerate() {
            match &lexeme.kind {
                Kind::Reference(reference) => {
                    builder.token(Token::Reference(relative(reference, host)));
                }
                Kind::Name => builder.token(Token::Reference(Reference {
                    sheet: SheetSpec::Own,
                    target: Target::Name(SmolStr::new(lexeme.text)),
                })),
                Kind::Function { prefix, name } => {
                    if let Some(single) = singles.iter().find(|single| single.name == at) {
                        builder.token(Token::Single {
                            wrapped: single.wrapped,
                        });
                        continue;
                    }
                    builder.token(Token::Function {
                        prefix: *prefix,
                        name: name.clone(),
                    });
                }
                Kind::Open if singles.iter().any(|single| single.name + 1 == at) => {}
                Kind::Close => match singles.iter().find(|single| single.close == at) {
                    Some(single) => builder.token(Token::SingleEnd {
                        wrapped: single.wrapped,
                    }),
                    None => builder.text(lexeme.text),
                },
                Kind::Structured => {
                    held.get_or_insert(Held::Structured);
                    builder.text(lexeme.text);
                }
                Kind::Spill => {
                    held.get_or_insert(Held::Spill);
                    builder.text(lexeme.text);
                }
                Kind::Opaque | Kind::String { closed: false } => {
                    held.get_or_insert(Held::Unrecognized);
                    builder.text(lexeme.text);
                }
                _ => builder.text(lexeme.text),
            }
        }
        let mut shape = Self::of(builder.finish());
        if shape.held.is_none() {
            shape.held = held;
        }
        shape
    }

    /// The shape with every reference naming the sheet `from` naming `to`
    /// instead, `None` when none names it.
    pub(crate) fn renamed(&self, from: &str, to: &str) -> Option<Self> {
        if !self.sheets.iter().any(|held| same_sheet(held, from)) {
            return None;
        }
        let rename = |name: &SmolStr| {
            if same_sheet(name, from) {
                SmolStr::new(to)
            } else {
                name.clone()
            }
        };
        let tokens = self
            .tokens
            .iter()
            .map(|token| match token {
                Token::Reference(reference) => Token::Reference(Reference {
                    sheet: match &reference.sheet {
                        SheetSpec::Named { name, quoted } if same_sheet(name, from) => {
                            SheetSpec::Named {
                                name: SmolStr::new(to),
                                // The file's quoting stood for the old name.
                                quoted: *quoted && super::reference::needs_quotes(to),
                            }
                        }
                        SheetSpec::Span {
                            first,
                            last,
                            quoted,
                        } => SheetSpec::Span {
                            first: rename(first),
                            last: rename(last),
                            quoted: *quoted,
                        },
                        other => other.clone(),
                    },
                    target: reference.target.clone(),
                }),
                other => other.clone(),
            })
            .collect();
        Some(Self::of_held(tokens, self.held))
    }

    /// The shape with every reference to the sheet `removed` made `#REF!`,
    /// a span of sheets ending on it drawn in to the next sheet inside the
    /// span in `order` (the tabs before the removal); `None` when no
    /// reference names it.
    pub(crate) fn removed(&self, removed: &str, order: &[&str]) -> Option<Self> {
        if !self.sheets.iter().any(|held| same_sheet(held, removed)) {
            return None;
        }
        let position = |name: &str| order.iter().position(|held| same_sheet(held, name));
        let tokens = self
            .tokens
            .iter()
            .map(|token| match token {
                Token::Reference(reference) if reference.sheet.names(removed) => {
                    Token::Reference(Reference {
                        sheet: match &reference.sheet {
                            SheetSpec::Span {
                                first,
                                last,
                                quoted,
                            } => match (position(first), position(last)) {
                                (Some(start), Some(end)) if start != end => {
                                    // The endpoint moves one tab inward.
                                    let step: isize = if start < end { 1 } else { -1 };
                                    let inward = |at: usize| {
                                        SmolStr::new(order[(at as isize + step) as usize])
                                    };
                                    let back = |at: usize| {
                                        SmolStr::new(order[(at as isize - step) as usize])
                                    };
                                    let (first, last) = if same_sheet(first, removed) {
                                        (inward(start), last.clone())
                                    } else {
                                        (first.clone(), back(end))
                                    };
                                    if same_sheet(&first, &last) {
                                        SheetSpec::Named {
                                            quoted: super::reference::needs_quotes(&first),
                                            name: first,
                                        }
                                    } else {
                                        SheetSpec::Span {
                                            first,
                                            last,
                                            quoted: *quoted,
                                        }
                                    }
                                }
                                _ => SheetSpec::Invalid,
                            },
                            _ => SheetSpec::Invalid,
                        },
                        target: reference.target.clone(),
                    })
                }
                other => other.clone(),
            })
            .collect();
        Some(Self::of_held(tokens, self.held))
    }

    /// This shape's reason to be held, over other `tokens`: what a rewrite
    /// of its references answers.
    pub(crate) fn with_tokens(&self, tokens: Vec<Token>) -> Self {
        Self::of_held(tokens, self.held)
    }

    /// [`Self::of`], keeping a reason `held` the tokens alone do not show.
    fn of_held(tokens: Vec<Token>, held: Option<Held>) -> Self {
        let mut shape = Self::of(tokens);
        if shape.held.is_none() {
            shape.held = held;
        }
        shape
    }

    /// Write the shape at `host`: the file spelling, or what a user types.
    pub(crate) fn write(
        &self,
        formatter: &mut fmt::Formatter<'_>,
        host: CellRef,
        entry: bool,
    ) -> fmt::Result {
        for token in &*self.tokens {
            match token {
                Token::Text(text) => formatter.write_str(text)?,
                Token::Reference(reference) => write!(formatter, "{}", reference.a1(host))?,
                Token::Function { prefix, name } => {
                    if !entry {
                        formatter.write_str(prefix.as_str())?;
                    }
                    formatter.write_str(name)?;
                }
                Token::Single { wrapped } => formatter.write_str(match (entry, wrapped) {
                    (false, _) => "_xlfn.SINGLE(",
                    (true, false) => "@",
                    (true, true) => "@(",
                })?,
                Token::SingleEnd { wrapped } => {
                    if !entry || *wrapped {
                        formatter.write_str(")")?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Write the shape in R1C1 notation, which reads the same at every host.
    pub(crate) fn write_r1c1(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for token in &*self.tokens {
            match token {
                Token::Reference(reference) => write!(formatter, "{}", reference.r1c1())?,
                Token::Text(text) => formatter.write_str(text)?,
                Token::Function { prefix, name } => {
                    formatter.write_str(prefix.as_str())?;
                    formatter.write_str(name)?;
                }
                Token::Single { .. } => formatter.write_str("_xlfn.SINGLE(")?,
                Token::SingleEnd { .. } => formatter.write_str(")")?,
            }
        }
        Ok(())
    }
}

/// Tokens gathered in order, adjacent text joined into one run.
#[derive(Default)]
pub(crate) struct Builder {
    tokens: Vec<Token>,
    text: String,
}

impl Builder {
    /// Add text to the current run.
    pub(crate) fn text(&mut self, text: &str) {
        self.text.push_str(text);
    }

    /// Close the current run and add `token`.
    pub(crate) fn token(&mut self, token: Token) {
        self.flush();
        self.tokens.push(token);
    }

    fn flush(&mut self) {
        if !self.text.is_empty() {
            self.tokens
                .push(Token::Text(SmolStr::new(std::mem::take(&mut self.text))));
        }
    }

    /// The tokens gathered.
    pub(crate) fn finish(mut self) -> Vec<Token> {
        self.flush();
        self.tokens
    }
}

/// An `_xlfn.SINGLE(...)` in a lexed file spelling: the lexeme of its name,
/// the lexeme of its matching `)`, and whether what it applies to is more
/// than one operand.
struct Single {
    name: usize,
    close: usize,
    wrapped: bool,
}

/// Every `_xlfn.SINGLE(` whose `)` the text closes.
fn singles(lexemes: &[Lexeme<'_>]) -> Vec<Single> {
    let mut found = Vec::new();
    for (at, lexeme) in lexemes.iter().enumerate() {
        let Kind::Function { prefix, name } = &lexeme.kind else {
            continue;
        };
        if *prefix != Prefix::Future
            || !name.eq_ignore_ascii_case("SINGLE")
            || lexemes.get(at + 1).map(|next| &next.kind) != Some(&Kind::Open)
        {
            continue;
        }
        if let Some(close) = matching_close(lexemes, at + 1) {
            found.push(Single {
                name: at,
                close,
                wrapped: operand_end(lexemes, at + 2) != Some(close),
            });
        }
    }
    found
}

/// The lexeme of the `)` matching the `(` at `open`.
pub(crate) fn matching_close(lexemes: &[Lexeme<'_>], open: usize) -> Option<usize> {
    let mut depth = 0_usize;
    for (at, lexeme) in lexemes.iter().enumerate().skip(open) {
        match lexeme.kind {
            Kind::Open => depth += 1,
            Kind::Close => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(at);
                }
            }
            _ => {}
        }
    }
    None
}

/// Where the one operand starting at lexeme `at` ends (the lexeme after
/// it): a reference and the ranges it spans, a call through its `)`, a
/// parenthesized group, a name, a structured reference or a literal.
pub(crate) fn operand_end(lexemes: &[Lexeme<'_>], mut at: usize) -> Option<usize> {
    // File intake has no entry-length bound. A flat colon chain must not
    // consume the stack before the computation grammar can examine it.
    loop {
        let lexeme = lexemes.get(at)?;
        let end = match &lexeme.kind {
            // Nested @ prefixes share their operand's end. Skip them without
            // recursion; entry's parser already proves each required operand.
            Kind::At | Kind::Space => {
                at += 1;
                continue;
            }
            Kind::Function { .. } => {
                if lexemes.get(at + 1)?.kind != Kind::Open {
                    return None;
                }
                matching_close(lexemes, at + 1)? + 1
            }
            Kind::Open => matching_close(lexemes, at)? + 1,
            Kind::Reference(_)
            | Kind::Name
            | Kind::Structured
            | Kind::Number
            | Kind::Boolean
            | Kind::Error
            | Kind::String { closed: true } => at + 1,
            _ => return None,
        };
        // A reference spans on through `:` to the next one: `A1:INDEX(..)`.
        if !lexemes
            .get(end)
            .is_some_and(|next| next.kind == Kind::Range)
        {
            return Some(end);
        }
        at = end + 1;
    }
}

/// A written reference relative to `host`.
pub(crate) fn relative(reference: &RawReference, host: CellRef) -> Reference {
    let row = |coord: RawCoord| Coord::of(coord.index, host.row(), coord.absolute);
    let column = |coord: RawCoord| Coord::of(coord.index, host.column(), coord.absolute);
    Reference {
        sheet: match &reference.sheet {
            RawSheet::Own => SheetSpec::Own,
            RawSheet::Named { name, quoted } => SheetSpec::Named {
                name: name.clone(),
                quoted: *quoted,
            },
            RawSheet::Span {
                first,
                last,
                quoted,
            } => SheetSpec::Span {
                first: first.clone(),
                last: last.clone(),
                quoted: *quoted,
            },
            RawSheet::External(prefix) => SheetSpec::External(prefix.clone()),
            RawSheet::Invalid => SheetSpec::Invalid,
        },
        target: match &reference.target {
            RawTarget::Cell {
                row: at_row,
                column: at_column,
            } => Target::Cell {
                row: row(*at_row),
                column: column(*at_column),
            },
            RawTarget::Area { first, last } => Target::Area {
                first: (row(first.0), column(first.1)),
                last: (row(last.0), column(last.1)),
            },
            RawTarget::Rows { first, last } => Target::Rows {
                first: row(*first),
                last: row(*last),
            },
            RawTarget::Columns { first, last } => Target::Columns {
                first: column(*first),
                last: column(*last),
            },
            RawTarget::Name(name) => Target::Name(name.clone()),
            RawTarget::Invalid => Target::Invalid,
        },
    }
}
