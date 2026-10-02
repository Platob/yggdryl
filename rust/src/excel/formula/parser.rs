//! One grammar for entered and held Excel formulas.
//!
//! References arrive as typed, host-relative values. File intake remains
//! total; a failed lazy parse is held, while entry returns a located error.

use smol_str::{SmolStr, format_smolstr};

use crate::excel::cell::{CellRef, ExcelError};
use crate::{Error, Result, Str};

use super::MAX_FORMULA_NESTING;
use super::functions::Function;
use super::lexer::{self, Kind, Lexeme};
use super::number;
use super::reference::{Reference, SheetSpec, Target};
use super::shape::{self, Held, Token};

type Id = usize;

/// Excel applies this call bound even to an unrecognized function name.
const MAX_ARGUMENTS: usize = 255;

/// A compiled formula's nodes, indexed by child ID; no per-cell AST copies.
#[derive(Debug)]
pub(crate) struct Expr {
    pub(crate) nodes: Box<[Node]>,
    pub(crate) root: Id,
}

#[derive(Debug)]
pub(crate) enum Node {
    Literal(Literal),
    Reference(Reference),
    Held(Held),
    Group(Id),
    Unary {
        op: UnaryOp,
        value: Id,
    },
    Binary {
        op: BinaryOp,
        left: Id,
        right: Id,
    },
    Percent(Id),
    Spill(Id),
    Call {
        /// `None` is a syntactically valid unknown name, held by `Shape`.
        function: Option<Function>,
        args: Box<[Option<Id>]>,
    },
    Array(Box<[Box<[Id]>]>),
}

/// Which children may be evaluated, and therefore become dependencies.
/// Unsupported and lazy expressions expose no speculative child edges.
pub(crate) enum EvaluationPolicy<'a> {
    Leaf,
    Strict(Children<'a>),
    Held,
}

/// How a legacy formula consumes a reference. Scalar uses implicit
/// intersection; a direct aggregate argument preserves its entire range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReferenceUse {
    Scalar,
    Range,
}

/// Borrowed child IDs in evaluator stack order; visiting costs no allocation.
pub(crate) enum Children<'a> {
    One(Id, Option<ReferenceUse>),
    Two(Id, Id),
    Call(&'a [Option<Id>], ReferenceUse),
}

impl Children<'_> {
    /// Grouping passes its consumer through; every other strict owner states
    /// the reference use once for both scheduling and dependency intake.
    pub(crate) fn reference_use(&self, inherited: ReferenceUse) -> ReferenceUse {
        match self {
            Self::One(_, value) => value.unwrap_or(inherited),
            Self::Two(_, _) => ReferenceUse::Scalar,
            Self::Call(_, value) => *value,
        }
    }

    pub(crate) fn visit_reverse(self, mut visit: impl FnMut(Id)) {
        self.visit_references(ReferenceUse::Scalar, |id, _| visit(id));
    }

    pub(crate) fn visit_references(
        self,
        inherited: ReferenceUse,
        mut visit: impl FnMut(Id, ReferenceUse),
    ) {
        let usage = self.reference_use(inherited);
        match self {
            Self::One(child, _) => visit(child, usage),
            Self::Two(left, right) => {
                visit(right, usage);
                visit(left, usage);
            }
            Self::Call(args, _) => {
                for child in args.iter().rev().flatten() {
                    visit(*child, usage);
                }
            }
        }
    }
}

