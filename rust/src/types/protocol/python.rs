//! The `python:` vocabulary, on the field views that carry it.
//!
//! A Python runtime declares a schema by writing a class: a dataclass, a
//! `TypedDict`, a `NamedTuple`, an enumeration, a `NewType`, a type alias, or
//! an ordinary class the annotation named. The three properties here are what
//! the field remembers of that declaration - where the class lives, what it is
//! called there, and which of those forms it is - so a reader can name the
//! class again without the runtime that built it.
//!
//! [`PythonMetadata`] is the whole declaration as one validated value: it is
//! parsed once at the boundary, travels typed, and is written back atomically.
//! The property names are private to this module, so a caller writes
//! `set_class`, never `"python:qualname"`.

use std::fmt;
use std::str::FromStr;

use smol_str::SmolStr;

use super::{PythonField, PythonFieldMut};
use crate::{Error, Result};

/// Where the declaring class lives, as a dotted module path.
const MODULE: &str = "module";
/// The full key the module is stored under, spelled once.
pub(crate) const PYTHON_MODULE_KEY: &str = "python:module";
/// What the class is called inside its module, dots and all.
const QUALNAME: &str = "qualname";
/// The full key the qualified name is stored under.
pub(crate) const PYTHON_QUALNAME_KEY: &str = "python:qualname";
/// Which Python form the declaration takes.
const KIND: &str = "kind";
/// The full key the form is stored under.
pub(crate) const PYTHON_KIND_KEY: &str = "python:kind";

/// The segment a qualified name carries for a class declared in a function.
///
/// Python spells it exactly this way and it is not an identifier, so it is the
/// one non-identifier segment a qualified name may hold - and the one thing
/// that makes a declaration unimportable.
const LOCALS: &str = "<locals>";

/// What a module path is, spelled once for every refusal.
const MODULE_SHAPE: &str = "a dotted Python module path";

/// What a qualified name is, spelled once for every refusal.
const QUALNAME_SHAPE: &str = "a dotted Python qualified name";

/// What a form is, spelled once for every refusal.
///
/// Held to [`PythonKind::ALL`] by a test rather than built from it: a refusal
/// is a cold path, and one sentence is cheaper to read than a joined list.
const KIND_SHAPE: &str =
    "one of field, dataclass, typed_dict, named_tuple, enum, newtype, type_alias, class";

/// Python's hard keywords, which no identifier may spell.
///
/// The soft keywords - `match`, `case`, `type`, `_` - are deliberately absent:
/// Python itself accepts them as names, and `keyword.iskeyword` answers false
/// for them, so refusing them here would refuse a class the runtime allows.
const KEYWORDS: [&str; 35] = [
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

/// The Python form a declaration takes.
///
/// The distinction is what a reader needs to rebuild the declaration: a
/// dataclass is constructed by keyword, a named tuple positionally, an
/// enumeration by member, a type alias not at all. [`Self::Field`] is the one
/// form this library mints - a class carrying its own native `Field` - and is
/// what tells a reader the schema is authoritative rather than inferred.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PythonKind {
    /// A class decorated by this library, carrying its own native field.
    Field,
    /// A `dataclasses.dataclass`.
    Dataclass,
    /// A `typing.TypedDict`.
    TypedDict,
    /// A `typing.NamedTuple` or a `collections.namedtuple`.
    NamedTuple,
    /// An `enum.Enum` subclass.
    Enum,
    /// A `typing.NewType`.
    NewType,
    /// A `type` statement or a `typing.TypeAliasType`.
    TypeAlias,
    /// An ordinary class, named by an annotation and nothing more.
    Class,
}

impl PythonKind {
    /// Every form, in declaration order.
    pub const ALL: [Self; 8] = [
        Self::Field,
        Self::Dataclass,
        Self::TypedDict,
        Self::NamedTuple,
        Self::Enum,
        Self::NewType,
        Self::TypeAlias,
        Self::Class,
    ];

