//! User-defined functions: the one open door in the closed function set.
//!
//! The grammar's [`Function`] set is closed so that the three evaluators
//! agree about every function they know. A user-defined function is spelled
//! `namespace.name(arguments)` and is not in the grammar at all: it is
//! *registered*, once per process, with a [`FunctionSignature`] that types it
//! exactly as a grammar function is typed, and an implementation the scalar
//! and vectorized tiers both call. The statistics tier never learns it: a
//! user function is opaque to pushdown, so a filter over one reads the rows.
//!
//! # The signature is a field
//!
//! A signature is a struct [`Field`] named `namespace.name`: each child is one
//! parameter, in position order, typed by its datatype and nullability, and a
//! child declaring a [default](FunctionSignature::parameter_default) is
//! optional. The return is the field's `function:returns` property. This is
//! what a binding registers from a decorated callable, what a stored field
//! carries as its [`transform:function`](crate::TransformField), and what
//! [`FunctionSignature::as_field`] and [`FunctionSignature::from_field`] turn
//! into each other without loss.
//!
//! # Calling
//!
//! A call binds its arguments to the parameters by position, fills the
//! defaults the caller left out, and casts each argument to its parameter's
//! datatype through [`DataType::cast_scalar`], so an implementation receives
//! exactly the values its signature declares. A null argument meeting a
//! parameter declared `not null` answers null without calling, the same rule
//! every grammar function follows; a nullable parameter receives the null.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock, RwLock};

use smol_str::{SmolStr, format_smolstr};

use super::term::Term;
use super::{Function, named};
use crate::{DataType, Error, Field, Result, Scalar};

/// The metadata property naming a signature's return field.
const RETURNS_KEY: &str = "function:returns";

/// The metadata property holding a parameter's default, in literal spelling.
const DEFAULT_KEY: &str = "function:default";

/// The qualified name of a user-defined function: `namespace.name`.
///
/// Both parts are identifiers - ASCII letters, digits and underscores, not
/// starting with a digit - and the reference is ASCII case-insensitive, held
/// lowercase, so `Py.Double` and `py.double` are one function.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, ::serde::Serialize)]
#[serde(transparent)]
pub struct UserRef {
    qualified: SmolStr,
}

impl UserRef {
    /// Name a function in a namespace.
    ///
    /// # Errors
    ///
    /// Returns an error when either part is not an identifier.
    pub fn new(namespace: &str, name: &str) -> Result<Self> {
        for (what, part) in [("namespace", namespace), ("name", name)] {
            let mut characters = part.chars();
            let head = characters.next();
            let valid = head.is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
                && characters.all(|held| held.is_ascii_alphanumeric() || held == '_');
            if !valid {
                return Err(Error::InvalidRecord {
                    path: SmolStr::new_static("$.function"),
                    reason: crate::text::expected_got(
                        format_args!("an identifier as the function {what}"),
                        format_args!("{part:?}"),
                    ),
                });
            }
        }
        Ok(Self {
            qualified: format_smolstr!(
                "{}.{}",
                namespace.to_ascii_lowercase(),
                name.to_ascii_lowercase()
            ),
        })
    }

    /// Read `namespace.name`.
    ///
    /// # Errors
    ///
    /// Returns an error when the text is not two identifiers around one dot.
    pub fn parse(text: &str) -> Result<Self> {
        match text.split_once('.') {
            Some((namespace, name)) => Self::new(namespace, name),
            None => Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.function"),
                reason: crate::text::expected_got(
                    "a function spelled `namespace.name`",
                    format_args!("{text:?}"),
                ),
            }),
        }
    }

    /// The qualified `namespace.name`, lowercase.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.qualified.as_str()
    }

    /// The namespace half.
    #[must_use]
    pub fn namespace(&self) -> &str {
        self.qualified
            .split_once('.')
            .map_or("", |(namespace, _)| namespace)
    }

    /// The name half.
    #[must_use]
    pub fn name(&self) -> &str {
        self.qualified.split_once('.').map_or("", |(_, name)| name)
    }
}

impl<'de> ::serde::Deserialize<'de> for UserRef {
    fn deserialize<D: ::serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let text = SmolStr::deserialize(deserializer)?;
        Self::parse(&text).map_err(::serde::de::Error::custom)
    }
}

