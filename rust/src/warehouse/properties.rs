//! [`Properties`]: the name/value bag every object, target and handle door
//! reads.
//!
//! One type owns every bag that is not field metadata: a target's `with
//! (...)` clause, what [`Holder::from_url`](crate::holder::Holder::from_url)
//! and the backend option doors read, and what a catalog, a namespace or a
//! table states or keeps. The bag is ordered as written, holds one value per
//! name - a later set replaces in place - and keeps every name exactly as it
//! was written, because each backend folds the names it reads itself.

use std::fmt;

use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::{SmolStr, format_smolstr};

use crate::boolean::{BOOLEAN_SPELLINGS, bool_from_text};
use crate::integer::{INTEGER_SPELLINGS, integer_from_text_as};
use crate::{Error, Result};

/// An ordered bag of name/value pairs, one value per name.
///
/// `Debug` renders `<redacted>` for the value of every name that may hold a
/// credential - one spelled with `secret`, `token`, `key`, `password`,
/// `credential`, `auth`, `signature`, `sas`, `connection_string` or
/// `service_account` in any case - so a catalog, a namespace, a table or a
/// handle holding a bag prints no secret; `Display` and serde are the data
/// forms and carry every value as it is.
#[derive(Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Properties {
    entries: Vec<(SmolStr, SmolStr)>,
}

impl Properties {
    /// The empty bag.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Return this bag with one more property.
    ///
    /// A name already set is replaced in place, so a bag never carries one
    /// name twice and the order of first writing is kept.
    #[must_use]
    pub fn with_property(mut self, name: impl Into<SmolStr>, value: impl Into<SmolStr>) -> Self {
        self.set(name, value);
        self
    }

    /// Return this bag with every property of an iterator, in its order.
    #[must_use]
    pub fn with_properties<K, V>(self, properties: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<SmolStr>,
        V: Into<SmolStr>,
    {
        properties
            .into_iter()
            .fold(self, |bag, (name, value)| bag.with_property(name, value))
    }

    /// Set one property, replacing an existing value of that name in place.
    pub fn set(&mut self, name: impl Into<SmolStr>, value: impl Into<SmolStr>) {
        let (name, value) = (name.into(), value.into());
        match self.entries.iter_mut().find(|(held, _)| *held == name) {
            Some(held) => held.1 = value,
            None => self.entries.push((name, value)),
        }
    }

    /// Remove one property, answering the value it held.
    pub fn remove(&mut self, name: &str) -> Option<SmolStr> {
        let at = self.entries.iter().position(|(held, _)| held == name)?;
        Some(self.entries.remove(at).1)
    }

    /// One property by its exact name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(held, _)| held == name)
            .map(|(_, value)| value.as_str())
    }

    /// Whether a property of that exact name is set.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// The pairs, in the order they were first written.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&str, &str)> + DoubleEndedIterator + '_ {
        self.entries
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    /// How many properties are set.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no property is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Read one property as a flag, in every spelling a boolean is read
    /// from: `true`, `yes`, `on`, `1` and their opposites, in any case.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] at `$.with.<name>` when the value is
    /// set and spells no boolean.
    pub fn knob_bool(&self, name: &str) -> Result<Option<bool>> {
        self.knob_read(name, BOOLEAN_SPELLINGS, bool_from_text)
    }

    /// Read one property as a count, at the width `T` of the option it
    /// shapes, in every spelling a whole number is read from.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] at `$.with.<name>` when the value is
    /// set and is not a whole number `T` holds.
    pub fn knob_count<T: TryFrom<i128> + TryFrom<u128>>(&self, name: &str) -> Result<Option<T>> {
        self.knob_read(name, INTEGER_SPELLINGS, integer_from_text_as)
    }

    /// Read one property through the one reader of the type it has.
    fn knob_read<T>(
        &self,
        name: &str,
        expected: &str,
        read: impl FnOnce(&str) -> Option<T>,
    ) -> Result<Option<T>> {
        self.get(name)
            .map(|value| {
                read(value).ok_or_else(|| Error::InvalidRecord {
                    path: format_smolstr!("$.with.{name}"),
                    reason: crate::text::expected_got(expected, format_args!("{value:?}")),
                })
            })
            .transpose()
    }

