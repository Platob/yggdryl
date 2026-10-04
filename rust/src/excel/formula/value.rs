//! The evaluator's typed scalar operands and uncomputed result.

use crate::Str;
use crate::excel::entry;
use std::cmp::Ordering;

use super::functions::Function;
use super::parser::{BinaryOp, Literal};
use super::shape::Held;
use crate::excel::cell::{DateSystem, ExcelError};

/// One slot in the current context's formula-scoped range arena. It cannot
/// escape an evaluation or become a published cell cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReferenceId(pub(crate) usize);

/// Index of an existing definition in the current immutable workbook. It is
/// carried only by formula-scoped continuations, never a published cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct NameId(pub(crate) usize);

/// A computed scalar or a resolved range handle; no range is materialized.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Operand {
    Blank,
    Reference(ReferenceId),
    Number(f64),
    Text(Str),
    Boolean(bool),
    Error(ExcelError),
}

impl Operand {
    /// SUMPRODUCT treats text, Booleans and blanks as zero even when direct;
    /// its numeric-factor policy differs from arithmetic and PRODUCT.
    pub(crate) fn sumproduct_factor(self) -> Result<f64, ExcelError> {
        match self {
            Self::Number(value) => Ok(value),
            Self::Blank | Self::Text(_) | Self::Boolean(_) => Ok(0.0),
            Self::Error(error) => Err(error),
            Self::Reference(_) => unreachable!("the caller resolves reference geometry"),
        }
    }

    /// Direct arithmetic/function number coercion. Reference and array
    /// arguments retain their source context and decide inclusion upstream.
    /// Entry owns numeric and temporal text grammar; DateSystem owns the
    /// serial, so the same authored date has the correct 1900/1904 number.
    pub(crate) fn number(self, system: DateSystem) -> Option<Result<f64, ExcelError>> {
        match self {
            Self::Blank => Some(Ok(0.0)),
            Self::Reference(_) => None,
            Self::Number(value) => Some(Ok(value)),
            Self::Boolean(value) => Some(Ok(if value { 1.0 } else { 0.0 })),
            Self::Text(value) => {
                let text = value.as_str().trim();
                if let Some(parsed) = entry::number(text) {
                    Some(Ok(parsed.value))
                } else if let Some((temporal, _)) = entry::temporal(text, system) {
                    system
                        .serial_of(&temporal)
                        .ok()
                        .flatten()
                        .map(|(serial, _)| Ok(serial))
                } else {
                    Some(Err(ExcelError::Value))
                }
            }
            Self::Error(error) => Some(Err(error)),
        }
    }

    /// Scalar logical intake proven for NOT. Text depends on the host Excel
    /// formula locale, which an OOXML workbook does not declare; hold it until
    /// that context has one typed owner. Range functions decide inclusion at
    /// their reference visitor, outside this scalar boundary.
    pub(crate) fn logical(self) -> Option<Result<bool, ExcelError>> {
        match self {
            Self::Blank => Some(Ok(false)),
            Self::Reference(_) | Self::Text(_) => None,
            Self::Number(value) => Some(Ok(value != 0.0)),
            Self::Boolean(value) => Some(Ok(value)),
            Self::Error(error) => Some(Err(error)),
        }
    }

    /// Integer-valued Excel functions accept typed numbers, blanks and
    /// numeric/temporal text. Entry's parser-proved currency spelling
    /// is a value error in the native function despite valid cell entry.
    /// Boolean is a value error rather than the arithmetic 1/0 coercion;
    /// references have to cross Context's scalar door first.
    pub(crate) fn integer_number(self, system: DateSystem) -> Option<Result<f64, ExcelError>> {
        match self {
            Self::Number(value) => Some(Ok(value)),
            Self::Blank => Some(Ok(0.0)),
            Self::Boolean(_) => Some(Err(ExcelError::Value)),
            Self::Text(value) => {
                let text = value.as_str().trim();
                if let Some(parsed) = entry::number(text) {
                    if parsed.currency {
                        Some(Err(ExcelError::Value))
                    } else {
                        Some(Ok(parsed.value))
                    }
                } else if let Some((temporal, _)) = entry::temporal(text, system) {
                    Some(
                        system
                            .serial_of(&temporal)
                            .ok()
                            .flatten()
                            .map(|(serial, _)| serial)
                            .ok_or(ExcelError::Value),
                    )
                } else {
                    Some(Err(ExcelError::Value))
                }
            }
            Self::Error(error) => Some(Err(error)),
            Self::Reference(_) => None,
        }
    }

