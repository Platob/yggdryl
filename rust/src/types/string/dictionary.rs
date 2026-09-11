//! Named member dictionaries for ASCII-encoded fields.

use std::collections::BTreeMap;

use smol_str::{SmolStr, format_smolstr};

use crate::{DataType, Error, Result};

/// The enum an ASCII field's values name: one value per member name.
///
/// This is the vocabulary a declaration named itself, and it is what a
/// [`crate::Field`] stores under `field:enum` so the enum crosses Arrow, a
/// file, and another runtime intact.
///
/// The width lives in the field's datatype and is never copied here: a
/// member's code is [`DataType::ascii_packed`] of its value under that width,
/// so every reader of one enum answers the same integers. Members are held by
/// name, which is what makes the rendered document deterministic - the order a
/// declaration happened to use is not part of a member's identity once the
/// code is the value's own bytes.
///
/// ```
/// use yggdryl::{AsciiEnum, DataType};
///
/// # fn main() -> yggdryl::Result<()> {
/// let side = AsciiEnum::from_members("Side", [("BUY", "B"), ("SELL", "S")])?;
/// assert_eq!(side.get("BUY"), Some("B"));
/// assert_eq!(side.get_member("S"), Some("SELL"));
/// assert_eq!(
///     side.into_members(&DataType::FixedAscii(4))?,
///     [("BUY".into(), 0x4200_0000), ("SELL".into(), 0x5300_0000)]
/// );
/// assert_eq!(
///     side.into_json(),
///     r#"{"members":{"BUY":"B","SELL":"S"},"name":"Side"}"#
/// );
/// assert_eq!(AsciiEnum::from_json(&side.into_json())?, side);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AsciiEnum {
    /// The enum's own name, which is not the field's name.
    name: SmolStr,
    /// Member name to ASCII value, ordered by name so the document is one text.
    members: BTreeMap<SmolStr, SmolStr>,
}

impl AsciiEnum {
    /// Creates an enum of no members under one name.
    ///
    /// # Errors
    ///
    /// Returns an error when `name` is empty or holds a control character.
    pub fn new(name: impl Into<SmolStr>) -> Result<Self> {
        let name = name.into();
        validate_enum_text("enum name", &name)?;
        Ok(Self {
            name,
            members: BTreeMap::new(),
        })
    }

    /// Creates an enum from its members, one ASCII value per member name.
    ///
    /// A repeated member name keeps the last value, exactly as [`Self::insert`]
    /// would; two members may share a value, because two spellings of one code
    /// is what an alias is.
    ///
    /// # Errors
    ///
    /// Returns an error when the enum name or a member name is empty or holds
    /// a control character.
    pub fn from_members<I, N, V>(name: impl Into<SmolStr>, members: I) -> Result<Self>
    where
        I: IntoIterator<Item = (N, V)>,
        N: Into<SmolStr>,
        V: Into<SmolStr>,
    {
        let mut enumeration = Self::new(name)?;
        for (member, value) in members {
            enumeration.insert(member, value)?;
        }
        Ok(enumeration)
    }

    /// Parses the `field:enum` document.
    ///
    /// # Errors
    ///
    /// Returns an error when the document is not a JSON object of a string
    /// `"name"` and an object `"members"` of strings, and one naming the part
    /// when a name is empty or holds a control character.
    pub fn from_json(document: &str) -> Result<Self> {
        let value: serde_json::Value = serde_json::from_str(document.trim()).map_err(|error| {
            enum_document_refusal(format_smolstr!(
                "expected an enum JSON document, got unparsable JSON: {error}"
            ))
        })?;
        let Some(object) = value.as_object() else {
            return Err(enum_document_refusal(format_smolstr!(
                "expected an enum JSON object, got {}",
                crate::text::elide_display(&value)
            )));
        };
        let Some(serde_json::Value::String(name)) = object.get("name") else {
            return Err(enum_document_refusal(SmolStr::new_static(
                "expected a JSON string \"name\"",
            )));
        };
        let empty = serde_json::Map::new();
        let members = match object.get("members") {
            None | Some(serde_json::Value::Null) => &empty,
            Some(serde_json::Value::Object(members)) => members,
            Some(other) => {
                return Err(enum_document_refusal(format_smolstr!(
                    "expected a JSON object \"members\", got {}",
                    crate::text::elide_display(other)
                )));
            }
        }
        .iter()
        .map(|(member, value)| match value {
            serde_json::Value::String(value) => Ok((member.as_str(), value.as_str())),
            other => Err(enum_document_refusal(format_smolstr!(
                "expected a JSON string for the member {member:?}, got {}",
                crate::text::elide_display(other)
            ))),
        })
        .collect::<Result<Vec<_>>>()?;
        Self::from_members(name.as_str(), members)
    }