impl std::fmt::Display for UserRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.qualified)
    }
}

/// How a user-defined function is typed: its parameters and its return.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FunctionSignature {
    reference: UserRef,
    parameters: Vec<Field>,
    returns: Field,
}

impl FunctionSignature {
    /// Declare a function's parameters, in position order, and its return.
    ///
    /// # Errors
    ///
    /// Returns an error when two parameters share a name, when a parameter
    /// without a default follows one with a default, or when a default does
    /// not read as its parameter's datatype.
    pub fn new(
        reference: UserRef,
        parameters: impl IntoIterator<Item = Field>,
        returns: Field,
    ) -> Result<Self> {
        let parameters: Vec<Field> = parameters.into_iter().collect();
        let mut defaulted = false;
        for (index, parameter) in parameters.iter().enumerate() {
            if parameters[..index]
                .iter()
                .any(|held| held.name().eq_ignore_ascii_case(parameter.name()))
            {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("$.{}", reference),
                    reason: format_smolstr!(
                        "expected each parameter once, got {:?} twice",
                        parameter.name()
                    ),
                });
            }
            let has_default = Self::parameter_default(parameter)?.is_some();
            if defaulted && !has_default {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("$.{}.{}", reference, parameter.name()),
                    reason: SmolStr::new_static(
                        "expected every parameter after a defaulted one to carry a default too",
                    ),
                });
            }
            defaulted |= has_default;
        }
        Ok(Self {
            reference,
            parameters,
            returns,
        })
    }

    /// The function this signature types.
    #[must_use]
    pub const fn reference(&self) -> &UserRef {
        &self.reference
    }

    /// The parameters, in position order.
    #[must_use]
    pub fn parameters(&self) -> &[Field] {
        &self.parameters
    }

    /// The field a call publishes: its datatype and whether it may be null.
    #[must_use]
    pub const fn returns(&self) -> &Field {
        &self.returns
    }

    /// How many arguments a call takes: at least the parameters without a
    /// default, at most every parameter.
    #[must_use]
    pub fn arity(&self) -> (usize, usize) {
        let required = self
            .parameters
            .iter()
            .take_while(|parameter| !matches!(Self::parameter_default(parameter), Ok(Some(_))))
            .count();
        (required, self.parameters.len())
    }

    /// The default a parameter declares, if it declares one.
    ///
    /// A default is stored as the literal the grammar spells it with, so
    /// `1.5`, `'EUR'` and `date32 '2024-01-01'` read back as the values they
    /// name, cast to the parameter's datatype.
    ///
    /// # Errors
    ///
    /// Returns an error when the stored text is not a literal, or does not
    /// read as the parameter's datatype.
    pub fn parameter_default(parameter: &Field) -> Result<Option<Scalar>> {
        let Some(stored) = parameter.get_metadata(DEFAULT_KEY) else {
            return Ok(None);
        };
        let term: Term = stored
            .parse()
            .map_err(|error| Error::InvalidMetadataValue {
                key: SmolStr::new_static(DEFAULT_KEY),
                reason: format_smolstr!("{error}"),
            })?;
        let Some(literal) = term.as_literal() else {
            return Err(Error::InvalidMetadataValue {
                key: SmolStr::new_static(DEFAULT_KEY),
                reason: crate::text::expected_got("a literal", format_args!("`{term}`")),
            });
        };
        if literal.value().is_null() {
            return Ok(Some(Scalar::Null));
        }
        parameter
            .dtype()
            .cast_scalar(literal.value())
            .map(Some)
            .map_err(|error| Error::InvalidMetadataValue {
                key: SmolStr::new_static(DEFAULT_KEY),
                reason: format_smolstr!("{error}"),
            })
    }

    /// Return `parameter` declaring `value` as its default.
    ///
    /// # Errors
    ///
    /// Returns an error when the value does not read as the parameter's
    /// datatype, or the property write is refused.
    pub fn with_default(parameter: Field, value: &Scalar) -> Result<Field> {
        let held = if value.is_null() {
            Scalar::Null
        } else {
            parameter.dtype().cast_scalar(value)?
        };
        let spelled = Term::literal(held).to_string();
        parameter.try_with_metadata_entries([(DEFAULT_KEY, spelled.as_str())])
    }

    /// The struct field this signature is: one child per parameter under the
    /// qualified name, the return spelled as the `function:returns` property.
    ///
    /// # Errors
    ///
    /// Returns an error when the parameters do not form a struct.
    pub fn as_field(&self) -> Result<Field> {
        let returns = format!(
            "{} {}",
            self.returns.dtype(),
            if self.returns.is_nullable() {
                "null"
            } else {
                "not null"
            }
        );
        DataType::from_fields(self.parameters.clone())?
            .required_field(self.reference.as_str())
            .try_with_metadata_entries([(RETURNS_KEY, returns.as_str())])
    }

    /// Read a signature back from the field [`as_field`](Self::as_field)
    /// wrote.
    ///
    /// # Errors
    ///
    /// Returns an error when the field is not a struct named
    /// `namespace.name`, or declares no readable return.
    pub fn from_field(field: &Field) -> Result<Self> {
        let reference = UserRef::parse(field.name())?;
        field.require_struct()?;
        let stored =
            field
                .get_metadata(RETURNS_KEY)
                .ok_or_else(|| Error::InvalidMetadataValue {
                    key: SmolStr::new_static(RETURNS_KEY),
                    reason: SmolStr::new_static("expected the return of the function, got none"),
                })?;
        let (dtype, nullable) = match stored.trim().rsplit_once(' ') {
            Some((dtype, "null")) => (dtype.trim(), true),
            Some((dtype, "not null")) | Some((dtype, "notnull")) => (dtype.trim(), false),
            _ => match stored.trim().strip_suffix("not null") {
                Some(dtype) => (dtype.trim(), false),
                None => (stored.trim(), true),
            },
        };
        let dtype: DataType = dtype.parse().map_err(|error| Error::InvalidMetadataValue {
            key: SmolStr::new_static(RETURNS_KEY),
            reason: format_smolstr!("{error}"),
        })?;
        Self::new(
            reference,
            field.fields().iter().cloned(),
            Field::new("returns", dtype, nullable),
        )
    }

    /// The field one call publishes, given the fields its arguments resolve
    /// to: the return, nullable as well whenever a nullable argument meets a
    /// parameter declared `not null`, because that call answers null.
    ///
    /// # Errors
    ///
    /// Returns an error naming the function when the argument count is
    /// outside its arity.
    pub fn output_field(&self, arguments: &[Field], term: &Term) -> Result<Field> {
        let (least, most) = self.arity();
        if arguments.len() < least || arguments.len() > most {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: format_smolstr!(
                    "expected {} to take {}, got {} argument(s)",
                    self.reference,
                    if least == most {
                        format!("{least}")
                    } else {
                        format!("{least} to {most}")
                    },
                    arguments.len()
                ),
            });
        }
        let nulled = arguments
            .iter()
            .zip(&self.parameters)
            .any(|(argument, parameter)| argument.is_nullable() && !parameter.is_nullable());
        Ok(named(
            term,
            self.returns.dtype().clone(),
            self.returns.is_nullable() || nulled,
        ))
    }

    /// The arguments an implementation receives for the values a call was
    /// given: defaults filled in, each cast to its parameter's datatype.
    ///
    /// `None` is a call that answers null without running: a null met a
    /// parameter declared `not null`.
    ///
    /// # Errors
    ///
    /// Returns an error naming the parameter when a value does not cast to
    /// it, or when a required parameter was left out.
    pub fn fill(&self, values: &[Scalar]) -> Result<Option<Vec<Scalar>>> {
        let mut filled = Vec::with_capacity(self.parameters.len());
        for (index, parameter) in self.parameters.iter().enumerate() {
            let value = match values.get(index) {
                Some(value) => value.clone(),
                None => match Self::parameter_default(parameter)? {
                    Some(default) => default,
                    None => {
                        return Err(Error::InvalidRecord {
                            path: format_smolstr!("$.{}.{}", self.reference, parameter.name()),
                            reason: SmolStr::new_static("expected an argument, got none"),
                        });
                    }
                },
            };
            if value.is_null() {
                if !parameter.is_nullable() {
                    return Ok(None);
                }
                filled.push(Scalar::Null);
                continue;
            }
            let cast =
                parameter
                    .dtype()
                    .cast_scalar(&value)
                    .map_err(|error| Error::InvalidRecord {
                        path: format_smolstr!("$.{}.{}", self.reference, parameter.name()),
                        reason: format_smolstr!("{error}"),
                    })?;
            filled.push(cast);
        }
        Ok(Some(filled))
    }
}