    /// Returns the canonical stored spelling without allocating.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Field => "field",
            Self::Dataclass => "dataclass",
            Self::TypedDict => "typed_dict",
            Self::NamedTuple => "named_tuple",
            Self::Enum => "enum",
            Self::NewType => "newtype",
            Self::TypeAlias => "type_alias",
            Self::Class => "class",
        }
    }

    /// Parses one stored spelling.
    ///
    /// # Errors
    ///
    /// Returns an error naming the full `python:kind` key when the text is not
    /// one of [`Self::ALL`].
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Returns whether this form constructs its values by keyword.
    ///
    /// A dataclass and a `TypedDict` are filled by name; a named tuple is
    /// filled by position, and the remaining forms wrap a single value.
    pub const fn is_keyword_constructed(self) -> bool {
        matches!(self, Self::Field | Self::Dataclass | Self::TypedDict)
    }
}

impl FromStr for PythonKind {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == value)
            .ok_or_else(|| Error::InvalidMetadataValue {
                key: SmolStr::new_static(PYTHON_KIND_KEY),
                reason: crate::text::expected_got(KIND_SHAPE, format_args!("{value:?}")),
            })
    }
}

impl fmt::Display for PythonKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl AsRef<str> for PythonKind {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// The Python class a field's `python:` properties name.
///
/// One value carries the whole declaration, so the three properties are
/// validated together, written together, and never read back half-set. The
/// bare class name is derived from the qualified name rather than stored:
/// Python's `__name__` is always the last segment of its `__qualname__`, and
/// storing both would let a hand-edited record disagree with itself.
///
/// ```
/// use yggdryl::{DataType, PythonKind, PythonMetadata};
///
/// # fn main() -> yggdryl::Result<()> {
/// let quote = PythonMetadata::new("trading.book", "Quote", PythonKind::Dataclass)?;
/// assert_eq!(quote.class_name(), "Quote");
/// assert_eq!(quote.import_path(), "trading.book.Quote");
///
/// let mut field = DataType::Int64.required_field("price");
/// field.as_python_mut().set_class(&quote)?;
///
/// assert_eq!(field.as_python().class()?, Some(quote));
/// assert_eq!(field.get_metadata("python:qualname"), Some("Quote"));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PythonMetadata {
    module: SmolStr,
    qualname: SmolStr,
    kind: PythonKind,
}

impl PythonMetadata {
    /// Validates one Python class declaration.
    ///
    /// # Errors
    ///
    /// Returns an error naming the full `python:module` or `python:qualname`
    /// key when either is not the dotted name it must be.
    pub fn new(
        module: impl Into<SmolStr>,
        qualname: impl Into<SmolStr>,
        kind: PythonKind,
    ) -> Result<Self> {
        let module = module.into();
        let qualname = qualname.into();
        validate_python_module(&module)?;
        validate_python_qualname(&qualname)?;
        Ok(Self {
            module,
            qualname,
            kind,
        })
    }

    /// Assembles a declaration out of properties a field already stores.
    ///
    /// Every write path canonicalizes `python:module` and `python:qualname`
    /// through [`validate_python_module`] and [`validate_python_qualname`], so a stored pair
    /// is proven before it is read. Re-validating here would spend the read on
    /// a question the edge already answered.
    fn from_stored(module: &str, qualname: &str, kind: PythonKind) -> Self {
        Self {
            module: SmolStr::new(module),
            qualname: SmolStr::new(qualname),
            kind,
        }
    }

    /// Returns the dotted module path the class is declared in.
    pub fn module(&self) -> &str {
        &self.module
    }

    /// Returns the qualified name the class has inside its module.
    pub fn qualname(&self) -> &str {
        &self.qualname
    }

    /// Returns the bare class name, the last segment of the qualified name.
    pub fn class_name(&self) -> &str {
        self.qualname
            .rsplit_once('.')
            .map_or(self.qualname.as_str(), |(_, name)| name)
    }

