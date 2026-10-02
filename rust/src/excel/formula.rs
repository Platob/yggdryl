//! A cell's formula: what its `<f>` states, held once as a shape and shared.
//!
//! A [`Formula`] is one reference-counted shape: its text as
//! verbatim runs and references stored relative to the cell holding it. So
//! cloning one - into a copy of its cell, a sheet sliced out of another,
//! every dependent of a shared formula - costs one counter increment and
//! never the text, and rendering it at another host yields the translated
//! text a fill or a copy would write. Two spellings exist, both total over
//! what they render:
//!
//! - the **file** spelling ([`Formula::at`]), what a part's `<f>` holds:
//!   functions a later Excel added under their `_xlfn.` prefix, implicit
//!   intersection as `_xlfn.SINGLE(..)`;
//! - the **entry** spelling ([`Formula::entry_at`]), what a user types after
//!   `=`: no prefix, and `@` for implicit intersection.
//!
//! [`Formula::from_file`] reads any text (a run it cannot read is carried
//! verbatim), and [`Formula::from_entry`] refuses a syntax error at its byte.
//! The function registry here is the part the spellings need - which names
//! carry a prefix, which are volatile - and the engine that evaluates a
//! shape is not in this module.

pub(crate) mod aggregate;
pub(crate) mod criteria;
pub(crate) mod eval;
pub(crate) mod functions;
pub(crate) mod graph;
pub(crate) mod lexer;
pub(crate) mod number;
pub(crate) mod parser;
pub(crate) mod reference;
pub(crate) mod shape;
pub(crate) mod value;

use std::collections::HashSet;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result};

use super::cell::CellRef;
use lexer::{Kind, Lexeme, Prefix};
use shape::Shape;

/// The longest formula Excel accepts, in characters.
pub const MAX_FORMULA_LENGTH: usize = 8_192;

/// Calls below the outermost call (65 total), including implicit intersection.
/// Parentheses and arrays share a separate resource bound of 64 nested groups.
pub const MAX_FORMULA_NESTING: usize = 64;

/// Results of one workbook formula pass.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recalculation {
    /// Computed formula cells visited during this pass, including Excel errors.
    pub evaluated: u64,
    /// Current workbook-wide formula cells retaining uncomputed caches,
    /// including circular cells and their blocked consumers.
    pub uncomputed: u64,
    /// The first 256 current circular cells in stable sheet-key/coordinate order.
    pub circular: Vec<(SmolStr, CellRef)>,
    /// Current circular cells throughout the workbook, including omitted entries.
    pub circular_count: u64,
}

/// The formula a cell holds, whose cached result is the cell's value.
///
/// Equality and hashing read the shape, never the pointer: the formulas of
/// two cells are equal when one translates into the other, so `A1*B1` held
/// by `C1` equals `A2*B2` held by `C2`.
///
/// ```
/// use yggdryl::excel::{CellRef, Formula};
///
/// let host: CellRef = "C1".parse()?;
/// let formula = Formula::from_file("A1*$B$1+_xlfn.XLOOKUP(A1,D:D,E:E)", host);
/// assert_eq!(formula.at(host).to_string(), "A1*$B$1+_xlfn.XLOOKUP(A1,D:D,E:E)");
///
/// // Rendered one row down, the relative references follow.
/// let below: CellRef = "C2".parse()?;
/// assert_eq!(formula.at(below).to_string(), "A2*$B$1+_xlfn.XLOOKUP(A2,D:D,E:E)");
/// assert_eq!(formula.entry_at(below).to_string(), "A2*$B$1+XLOOKUP(A2,D:D,E:E)");
/// assert_eq!(formula, Formula::from_file("A2*$B$1+_xlfn.XLOOKUP(A2,D:D,E:E)", below));
///
/// // What a user types gains the prefix the file stores.
/// let typed = Formula::from_entry("xlookup(a1,d:d,e:e)", host)?;
/// assert_eq!(typed.at(host).to_string(), "_xlfn.XLOOKUP(A1,D:D,E:E)");
/// assert!(Formula::from_entry("SUM(A1", host).is_err());
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone)]
pub struct Formula(Arc<Shape>);

impl Formula {
    /// The formula a `<f>` element spells for the cell at `host`, without
    /// the leading `=` the file never writes.
    ///
    /// Total: a run the grammar does not read is carried verbatim, and the
    /// formula holding one is never computed.
    #[must_use]
    pub fn from_file(text: &str, host: CellRef) -> Self {
        Self(Arc::new(Shape::from_lexemes(&lexer::lex(text), host)))
    }

