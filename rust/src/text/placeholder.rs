//! Jinja-*style* `{{ }}` placeholders, resolved **after** parsing.
//!
//! Configuration documents want to carry `{{ LOG_ROOT }}` and resolve it at
//! load time. This is that, as a closed and minimal feature - **not a template
//! engine**. There are no loops, no conditionals, no includes, no expressions,
//! no filter chains, and nothing that evaluates code. The whole grammar fits in
//! a paragraph, and that is the point: a reader can hold it in their head.
//!
//! # The grammar, in full
//!
//! - `{{ NAME }}` resolves `NAME`. A name absent from every source is an
//!   **error** naming the variable and where it sits - never a silent empty
//!   string, which is how a configuration quietly points at the wrong place.
//! - `{{ NAME | default(LITERAL) }}` makes `NAME` optional, falling back to
//!   `LITERAL`: a JSON scalar literal, so `default("logs")`, `default(8080)`,
//!   `default(1.5)`, `default(true)`, and `default(null)` are all spellable and
//!   all carry their own type. `default` is the only filter, and a literal may
//!   not contain `}}`.
//! - `{{{{` is a literal `{{`. Nothing else needs escaping: `}}` outside a
//!   placeholder is ordinary text.
//!
//! A name starts with an ASCII letter or `_` and continues with letters,
//! digits, `_`, `.`, or `-`.
//!
//! # Two typing rules
//!
//! - A string scalar that is **exactly** one placeholder adopts the resolved
//!   value's own type. With `PORT=8080`, `port: "{{ PORT }}"` yields the
//!   integer `8080` - a quoted placeholder is not forced to stay a string just
//!   because YAML made the caller quote it.
//! - A placeholder **embedded** in a larger string substitutes textually and
//!   the result stays a string: `path: "{{ ROOT }}/logs"`. An embedded
//!   placeholder must therefore resolve to something with a text form; a
//!   sequence or mapping is refused rather than rendered.
//!
//! The asymmetry is deliberate.
//!
//! # Why after parsing
//!
//! Rendering the *text* before parsing would destroy the byte positions
//! [`position`](crate::text::position) exists to provide - every parse
//! diagnostic would point into rendered text rather than into the file the user
//! wrote - and a valid template could render a syntactically invalid document.
//! Walking the parsed [`Value`] instead keeps positions exact, still fails a
//! malformed document where it is malformed, and makes it impossible for a
//! substitution to change the document's shape.
//!
//! It also fits the formats. A placeholder has to sit inside a string anyway:
//! JSON and TOML require typed values, and in YAML a bare `{{ PORT }}` is not a
//! scalar at all but a flow mapping containing a flow mapping. **In YAML, quote
//! it** - `port: "{{ PORT }}"` - which is the single most common way people get
//! this wrong.
//!
//! # The environment is its own switch
//!
//! Nothing in this library reads the process environment on its own. A document
//! that resolves `{{ AWS_SECRET_ACCESS_KEY }}` into a value that is then
//! dumped, logged, or written to a table has leaked it, so substitution is off
//! unless a caller turns it on, and environment access is a *separate* switch
//! on top of that. With it off, no `std::env` call happens at all - not "reads
//! and ignores". A caller can always resolve entirely from a supplied mapping,
//! which is also what makes a parse deterministic and testable.

use std::borrow::Cow;
use std::hash::{Hash, Hasher};

use smol_str::{SmolStr, format_smolstr};

use crate::generic::iso;
use crate::{Error, Result, Value};

/// The two bytes that open a placeholder.
const OPEN: &[u8; 2] = b"{{";