    /// Returns which Python form the declaration takes.
    pub const fn kind(&self) -> PythonKind {
        self.kind
    }

    /// Returns the dotted path an importing reader would spell.
    ///
    /// This is `module.qualname` whether or not the class can actually be
    /// reached that way; [`Self::is_importable`] is what answers that.
    pub fn import_path(&self) -> String {
        import_path(&self.module, &self.qualname)
    }

    /// Returns whether [`Self::import_path`] resolves to this class.
    ///
    /// A class declared inside a function body carries `<locals>` in its
    /// qualified name and no import reaches it, so a reader must rebuild it
    /// rather than look it up.
    pub fn is_importable(&self) -> bool {
        !self.qualname.split('.').any(|segment| segment == LOCALS)
    }

    /// Returns a deterministic cross-language hash of the whole declaration.
    ///
    /// The form is hashed with the path, not beside it: two declarations of one
    /// class that disagree on what it is are different values, and
    /// [`Self::import_path`] alone cannot tell them apart.
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_display(&Rendered(self))
    }

    /// Returns the three properties in their stored order.
    ///
    /// This is what a protocol write overlays, and what a caller building a
    /// whole metadata snapshot at once inserts.
    pub fn properties(&self) -> [(&'static str, &str); 3] {
        [
            (PYTHON_KIND_KEY, self.kind.as_str()),
            (PYTHON_MODULE_KEY, self.module.as_str()),
            (PYTHON_QUALNAME_KEY, self.qualname.as_str()),
        ]
    }
}

impl fmt::Display for PythonMetadata {
    /// Renders the dotted path an importing reader would spell.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.module, self.qualname)
    }
}

/// A declaration rendered with its form, for [`PythonMetadata::stable_hash`].
struct Rendered<'value>(&'value PythonMetadata);

impl fmt::Display for Rendered<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.0.kind, self.0)
    }
}

impl<'field> PythonField<'field> {
    /// Returns the dotted module path the declaring class lives in.
    pub fn module(&self) -> Option<&'field str> {
        self.get(MODULE)
    }

    /// Returns the qualified name the declaring class has in its module.
    pub fn qualname(&self) -> Option<&'field str> {
        self.get(QUALNAME)
    }

    /// Returns the bare class name, the last segment of the qualified name.
    ///
    /// Derived on every read and never stored, so it cannot disagree with
    /// [`Self::qualname`].
    pub fn class_name(&self) -> Option<&'field str> {
        self.qualname()
            .map(|qualname| qualname.rsplit_once('.').map_or(qualname, |(_, name)| name))
    }

    /// Parses which Python form the declaration takes.
    ///
    /// # Errors
    ///
    /// Returns an error naming the full `python:kind` key when the stored text
    /// is not one of [`PythonKind::ALL`]: every write canonicalizes it, so
    /// this can only come from externally edited state.
    pub fn kind(&self) -> Result<Option<PythonKind>> {
        self.get(KIND).map(PythonKind::from_str).transpose()
    }

    /// Builds the whole declaration, absent unless all three parts are stored.
    ///
    /// A field carrying only some of them has no class to name, so this
    /// answers `None` rather than a value with a part invented for it; the
    /// three accessors above are what read a partial declaration.
    ///
    /// # Errors
    ///
    /// Returns [`Self::kind`]'s failure. The two names are not re-checked:
    /// every write path validated them, so the read spends nothing on them.
    pub fn class(&self) -> Result<Option<PythonMetadata>> {
        let (Some(module), Some(qualname), Some(kind)) =
            (self.module(), self.qualname(), self.kind()?)
        else {
            return Ok(None);
        };
        Ok(Some(PythonMetadata::from_stored(module, qualname, kind)))
    }

    /// Returns the dotted path an importing reader would spell.
    ///
    /// Absent exactly when either half of it is.
    pub fn import_path(&self) -> Option<String> {
        Some(import_path(self.module()?, self.qualname()?))
    }
}