    /// The formula a user types for the cell at `host`, what follows the
    /// `=`: function names are spelled upper case and take the prefix the
    /// file stores them under, references are spelled as Excel spells
    /// them, and `@x` is implicit intersection, `_xlfn.SINGLE(x)` in the
    /// file.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] at the byte of the first syntax error: an
    /// empty formula, an unclosed text or parenthesis, a character the
    /// grammar does not read, an operator with no operand, two operands
    /// with no operator, text over [`MAX_FORMULA_LENGTH`] characters or
    /// group/array nesting over [`MAX_FORMULA_NESTING`] levels or more than
    /// that many calls below the outermost call (including implicit intersection).
    pub fn from_entry(text: &str, host: CellRef) -> Result<Self> {
        if let Some((position, _)) = text.char_indices().nth(MAX_FORMULA_LENGTH) {
            return Err(refused(
                position,
                format_smolstr!("expected at most {MAX_FORMULA_LENGTH} characters"),
            ));
        }
        let lexemes = lexer::lex(text);
        let parsed = parser::entry(&lexemes, host, text.len())?;
        let file = file_spelling(&lexemes, host)?;
        let mut shape = Shape::from_lexemes(&lexer::lex(&file), host);
        // The entry and file trees agree when no implicit-intersection
        // lowering occurred. Otherwise the held spelling is parsed lazily.
        if shape.held.is_none()
            && !lexemes.iter().any(|item| {
                matches!(item.kind, Kind::At)
                    || matches!(&item.kind, Kind::Function { prefix: Prefix::Future, name }
                    if name.eq_ignore_ascii_case("SINGLE"))
            })
        {
            shape.install(parsed);
        }
        Ok(Self(Arc::new(shape)))
    }

    /// The file spelling of the formula held at `host`, rendered as it is
    /// written: nothing is allocated until the caller writes it somewhere.
    pub fn at(&self, host: CellRef) -> impl fmt::Display + '_ {
        Rendered {
            shape: &self.0,
            host,
            entry: false,
        }
    }

    /// What a user sees and types for the formula held at `host`: no
    /// `_xlfn.`/`_xlws.` prefix, `@` for implicit intersection.
    pub fn entry_at(&self, host: CellRef) -> impl fmt::Display + '_ {
        Rendered {
            shape: &self.0,
            host,
            entry: true,
        }
    }

    /// Whether the formula can be computed here: false for one holding a
    /// reference into another workbook, a structured or spilled reference,
    /// a dynamic array's own function, or text the grammar does not read -
    /// a formula carried with the value Excel cached for it.
    #[must_use]
    pub fn is_computed(&self) -> bool {
        self.0.held_reason().is_none()
    }

    /// Whether the formula calls a function whose value changes without its
    /// arguments changing: `NOW`, `TODAY`, `RAND`, `RANDBETWEEN`, `OFFSET`,
    /// `INDIRECT`.
    pub(crate) fn is_volatile(&self) -> bool {
        self.0.volatile
    }

    /// The shape's address: one per shape while it is held, which a rewrite
    /// of many cells memoizes its answer by.
    pub(crate) fn address(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }

    /// Whether a reference of the formula names the sheet `sheet`.
    pub(crate) fn names(&self, sheet: &str) -> bool {
        self.0
            .sheets
            .iter()
            .any(|held| reference::same_sheet(held, sheet))
    }

    /// The formula with its references to the sheet `from` naming `to`,
    /// `None` when none names it.
    pub(crate) fn renamed(&self, from: &str, to: &str) -> Option<Self> {
        self.0.renamed(from, to).map(|shape| Self(Arc::new(shape)))
    }

    /// The formula with its references to the removed sheet `removed` made
    /// `#REF!`, `order` the tabs before the removal; `None` when none names
    /// it.
    pub(crate) fn removed(&self, removed: &str, order: &[&str]) -> Option<Self> {
        self.0
            .removed(removed, order)
            .map(|shape| Self(Arc::new(shape)))
    }

    /// The shape.
    pub(crate) fn shape(&self) -> &Shape {
        &self.0
    }

    /// The one compiled arena, borrowed from this formula's shared shape.
    pub(crate) fn expression(&self) -> std::result::Result<&parser::Expr, shape::Held> {
        if let Some(reason) = self.shape().held {
            return Err(reason);
        }
        self.shape().compiled().as_ref().map_err(|reason| *reason)
    }

    /// The formula holding `shape`.
    pub(crate) fn from_shape(shape: Shape) -> Self {
        Self(Arc::new(shape))
    }
}