/// Where a caller's `{{ }}` placeholders resolve from.
///
/// Empty and environment-free by default: [`Placeholders::new`] resolves
/// nothing, so every placeholder is an error until a source is added. That is
/// the safe default for a feature whose failure mode is a silently wrong value.
///
/// ```
/// use yggdryl::text::{Format, Loading, Placeholders};
/// use yggdryl::Value;
///
/// # fn main() -> yggdryl::Result<()> {
/// let placeholders = Placeholders::new()
///     .with_variable("ROOT", Value::from("/var/log"))
///     .with_variable("PORT", Value::I64(8080));
/// let loading = Loading::new().with_placeholders(placeholders);
///
/// let document = "path: \"{{ ROOT }}/app\"\nport: \"{{ PORT }}\"\n";
/// let value = yggdryl::text::from_utf8_with(document, Format::Yaml, &loading)?;
///
/// // Embedded: textual, and the result is a string.
/// assert_eq!(value.get_key_str("path").and_then(Value::as_utf8), Some("/var/log/app"));
/// // Whole-scalar: the resolved value's own type.
/// assert_eq!(value.get_key_str("port"), Some(&Value::I64(8080)));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Default)]
pub struct Placeholders {
    /// Caller-supplied variables, in the order they were given.
    variables: Vec<(SmolStr, Value)>,
    /// Whether the process environment is consulted when the mapping misses.
    environment: bool,
}

#[derive(Eq, Hash, Ord, PartialEq, PartialOrd)]
struct PlaceholdersIdentity<'a> {
    variables: Vec<&'a (SmolStr, Value)>,
    environment: bool,
}

impl Placeholders {
    /// Resolve from nothing: no variables, no environment.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            variables: Vec::new(),
            environment: false,
        }
    }

    /// Resolve from a named record or string-keyed mapping [`Value`].
    ///
    /// # Errors
    ///
    /// Returns an error when `variables` is neither shape, or when a mapping
    /// key is not a string.
    pub fn from_variables(variables: &Value) -> Result<Self> {
        let mut placeholders = Self::new();
        if let Some(entries) = variables.as_record() {
            placeholders.variables.extend(
                entries
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone())),
            );
            return Ok(placeholders);
        }
        let entries = variables.as_mapping().ok_or_else(|| Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: crate::text::expected_got(
                "a record or string-keyed mapping of variables",
                variables.kind(),
            ),
        })?;
        placeholders.variables.reserve(entries.len());
        for (name, value) in entries {
            let name = name.as_str().ok_or_else(|| Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: crate::text::expected_got("string variable names", name.kind()),
            })?;
            placeholders.variables.push((name.into(), value.clone()));
        }
        Ok(placeholders)
    }

    /// Add one variable, replacing any earlier one of the same name.
    #[must_use]
    pub fn with_variable(mut self, name: impl Into<SmolStr>, value: Value) -> Self {
        let name = name.into();
        match self.variables.iter_mut().find(|(held, _)| *held == name) {
            Some(held) => held.1 = value,
            None => self.variables.push((name, value)),
        }
        self
    }

    /// Consult the process environment when the supplied mapping misses.
    ///
    /// Off by default. See the module documentation for why this is its own
    /// switch rather than part of turning substitution on.
    #[must_use]
    pub const fn with_environment(mut self, environment: bool) -> Self {
        self.environment = environment;
        self
    }

    /// The supplied variables, in order.
    #[must_use]
    pub fn variables(&self) -> &[(SmolStr, Value)] {
        &self.variables
    }

    /// Whether the process environment is consulted.
    #[must_use]
    pub const fn environment(&self) -> bool {
        self.environment
    }

    fn identity(&self) -> PlaceholdersIdentity<'_> {
        PlaceholdersIdentity {
            variables: crate::generic::sorted_pairs(&self.variables),
            environment: self.environment,
        }
    }

    /// Resolve one name: the supplied mapping first, the environment second.
    ///
    /// The mapping wins so a test can override anything without touching the
    /// process it runs in.
    fn resolve(&self, name: &str) -> Option<Value> {
        if let Some((_, value)) = self.variables.iter().find(|(held, _)| held == name) {
            return Some(value.clone());
        }
        // Not "read and ignore": with the switch off there is no call at all.
        if !self.environment {
            return None;
        }
        std::env::var(name)
            .ok()
            .map(|value| Value::String(SmolStr::new(value)))
    }
}