/// One registered implementation: what a call runs.
///
/// [`call`](Self::call) is the scalar tier and is what a binding implements;
/// [`call_arrow`](Self::call_arrow) is the vectorized tier, and its default
/// runs the scalar call once per row through the one array-to-value crossing,
/// so an implementation that has no columnar form still runs over batches.
pub trait UserFunction: Send + Sync {
    /// How this function is typed.
    fn signature(&self) -> &FunctionSignature;

    /// Answer one call over arguments already
    /// [filled](FunctionSignature::fill) to the signature.
    ///
    /// # Errors
    ///
    /// Returns whatever the implementation refuses; the answer is cast to the
    /// declared return by the caller.
    fn call(&self, arguments: &[Scalar]) -> Result<Scalar>;

    /// Answer one call over whole columns, `rows` long each.
    ///
    /// `fields` type the argument columns as the evaluator resolved them and
    /// `output` is the field the call publishes. The default fills and calls
    /// row by row.
    ///
    /// # Errors
    ///
    /// Returns a crossing failure, or whatever [`call`](Self::call) refuses.
    #[cfg(feature = "arrow")]
    fn call_arrow(
        &self,
        fields: &[Field],
        arguments: &[arrow_array::ArrayRef],
        rows: usize,
        output: &Field,
    ) -> Result<arrow_array::ArrayRef> {
        let mut columns = Vec::with_capacity(arguments.len());
        for (field, array) in fields.iter().zip(arguments) {
            let values = crate::arrow::array_to_value(field, array.as_ref())?;
            let Some(items) = values.as_sequence() else {
                return Err(registry_error("expected a column to cross as a sequence"));
            };
            columns.push(items.to_vec());
        }
        let mut answers = Vec::with_capacity(rows);
        let mut values = Vec::with_capacity(arguments.len());
        for row in 0..rows {
            values.clear();
            for column in &columns {
                values.push(column.get(row).cloned().unwrap_or(Scalar::Null));
            }
            answers.push(match self.signature().fill(&values)? {
                Some(filled) => self.call(&filled)?,
                None => Scalar::Null,
            });
        }
        let answers = Scalar::from_sequence(answers);
        Ok(crate::arrow::array_from_value(output, &answers)?)
    }
}