impl PythonFieldMut<'_> {
    /// Records the whole declaration, replacing every part of a prior one.
    ///
    /// The three properties are overlaid in one validated write, so a refusal
    /// leaves the field exactly as it was rather than half-moved to a new
    /// class.
    ///
    /// # Errors
    ///
    /// Returns the property write's refusal, leaving the field unchanged.
    pub fn set_class(&mut self, value: &PythonMetadata) -> Result<()> {
        self.update([
            (KIND, value.kind.as_str()),
            (MODULE, value.module.as_str()),
            (QUALNAME, value.qualname.as_str()),
        ])
    }

    /// Records the dotted module path the declaring class lives in.
    ///
    /// # Errors
    ///
    /// Returns an error naming the full `python:module` key when the text is
    /// not a dotted module path, leaving the field unchanged.
    pub fn set_module(&mut self, value: &str) -> Result<()> {
        self.insert(MODULE, value)?;
        Ok(())
    }

    /// Records the qualified name the declaring class has in its module.
    ///
    /// # Errors
    ///
    /// Returns an error naming the full `python:qualname` key when the text is
    /// not a dotted qualified name, leaving the field unchanged.
    pub fn set_qualname(&mut self, value: &str) -> Result<()> {
        self.insert(QUALNAME, value)?;
        Ok(())
    }

    /// Records which Python form the declaration takes.
    ///
    /// # Errors
    ///
    /// Returns the property write's refusal, leaving the field unchanged.
    pub fn set_kind(&mut self, value: PythonKind) -> Result<()> {
        self.insert(KIND, value.as_str())?;
        Ok(())
    }

    /// Removes the whole declaration and answers what stood there.
    ///
    /// The prior value is answered only when all three parts were present and
    /// valid, which is the same completeness [`PythonField::class`] reports;
    /// every part is removed either way.
    pub fn remove_class(&mut self) -> Option<PythonMetadata> {
        let prior = self.as_protocol().class().ok().flatten();
        self.remove(KIND);
        self.remove(MODULE);
        self.remove(QUALNAME);
        prior
    }
}

/// Join a module and a qualified name into the path an import would spell.
///
/// Sized once rather than grown, so the one value a read hands back costs the
/// one allocation it is.
fn import_path(module: &str, qualname: &str) -> String {
    let mut path = String::with_capacity(module.len() + 1 + qualname.len());
    path.push_str(module);
    path.push('.');
    path.push_str(qualname);
    path
}

/// Return whether one segment is a Python identifier this crate will store.
///
/// Python identifiers are Unicode, and reproducing `XID_Start`/`XID_Continue`
/// here would pin this crate to one Unicode revision to refuse names the
/// runtime accepts. The structural rules are what a stored name is held to
/// instead - non-empty, not digit-led, no separator, no ASCII punctuation
/// beyond `_`, no whitespace or control - so every valid identifier passes and
/// the shapes that would break a dotted path do not.
fn is_identifier(segment: &str) -> bool {
    let Some(first) = segment.chars().next() else {
        return false;
    };
    if first.is_ascii_digit() || KEYWORDS.contains(&segment) {
        return false;
    }
    segment
        .chars()
        .all(|character| character == '_' || !is_refused_in_identifier(character))
}

/// Return whether one character can never appear in a stored identifier.
fn is_refused_in_identifier(character: char) -> bool {
    character.is_ascii_punctuation()
        || character.is_whitespace()
        || character.is_control()
        || !character.is_ascii() && !char::is_alphanumeric(character)
}

/// Validate a dotted module path.
///
/// # Errors
///
/// Returns an error naming the full `python:module` key.
pub(crate) fn validate_python_module(value: &str) -> Result<()> {
    if !value.is_empty() && value.split('.').all(is_identifier) {
        return Ok(());
    }
    Err(Error::InvalidMetadataValue {
        key: SmolStr::new_static(PYTHON_MODULE_KEY),
        reason: crate::text::expected_got(
            MODULE_SHAPE,
            format_args!("{:?}", crate::text::elide_to(value, 256)),
        ),
    })
}