impl PartialEq for Placeholders {
    fn eq(&self, other: &Self) -> bool {
        self.identity() == other.identity()
    }
}

impl Eq for Placeholders {}

impl PartialOrd for Placeholders {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Placeholders {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.identity().cmp(&other.identity())
    }
}

impl Hash for Placeholders {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.identity().hash(state);
    }
}

/// Whether `bytes` could possibly contain a placeholder.
///
/// The cheap guard that keeps this feature free for the overwhelming majority
/// of documents, which have none: one linear pass for `{{`, and if it is not
/// there the parsed value is returned untouched - no walk, no allocation, no
/// per-scalar inspection.
///
/// Written as a vectorized single-byte search that then checks its neighbour,
/// rather than as `windows(2).any(..)`: the two-byte comparison per position
/// does not vectorize, and on a document of a few kilobytes that difference is
/// the whole measured cost of the guard. `memchr` is already in the tree under
/// the regex engine, so the SIMD scan costs no extra crate.
#[must_use]
pub(crate) fn present(bytes: &[u8]) -> bool {
    let mut cursor = 0;
    while let Some(found) = memchr::memchr(OPEN[0], &bytes[cursor..]) {
        let at = cursor + found;
        if bytes.get(at + 1) == Some(&OPEN[1]) {
            return true;
        }
        cursor = at + 1;
    }
    false
}

/// Replace every placeholder in `value`, in place where nothing changes.
///
/// # Errors
///
/// Returns the first unresolved name, malformed placeholder, or embedded value
/// with no text form, naming the document path it sits at.
pub(crate) fn substitute(value: Value, placeholders: &Placeholders) -> Result<Value> {
    let mut path = String::from("$");
    walk(value, placeholders, &mut path)
}

/// Substitute through one node, tracking where it sits for diagnostics.
fn walk(value: Value, placeholders: &Placeholders, path: &mut String) -> Result<Value> {
    match value {
        Value::String(text) => scalar(&text, placeholders, path),
        Value::Sequence(values) => {
            let mut replaced = Vec::with_capacity(values.len());
            for (index, held) in values.iter().enumerate() {
                let mark = path.len();
                path.push_str(&format!("[{index}]"));
                replaced.push(walk(held.clone(), placeholders, path)?);
                path.truncate(mark);
            }
            Ok(Value::from_sequence(replaced))
        }
        Value::Mapping(entries) => {
            let mut replaced = Vec::with_capacity(entries.len());
            for (key, held) in entries.iter() {
                let mark = path.len();
                if let Some(name) = key.as_str() {
                    path.push('.');
                    path.push_str(name);
                }
                // Keys carry placeholders too: a document naming its own
                // sections from a variable is exactly the case this serves.
                let key = walk(key.clone(), placeholders, path)?;
                let held = walk(held.clone(), placeholders, path)?;
                path.truncate(mark);
                replaced.push((key, held));
            }
            Value::from_mapping(replaced)
        }
        Value::Record(entries) => {
            let mut replaced = Vec::with_capacity(entries.len());
            for (name, held) in entries.iter() {
                let mark = path.len();
                path.push('.');
                path.push_str(name);
                let name = scalar(name, placeholders, path)?;
                let name = name.as_str().ok_or_else(|| {
                    refusal(path, 0, "a placeholder in an object key to resolve to text")
                })?;
                let held = walk(held.clone(), placeholders, path)?;
                path.truncate(mark);
                replaced.push((SmolStr::new(name), held));
            }
            Value::from_record(replaced)
        }
        // Every other value is moved through untouched: only string scalars can
        // hold a placeholder, in any of the three formats.
        other => Ok(other),
    }
}