/// A refusal of a formula at byte `position`.
fn refused(position: usize, reason: SmolStr) -> Error {
    Error::Parse {
        target: "formula",
        position,
        reason,
    }
}

/// The file spelling of entry text already checked: function names upper
/// case under their prefix, references spelled canonically, `@x` as
/// `_xlfn.SINGLE(x)`.
fn file_spelling(lexemes: &[Lexeme<'_>], host: CellRef) -> Result<String> {
    use std::fmt::Write as _;

    let mut file = String::new();
    // The `(` an `@(x)` spells, which the `SINGLE(` it becomes replaces.
    let mut skipped: Vec<usize> = Vec::new();
    // The lexemes after which the `)` of an `@x` is written.
    let mut closes: Vec<usize> = Vec::new();
    for (at, lexeme) in lexemes.iter().enumerate() {
        if !skipped.contains(&at) {
            match &lexeme.kind {
                Kind::Function { prefix, name } => {
                    let upper = name.to_ascii_uppercase();
                    let prefix = match prefix {
                        Prefix::None => functions::Function::lookup(&upper)
                            .map_or(Prefix::None, |known| known.info().prefix),
                        stated => *stated,
                    };
                    file.push_str(prefix.as_str());
                    file.push_str(&upper);
                }
                Kind::Reference(reference) => {
                    let mut reference = shape::relative(reference, host);
                    if let reference::SheetSpec::Named { quoted, .. }
                    | reference::SheetSpec::Span { quoted, .. } = &mut reference.sheet
                    {
                        *quoted = false;
                    }
                    let _ = write!(file, "{}", reference.a1(host));
                }
                Kind::Boolean | Kind::Error => file.push_str(&lexeme.text.to_ascii_uppercase()),
                Kind::At => {
                    let Some(end) = shape::operand_end(lexemes, at + 1) else {
                        return Err(refused(
                            lexeme.start,
                            SmolStr::new_static("expected an operand right after @"),
                        ));
                    };
                    file.push_str("_xlfn.SINGLE(");
                    let grouped = lexemes[at + 1].kind == Kind::Open
                        && shape::matching_close(lexemes, at + 1).map(|close| close + 1)
                            == Some(end);
                    if grouped {
                        skipped.push(at + 1);
                    } else {
                        closes.push(end - 1);
                    }
                }
                _ => file.push_str(lexeme.text),
            }
        }
        while let Some(index) = closes.iter().position(|close| *close == at) {
            closes.swap_remove(index);
            file.push(')');
        }
    }
    Ok(file)
}

/// A formula spelled at one host.
struct Rendered<'shape> {
    shape: &'shape Shape,
    host: CellRef,
    entry: bool,
}

impl fmt::Display for Rendered<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.shape.write(formatter, self.host, self.entry)
    }
}

impl PartialEq for Formula {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0 == other.0
    }
}

impl Eq for Formula {}

impl Hash for Formula {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl fmt::Debug for Formula {
    /// The shape in R1C1 notation, the one spelling that reads the same at
    /// every host: `Formula("RC[-2]*R1C2")`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        struct R1c1<'a>(&'a Shape);
        impl fmt::Debug for R1c1<'_> {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("\"")?;
                self.0.write_r1c1(formatter)?;
                formatter.write_str("\"")
            }
        }
        formatter
            .debug_tuple("Formula")
            .field(&R1c1(&self.0))
            .finish()
    }
}

/// The shapes one sheet's formulas hold while its part is read, so cells
/// whose formulas translate into one another share one shape; dropped with
/// the parse.
#[derive(Default)]
pub(crate) struct Interner {
    held: HashSet<Formula>,
}

