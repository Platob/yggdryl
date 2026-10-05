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
    /// Parsed once per shared shape; source SUBTOTAL results are excluded by
    /// an outer SUBTOTAL without rescanning this formula per referenced row.
    pub(crate) has_subtotal: bool,
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
    Select(Selection<'a>),
    Held,
}

/// A selector borrows the same arena children as strict evaluation. Empty
/// and absent IF arguments remain distinct; no branch expression is copied.
#[derive(Clone, Copy)]
pub(crate) enum Selection<'a> {
    If {
        test: Option<Id>,
        yes: Option<Id>,
        no: Option<Option<Id>>,
    },
    Error {
        value: Option<Id>,
        fallback: Option<Id>,
        na_only: bool,
    },
    Choose {
        index: Option<Id>,
        choices: &'a [Option<Id>],
    },
    Ifs {
        pairs: &'a [Option<Id>],
    },
    Switch {
        expression: Option<Id>,
        pairs: &'a [Option<Id>],
        fallback: Option<Option<Id>>,
    },
    Xlookup {
        args: &'a [Option<Id>],
    },
}

impl Selection<'_> {
    /// Initial Scalar inputs, in stack order. Every condition/key is a
    /// dependency; result/default expressions are admitted only when chosen.
    pub(crate) fn visit_inputs_reverse(self, mut visit: impl FnMut(Id, ReferenceUse)) {
        match self {
            Self::If { test, .. } => test
                .into_iter()
                .for_each(|id| visit(id, ReferenceUse::Scalar)),
            Self::Error { value, .. } => value
                .into_iter()
                .for_each(|id| visit(id, ReferenceUse::Scalar)),
            Self::Choose { index, .. } => index
                .into_iter()
                .for_each(|id| visit(id, ReferenceUse::Scalar)),
            Self::Ifs { pairs } => {
                for pair in pairs.as_chunks::<2>().0.iter().rev() {
                    if let Some(test) = pair[0] {
                        visit(test, ReferenceUse::Scalar);
                    }
                }
            }
            Self::Switch {
                expression, pairs, ..
            } => {
                for pair in pairs.as_chunks::<2>().0.iter().rev() {
                    if let Some(key) = pair[0] {
                        visit(key, ReferenceUse::Scalar);
                    }
                }
                if let Some(expression) = expression {
                    visit(expression, ReferenceUse::Scalar);
                }
            }
            Self::Xlookup { args } => {
                for index in [5, 4, 2, 1, 0] {
                    if let Some(Some(child)) = args.get(index) {
                        let use_as = match index {
                            1 | 2 => ReferenceUse::Geometry,
                            _ => ReferenceUse::Scalar,
                        };
                        visit(*child, use_as);
                    }
                }
            }
        }
    }
}

/// How a legacy formula consumes a reference. Scalar uses implicit
/// intersection; a direct aggregate reads its range; ISREF reads only
/// the identity of a resolved reference, with no value dependency.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ReferenceUse {
    Scalar,
    Range,
    /// One rectangle only; invalid unions admit no value dependencies.
    SingleRange,
    Geometry,
}

/// Borrowed child IDs in evaluator stack order; visiting costs no allocation.
pub(crate) enum Children<'a> {
    One(Id, Option<ReferenceUse>),
    Intersection(Id),
    Two(Id, Id),
    GeometryPair(Id, Id),
    Call(&'a [Option<Id>], ReferenceUse),
    /// Mixed argument roles share one traversal. A dynamic result registers
    /// its returned reference only when consumed by the caller.
    MixedCall {
        args: &'a [Option<Id>],
        prefix: &'static [ReferenceUse],
        rest: &'static [ReferenceUse],
        dynamic: bool,
    },
}