    /// Renders the `field:enum` document: every name in order, so one enum
    /// is one text however it was built.
    pub fn into_json(&self) -> String {
        let members = self
            .members
            .iter()
            .map(|(member, value)| {
                (
                    member.as_str().to_owned(),
                    serde_json::Value::String(value.as_str().to_owned()),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        serde_json::Value::Object(serde_json::Map::from_iter([
            (
                "name".to_owned(),
                serde_json::Value::String(self.name.as_str().to_owned()),
            ),
            ("members".to_owned(), serde_json::Value::Object(members)),
        ]))
        .to_string()
    }

    /// The enum's own name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The ASCII value one member names, or `None` for a member it has not.
    pub fn get(&self, member: &str) -> Option<&str> {
        self.members.get(member).map(SmolStr::as_str)
    }

    /// The first member naming one ASCII value, or `None` when none does.
    ///
    /// Two members may share a value; the first by name answers, so an alias
    /// never changes which member a stored value reads back as.
    pub fn get_member(&self, value: &str) -> Option<&str> {
        self.members
            .iter()
            .find(|(_, held)| held.as_str() == value)
            .map(|(member, _)| member.as_str())
    }

    /// Names one ASCII value and returns the value the member had.
    ///
    /// # Errors
    ///
    /// Returns an error when `member` is empty or holds a control character.
    pub fn insert(
        &mut self,
        member: impl Into<SmolStr>,
        value: impl Into<SmolStr>,
    ) -> Result<Option<SmolStr>> {
        let member = member.into();
        validate_enum_text("member name", &member)?;
        Ok(self.members.insert(member, value.into()))
    }

    /// Removes one member and returns the ASCII value it named.
    pub fn remove(&mut self, member: &str) -> Option<SmolStr> {
        self.members.remove(member)
    }

    /// The number of members.
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// Returns whether this enum names nothing.
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// The members by name, each with the ASCII value it names.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.members
            .iter()
            .map(|(member, value)| (member.as_str(), value.as_str()))
    }

    /// The enum member name one ASCII value takes.
    ///
    /// An ASCII letter is kept uppercased, a digit is kept, every other byte
    /// becomes `_`, a leading digit takes a `_` in front, and a name that both
    /// opens and closes with `_` drops its trailing underscores - that shape
    /// is what Python reserves for `_sunder_` and `__dunder__` names, where a
    /// member is refused or silently dropped.
    ///
    /// The rule belongs to the vocabulary rather than to one width, so an enum
    /// that registers one value at a time names it exactly as generating a
    /// whole listing at once would.
    ///
    /// ```
    /// use yggdryl::AsciiEnum;
    ///
    /// assert_eq!(AsciiEnum::member_name("USD").as_str(), "USD");
    /// assert_eq!(AsciiEnum::member_name("n/a").as_str(), "N_A");
    /// assert_eq!(AsciiEnum::member_name("-a-").as_str(), "_A");
    /// assert_eq!(AsciiEnum::member_name("").as_str(), "_");
    /// ```
    #[must_use]
    pub fn member_name(value: &str) -> SmolStr {
        let mut name = String::with_capacity(value.len() + 1);
        // A registered value passed `ascii_text`, so one byte is one character.
        // Any other byte is not ASCII alphanumeric, so it becomes `_` like the
        // rest of what the rule replaces.
        for byte in value.bytes() {
            name.push(if byte.is_ascii_alphanumeric() {
                char::from(byte.to_ascii_uppercase())
            } else {
                '_'
            });
        }
        if name.starts_with(|first: char| first.is_ascii_digit()) {
            name.insert(0, '_');
        }
        // A name that both opens and closes with `_` carries the shape Python
        // reserves for `_sunder_` and `__dunder__`, where a member is refused or
        // silently dropped; a name of nothing but `_` has no other spelling.
        let named = name.trim_end_matches('_').len();
        if name.starts_with('_') && named > 0 {
            name.truncate(named);
        }
        if name.is_empty() {
            return SmolStr::new_static("_");
        }
        SmolStr::new(name)
    }

    /// The members paired with their packed codes under one ASCII width.
    ///
    /// # Errors
    ///
    /// Returns an error naming the accepted widths when `width` is not one,
    /// and one naming the width when a value does not fit it.
    pub fn into_members(&self, width: &DataType) -> Result<Vec<(SmolStr, i128)>> {
        self.members
            .iter()
            .map(|(member, value)| Ok((member.clone(), width.ascii_packed(value.as_bytes())?)))
            .collect()
    }
}

fn enum_document_refusal(reason: SmolStr) -> Error {
    Error::InvalidDataType {
        kind: "ascii-enum",
        reason,
    }
}

/// Refuses the two spellings a stored document could not carry back.
fn validate_enum_text(part: &'static str, value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(enum_document_refusal(format_smolstr!(
            "expected a non-empty {part}"
        )));
    }
    if let Some(position) = value.chars().position(char::is_control) {
        return Err(enum_document_refusal(format_smolstr!(
            "expected a {part} with no control character, got one at {position}"
        )));
    }
    Ok(())
}
