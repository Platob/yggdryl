//! The `partition:` declaration: how a column derives its value, in the shape
//! an Iceberg partition spec has.
//!
//! A partition column declares its derivation as the pair `partition:sources`
//! and `partition:transform` - one function over one source path. The pair is
//! what a spec is read from and written to; the derivation itself is read
//! through the [`transform`](crate::TransformField::term) protocol, which
//! answers this pair where no `transform:expression` is declared, so a column
//! derives one way whichever protocol declared it.

use crate::expression::{Function, Term};
use crate::types::protocol::{PartitionField, PartitionFieldMut};
use crate::{Error, Result};

/// The partition property naming how a column derives its value.
const TRANSFORM: &str = "transform";

/// The partition property naming the fields a column derives its value from.
const SOURCES: &str = "sources";

impl<'field> PartitionField<'field> {
    /// Parses the field paths this column derives its value from.
    ///
    /// The shape is the one every `sources` property has, the
    /// [digest holder's](crate::DigestField::sources) included: a JSON array
    /// of dotted paths, spelled the way [`Field::get_field_by_path`](crate::Field::get_field_by_path) and the
    /// expression grammar both spell one, so a partition column can read a
    /// struct child as easily as a top-level one.
    ///
    /// One source is every transform this crate evaluates today, and
    /// [`Self::term`] is where a longer list is refused; the list shape
    /// is what leaves room for the transforms that read more than one column.
    ///
    /// # Errors
    ///
    /// Returns an error naming `partition:sources` when the stored text is not
    /// that array.
    pub fn sources(&self) -> Result<Option<Vec<String>>> {
        self.get(SOURCES)
            .map(|stored| crate::metadata::parse_source_list(&self.key(SOURCES), stored))
            .transpose()
    }

    /// Parses how this column derives its value from that field.
    ///
    /// The vocabulary is the expression grammar's own [`Function`] set, which
    /// is what keeps one implementation behind a derived partition column and
    /// behind a predicate over the same value. An absent transform is the
    /// identity: the source value unchanged.
    ///
    /// # Errors
    ///
    /// Returns an error when the stored text names no function, or names one
    /// that cannot take a single argument.
    pub fn transform(&self) -> Result<Option<Function>> {
        self.get(TRANSFORM)
            .map(|stored| crate::metadata::parse_partition_transform(&self.key(TRANSFORM), stored))
            .transpose()
    }

    /// Returns the term that fills this column, if it declares one.
    ///
    /// `None` is a column no `partition:sources` names, which is every column
    /// a directory spells out rather than derives. The
    /// [`transform`](crate::TransformField::term) protocol reads this, so a
    /// partition column derives exactly as a `transform:expression` does.
    ///
    /// ```
    /// use yggdryl::DataType;
    /// use yggdryl::expression::Function;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut year = DataType::Int32.nullable_field("year");
    /// year.as_partition_mut().set_sources(["event"])?;
    /// year.as_partition_mut().set_transform(Function::Year)?;
    ///
    /// assert_eq!(year.as_partition().sources()?, Some(vec!["event".to_owned()]));
    /// assert_eq!(year.get_metadata("partition:sources"), Some(r#"["event"]"#));
    /// assert_eq!(year.get_metadata("partition:transform"), Some("year"));
    /// assert_eq!(
    ///     year.as_partition().term()?.map(|value| value.to_string()),
    ///     Some("year(event)".to_owned()),
    /// );
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when a transform is declared with no sources beside
    /// it, when the list does not name exactly one - the only shape a
    /// transform of one argument reads - or when the transform is not such a
    /// function.
    pub fn term(&self) -> Result<Option<Term>> {
        let Some(sources) = self.sources()? else {
            if self.contains_key(TRANSFORM) {
                return Err(self.invalid_sources(smol_str::format_smolstr!(
                    "expected a {} beside {}, got none",
                    self.key(SOURCES),
                    self.key(TRANSFORM)
                )));
            }
            return Ok(None);
        };
        // One source is every transform this crate evaluates today; the list
        // is what a transform reading more than one column will grow into.
        let [source] = sources.as_slice() else {
            return Err(self.invalid_sources(crate::text::expected_got(
                "exactly one source, the only shape a transform reads today",
                format_args!("{} of them", sources.len()),
            )));
        };
        let mut segments = source.split('.');
        let root = segments.next().unwrap_or_default();
        let read = segments.fold(Term::column(root), Term::child);
        Ok(Some(match self.transform()? {
            Some(function) => Term::call(function, [read]),
            None => read,
        }))
    }

    /// Returns whether this column declares a derivation, complete or not.
    ///
    /// Answered on the stored properties rather than the parsed term, so a
    /// malformed declaration still reports as one and is refused where it is
    /// read.
    #[must_use]
    pub fn is_derived(&self) -> bool {
        self.contains_key(SOURCES) || self.contains_key(TRANSFORM)
    }

    /// Name the full source key a declaration was refused under.
    fn invalid_sources(&self, reason: smol_str::SmolStr) -> Error {
        Error::InvalidMetadataValue {
            key: smol_str::SmolStr::new(self.key(SOURCES)),
            reason,
        }
    }
}

impl PartitionFieldMut<'_> {
    /// Records the field paths this column derives its value from.
    ///
    /// The list is stored in the one canonical spelling every `sources`
    /// property has. One path is every transform this crate evaluates today,
    /// and [`PartitionField::term`] is where a longer list is refused -
    /// storing one states the intent without pretending it runs.
    ///
    /// # Errors
    ///
    /// Returns an error when a path is empty or repeated, or when the property
    /// write fails the validation every metadata write goes through, leaving
    /// the field unchanged. Both writes here fail the same way.
    pub fn set_sources<I, P>(&mut self, sources: I) -> Result<()>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<str>,
    {
        let rendered = crate::metadata::render_source_list(&self.key(SOURCES), sources)?;
        self.insert(SOURCES, rendered)?;
        Ok(())
    }

    /// Records how this column derives its value from that field.
    ///
    /// The function is stored in its canonical spelling, so a dialect alias a
    /// caller resolved reads back as the one name the grammar owns.
    ///
    /// # Errors
    ///
    /// [`Self::set_sources`] carries the rule, and a function that cannot take
    /// a single argument is refused before anything is written.
    pub fn set_transform(&mut self, transform: Function) -> Result<()> {
        self.insert(TRANSFORM, transform.as_str())?;
        Ok(())
    }
}