impl Children<'_> {
    /// Grouping passes its consumer through; every other strict owner states
    /// the reference use once for both scheduling and dependency intake.
    pub(crate) fn reference_use(&self, inherited: ReferenceUse) -> ReferenceUse {
        match self {
            Self::One(_, value) => value.unwrap_or(inherited),
            Self::Intersection(_) if inherited == ReferenceUse::Geometry => ReferenceUse::Geometry,
            Self::Intersection(_) | Self::Two(_, _) => ReferenceUse::Scalar,
            Self::GeometryPair(_, _) => ReferenceUse::Geometry,
            Self::Call(_, value) => *value,
            Self::MixedCall { .. } => unreachable!("mixed roles are visited individually"),
        }
    }

    #[cfg(feature = "internals")]
    pub(crate) fn visit_reverse(self, mut visit: impl FnMut(Id)) {
        self.visit_references(ReferenceUse::Scalar, |id, _| visit(id));
    }

    pub(crate) fn visit_references(
        self,
        inherited: ReferenceUse,
        mut visit: impl FnMut(Id, ReferenceUse),
    ) {
        if let Self::MixedCall {
            args, prefix, rest, ..
        } = self
        {
            for (index, child) in args.iter().enumerate().rev() {
                if let Some(child) = child {
                    visit(
                        *child,
                        prefix
                            .get(index)
                            .copied()
                            .unwrap_or_else(|| rest[(index - prefix.len()) % rest.len()]),
                    );
                }
            }
            return;
        }
        let usage = self.reference_use(inherited);
        match self {
            Self::One(child, _) | Self::Intersection(child) => visit(child, usage),
            Self::Two(left, right) | Self::GeometryPair(left, right) => {
                visit(right, usage);
                visit(left, usage);
            }
            Self::Call(args, _) => {
                for child in args.iter().rev().flatten() {
                    visit(*child, usage);
                }
            }
            Self::MixedCall { .. } => unreachable!("handled before shared-use children"),
        }
    }
}