/// Validate a dotted qualified name.
///
/// `<locals>` is the one non-identifier segment Python itself writes, for a
/// class declared inside a function body, so it is the one this accepts.
///
/// # Errors
///
/// Returns an error naming the full `python:qualname` key.
pub(crate) fn validate_python_qualname(value: &str) -> Result<()> {
    if !value.is_empty()
        && value
            .split('.')
            .all(|segment| segment == LOCALS || is_identifier(segment))
    {
        return Ok(());
    }
    Err(Error::InvalidMetadataValue {
        key: SmolStr::new_static(PYTHON_QUALNAME_KEY),
        reason: crate::text::expected_got(
            QUALNAME_SHAPE,
            format_args!("{:?}", crate::text::elide_to(value, 256)),
        ),
    })
}

/// Canonicalize a stored form through the one parse that names them.
///
/// # Errors
///
/// [`PythonKind::from_str`] carries the rule.
pub(crate) fn canonicalize_python_kind(value: &str) -> Result<String> {
    PythonKind::from_str(value).map(|kind| kind.as_str().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DataType;

    #[test]
    fn a_declaration_round_trips_through_the_field() {
        let declared =
            PythonMetadata::new("trading.book", "Book.Quote", PythonKind::Field).unwrap();
        let mut field = DataType::Int64.required_field("price");

        field.as_python_mut().set_class(&declared).unwrap();

        assert_eq!(field.get_metadata(PYTHON_KIND_KEY), Some("field"));
        assert_eq!(field.get_metadata(PYTHON_MODULE_KEY), Some("trading.book"));
        assert_eq!(field.get_metadata(PYTHON_QUALNAME_KEY), Some("Book.Quote"));
        assert_eq!(field.as_python().class().unwrap(), Some(declared.clone()));
        assert_eq!(field.as_python().class_name(), Some("Quote"));
        assert_eq!(
            field.as_python().import_path().as_deref(),
            Some("trading.book.Book.Quote")
        );
        assert_eq!(field.as_python_mut().remove_class(), Some(declared));
        assert!(field.as_python().is_empty());
    }

    #[test]
    fn a_partial_declaration_names_no_class() {
        let mut field = DataType::Int64.required_field("price");
        field.as_python_mut().set_module("trading.book").unwrap();

        assert_eq!(field.as_python().module(), Some("trading.book"));
        assert_eq!(field.as_python().class().unwrap(), None);
        assert_eq!(field.as_python().import_path(), None);
    }

    #[test]
    fn a_class_declared_in_a_function_is_not_importable() {
        let declared =
            PythonMetadata::new("app", "build.<locals>.Row", PythonKind::Dataclass).unwrap();

        assert_eq!(declared.class_name(), "Row");
        assert!(!declared.is_importable());
        assert!(
            PythonMetadata::new("app", "Row", PythonKind::Dataclass)
                .unwrap()
                .is_importable()
        );
    }

    #[test]
    fn every_stored_form_round_trips_its_spelling() {
        for kind in PythonKind::ALL {
            assert_eq!(PythonKind::from_str(kind.as_str()).unwrap(), kind);
            // The refusal sentence is written out, so it is the one thing that
            // can fall behind a form added to the list.
            assert!(KIND_SHAPE.contains(kind.as_str()), "{kind}");
        }
        assert!(PythonKind::from_str("record").is_err());
    }

    #[test]
    fn a_name_python_could_not_have_written_is_refused() {
        for refused in ["", "trading.", ".book", "trading book", "1book", "class"] {
            assert!(validate_python_module(refused).is_err(), "{refused:?}");
        }
        for accepted in ["trading", "trading.book", "_private", "match", "données"] {
            assert!(validate_python_module(accepted).is_ok(), "{accepted:?}");
        }
        assert!(validate_python_qualname("build.<locals>.Row").is_ok());
        assert!(validate_python_module("build.<locals>.Row").is_err());
    }
}