impl Interner {
    /// The formula equal to `formula` already held, else `formula` itself,
    /// held from now on.
    pub(crate) fn intern(&mut self, formula: Formula) -> Formula {
        if let Some(held) = self.held.get(&formula) {
            return held.clone();
        }
        self.held.insert(formula.clone());
        formula
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/excel/formula/shape.rs` pins and a caller cannot
    //! reach: the tokens a shape holds, the facts read off them, and
    //! whether two formulas share one shape.

    use super::Formula;
    use super::parser::{EvaluationPolicy, Expr, Node};
    use super::shape::{Held, Token};

    /// Each token of the shape, spelled `text:..`, `ref:..` (R1C1),
    /// `function:prefix|name`, `single` or `end`.
    #[must_use]
    pub fn tokens(formula: &Formula) -> Vec<String> {
        formula
            .shape()
            .tokens
            .iter()
            .map(|token| match token {
                Token::Text(text) => format!("text:{text}"),
                Token::Reference(reference) => format!("ref:{}", reference.r1c1()),
                Token::Function { prefix, name } => {
                    format!("function:{}|{name}", prefix.as_str())
                }
                Token::Single { wrapped } => format!("single:{wrapped}"),
                Token::SingleEnd { wrapped } => format!("end:{wrapped}"),
            })
            .collect()
    }

    /// The sheets the shape's references name.
    #[must_use]
    pub fn sheets(formula: &Formula) -> Vec<String> {
        formula
            .shape()
            .sheets
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// Why the shape is never computed, `None` when it is.
    #[must_use]
    pub fn held(formula: &Formula) -> Option<&'static str> {
        formula.shape().held_reason().map(Held::as_str)
    }

    /// The once-compiled arena, exposing topology to the mirrored parser test.
    #[must_use]
    pub fn arena(formula: &Formula) -> Option<(usize, Vec<String>)> {
        let Expr { nodes, root } = formula.shape().compiled().as_ref().ok()?;
        Some((
            *root,
            nodes.iter().map(|node| format!("{node:?}")).collect(),
        ))
    }

    /// References under strict ancestors, in evaluation order. Held/lazy
    /// subtrees contribute no speculative graph edges.
    #[must_use]
    pub fn strict_references(formula: &Formula) -> Option<Vec<String>> {
        let Expr { nodes, root } = formula.shape().compiled().as_ref().ok()?;
        let mut stack = vec![*root];
        let mut references = Vec::new();
        while let Some(id) = stack.pop() {
            match nodes[id].evaluation_children() {
                EvaluationPolicy::Leaf => {
                    if let Node::Reference(reference) = &nodes[id] {
                        references.push(reference.r1c1().to_string());
                    }
                }
                EvaluationPolicy::Strict(children) => {
                    children.visit_reverse(|child| stack.push(child));
                }
                EvaluationPolicy::Held => {}
            }
        }
        Some(references)
    }

    /// Number of strict root children, to pin allocation-free policy calls.
    #[must_use]
    pub fn strict_root_child_count(formula: &Formula) -> Option<usize> {
        let Expr { nodes, root } = formula.shape().compiled().as_ref().ok()?;
        match nodes[*root].evaluation_children() {
            EvaluationPolicy::Strict(children) => {
                let mut count = 0;
                children.visit_reverse(|_| count += 1);
                Some(count)
            }
            EvaluationPolicy::Leaf | EvaluationPolicy::Held => None,
        }
    }

    /// Entry's parsed tree before SINGLE lowering, for grammar parity.
    pub fn entry_arena(text: &str, host: super::CellRef) -> crate::Result<(usize, Vec<String>)> {
        let parsed = super::parser::entry(&super::lexer::lex(text), host, text.len())?;
        Ok((
            parsed.root,
            parsed
                .nodes
                .iter()
                .map(|node| format!("{node:?}"))
                .collect(),
        ))
    }

    /// Whether the shared shape has already compiled its arena.
    #[must_use]
    pub fn is_cached(formula: &Formula) -> bool {
        formula.shape().is_cached()
    }

    /// Whether the shape calls a volatile function.
    #[must_use]
    pub fn is_volatile(formula: &Formula) -> bool {
        formula.is_volatile()
    }

    /// Whether two formulas hold one shape, not two equal ones.
    #[must_use]
    pub fn shares_shape(first: &Formula, second: &Formula) -> bool {
        std::sync::Arc::ptr_eq(&first.0, &second.0)
    }

    /// The formula with references to `from` naming `to`.
    #[must_use]
    pub fn renamed(formula: &Formula, from: &str, to: &str) -> Option<Formula> {
        formula.renamed(from, to)
    }

    /// The formula with references to `removed` made `#REF!`.
    #[must_use]
    pub fn removed(formula: &Formula, removed: &str, order: &[&str]) -> Option<Formula> {
        formula.removed(removed, order)
    }
}