impl Node {
    /// One policy for scheduling and dependency intake. Exact call shapes
    /// match current ready-stage dispatch; held/lazy calls do not expose
    /// references that could create false cycles.
    pub(crate) fn evaluation_children(&self) -> EvaluationPolicy<'_> {
        match self {
            Self::Literal(_) | Self::Reference(_) | Self::Array(_) => EvaluationPolicy::Leaf,
            Self::Group(child) => EvaluationPolicy::Strict(Children::One(*child, None)),
            Self::Unary {
                op: UnaryOp::ImplicitIntersection,
                value,
            } => EvaluationPolicy::Strict(Children::Intersection(*value)),
            Self::Percent(child) | Self::Unary { value: child, .. } => {
                EvaluationPolicy::Strict(Children::One(*child, Some(ReferenceUse::Scalar)))
            }
            Self::Binary {
                op: BinaryOp::Range | BinaryOp::Intersection | BinaryOp::Union,
                left,
                right,
            } => EvaluationPolicy::Strict(Children::GeometryPair(*left, *right)),
            Self::Binary { op, left, right }
                if op.comparison().is_some()
                    || matches!(
                        *op,
                        BinaryOp::Add
                            | BinaryOp::Subtract
                            | BinaryOp::Multiply
                            | BinaryOp::Divide
                            | BinaryOp::Power
                            | BinaryOp::Concat
                    ) =>
            {
                EvaluationPolicy::Strict(Children::Two(*left, *right))
            }
            Self::Call {
                function: Some(Function::If),
                args,
            } if (2..=3).contains(&args.len()) => EvaluationPolicy::Select(Selection::If {
                test: args[0],
                yes: args[1],
                no: args.get(2).copied(),
            }),
            Self::Call {
                function: Some(function @ (Function::Iferror | Function::Ifna)),
                args,
            } if args.len() == 2 => EvaluationPolicy::Select(Selection::Error {
                value: args[0],
                fallback: args[1],
                na_only: *function == Function::Ifna,
            }),
            Self::Call {
                function: Some(Function::Choose),
                args,
            } if args.len() >= 2 => EvaluationPolicy::Select(Selection::Choose {
                index: args[0],
                choices: &args[1..],
            }),
            Self::Call {
                function: Some(Function::Ifs),
                args,
            } => EvaluationPolicy::Select(Selection::Ifs { pairs: args }),
            Self::Call {
                function: Some(Function::Switch),
                args,
            } => {
                let tail = &args[1..];
                let paired = tail.len() / 2 * 2;
                EvaluationPolicy::Select(Selection::Switch {
                    expression: args[0],
                    pairs: &tail[..paired],
                    fallback: tail.get(paired).copied(),
                })
            }
            Self::Call {
                function: Some(Function::Match),
                args,
            } if (2..=3).contains(&args.len()) && args.iter().all(Option::is_some) => {
                EvaluationPolicy::Strict(Children::MixedCall {
                    args,
                    prefix: &[ReferenceUse::Scalar, ReferenceUse::Geometry],
                    rest: &[ReferenceUse::Scalar],
                    dynamic: true,
                })
            }
            Self::Call {
                function: Some(Function::Vlookup),
                args,
            } if (3..=4).contains(&args.len()) && args.iter().all(Option::is_some) => {
                EvaluationPolicy::Strict(Children::MixedCall {
                    args,
                    prefix: &[ReferenceUse::Scalar, ReferenceUse::Geometry],
                    rest: &[ReferenceUse::Scalar],
                    dynamic: true,
                })
            }
            Self::Call {
                function: Some(Function::Hlookup),
                args,
            } if (3..=4).contains(&args.len()) && args.iter().all(Option::is_some) => {
                EvaluationPolicy::Strict(Children::MixedCall {
                    args,
                    prefix: &[ReferenceUse::Scalar, ReferenceUse::Geometry],
                    rest: &[ReferenceUse::Scalar],
                    dynamic: true,
                })
            }
            Self::Call {
                function: Some(Function::Lookup),
                args,
            } if args.len() == 3 && args.iter().all(Option::is_some) => {
                EvaluationPolicy::Strict(Children::MixedCall {
                    args,
                    prefix: &[
                        ReferenceUse::Scalar,
                        ReferenceUse::Geometry,
                        ReferenceUse::Geometry,
                    ],
                    rest: &[ReferenceUse::Scalar],
                    dynamic: true,
                })
            }
            Self::Call {
                function: Some(Function::Xlookup),
                args,
            } if (3..=6).contains(&args.len()) && args[..3].iter().all(Option::is_some) => {
                EvaluationPolicy::Select(Selection::Xlookup { args })
            }
            Self::Call {
                function: Some(Function::Indirect),
                args,
            } if (1..=2).contains(&args.len()) && args.iter().all(Option::is_some) => {
                EvaluationPolicy::Strict(Children::MixedCall {
                    args,
                    prefix: &[],
                    rest: &[ReferenceUse::Scalar],
                    dynamic: true,
                })
            }
            Self::Call {
                function: Some(Function::Index | Function::Offset),
                args,
            } => EvaluationPolicy::Strict(Children::MixedCall {
                args,
                prefix: &[ReferenceUse::Geometry],
                rest: &[ReferenceUse::Scalar],
                dynamic: true,
            }),
            Self::Call {
                function:
                    Some(
                        Function::Large
                        | Function::Small
                        | Function::Percentile
                        | Function::PercentileDotInc
                        | Function::Quartile
                        | Function::QuartileDotInc,
                    ),
                args,
            } => {
                if args.iter().all(Option::is_some) {
                    EvaluationPolicy::Strict(Children::MixedCall {
                        args,
                        prefix: &[ReferenceUse::Range],
                        rest: &[ReferenceUse::Scalar],
                        dynamic: false,
                    })
                } else {
                    EvaluationPolicy::Held
                }
            }
            Self::Call {
                function: Some(Function::Rank | Function::RankDotEq),
                args,
            } => {
                if args.iter().all(Option::is_some) {
                    EvaluationPolicy::Strict(Children::MixedCall {
                        args,
                        prefix: &[ReferenceUse::Scalar, ReferenceUse::Range],
                        rest: &[ReferenceUse::Scalar],
                        dynamic: false,
                    })
                } else {
                    EvaluationPolicy::Held
                }
            }
            Self::Call {
                function: Some(Function::Countif | Function::Sumif | Function::Averageif),
                args,
            } if args.iter().all(Option::is_some) => {
                EvaluationPolicy::Strict(Children::MixedCall {
                    args,
                    prefix: &[],
                    rest: &[
                        ReferenceUse::SingleRange,
                        ReferenceUse::Scalar,
                        ReferenceUse::SingleRange,
                    ],
                    dynamic: false,
                })
            }
            Self::Call {
                function: Some(Function::Countifs),
                args,
            } if args.iter().all(Option::is_some) => {
                EvaluationPolicy::Strict(Children::MixedCall {
                    args,
                    prefix: &[],
                    rest: &[ReferenceUse::SingleRange, ReferenceUse::Scalar],
                    dynamic: false,
                })
            }
            Self::Call {
                function:
                    Some(
                        Function::Sumifs
                        | Function::Averageifs
                        | Function::Maxifs
                        | Function::Minifs,
                    ),
                args,
            } if args.iter().all(Option::is_some) => {
                EvaluationPolicy::Strict(Children::MixedCall {
                    args,
                    prefix: &[ReferenceUse::SingleRange],
                    rest: &[ReferenceUse::SingleRange, ReferenceUse::Scalar],
                    dynamic: false,
                })
            }
            Self::Call {
                function: Some(Function::Subtotal),
                args,
            } if args.len() >= 2 && args.iter().all(Option::is_some) => {
                EvaluationPolicy::Strict(Children::MixedCall {
                    args,
                    prefix: &[ReferenceUse::Scalar],
                    rest: &[ReferenceUse::Geometry],
                    dynamic: true,
                })
            }
            Self::Call {
                function: Some(Function::Countblank),
                args,
            } if args.len() == 1 && args[0].is_some() => {
                EvaluationPolicy::Strict(Children::Call(args.as_ref(), ReferenceUse::SingleRange))
            }
            Self::Call {
                function: Some(Function::Sumproduct),
                args,
            } if args.iter().all(Option::is_some) => {
                EvaluationPolicy::Strict(Children::Call(args.as_ref(), ReferenceUse::SingleRange))
            }
            Self::Call {
                function: Some(Function::Pv | Function::Fv | Function::Pmt),
                args,
            } => EvaluationPolicy::Strict(Children::Call(args.as_ref(), ReferenceUse::Scalar)),
            Self::Call {
                function: Some(Function::Npv),
                args,
            } => EvaluationPolicy::Strict(Children::MixedCall {
                args,
                prefix: &[ReferenceUse::Scalar],
                rest: &[ReferenceUse::Range],
                dynamic: false,
            }),
            Self::Call {
                function: Some(Function::Textjoin),
                args,
            } => {
                if args.iter().all(Option::is_some) {
                    EvaluationPolicy::Strict(Children::MixedCall {
                        args,
                        prefix: &[ReferenceUse::Scalar, ReferenceUse::Scalar],
                        rest: &[ReferenceUse::SingleRange],
                        dynamic: false,
                    })
                } else {
                    EvaluationPolicy::Held
                }
            }
            Self::Call {
                function: Some(function),
                args,
            } => {
                let (shape, usage) = match *function {
                    Function::Len
                    | Function::Abs
                    | Function::Sqrt
                    | Function::Fact
                    | Function::Not
                    | Function::Exp
                    | Function::Ln
                    | Function::Log10
                    | Function::Degrees
                    | Function::Radians
                    | Function::Cos
                    | Function::Asin
                    | Function::Sin
                    | Function::Tan
                    | Function::Acos
                    | Function::Atan
                    | Function::Sign
                    | Function::Int
                    | Function::ErrorDotType
                    | Function::Isblank
                    | Function::Iserr
                    | Function::Iserror
                    | Function::Iseven
                    | Function::Islogical
                    | Function::Isna
                    | Function::Isnontext
                    | Function::Isnumber
                    | Function::Isodd
                    | Function::Istext
                    | Function::N => (args.len() == 1, ReferenceUse::Scalar),
                    Function::Year | Function::Month | Function::Day => {
                        (args.len() == 1, ReferenceUse::Scalar)
                    }
                    Function::Weekday | Function::Log => {
                        ((1..=2).contains(&args.len()), ReferenceUse::Scalar)
                    }
                    Function::Days => (args.len() == 2, ReferenceUse::Scalar),
                    Function::Date | Function::Time => (args.len() == 3, ReferenceUse::Scalar),
                    Function::Edate | Function::Eomonth => (args.len() == 2, ReferenceUse::Scalar),
                    Function::Hour
                    | Function::Minute
                    | Function::Second
                    | Function::Datevalue
                    | Function::Timevalue => (args.len() == 1, ReferenceUse::Scalar),
                    Function::T
                    | Function::Clean
                    | Function::Trim
                    | Function::Left
                    | Function::Right
                    | Function::Mid
                    | Function::Exact
                    | Function::Rept
                    | Function::Lower
                    | Function::Upper
                    | Function::Search
                    | Function::Char
                    | Function::Code
                    | Function::Proper
                    | Function::Value
                    | Function::Text
                    | Function::Substitute
                    | Function::Find
                    | Function::Replace => (true, ReferenceUse::Scalar),
                    Function::Concat => (true, ReferenceUse::SingleRange),
                    Function::Concatenate => (true, ReferenceUse::Scalar),
                    Function::Address => ((2..=5).contains(&args.len()), ReferenceUse::Scalar),
                    Function::Isref => (args.len() == 1, ReferenceUse::Geometry),
                    Function::Row | Function::Column | Function::Rows | Function::Columns => {
                        (true, ReferenceUse::Geometry)
                    }
                    Function::Trunc => ((1..=2).contains(&args.len()), ReferenceUse::Scalar),
                    Function::True
                    | Function::False
                    | Function::Na
                    | Function::Pi
                    | Function::Now
                    | Function::Today
                    | Function::Rand => (args.is_empty(), ReferenceUse::Scalar),
                    Function::Even | Function::Odd => (args.len() == 1, ReferenceUse::Scalar),
                    Function::CeilingDotMath | Function::FloorDotMath => {
                        ((1..=3).contains(&args.len()), ReferenceUse::Scalar)
                    }
                    Function::Round
                    | Function::Roundup
                    | Function::Rounddown
                    | Function::Atan2
                    | Function::Quotient
                    | Function::Ceiling
                    | Function::Floor
                    | Function::Mround
                    | Function::Mod
                    | Function::Power
                    | Function::Randbetween => (args.len() == 2, ReferenceUse::Scalar),
                    Function::Var
                    | Function::Varp
                    | Function::VarDotS
                    | Function::VarDotP
                    | Function::Stdev
                    | Function::Stdevp
                    | Function::StdevDotS
                    | Function::StdevDotP
                    | Function::Sum
                    | Function::Count
                    | Function::Counta
                    | Function::Min
                    | Function::Max
                    | Function::And
                    | Function::Or
                    | Function::Xor => (true, ReferenceUse::Range),
                    Function::Gcd
                    | Function::Lcm
                    | Function::Average
                    | Function::Averagea
                    | Function::Mina
                    | Function::Maxa
                    | Function::Product
                    | Function::Median
                    | Function::Mode
                    | Function::ModeDotSngl => {
                        ((1..=255).contains(&args.len()), ReferenceUse::Range)
                    }
                    _ => (false, ReferenceUse::Scalar),
                };
                if shape && args.iter().all(Option::is_some) {
                    EvaluationPolicy::Strict(Children::Call(args.as_ref(), usage))
                } else {
                    EvaluationPolicy::Held
                }
            }
            Self::Held(_) | Self::Binary { .. } | Self::Spill(_) | Self::Call { .. } => {
                EvaluationPolicy::Held
            }
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

impl BinaryOp {
    /// Excel owns operand ordering; the shared comparison owns its predicate.
    pub(crate) const fn comparison(self) -> Option<crate::expression::Comparison> {
        use crate::expression::Comparison;
        Some(match self {
            Self::Equal => Comparison::Eq,
            Self::NotEqual => Comparison::NotEq,
            Self::Less => Comparison::Lt,
            Self::Greater => Comparison::Gt,
            Self::LessEqual => Comparison::LtEq,
            Self::GreaterEqual => Comparison::GtEq,
            _ => return None,
        })
    }
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
        let has_subtotal = self.nodes.iter().any(|node| {
            matches!(
                node,
                Node::Call {
                    function: Some(Function::Subtotal),
                    ..
                }
            )
        });
        Ok(Expr {
            nodes: self.nodes.into_boxed_slice(),
            root,
            has_subtotal,
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

    /// Literal array operations have a streaming evaluator. Range-expression
    /// array arithmetic is still distinct from legacy scalar intersection;
    /// hold that unrepresented mode before it can admit false dependencies.
    fn array_expression_argument(&self, mut id: Id) -> bool {
        while let Node::Group(child) = &self.nodes[id] {
            id = *child;
        }
        if matches!(
            &self.nodes[id],
            Node::Binary {
                op: BinaryOp::Range | BinaryOp::Intersection | BinaryOp::Union,
                ..
            }
        ) {
            return false;
        }
        if matches!(&self.nodes[id], Node::Spill(_)) {
            return true;
        }
        if !matches!(
            &self.nodes[id],
            Node::Binary { .. } | Node::Unary { .. } | Node::Percent(_)
        ) {
            return false;
        }
        let mut pending = vec![id];
        while let Some(node) = pending.pop() {
            match &self.nodes[node] {
                Node::Reference(reference)
                    if matches!(
                        reference.target,
                        Target::Area { .. }
                            | Target::Rows { .. }
                            | Target::Columns { .. }
                            | Target::Name(_)
                    ) =>
                {
                    return true;
                }
                Node::Group(child) | Node::Percent(child) | Node::Spill(child) => {
                    pending.push(*child)
                }
                Node::Unary { value, .. } => pending.push(*value),
                Node::Binary { left, right, .. } => {
                    pending.push(*right);
                    pending.push(*left);
                }
                Node::Call { args, .. } => pending.extend(args.iter().flatten().copied()),
                Node::Array(_) | Node::Literal(_) | Node::Reference(_) | Node::Held(_) => {}
            }
        }
        false
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
        if let Some(signature) = function.and_then(|function| function.info().signature)
            && !signature.arity.accepts(args.len())
        {
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
        if function == Some(Function::Sumproduct)
            && args
                .iter()
                .flatten()
                .any(|id| self.array_expression_argument(*id))
        {
            return Ok(self.push(Node::Held(Held::ArrayExpression)));
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
                    if let Some(first) = rows.first()
                        && first.len() != row.len()
                    {
                        return Err(error(
                            delimiter.at,
                            format_smolstr!(
                                "expected {} columns in an array row, got {}",
                                first.len(),
                                row.len()
                            ),
                        ));
                    }
                    rows.push(row.into_boxed_slice());
                    row = Vec::new();
                }
                Some(AtomKind::ArrayClose) => {
                    let close = self.take().expect("peeked array close");
                    if let Some(first) = rows.first()
                        && first.len() != row.len()
                    {
                        return Err(error(
                            close.at,
                            format_smolstr!(
                                "expected {} columns in an array row, got {}",
                                first.len(),
                                row.len()
                            ),
                        ));
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