    /// Excel comparison preserves operand kinds: number < text < Boolean.
    /// Blank takes the other operand's zero/empty/FALSE comparison value.
    /// Unproven text collation retains the cache instead of guessing a locale.
    pub(crate) fn order(&self, other: &Self) -> Option<Result<Ordering, ExcelError>> {
        use Ordering::{Equal, Greater, Less};
        match (self, other) {
            (Self::Error(error), _) | (_, Self::Error(error)) => Some(Err(*error)),
            (Self::Reference(_), _) | (_, Self::Reference(_)) => None,
            (Self::Blank, Self::Blank) => Some(Ok(Equal)),
            (Self::Number(left), Self::Number(right)) => Some(super::number::compare(*left, *right)),
            (Self::Blank, Self::Number(right)) => Some(super::number::compare(0.0, *right)),
            (Self::Number(left), Self::Blank) => Some(super::number::compare(*left, 0.0)),
            (Self::Boolean(left), Self::Boolean(right)) => Some(Ok(left.cmp(right))),
            (Self::Blank, Self::Boolean(right)) => Some(Ok(false.cmp(right))),
            (Self::Boolean(left), Self::Blank) => Some(Ok(left.cmp(&false))),
            (Self::Text(left), Self::Text(right)) => Self::text_order(left.as_str(), right.as_str()).map(Ok),
            (Self::Blank, Self::Text(right)) => Self::text_order("", right.as_str()).map(Ok),
            (Self::Text(left), Self::Blank) => Self::text_order(left.as_str(), "").map(Ok),
            (Self::Number(_), Self::Text(_) | Self::Boolean(_))
                | (Self::Text(_), Self::Boolean(_)) => Some(Ok(Less)),
            (Self::Text(_) | Self::Boolean(_), Self::Number(_))
                | (Self::Boolean(_), Self::Text(_)) => Some(Ok(Greater)),
        }
    }

    fn text_order(left: &str, right: &str) -> Option<Ordering> {
        if left == right {
            return Some(Ordering::Equal);
        }
        // The native corpus establishes this portable subset. Punctuation,
        // accents and mixed Unicode strings require an explicit collation.
        if !left.bytes().all(|byte| byte.is_ascii_alphanumeric())
            || !right.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return None;
        }
        Some(left.bytes().map(|byte| byte.to_ascii_lowercase())
            .cmp(right.bytes().map(|byte| byte.to_ascii_lowercase())))
    }

    pub(crate) fn literal(value: &Literal) -> Self {
        match value {
            Literal::Number(value) => Self::Number(*value),
            Literal::Text(value) => Self::Text(value.clone()),
            Literal::Boolean(value) => Self::Boolean(*value),
            Literal::Error(value) => Self::Error(*value),
        }
    }
}

/// An explicit semantic boundary: the prior cached value stays untouched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unevaluated {
    Held(Held),
    Reference,
    ReferenceAnchor,
    ReferenceScope,
    ReferenceNonWorksheet,
    ReferenceComplexity,
    Function(Function),
    Array,
    Spill,
    Binary(BinaryOp),
    NumericPolicy,
    TemporalSerial,
    TextCompatibility,
    Coercion,
}

/// A literal in an immutable expression, or a formula-scoped execution plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArrayId {
    Literal { name: Option<NameId>, node: usize },
    Mapped(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Outcome {
    Computed(Operand),
    /// Borrowed constants or a retained element operation, without a value
    /// grid. Scalar-selected results keep their legacy projection boundary.
    Array { array: ArrayId, implicit: bool },
    /// A scalar produced by @/SINGLE; names and grouping preserve this
    /// provenance until the next value consumer removes it.
    Intersection { value: Operand, referenced: bool },
    Uncomputed(Unevaluated),
}

impl Outcome {
    pub(crate) fn operand(self) -> std::result::Result<Operand, Unevaluated> {
        match self {
            Self::Computed(value) | Self::Intersection { value, .. } => Ok(value),
            Self::Uncomputed(reason) => Err(reason),
            Self::Array { .. } => Err(Unevaluated::Array),
        }
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! Typed direct-operand numeric intake over the Entry-owned grammar.

    use super::Operand;
    use crate::Str;
    use crate::excel::{DateSystem, ExcelError};

    /// Parse one text operand without entering the cell-entry formula grammar.
    pub fn number_text(text: &str, system: DateSystem) -> Option<Result<f64, ExcelError>> {
        Operand::Text(Str::new(text)).number(system)
    }

    /// Coerce an already typed Boolean operand.
    pub fn number_boolean(value: bool) -> Option<Result<f64, ExcelError>> {
        Operand::Boolean(value).number(DateSystem::Year1900)
    }

    /// Coerce a missing referenced cell independently of empty text.
    pub fn number_blank() -> Option<Result<f64, ExcelError>> {
        Operand::Blank.number(DateSystem::Year1900)
    }
}
