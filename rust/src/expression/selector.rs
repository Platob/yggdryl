//! `&holder.*` attributes: what a handle knows about itself, and what it costs
//! to ask.
//!
//! A predicate over a lake asks two different kinds of question. `ccy = 'EUR'`
//! is about the rows, and answering it means decoding them. `&holder.size > 0`
//! and `&holder.partition['year'] = '2024'` are about the *container* - the
//! file, the directory, the manifest - and answering those can skip the decode
//! entirely. They live in one grammar because a reader that has to run two
//! filters in two languages ends up running the expensive one first.
//!
//! # Cost is part of the type
//!
//! Every selector declares a [`Cost`], and the cost is what makes ordering
//! sound rather than lucky:
//!
//! * [`Cost::Free`] - derived from the [`Url`] alone. No syscall, no request,
//!   no listing. `&holder.name`, `&holder.partition['year']`, `&holder.depth`.
//! * [`Cost::Stat`] - one call into the backing store. `&holder.size`,
//!   `&holder.kind`.
//!
//! [`bind`](super::bind) sorts a conjunction cheapest-first and the evaluator
//! short-circuits, so a free-attribute conjunct that answers `false` costs
//! exactly zero backend calls. A cost class may be *over*-stated without
//! harming correctness - a selector ordered later is still answered - so an
//! attribute whose price depends on the backend is classified by its worst
//! case.

use smol_str::{SmolStr, format_smolstr};

use crate::io::IOBase;
use crate::{DataType, Error, Field, Result, Url, Value};

/// What answering one attribute costs.
///
/// Ordering is the point: `Free < Stat`, so sorting conjuncts by the highest
/// cost they contain is a plain `sort_by_key`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Default)]
pub enum Cost {
    /// Answered from the [`Url`] alone - no call into the backing store.
    #[default]
    Free,
    /// One call into the backing store.
    Stat,
}

impl Cost {
    /// The canonical text of this cost class.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Stat => "stat",
        }
    }
}

/// One attribute of the handle a predicate is being asked about.
///
/// Written `&holder.name` in the grammar - the `&` marks it as a question about
/// the container rather than about a column, which is what keeps a lake with a
/// column literally named `size` unambiguous.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[non_exhaustive]
pub enum Selector {
    /// `&holder.url` - the whole identifier, as text.
    Url,
    /// `&holder.path` - the path component, as text.
    Path,
    /// `&holder.name` - the last path segment, extensions included.
    Name,
    /// `&holder.stem` - the last path segment with its extensions removed.
    Stem,
    /// `&holder.extension` - the final extension, without the dot.
    Extension,
    /// `&holder.scheme` - `file`, `s3`, `https`, and so on.
    Scheme,
    /// `&holder.parent` - the containing directory, as text.
    Parent,
    /// `&holder.depth` - how many path segments the identifier has.
    Depth,
    /// `&holder.mime_type` - the media type the *name* implies, which is free;
    /// the type the bytes prove is not this.
    MimeType,
    /// `&holder.partition['year']` - one Hive partition value off the path.
    ///
    /// Null when the path does not spell that column, which is what makes
    /// `&holder.partition['year'] = '2024'` prune an unpartitioned lake to
    /// nothing rather than erroring.
    Partition(SmolStr),
    /// `&holder.size` - the byte length, which costs one stat.
    Size,
    /// `&holder.kind` - `file`, `directory`, and the rest of [`IOKind`].
    ///
    /// [`IOKind`]: crate::IOKind
    Kind,
    /// `&holder.is_container` - whether this handle can hold others.
    IsContainer,
    /// `&holder.is_empty` - whether it holds no bytes.
    IsEmpty,
}

impl Selector {
    /// Every selector that takes no argument, in canonical spelling.
    pub const ALL: [Self; 13] = [
        Self::Url,
        Self::Path,
        Self::Name,
        Self::Stem,
        Self::Extension,
        Self::Scheme,
        Self::Parent,
        Self::Depth,
        Self::MimeType,
        Self::Size,
        Self::Kind,
        Self::IsContainer,
        Self::IsEmpty,
    ];