    /// The parent's entries with this bag's own replacing them by name.
    ///
    /// This is how effective properties are built: a child inherits
    /// everything its parent states, and what the child states itself wins,
    /// because an explicit statement nearer the object is the more specific
    /// one. The parent's order is kept, so a bag inherited down a chain reads
    /// as the chain was declared.
    #[must_use]
    pub fn inherit(&self, parent: &Self) -> Self {
        let mut effective = parent.clone();
        for (name, value) in &self.entries {
            effective.set(name.clone(), value.clone());
        }
        effective
    }
}

impl<'a> IntoIterator for &'a Properties {
    type Item = (&'a str, &'a str);
    type IntoIter = std::iter::Map<
        std::slice::Iter<'a, (SmolStr, SmolStr)>,
        fn(&'a (SmolStr, SmolStr)) -> (&'a str, &'a str),
    >;

    fn into_iter(self) -> Self::IntoIter {
        fn pair((name, value): &(SmolStr, SmolStr)) -> (&str, &str) {
            (name.as_str(), value.as_str())
        }
        self.entries.iter().map(pair)
    }
}

impl IntoIterator for Properties {
    type Item = (SmolStr, SmolStr);
    type IntoIter = std::vec::IntoIter<(SmolStr, SmolStr)>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}

impl<K: Into<SmolStr>, V: Into<SmolStr>> FromIterator<(K, V)> for Properties {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self::new().with_properties(iter)
    }
}

impl<K: Into<SmolStr>, V: Into<SmolStr>> Extend<(K, V)> for Properties {
    fn extend<I: IntoIterator<Item = (K, V)>>(&mut self, iter: I) {
        for (name, value) in iter {
            self.set(name, value);
        }
    }
}

/// The words the doors that read a bag spell a credential's name with:
/// `secret_access_key`, `s3tables.session-token`, `account_key`, `sse_key`,
/// `client_secret`, `basic_auth`, `header.Authorization`, `sas`.
const CREDENTIAL_WORDS: [&str; 10] = [
    "secret",
    "token",
    "key",
    "password",
    "credential",
    "auth",
    "signature",
    "sas",
    "connection_string",
    "service_account",
];

/// Whether a property of this name may hold a credential: its name holds
/// one of [`CREDENTIAL_WORDS`] in any case. The rule reads the name alone
/// and errs towards a credential, since it decides only what `Debug` hides.
fn is_credential(name: &str) -> bool {
    CREDENTIAL_WORDS.iter().any(|word| {
        name.as_bytes()
            .windows(word.len())
            .any(|window| window.eq_ignore_ascii_case(word.as_bytes()))
    })
}

impl fmt::Debug for Properties {
    /// Every name with its value, in the bag's order - `<redacted>` for the
    /// value of a name that may hold a credential.
    ///
    /// ```
    /// use yggdryl::Properties;
    ///
    /// let bag = Properties::new()
    ///     .with_property("region", "eu-west-3")
    ///     .with_property("s3tables.session-token", "IQoJb3JpZ2luX2Vj");
    /// assert_eq!(
    ///     format!("{bag:?}"),
    ///     r#"{"region": "eu-west-3", "s3tables.session-token": <redacted>}"#
    /// );
    /// ```
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut map = formatter.debug_map();
        for (name, value) in self {
            if is_credential(name) {
                map.entry(&name, &format_args!("<redacted>"));
            } else {
                map.entry(&name, &value);
            }
        }
        map.finish()
    }
}

impl fmt::Display for Properties {
    /// The `with (...)` clause the plan grammar writes, without the keyword:
    /// `name = 'value', other = 'value'`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, (name, value)) in self.entries.iter().enumerate() {
            if index != 0 {
                formatter.write_str(", ")?;
            }
            crate::expression::write_identifier(formatter, name)?;
            formatter.write_str(" = ")?;
            crate::expression::write_text_literal(formatter, value)?;
        }
        Ok(())
    }
}

impl Serialize for Properties {
    /// A JSON object, in the bag's order.
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.entries.len()))?;
        for (name, value) in &self.entries {
            map.serialize_entry(name.as_str(), value.as_str())?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Properties {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct PropertiesVisitor;

        impl<'de> Visitor<'de> for PropertiesVisitor {
            type Value = Properties;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object of text values")
            }

            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut properties = Properties::new();
                while let Some((name, value)) = map.next_entry::<String, String>()? {
                    properties.set(name, value);
                }
                Ok(properties)
            }
        }

        deserializer.deserialize_map(PropertiesVisitor)
    }
}