impl Node {
    /// One policy for scheduling and dependency intake. Exact call shapes
    /// match current ready-stage dispatch; held/lazy calls do not expose
    /// references that could create false cycles.
    pub(crate) fn evaluation_children(&self) -> EvaluationPolicy<'_> {
        match self {
            Self::Literal(_) | Self::Reference(_) => EvaluationPolicy::Leaf,
            Self::Group(child) => EvaluationPolicy::Strict(Children::One(*child, None)),
            Self::Percent(child) | Self::Unary { value: child, .. } => {
                EvaluationPolicy::Strict(Children::One(*child, Some(ReferenceUse::Scalar)))
            }
            Self::Binary { op, left, right }
                if matches!(
                    *op,
                    BinaryOp::Add
                        | BinaryOp::Subtract
                        | BinaryOp::Multiply
                        | BinaryOp::Divide
                        | BinaryOp::Equal
                ) =>
            {
                EvaluationPolicy::Strict(Children::Two(*left, *right))
            }
            Self::Call {
                function: Some(function),
                args,
            } => {
                let (shape, usage) = match *function {
                    Function::Abs | Function::Sqrt => (args.len() == 1, ReferenceUse::Scalar),
                    Function::Round | Function::Mod => (args.len() == 2, ReferenceUse::Scalar),
                    Function::Sum => (true, ReferenceUse::Range),
                    _ => (false, ReferenceUse::Scalar),
                };
                if shape && args.iter().all(Option::is_some) {
                    EvaluationPolicy::Strict(Children::Call(args.as_ref(), usage))
                } else {
                    EvaluationPolicy::Held
                }
            }
            Self::Held(_)
            | Self::Binary { .. }
            | Self::Spill(_)
            | Self::Call { .. }
            | Self::Array(_) => EvaluationPolicy::Held,
        }
    }
}