/// Substitute inside one string scalar.
fn scalar(text: &str, placeholders: &Placeholders, path: &str) -> Result<Value> {
    let bytes = text.as_bytes();
    if !present(bytes) {
        // The common case: nothing to do, and nothing allocated to prove it.
        return Ok(Value::String(SmolStr::new(text)));
    }
    // A scalar that is exactly one placeholder adopts the resolved value's own
    // type, so this is decided before any rendering happens.
    if let Some(inner) = whole(text) {
        return resolved(inner, placeholders, path, 0);
    }

    let mut rendered = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(found) = bytes[cursor..]
        .windows(OPEN.len())
        .position(|window| window == OPEN)
    {
        let start = cursor + found;
        rendered.push_str(&text[cursor..start]);
        if text[start..].starts_with("{{{{") {
            // The one escape: a doubled opener is a literal one.
            rendered.push_str("{{");
            cursor = start + 4;
            continue;
        }
        let end = text[start..].find("}}").ok_or_else(|| {
            refusal(
                path,
                start,
                "an unterminated placeholder: `{{` with no closing `}}`",
            )
        })?;
        let inner = &text[start + 2..start + end];
        let value = resolved(inner, placeholders, path, start)?;
        rendered.push_str(&text_form(&value).ok_or_else(|| {
            refusal(
                path,
                start,
                format_smolstr!(
                    "a placeholder inside a larger string to resolve to a scalar, \
                     but {} has none",
                    value.kind()
                ),
            )
        })?);
        cursor = start + end + 2;
    }
    rendered.push_str(&text[cursor..]);
    Ok(Value::String(SmolStr::new(rendered)))
}

/// The inner text when `text` is exactly one placeholder, and nothing else.
fn whole(text: &str) -> Option<&str> {
    let inner = text.strip_prefix("{{")?.strip_suffix("}}")?;
    // `{{{{ }}` is an escape followed by text, not a placeholder, and
    // `{{ A }}{{ B }}` is two - both belong on the embedded path.
    if inner.starts_with('{') || inner.contains("}}") || inner.contains("{{") {
        return None;
    }
    Some(inner)
}

/// Read one placeholder's body and resolve it.
fn resolved(inner: &str, placeholders: &Placeholders, path: &str, at: usize) -> Result<Value> {
    let (name, fallback) = match inner.split_once('|') {
        Some((name, filter)) => (name.trim(), Some(default_literal(filter, path, at)?)),
        None => (inner.trim(), None),
    };
    if !named(name) {
        return Err(refusal(
            path,
            at,
            format_smolstr!(
                "a placeholder naming a variable - a letter or `_` then letters, digits, \
                 `_`, `.`, or `-` - but found {name:?}"
            ),
        ));
    }
    match placeholders.resolve(name) {
        Some(value) => Ok(value),
        None => fallback.ok_or_else(|| {
            refusal(
                path,
                at,
                format_smolstr!(
                    "a value for {{{{ {name} }}}}: it is in neither the supplied variables nor \
                     {}, and it declares no default",
                    if placeholders.environment() {
                        "the environment"
                    } else {
                        "the environment, which is not consulted"
                    }
                ),
            )
        }),
    }
}

/// Read the one filter this grammar has: `default(LITERAL)`.
fn default_literal(filter: &str, path: &str, at: usize) -> Result<Value> {
    let filter = filter.trim();
    let literal = filter
        .strip_prefix("default(")
        .and_then(|rest| rest.strip_suffix(')'))
        .ok_or_else(|| {
            refusal(
                path,
                at,
                format_smolstr!(
                    "the only filter this grammar has, `default(LITERAL)`, but found {filter:?}; \
                     there are no other filters, no chains, and no expressions"
                ),
            )
        })?;
    // One literal syntax, and it is one the workspace already parses: a JSON
    // scalar, so a default carries its own type rather than always being text.
    let value = crate::json::from_utf8(literal.trim()).map_err(|error| {
        refusal(
            path,
            at,
            format_smolstr!("a JSON scalar literal in `default(...)`: {error}"),
        )
    })?;
    if matches!(
        value,
        Value::Sequence(_) | Value::Mapping(_) | Value::Record(_)
    ) {
        return Err(refusal(
            path,
            at,
            "a scalar in `default(...)`, not a container",
        ));
    }
    Ok(value)
}