/// The refusal the registry itself raises.
fn registry_error(reason: &'static str) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$.function"),
        reason: SmolStr::new_static(reason),
    }
}

/// The process-wide registry: one implementation per qualified name.
fn registry() -> &'static RwLock<BTreeMap<SmolStr, Arc<dyn UserFunction>>> {
    static REGISTRY: OnceLock<RwLock<BTreeMap<SmolStr, Arc<dyn UserFunction>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(BTreeMap::new()))
}

/// Register a function under its signature's name, replacing one registered
/// before, so a decorator run twice keeps the latest definition.
///
/// # Errors
///
/// Returns an internal error when the registry lock is poisoned.
pub fn register_function(function: Arc<dyn UserFunction>) -> Result<()> {
    let name = SmolStr::new(function.signature().reference().as_str());
    registry()
        .write()
        .map_err(|_| registry_error("the function registry lock is poisoned"))?
        .insert(name, function);
    Ok(())
}

/// Remove a registered function, answering whether one was registered.
pub fn unregister_function(reference: &UserRef) -> bool {
    registry()
        .write()
        .ok()
        .is_some_and(|mut held| held.remove(reference.as_str()).is_some())
}

/// The registered function a reference names.
///
/// # Errors
///
/// Returns an error naming the reference when nothing is registered under it:
/// a user function is refused where it is typed or bound, never silently
/// null.
pub fn lookup_function(reference: &UserRef) -> Result<Arc<dyn UserFunction>> {
    registry()
        .read()
        .map_err(|_| registry_error("the function registry lock is poisoned"))?
        .get(reference.as_str())
        .cloned()
        .ok_or_else(|| Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: format_smolstr!(
                "expected a registered function for {reference}, got none; register it before binding"
            ),
        })
}