    /// The canonical name of this selector, without the `&holder.` prefix.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Url => "url",
            Self::Path => "path",
            Self::Name => "name",
            Self::Stem => "stem",
            Self::Extension => "extension",
            Self::Scheme => "scheme",
            Self::Parent => "parent",
            Self::Depth => "depth",
            Self::MimeType => "mime_type",
            Self::Partition(_) => "partition",
            Self::Size => "size",
            Self::Kind => "kind",
            Self::IsContainer => "is_container",
            Self::IsEmpty => "is_empty",
        }
    }

    /// Resolve an argument-free selector name, ASCII case-insensitively.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        let lowered = name.to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|selector| selector.as_str() == lowered)
    }

    /// Every selector name this grammar accepts, for an error message.
    #[must_use]
    pub fn vocabulary() -> String {
        let mut names: Vec<&str> = Self::ALL.iter().map(Self::as_str).collect();
        names.push("partition['column']");
        names.join(", ")
    }

    /// What answering this selector costs.
    #[must_use]
    pub const fn cost(&self) -> Cost {
        match self {
            Self::Size | Self::Kind | Self::IsContainer | Self::IsEmpty => Cost::Stat,
            _ => Cost::Free,
        }
    }

    /// The datatype this selector always answers in.
    ///
    /// Fixed per selector and never inferred, so `&holder.size > 100` compares
    /// two integers whatever the handle turns out to be.
    #[must_use]
    pub fn data_type(&self) -> DataType {
        match self {
            Self::Depth | Self::Size => DataType::Int64,
            Self::IsContainer | Self::IsEmpty => DataType::Boolean,
            _ => DataType::Utf8,
        }
    }

    /// The output field of this selector, named as it is written.
    ///
    /// Always nullable: a handle with no identifier answers null for every free
    /// selector rather than failing, because "this holder has no URL" is not a
    /// broken predicate.
    #[must_use]
    pub fn field(&self) -> Field {
        Field::new(format_smolstr!("&holder.{self}"), self.data_type(), true)
    }

    /// Answer this selector from an identifier alone.
    ///
    /// Every [`Cost::Free`] selector is answered here. A [`Cost::Stat`] one
    /// answers null, because a URL genuinely does not know a file's length -
    /// and null is this crate's spelling of "not known", not of "zero".
    #[must_use]
    pub fn read_url(&self, url: &Url) -> Value {
        let text = |value: Option<&str>| value.map_or(Value::Null, Value::from);
        match self {
            Self::Url => Value::from(url.to_string()),
            Self::Path => Value::from(url.path().as_str()),
            Self::Name => text(url.file_name()),
            Self::Stem => text(url.stem()),
            Self::Extension => text(url.extension()),
            Self::Scheme => Value::from(url.scheme().to_string()),
            Self::Parent => url
                .parent()
                .map_or(Value::Null, |parent| Value::from(parent.to_string())),
            Self::Depth => Value::I64(i64::try_from(url.path().segment_len()).unwrap_or(i64::MAX)),
            Self::MimeType => Value::from(url.mime_type().to_string()),
            Self::Partition(column) => url.hive_partition(column).map_or(Value::Null, Value::from),
            Self::Size | Self::Kind | Self::IsContainer | Self::IsEmpty => Value::Null,
        }
    }
}

impl std::fmt::Display for Selector {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Partition(column) => {
                formatter.write_str("partition[")?;
                super::display::write_text_literal(formatter, column)?;
                formatter.write_str("]")
            }
            other => formatter.write_str(other.as_str()),
        }
    }
}

/// Anything that can answer an attribute about itself.
///
/// One trait rather than one method per surface: a folder, a listing entry, a
/// manifest row, and a test double that counts its calls all answer the same
/// question, and the evaluator never learns which one it holds.
pub trait Attributes {
    /// Answer one attribute, or [`Value::Null`] when this holder has no answer.
    ///
    /// Absence is null rather than an error for the same reason a missing map
    /// key is: a predicate over a heterogeneous listing is asking *whether*,
    /// not asserting *that*.
    ///
    /// # Errors
    ///
    /// Returns the backing store's failure when a [`Cost::Stat`] selector
    /// cannot be answered.
    fn attribute(&self, selector: &Selector) -> Result<Value>;
}

impl Attributes for Url {
    fn attribute(&self, selector: &Selector) -> Result<Value> {
        Ok(selector.read_url(self))
    }
}

impl Attributes for dyn IOBase + '_ {
    fn attribute(&self, selector: &Selector) -> Result<Value> {
        read_handle(self, selector)
    }
}

/// Answer one selector against a live handle.
///
/// Free selectors reach only [`IOBase::url`]; the stat tier is the only place
/// that touches the backing store, and it is reached only when the selector
/// says so.
///
/// # Errors
///
/// Returns the backing store's failure.
pub fn read_handle(handle: &dyn IOBase, selector: &Selector) -> Result<Value> {
    match selector.cost() {
        Cost::Free => Ok(handle
            .url()
            .map_or(Value::Null, |url| selector.read_url(url))),
        Cost::Stat => Ok(match selector {
            Selector::Size => i64::try_from(handle.size()).map_or(Value::Null, Value::I64),
            Selector::Kind => Value::from(handle.kind().to_string()),
            Selector::IsContainer => Value::Bool(handle.is_container()),
            Selector::IsEmpty => Value::Bool(handle.is_empty()),
            // Unreachable by the cost table above, and stated rather than
            // panicked so a selector added later degrades to "not known".
            _ => Value::Null,
        }),
    }
}

/// The error a selector name that is not in the vocabulary produces.
pub(crate) fn unknown(name: &str, position: usize) -> Error {
    Error::Parse {
        target: "expression",
        position,
        reason: format_smolstr!(
            "expected one of the holder attributes {}, got {name:?}",
            Selector::vocabulary()
        ),
    }
}