/// Whether `name` is spellable as a variable name.
fn named(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    characters
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-'))
}

/// The text a value renders as when it is embedded in a larger string.
///
/// Scalars only, and each in the spelling the codecs already write, so an
/// embedded timestamp reads the same as a dumped one. A container has no
/// sensible text form inside a path, so it has none here.
fn text_form(value: &Value) -> Option<Cow<'_, str>> {
    let owned = match value {
        Value::String(text) => return Some(Cow::Borrowed(text.as_str())),
        Value::Bool(held) => held.to_string(),
        Value::I8(held) => held.to_string(),
        Value::I16(held) => held.to_string(),
        Value::I32(held) => held.to_string(),
        Value::I64(held) => held.to_string(),
        Value::U8(held) => held.to_string(),
        Value::U16(held) => held.to_string(),
        Value::U32(held) => held.to_string(),
        Value::U64(held) => held.to_string(),
        Value::I128(held) => held.to_string(),
        Value::U128(held) => held.to_string(),
        Value::F16(held) => held.as_f32().to_string(),
        Value::F32(held) => held.as_f32().to_string(),
        Value::F64(held) => held.as_f64().to_string(),
        Value::D128(unscaled, scale) => {
            crate::generic::decimal::decimal_text(crate::I256::from_i128(*unscaled), *scale)
        }
        Value::D256(unscaled, scale) => crate::generic::decimal::decimal_text(*unscaled, *scale),
        Value::Date32(days, _, _) => iso::format_date(*days)?.to_string(),
        Value::Date64(milliseconds, _, _) => {
            let days = milliseconds.checked_div(86_400_000)?;
            if days.checked_mul(86_400_000)? != *milliseconds {
                return None;
            }
            iso::format_date(i32::try_from(days).ok()?)?.to_string()
        }
        Value::Time32(count, unit, zone) => time_text(i64::from(*count), *unit, zone)?,
        Value::Time64(count, unit, zone) => time_text(*count, *unit, zone)?,
        Value::DateTime64(count, unit, zone) if zone.is_naive() => {
            iso::format_datetime(*count, *unit)?.to_string()
        }
        Value::DateTime64(count, unit, zone) => {
            iso::format_timestamp(*count, *unit, zone)?.to_string()
        }
        Value::Duration32(count, unit, _) => {
            iso::format_duration(i64::from(*count), *unit)?.to_string()
        }
        Value::Duration64(count, unit, _) => iso::format_duration(*count, *unit)?.to_string(),
        // A geometry's canonical text is WKT, the spelling every geospatial
        // reader already reads. Malformed WKB still embeds losslessly - as the
        // hex of its bytes - rather than refusing, because the value holds
        // exactly those bytes and hiding them would make the document
        // unwritable over one broken buffer.
        Value::Geospatial(bytes) => {
            crate::generic::wkb::into_wkt(bytes).unwrap_or_else(|_| hex_text(bytes))
        }
        // Null included: rendering "nothing" into the middle of a path is how a
        // configuration silently points somewhere wrong.
        _ => return None,
    };
    Some(Cow::Owned(owned))
}

/// The lossless hex spelling of bytes no other renderer can read.
fn hex_text(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

fn time_text(count: i64, unit: crate::TimeUnit, zone: &crate::Timezone) -> Option<String> {
    let text = iso::format_time(count, unit)?.to_string();
    if zone.is_naive() {
        return Some(text);
    }
    None
}

/// A typed refusal naming the value's path and where in it the failure sits.
fn refusal(path: &str, at: usize, reason: impl std::fmt::Display) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new(path),
        reason: format_smolstr!("expected {reason}, at byte {at} of the value"),
    }
}