/// Every registered function, by qualified name.
#[must_use]
pub fn registered_functions() -> Vec<UserRef> {
    registry().read().map_or_else(
        |_| Vec::new(),
        |held| {
            held.keys()
                .filter_map(|name| UserRef::parse(name).ok())
                .collect()
        },
    )
}

impl Function {
    /// A user-defined function by qualified name; registered separately.
    ///
    /// # Errors
    ///
    /// Returns the error [`UserRef::new`] does.
    pub fn user(namespace: &str, name: &str) -> Result<Self> {
        UserRef::new(namespace, name).map(Self::User)
    }

    /// Read a function by name: a grammar function by canonical name or
    /// alias, or a user function spelled `namespace.name`.
    ///
    /// # Errors
    ///
    /// Returns an error listing the vocabulary when the text names neither.
    pub fn resolve(name: &str) -> Result<Self> {
        if let Some(function) = Self::from_name(name) {
            return Ok(function);
        }
        UserRef::parse(name)
            .map(Self::User)
            .map_err(|_| Error::InvalidRecord {
                path: SmolStr::new_static("$.function"),
                reason: crate::text::expected_got(
                    format_args!(
                        "one of the functions {}, or a user function spelled `namespace.name`",
                        Self::vocabulary()
                    ),
                    format_args!("{name:?}"),
                ),
            })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::expression::{Filter, Selector};

    /// `rs.double(value)`: twice an integer.
    struct Double(FunctionSignature);

    impl UserFunction for Double {
        fn signature(&self) -> &FunctionSignature {
            &self.0
        }

        fn call(&self, arguments: &[Scalar]) -> Result<Scalar> {
            let value = arguments[0].as_i64().unwrap_or_default();
            Ok(Scalar::from(value * 2))
        }
    }

    /// `rs.add(value, amount = 1)`: an integer plus a defaulted one.
    struct Add(FunctionSignature);

    impl UserFunction for Add {
        fn signature(&self) -> &FunctionSignature {
            &self.0
        }

        fn call(&self, arguments: &[Scalar]) -> Result<Scalar> {
            let value = arguments[0].as_i64().unwrap_or_default();
            let amount = arguments[1].as_i64().unwrap_or_default();
            Ok(Scalar::from(value + amount))
        }
    }

    fn registered() {
        let double = FunctionSignature::new(
            UserRef::new("rs", "double").unwrap(),
            [DataType::Int64.required_field("value")],
            DataType::Int64.nullable_field("returns"),
        )
        .unwrap();
        register_function(Arc::new(Double(double))).unwrap();
        let amount = FunctionSignature::with_default(
            DataType::Int64.required_field("amount"),
            &Scalar::from(1_i64),
        )
        .unwrap();
        let add = FunctionSignature::new(
            UserRef::new("rs", "add").unwrap(),
            [DataType::Int64.required_field("value"), amount],
            DataType::Int64.nullable_field("returns"),
        )
        .unwrap();
        register_function(Arc::new(Add(add))).unwrap();
    }

    fn rows() -> Field {
        DataType::from_fields([
            DataType::Int64.nullable_field("size"),
            DataType::utf8().nullable_field("ccy"),
        ])
        .unwrap()
        .required_field("rows")
    }

    #[test]
    fn a_qualified_name_parses_prints_and_types_through_its_registration() {
        registered();
        let term: Term = "RS.Double(size)".parse().unwrap();
        assert_eq!(term.to_string(), "rs.double(size)");
        let typed = term.field(&rows()).unwrap();
        assert_eq!(typed.dtype(), &DataType::Int64);
        // A nullable argument meeting a `not null` parameter answers null.
        assert!(typed.is_nullable());
        assert_eq!(term.field(&rows()).unwrap().name(), "rs.double(size)");
    }

    #[test]
    fn an_unregistered_function_is_refused_by_name_where_it_is_typed() {
        let term: Term = "nobody.knows(size)".parse().unwrap();
        let error = term.field(&rows()).unwrap_err().to_string();
        assert!(error.contains("nobody.knows"), "{error}");
        assert!(error.contains("register"), "{error}");
        let bad = UserRef::new("9lives", "f").unwrap_err().to_string();
        assert!(bad.contains("identifier"), "{bad}");
    }

    #[test]
    fn the_scalar_and_vectorized_tiers_agree_and_defaults_fill_in() {
        registered();
        let selector: Selector =
            "rs.double(size) as doubled, rs.add(size) as next, rs.add(size, 10) as later"
                .parse()
                .unwrap();
        let root = rows();
        let bound = selector.bind(&root).unwrap();
        let row = bound
            .apply_scalar(&Scalar::from_sequence([
                Scalar::from(2_i64),
                Scalar::from("EUR"),
            ]))
            .unwrap();
        assert_eq!(
            row,
            Scalar::from_sequence([
                Scalar::from(4_i64),
                Scalar::from(3_i64),
                Scalar::from(12_i64)
            ])
        );
        let batch = arrow_array::RecordBatch::try_from_iter([
            (
                "size",
                Arc::new(arrow_array::Int64Array::from(vec![Some(1), None, Some(3)]))
                    as arrow_array::ArrayRef,
            ),
            (
                "ccy",
                Arc::new(arrow_array::StringArray::from(vec!["a", "b", "c"]))
                    as arrow_array::ArrayRef,
            ),
        ])
        .unwrap();
        let projected = selector.apply_arrow_batch(&batch).unwrap();
        let doubled = projected
            .column(0)
            .as_any()
            .downcast_ref::<arrow_array::Int64Array>()
            .unwrap();
        assert_eq!(doubled.iter().collect::<Vec<_>>(), [Some(2), None, Some(6)]);
        let filter: Filter = "rs.double(size) > 2".parse().unwrap();
        assert_eq!(filter.apply_arrow_batch(&batch).unwrap().num_rows(), 1);
    }

    #[test]
    fn a_signature_is_a_struct_field_both_ways() {
        registered();
        let signature = lookup_function(&UserRef::parse("rs.add").unwrap())
            .unwrap()
            .signature()
            .clone();
        let field = signature.as_field().unwrap();
        assert_eq!(field.name(), "rs.add");
        assert_eq!(field.fields().len(), 2);
        assert_eq!(field.get_metadata("function:returns"), Some("int64 null"));
        assert_eq!(
            field.fields()[1].get_metadata("function:default"),
            Some("1")
        );
        assert_eq!(FunctionSignature::from_field(&field).unwrap(), signature);
        assert_eq!(signature.arity(), (1, 2));
        // A default has to trail.
        let leading = FunctionSignature::new(
            UserRef::new("rs", "wrong").unwrap(),
            [
                FunctionSignature::with_default(
                    DataType::Int64.required_field("a"),
                    &Scalar::from(1_i64),
                )
                .unwrap(),
                DataType::Int64.required_field("b"),
            ],
            DataType::Int64.nullable_field("returns"),
        );
        assert!(leading.unwrap_err().to_string().contains("default"));
    }

    #[test]
    fn a_call_over_columns_is_stored_as_the_function_and_its_sources() {
        registered();
        let selector: Selector = "ccy, rs.double(size) as doubled".parse().unwrap();
        let stored = selector.into_field(&rows()).unwrap();
        let doubled = &stored.fields()[1];
        assert_eq!(
            doubled.get_metadata("transform:function"),
            Some("rs.double")
        );
        assert_eq!(
            doubled.get_metadata("transform:sources"),
            Some(r#"["size"]"#)
        );
        assert_eq!(doubled.get_metadata("transform:expression"), None);
        assert_eq!(
            doubled.as_transform().term().unwrap().unwrap().to_string(),
            "rs.double(size)"
        );
        assert_eq!(
            Selector::from_field(&stored).to_string(),
            "ccy utf8 null, rs.double(size) as doubled int64 null"
        );
        // A grammar function over one column is stored the same way, and a
        // computed argument keeps the expression spelling.
        let mut year = DataType::Int32.nullable_field("year");
        year.as_transform_mut()
            .set_term(&"year(event)".parse().unwrap())
            .unwrap();
        assert_eq!(year.get_metadata("transform:function"), Some("year"));
        let mut twice = DataType::Int64.nullable_field("twice");
        twice
            .as_transform_mut()
            .set_term(&"size * 2".parse().unwrap())
            .unwrap();
        assert_eq!(twice.get_metadata("transform:expression"), Some("size * 2"));
        assert_eq!(twice.get_metadata("transform:function"), None);
        assert!(
            unregister_function(&UserRef::parse("rs.double").unwrap())
                || registered_functions().is_empty()
        );
        registered();
    }
}