#[derive(Debug)]
pub(crate) enum Literal {
    Number(f64),
    Text(Str),
    Boolean(bool),
    Error(ExcelError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnaryOp {
    Positive,
    Negative,
    ImplicitIntersection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BinaryOp {
    Range,
    Intersection,
    Union,
    Power,
    Multiply,
    Divide,
    Add,
    Subtract,
    Concat,
    Equal,
    NotEqual,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,
}

#[derive(Clone)]
struct Atom {
    kind: AtomKind,
    text: SmolStr,
    at: usize,
}

#[derive(Clone)]
enum AtomKind {
    Space,
    Number,
    Text,
    Boolean,
    Error,
    Reference(Reference),
    Structured,
    Function { name: SmolStr },
    Operator,
    Range,
    Open,
    Close,
    ImplicitOpen,
    ImplicitClose,
    ArrayOpen,
    ArrayClose,
    Comma,
    Semicolon,
    At,
    Spill,
    Opaque,
    UnclosedText,
}

fn error(at: usize, reason: impl Into<SmolStr>) -> Error {
    Error::Parse {
        target: "formula",
        position: at,
        reason: reason.into(),
    }
}

fn atom(lexeme: &Lexeme<'_>, host: Option<CellRef>, base: usize) -> Atom {
    let kind = match &lexeme.kind {
        Kind::Space => AtomKind::Space,
        Kind::Number => AtomKind::Number,
        Kind::String { closed: true } => AtomKind::Text,
        Kind::Boolean => AtomKind::Boolean,
        Kind::Error => AtomKind::Error,
        Kind::String { closed: false } => AtomKind::UnclosedText,
        Kind::Reference(reference) => match host {
            Some(host) => AtomKind::Reference(shape::relative(reference, host)),
            None => AtomKind::Opaque,
        },
        Kind::Name => AtomKind::Reference(Reference {
            sheet: SheetSpec::Own,
            target: Target::Name(SmolStr::new(lexeme.text)),
        }),
        Kind::Structured => AtomKind::Structured,
        Kind::Function { name, .. } => AtomKind::Function { name: name.clone() },
        Kind::Operator => AtomKind::Operator,
        Kind::Range => AtomKind::Range,
        Kind::Open => AtomKind::Open,
        Kind::Close => AtomKind::Close,
        Kind::ArrayOpen => AtomKind::ArrayOpen,
        Kind::ArrayClose => AtomKind::ArrayClose,
        Kind::Comma => AtomKind::Comma,
        Kind::Semicolon => AtomKind::Semicolon,
        Kind::At => AtomKind::At,
        Kind::Spill => AtomKind::Spill,
        Kind::Opaque => AtomKind::Opaque,
    };
    Atom {
        kind,
        text: SmolStr::new(lexeme.text),
        at: base + lexeme.start,
    }
}

/// Parse user entry once, with byte-accurate failures.
pub(crate) fn entry(lexemes: &[Lexeme<'_>], host: CellRef, length: usize) -> Result<Expr> {
    Parser::new(
        lexemes
            .iter()
            .map(|item| atom(item, Some(host), 0))
            .collect(),
        length,
    )
    .parse()
}

/// Parse held tokens without rendering at an invented host. Syntax failures
/// are held because an Excel file's formula text is always preserved.
pub(crate) fn compile(tokens: &[Token]) -> std::result::Result<Expr, Held> {
    let mut atoms = Vec::new();
    let mut offset = 0;
    for token in tokens {
        match token {
            Token::Text(text) => {
                atoms.extend(lexer::lex(text).iter().map(|item| atom(item, None, offset)));
                offset += text.len();
            }
            Token::Reference(reference) => {
                atoms.push(Atom {
                    kind: AtomKind::Reference(reference.clone()),
                    text: SmolStr::new_static("reference"),
                    at: offset,
                });
                offset += 1;
            }
            Token::Function { name, .. } => {
                atoms.push(Atom {
                    kind: AtomKind::Function { name: name.clone() },
                    text: name.clone(),
                    at: offset,
                });
                offset += 1;
            }
            Token::Single { .. } => {
                atoms.push(Atom {
                    kind: AtomKind::ImplicitOpen,
                    text: SmolStr::new_static("_xlfn.SINGLE("),
                    at: offset,
                });
                offset += 1;
            }
            Token::SingleEnd { .. } => {
                atoms.push(Atom {
                    kind: AtomKind::ImplicitClose,
                    text: SmolStr::new_static(")"),
                    at: offset,
                });
                offset += 1;
            }
        }
    }
    Parser::new(atoms, offset)
        .parse()
        .map_err(|_| Held::Unrecognized)
}

struct Parser {
    atoms: Vec<Atom>,
    cursor: usize,
    nodes: Vec<Node>,
    length: usize,
    groups: usize,
    calls: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Frame {
    Root,
    Group,
    Call,
    Implicit,
}

impl Parser {
    fn new(atoms: Vec<Atom>, length: usize) -> Self {
        Self {
            atoms,
            cursor: 0,
            nodes: Vec::new(),
            length,
            groups: 0,
            calls: 0,
        }
    }

    fn parse(mut self) -> Result<Expr> {
        self.space();
        if self.cursor == self.atoms.len() {
            return Err(error(0, "expected a formula"));
        }
        let root = self.expression(0, Frame::Root)?;
        self.space();
        if let Some(next) = self.peek() {
            return Err(self.trailing(next));
        }
        Ok(Expr {
            nodes: self.nodes.into_boxed_slice(),
            root,
        })
    }

    fn push(&mut self, node: Node) -> Id {
        let id = self.nodes.len();
        self.nodes.push(node);
        id
    }

    fn peek(&self) -> Option<&Atom> {
        self.atoms.get(self.cursor)
    }

    fn take(&mut self) -> Option<Atom> {
        let result = self.peek().cloned();
        self.cursor += usize::from(result.is_some());
        result
    }

    fn space(&mut self) -> bool {
        let mut seen = false;
        while matches!(self.peek().map(|atom| &atom.kind), Some(AtomKind::Space)) {
            self.cursor += 1;
            seen = true;
        }
        seen
    }

    fn open_group(&mut self, at: usize) -> Result<()> {
        if self.groups >= MAX_FORMULA_NESTING {
            return Err(error(
                at,
                format_smolstr!("expected at most {MAX_FORMULA_NESTING} levels of nesting"),
            ));
        }
        self.groups += 1;
        Ok(())
    }

    fn close_group(&mut self) {
        self.groups -= 1;
    }

    fn open_call(&mut self, at: usize) -> Result<()> {
        // Native Excel permits 64 calls below the outermost. SINGLE counts
        // exactly like an authored call; entry counts @ before lowering it.
        if self.calls > MAX_FORMULA_NESTING {
            return Err(error(
                at,
                format_smolstr!(
                    "expected at most {MAX_FORMULA_NESTING} calls below the outermost, got {}",
                    self.calls
                ),
            ));
        }
        self.calls += 1;
        Ok(())
    }

    fn close_call(&mut self) {
        self.calls -= 1;
    }

    fn expression(&mut self, min_bp: u8, frame: Frame) -> Result<Id> {
        self.space();
        let mut prefixes = Vec::new();
        while let Some(atom) = self.peek() {
            let prefix = match (&atom.kind, atom.text.as_str()) {
                (AtomKind::At, _) => Some((UnaryOp::ImplicitIntersection, 95)),
                (AtomKind::Operator, "+") => Some((UnaryOp::Positive, 70)),
                (AtomKind::Operator, "-") => Some((UnaryOp::Negative, 70)),
                _ => None,
            };
            let Some((op, bp)) = prefix else { break };
            let at = atom.at;
            self.take();
            if op == UnaryOp::ImplicitIntersection {
                self.open_call(at)?;
            }
            prefixes.push((op, bp));
            self.space();
        }
        let mut lhs = self.primary(frame)?;
        for (op, bp) in prefixes.into_iter().rev() {
            lhs = self.continue_expression(lhs, bp, frame)?;
            if op == UnaryOp::ImplicitIntersection {
                self.close_call();
            }
            lhs = self.push(Node::Unary { op, value: lhs });
        }
        self.continue_expression(lhs, min_bp, frame)
    }

    fn primary(&mut self, _frame: Frame) -> Result<Id> {
        let Some(atom) = self.take() else {
            return Err(error(self.length, "expected an operand at the end"));
        };
        if matches!(
            atom.kind,
            AtomKind::Number | AtomKind::Text | AtomKind::Boolean | AtomKind::Error
        ) {
            return self.literal(atom);
        }
        match atom.kind {
            AtomKind::Reference(reference) => Ok(self.push(Node::Reference(reference))),
            AtomKind::Structured => Ok(self.push(Node::Held(Held::Structured))),
            AtomKind::Function { name } => self.call(name, atom.at),
            AtomKind::Open => {
                self.open_group(atom.at)?;
                self.space();
                if matches!(self.peek().map(|item| &item.kind), Some(AtomKind::Close)) {
                    let at = self.take().expect("peeked close").at;
                    return Err(error(at, "expected an operand before )"));
                }
                let inside = self.expression(0, Frame::Group)?;
                self.space();
                let Some(close) = self.take() else {
                    return Err(error(atom.at, "expected ) to close this"));
                };
                if !matches!(close.kind, AtomKind::Close) {
                    return Err(error(atom.at, "expected ) to close this"));
                }
                self.close_group();
                Ok(self.push(Node::Group(inside)))
            }
            AtomKind::ImplicitOpen => {
                self.open_call(atom.at)?;
                let inside = self.expression(0, Frame::Implicit)?;
                self.space();
                let Some(close) = self.take() else {
                    return Err(error(atom.at, "expected ) to close SINGLE"));
                };
                if !matches!(close.kind, AtomKind::ImplicitClose) {
                    return Err(error(atom.at, "expected ) to close SINGLE"));
                }
                self.close_call();
                Ok(self.push(Node::Unary {
                    op: UnaryOp::ImplicitIntersection,
                    value: inside,
                }))
            }
            AtomKind::ArrayOpen => self.array(atom.at),
            AtomKind::Opaque => Err(error(
                atom.at,
                format_smolstr!("expected a formula, got {:?}", atom.text),
            )),
            AtomKind::UnclosedText => Err(error(atom.at, "expected the closing \" of a text")),
            AtomKind::Close | AtomKind::ImplicitClose => Err(error(atom.at, "expected no )")),
            AtomKind::Operator => Err(error(
                atom.at,
                format_smolstr!("expected an operand before {}", atom.text),
            )),
            AtomKind::Range => Err(error(atom.at, "expected an operand before :")),
            AtomKind::Comma => Err(error(
                atom.at,
                "expected , between the arguments of a call or inside parentheses",
            )),
            AtomKind::Semicolon => Err(error(
                atom.at,
                "expected ; only between the rows of an array",
            )),
            AtomKind::At => Err(error(atom.at, "expected an operand at the end")),
            AtomKind::Spill => Err(error(atom.at, "expected a reference before #")),
            AtomKind::Space | AtomKind::ArrayClose => {
                Err(error(atom.at, "expected an operand before this"))
            }
            AtomKind::Number | AtomKind::Text | AtomKind::Boolean | AtomKind::Error => {
                unreachable!("handled as a literal above")
            }
        }
    }

    fn literal(&mut self, atom: Atom) -> Result<Id> {
        let value = match atom.kind {
            AtomKind::Number => {
                let value = number::literal(&atom.text).ok_or_else(|| {
                    error(
                        atom.at,
                        format_smolstr!(
                            "expected a fifteen-digit Excel literal below 1E308, got {}",
                            atom.text
                        ),
                    )
                })?;
                Literal::Number(value)
            }
            AtomKind::Text => {
                let inside = &atom.text[1..atom.text.len() - 1];
                Literal::Text(Str::new(inside.replace("\"\"", "\"")))
            }
            AtomKind::Boolean => Literal::Boolean(atom.text.eq_ignore_ascii_case("TRUE")),
            AtomKind::Error => Literal::Error(ExcelError::from_text(&atom.text)),
            _ => unreachable!("literal caller proves the kind"),
        };
        Ok(self.push(Node::Literal(value)))
    }

    fn call(&mut self, name: SmolStr, at: usize) -> Result<Id> {
        self.space();
        if !matches!(self.peek().map(|item| &item.kind), Some(AtomKind::Open)) {
            return Err(error(at, "expected ( after a function"));
        }
        let opener = self.take().expect("peeked function open");
        self.open_call(opener.at)?;
        let mut args = Vec::new();
        loop {
            self.space();
            match self.peek().map(|item| &item.kind) {
                Some(AtomKind::Close) => {
                    self.take();
                    self.close_call();
                    return self.finish_call(name, args, opener.at);
                }
                Some(AtomKind::Comma) => {
                    args.push(None);
                    self.take();
                    self.space();
                    if matches!(self.peek().map(|item| &item.kind), Some(AtomKind::Close)) {
                        args.push(None);
                    }
                    continue;
                }
                None => return Err(error(opener.at, "expected ) to close this")),
                _ => {}
            }
            args.push(Some(self.expression(0, Frame::Call)?));
            self.space();
            match self.peek().map(|item| &item.kind) {
                Some(AtomKind::Comma) => {
                    self.take();
                    self.space();
                    if matches!(self.peek().map(|item| &item.kind), Some(AtomKind::Close)) {
                        args.push(None);
                    }
                }
                Some(AtomKind::Close) => {
                    self.take();
                    self.close_call();
                    return self.finish_call(name, args, opener.at);
                }
                None => return Err(error(opener.at, "expected ) to close this")),
                Some(_) => return Err(self.trailing(self.peek().expect("matched Some"))),
            }
        }
    }

    fn finish_call(&mut self, name: SmolStr, args: Vec<Option<Id>>, at: usize) -> Result<Id> {
        if args.len() > MAX_ARGUMENTS {
            return Err(error(
                at,
                format_smolstr!(
                    "expected at most {MAX_ARGUMENTS} arguments, got {}",
                    args.len()
                ),
            ));
        }
        let function = Function::lookup(&name);
        if let Some(signature) = function.and_then(|function| function.info().signature) {
            if !signature.arity.accepts(args.len()) {
                return Err(error(
                    at,
                    format_smolstr!(
                        "expected {} arguments for {}, got {}",
                        signature.text,
                        name,
                        args.len()
                    ),
                ));
            }
        }
        Ok(self.push(Node::Call {
            function,
            args: args.into_boxed_slice(),
        }))
    }

    fn array(&mut self, at: usize) -> Result<Id> {
        self.open_group(at)?;
        let mut rows: Vec<Box<[Id]>> = Vec::new();
        let mut row = Vec::new();
        loop {
            self.space();
            row.push(self.array_value()?);
            self.space();
            match self.peek().map(|item| &item.kind) {
                Some(AtomKind::Comma) => {
                    self.take();
                }
                Some(AtomKind::Semicolon) => {
                    let delimiter = self.take().expect("peeked array delimiter");
                    if let Some(first) = rows.first() {
                        if first.len() != row.len() {
                            return Err(error(
                                delimiter.at,
                                format_smolstr!(
                                    "expected {} columns in an array row, got {}",
                                    first.len(),
                                    row.len()
                                ),
                            ));
                        }
                    }
                    rows.push(row.into_boxed_slice());
                    row = Vec::new();
                }
                Some(AtomKind::ArrayClose) => {
                    let close = self.take().expect("peeked array close");
                    if let Some(first) = rows.first() {
                        if first.len() != row.len() {
                            return Err(error(
                                close.at,
                                format_smolstr!(
                                    "expected {} columns in an array row, got {}",
                                    first.len(),
                                    row.len()
                                ),
                            ));
                        }
                    }
                    rows.push(row.into_boxed_slice());
                    self.close_group();
                    return Ok(self.push(Node::Array(rows.into_boxed_slice())));
                }
                None => return Err(error(at, "expected } to close this array")),
                Some(_) => {
                    return Err(error(
                        self.peek().expect("matched Some").at,
                        "expected a constant value in an array",
                    ));
                }
            }
        }
    }

    fn array_value(&mut self) -> Result<Id> {
        self.space();
        let sign = match self.peek() {
            Some(Atom {
                kind: AtomKind::Operator,
                text,
                ..
            }) if text == "+" => Some(UnaryOp::Positive),
            Some(Atom {
                kind: AtomKind::Operator,
                text,
                ..
            }) if text == "-" => Some(UnaryOp::Negative),
            _ => None,
        };
        if sign.is_some() {
            self.take();
            self.space();
        }
        let Some(next) = self.peek().cloned() else {
            return Err(error(self.length, "expected a constant value in an array"));
        };
        if !matches!(
            next.kind,
            AtomKind::Number | AtomKind::Text | AtomKind::Boolean | AtomKind::Error
        ) || (sign.is_some() && !matches!(next.kind, AtomKind::Number))
        {
            return Err(error(next.at, "expected a constant value in an array"));
        }
        let literal = self.take().expect("peeked literal");
        let value = self.literal(literal)?;
        Ok(match sign {
            Some(op) => self.push(Node::Unary { op, value }),
            None => value,
        })
    }

    fn continue_expression(&mut self, mut lhs: Id, min_bp: u8, frame: Frame) -> Result<Id> {
        loop {
            let saved = self.cursor;
            let spaced = self.space();
            let Some(next) = self.peek().cloned() else {
                break;
            };
            if matches!(
                next.kind,
                AtomKind::Close
                    | AtomKind::ImplicitClose
                    | AtomKind::ArrayClose
                    | AtomKind::Semicolon
            ) {
                self.cursor = saved;
                break;
            }
            if matches!(next.kind, AtomKind::Comma) && frame != Frame::Group {
                self.cursor = saved;
                break;
            }
            if matches!(next.kind, AtomKind::Opaque) {
                return Err(error(
                    next.at,
                    format_smolstr!("expected a formula, got {:?}", next.text),
                ));
            }
            if matches!(next.kind, AtomKind::UnclosedText) {
                return Err(error(next.at, "expected the closing \" of a text"));
            }
            if matches!(next.kind, AtomKind::Spill) {
                if 75 < min_bp {
                    self.cursor = saved;
                    break;
                }
                if !self.reference_like(lhs) {
                    return Err(error(next.at, "expected a reference before #"));
                }
                self.take();
                lhs = self.push(Node::Spill(lhs));
                continue;
            }
            if matches!(next.kind, AtomKind::Operator) && next.text == "%" {
                if 65 < min_bp {
                    self.cursor = saved;
                    break;
                }
                self.take();
                lhs = self.push(Node::Percent(lhs));
                continue;
            }
            let (op, bp) = if spaced && self.reference_like(lhs) && self.starts_reference(&next) {
                (BinaryOp::Intersection, 90)
            } else if matches!(next.kind, AtomKind::Range) {
                (BinaryOp::Range, 100)
            } else if matches!(next.kind, AtomKind::Comma) && frame == Frame::Group {
                (BinaryOp::Union, 85)
            } else if matches!(next.kind, AtomKind::Operator) {
                binary(&next.text).ok_or_else(|| {
                    error(
                        next.at,
                        format_smolstr!("expected an operator, got {}", next.text),
                    )
                })?
            } else {
                if matches!(
                    next.kind,
                    AtomKind::Comma
                        | AtomKind::Semicolon
                        | AtomKind::Close
                        | AtomKind::ImplicitClose
                        | AtomKind::ArrayClose
                ) {
                    return Err(self.trailing(&next));
                }
                return Err(error(
                    next.at,
                    format_smolstr!("expected an operator before {:?}", next.text),
                ));
            };
            if bp < min_bp {
                self.cursor = saved;
                break;
            }
            if op != BinaryOp::Intersection {
                self.take();
            }
            let rhs = self.expression(bp + 1, frame)?;
            lhs = self.push(Node::Binary {
                op,
                left: lhs,
                right: rhs,
            });
        }
        Ok(lhs)
    }

    fn starts_reference(&self, atom: &Atom) -> bool {
        matches!(
            atom.kind,
            AtomKind::Reference(_)
                | AtomKind::Structured
                | AtomKind::Function { .. }
                | AtomKind::Open
        )
    }

    fn reference_like(&self, mut id: Id) -> bool {
        loop {
            match &self.nodes[id] {
                Node::Reference(_)
                | Node::Held(Held::Structured)
                | Node::Spill(_)
                | Node::Call { .. } => return true,
                Node::Group(inner)
                | Node::Unary {
                    op: UnaryOp::ImplicitIntersection,
                    value: inner,
                } => id = *inner,
                Node::Binary { op, .. } => {
                    return matches!(
                        op,
                        BinaryOp::Range | BinaryOp::Intersection | BinaryOp::Union
                    );
                }
                Node::Literal(_)
                | Node::Held(_)
                | Node::Unary { .. }
                | Node::Percent(_)
                | Node::Array(_) => return false,
            }
        }
    }

    fn trailing(&self, atom: &Atom) -> Error {
        match atom.kind {
            AtomKind::Close | AtomKind::ImplicitClose => error(atom.at, "expected no )"),
            AtomKind::Comma => error(
                atom.at,
                "expected , between the arguments of a call or inside parentheses",
            ),
            AtomKind::Semicolon => error(atom.at, "expected ; only between the rows of an array"),
            _ => error(
                atom.at,
                format_smolstr!("expected an operator before {:?}", atom.text),
            ),
        }
    }
}

fn binary(op: &str) -> Option<(BinaryOp, u8)> {
    match op {
        "^" => Some((BinaryOp::Power, 60)),
        "*" => Some((BinaryOp::Multiply, 50)),
        "/" => Some((BinaryOp::Divide, 50)),
        "+" => Some((BinaryOp::Add, 40)),
        "-" => Some((BinaryOp::Subtract, 40)),
        "&" => Some((BinaryOp::Concat, 30)),
        "=" => Some((BinaryOp::Equal, 20)),
        "<>" => Some((BinaryOp::NotEqual, 20)),
        "<" => Some((BinaryOp::Less, 20)),
        ">" => Some((BinaryOp::Greater, 20)),
        "<=" => Some((BinaryOp::LessEqual, 20)),
        ">=" => Some((BinaryOp::GreaterEqual, 20)),
        _ => None,
    }
}
