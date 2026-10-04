//! Typed PivotTable request document. Source cells and package parts are
//! validated by the workbook phases that own them.

pub(crate) mod compute;
pub(crate) mod layout;
pub(crate) mod part;

/// Authored default for an explicitly named blank pivot item.
pub(crate) const BLANK_CAPTION: &str = "(blank)";

use std::borrow::Cow;
use std::collections::BTreeMap;

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result, Scalar};

use super::cell::CellRange;
use super::formula::aggregate::Aggregate;

const SPEC_FIELDS: &[&str] = &[
    "name",
    "source",
    "rows",
    "columns",
    "values",
    "subtotals",
    "rowGrandTotals",
    "columnGrandTotals",
];
const SOURCE_FIELDS: &[&str] = &["sheet", "range"];
const AXIS_FIELDS: &[&str] = &["field", "order"];
const VALUE_FIELDS: &[&str] = &["field", "aggregate", "caption", "numberFormat"];

/// One worksheet rectangle whose first row names distinct source fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PivotSource {
    /// The source worksheet.
    pub sheet: SmolStr,
    /// The rectangle including source headers.
    pub range: CellRange,
}

/// The order of one pivot axis. Item comparison itself is native-calibrated
/// separately from generic expression sorting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ItemOrder {
    /// Sort source items in ascending native order.
    Ascending,
    /// Sort source items in descending native order.
    Descending,
}

impl ItemOrder {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ascending => "ascending",
            Self::Descending => "descending",
        }
    }

    fn read(text: &str) -> Option<Self> {
        match text {
            "ascending" => Some(Self::Ascending),
            "descending" => Some(Self::Descending),
            _ => None,
        }
    }
}

/// A source field placed on a row or column axis.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AxisField {
    /// The exact source header name.
    pub field: SmolStr,
    /// The visible item order.
    pub order: ItemOrder,
}

/// One value field and its aggregation chosen at intake.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValueField {
    /// The exact source header name.
    pub field: SmolStr,
    /// The shared aggregate kind.
    pub aggregate: Aggregate,
    /// The optional displayed caption.
    pub caption: Option<SmolStr>,
    /// The optional number format code.
    pub number_format: Option<SmolStr>,
}

/// A tabular pivot request. The source header and destination are validated
/// by `Workbook` against the current cells before any write.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PivotSpec {
    /// The workbook-local pivot name.
    pub name: SmolStr,
    /// The source worksheet and range.
    pub source: PivotSource,
    /// Row axis fields in display order.
    pub rows: Vec<AxisField>,
    /// Column axis fields in display order.
    pub columns: Vec<AxisField>,
    /// Value fields in display order.
    pub values: Vec<ValueField>,
    /// Whether to show subgroup totals.
    pub subtotals: bool,
    /// Whether to show the row-axis grand total.
    pub row_grand_totals: bool,
    /// Whether to show the column-axis grand total.
    pub column_grand_totals: bool,
}

impl PivotSpec {
    /// Read the one named document shape. Optional value captions/formats may
    /// be absent or null; all other fields are required and exactly typed.
    /// Workbook-dependent source headers, fields and collisions are checked
    /// by `Workbook::add_pivot`, not re-parsed here.
    ///
    /// # Example
    ///
    /// ```
    /// use yggdryl::excel::PivotSpec;
    /// use yggdryl::Scalar;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let source = Scalar::from_struct([
    ///     ("sheet", Scalar::from("Data")),
    ///     ("range", Scalar::from("A1:B3")),
    /// ])?;
    /// let row = Scalar::from_struct([
    ///     ("field", Scalar::from("Region")),
    ///     ("order", Scalar::from("ascending")),
    /// ])?;
    /// let value = Scalar::from_struct([
    ///     ("field", Scalar::from("Sales")),
    ///     ("aggregate", Scalar::from("sum")),
    ///     ("caption", Scalar::Null),
    ///     ("numberFormat", Scalar::Null),
    /// ])?;
    /// let document = Scalar::from_struct([
    ///     ("name", Scalar::from("Regional Sales")),
    ///     ("source", source),
    ///     ("rows", Scalar::from_sequence([row])),
    ///     ("columns", Scalar::from_sequence(std::iter::empty::<Scalar>())),
    ///     ("values", Scalar::from_sequence([value])),
    ///     ("subtotals", Scalar::from(false)),
    ///     ("rowGrandTotals", Scalar::from(true)),
    ///     ("columnGrandTotals", Scalar::from(false)),
    /// ])?;
    /// let spec = PivotSpec::from_scalar(&document)?;
    /// assert_eq!(spec.into_scalar(), document);
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_scalar(value: &Scalar) -> Result<Self> {
        let fields = record(value, "$", SPEC_FIELDS)?;
        let name = text(required(fields, "name", "$")?, "$.name")?;
        let source = source(required(fields, "source", "$")?)?;
        let rows = axis_fields(required(fields, "rows", "$")?, "$.rows")?;
        if rows.is_empty() {
            return Err(refused("$.rows", "at least one row field", rows.len()));
        }
        let columns = axis_fields(required(fields, "columns", "$")?, "$.columns")?;
        // One PivotField has one axis orientation. Reject contradiction at
        // intake before source cells or package parts are touched.
        let mut seen = std::collections::BTreeSet::new();
        for (path, axes) in [("$.rows", &rows), ("$.columns", &columns)] {
            for (index, axis) in axes.iter().enumerate() {
                if !seen.insert(axis.field.as_str()) {
                    return Err(refused(
                        format_smolstr!("{path}[{index}].field"),
                        "a field used on one pivot axis",
                        axis.field.as_str(),
                    ));
                }
            }
        }
        let values = value_fields(required(fields, "values", "$")?)?;
        if values.is_empty() {
            return Err(refused(
                "$.values",
                "at least one value field",
                values.len(),
            ));
        }
        Ok(Self {
            name,
            source,
            rows,
            columns,
            values,
            subtotals: boolean(required(fields, "subtotals", "$")?, "$.subtotals")?,
            row_grand_totals: boolean(
                required(fields, "rowGrandTotals", "$")?,
                "$.rowGrandTotals",
            )?,
            column_grand_totals: boolean(
                required(fields, "columnGrandTotals", "$")?,
                "$.columnGrandTotals",
            )?,
        })
    }

    /// The canonical named document consumed by both bindings and the service.
    pub fn into_scalar(&self) -> Scalar {
        let source = Scalar::from_struct([
            ("sheet", Scalar::from(self.source.sheet.clone())),
            ("range", Scalar::from(self.source.range.to_string())),
        ])
        .expect("two distinct source fields");
        let axes = |items: &[AxisField]| {
            Scalar::from_sequence(items.iter().map(|axis| {
                Scalar::from_struct([
                    ("field", Scalar::from(axis.field.clone())),
                    ("order", Scalar::from(axis.order.as_str())),
                ])
                .expect("two distinct axis fields")
            }))
        };
        let values = Scalar::from_sequence(self.values.iter().map(|value| {
            Scalar::from_struct([
                ("field", Scalar::from(value.field.clone())),
                ("aggregate", Scalar::from(value.aggregate.as_str())),
                (
                    "caption",
                    value.caption.clone().map_or(Scalar::Null, Scalar::from),
                ),
                (
                    "numberFormat",
                    value
                        .number_format
                        .clone()
                        .map_or(Scalar::Null, Scalar::from),
                ),
            ])
            .expect("four distinct value fields")
        }));
        Scalar::from_struct([
            ("name", Scalar::from(self.name.clone())),
            ("source", source),
            ("rows", axes(&self.rows)),
            ("columns", axes(&self.columns)),
            ("values", values),
            ("subtotals", Scalar::from(self.subtotals)),
            ("rowGrandTotals", Scalar::from(self.row_grand_totals)),
            ("columnGrandTotals", Scalar::from(self.column_grand_totals)),
        ])
        .expect("eight distinct pivot fields")
    }
}

/// Whether this pivot was authored here or read from a package. A foreign
/// structure the typed writer cannot reproduce remains visible but read-only.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PivotOrigin {
    /// The current workbook created the pivot from a typed request.
    Created,
    /// The package supplied it; `reason` explains a read-only boundary.
    Read {
        /// Whether this foreign structure is fully representable here.
        editable: bool,
        /// The first unsupported structure, when `editable` is false.
        reason: Option<SmolStr>,
    },
}

/// One source field's inferred PivotTable role and distinct item count.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PivotFieldInfo {
    /// Authored source-header spelling.
    pub name: SmolStr,
    /// Whether every nonblank source value is numeric.
    pub numeric: bool,
    /// The default reducer for a new value field.
    pub aggregate: Aggregate,
    /// Distinct source items, including a blank item when observed.
    pub items: u64,
}

/// The identity proven by workbook, host and cache relationships together.
pub(crate) struct PivotIdentity {
    pub name: SmolStr,
    pub sheet: SmolStr,
    pub location: CellRange,
    pub cache_id: u32,
    pub table_part: SmolStr,
    pub cache_part: SmolStr,
}

/// A pivot's identity and location, even when its foreign XML is read-only.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PivotTable {
    name: SmolStr,
    sheet: SmolStr,
    location: CellRange,
    cache_id: u32,
    spec: Option<PivotSpec>,
    origin: PivotOrigin,
    pub(crate) table_part: SmolStr,
    pub(crate) cache_part: SmolStr,
}

impl PivotTable {
    /// The workbook-unique pivot name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The worksheet whose ordinary cells show this pivot.
    #[must_use]
    pub fn host_sheet(&self) -> &str {
        &self.sheet
    }

    /// The output rectangle currently owned by the pivot.
    #[must_use]
    pub const fn location(&self) -> CellRange {
        self.location
    }

    /// Its workbook cache identity.
    #[must_use]
    pub const fn cache_id(&self) -> u32 {
        self.cache_id
    }

    /// The typed request when all selected foreign structures are representable.
    #[must_use]
    pub fn spec(&self) -> Option<&PivotSpec> {
        self.spec.as_ref()
    }

    /// Whether a refresh/update can re-emit this pivot without losing markup.
    #[must_use]
    pub fn editable(&self) -> bool {
        matches!(
            &self.origin,
            PivotOrigin::Created | PivotOrigin::Read { editable: true, .. }
        )
    }

    /// Its source and the located read-only reason, if any.
    #[must_use]
    pub fn origin(&self) -> &PivotOrigin {
        &self.origin
    }

    pub(crate) fn read(
        identity: PivotIdentity,
        spec: Option<PivotSpec>,
        reason: Option<SmolStr>,
    ) -> Self {
        let editable = spec.is_some() && reason.is_none();
        Self {
            name: identity.name,
            sheet: identity.sheet,
            location: identity.location,
            cache_id: identity.cache_id,
            spec,
            origin: PivotOrigin::Read { editable, reason },
            table_part: identity.table_part,
            cache_part: identity.cache_part,
        }
    }
}

fn refused(path: impl Into<SmolStr>, expected: &str, actual: impl std::fmt::Debug) -> Error {
    Error::InvalidRecord {
        path: path.into(),
        reason: format_smolstr!("expected {expected}, got {actual:?}"),
    }
}

fn record<'a>(
    value: &'a Scalar,
    path: &str,
    allowed: &[&str],
) -> Result<&'a BTreeMap<SmolStr, Scalar>> {
    let fields = value
        .as_struct()
        .ok_or_else(|| refused(path, "a named struct", value))?;
    if let Some(name) = fields.keys().find(|name| !allowed.contains(&name.as_str())) {
        return Err(refused(
            format_smolstr!("{path}.{name}"),
            "a declared pivot field",
            name,
        ));
    }
    Ok(fields)
}

fn required<'a>(
    fields: &'a BTreeMap<SmolStr, Scalar>,
    name: &str,
    path: &str,
) -> Result<&'a Scalar> {
    fields.get(name).ok_or_else(|| {
        refused(
            format_smolstr!("{path}.{name}"),
            "a present field",
            "absent",
        )
    })
}

fn text(value: &Scalar, path: &str) -> Result<SmolStr> {
    value
        .as_str()
        .filter(|text| !text.is_empty())
        .map(SmolStr::new)
        .ok_or_else(|| refused(path, "nonempty text", value))
}

fn optional_text(value: Option<&Scalar>, path: &str) -> Result<Option<SmolStr>> {
    match value {
        None | Some(Scalar::Null) => Ok(None),
        Some(value) => text(value, path).map(Some),
    }
}

fn boolean(value: &Scalar, path: &str) -> Result<bool> {
    value
        .as_bool()
        .ok_or_else(|| refused(path, "a Boolean", value))
}

fn sequence<'a>(value: &'a Scalar, path: &str) -> Result<Cow<'a, [Scalar]>> {
    value
        .sequence_rows()
        .ok_or_else(|| refused(path, "a sequence", value))
}

fn source(value: &Scalar) -> Result<PivotSource> {
    let fields = record(value, "$.source", SOURCE_FIELDS)?;
    let sheet = text(required(fields, "sheet", "$.source")?, "$.source.sheet")?;
    let range_value = required(fields, "range", "$.source")?;
    let range_text = text(range_value, "$.source.range")?;
    let range = range_text
        .parse::<CellRange>()
        .map_err(|_| refused("$.source.range", "a worksheet rectangle", range_value))?;
    Ok(PivotSource { sheet, range })
}

fn axis_fields(value: &Scalar, path: &str) -> Result<Vec<AxisField>> {
    let rows = sequence(value, path)?;
    rows.iter()
        .enumerate()
        .map(|(index, value)| {
            let at = format_smolstr!("{path}[{index}]");
            let fields = record(value, &at, AXIS_FIELDS)?;
            let field = text(
                required(fields, "field", &at)?,
                &format_smolstr!("{at}.field"),
            )?;
            let order_value = required(fields, "order", &at)?;
            let order = order_value
                .as_str()
                .and_then(ItemOrder::read)
                .ok_or_else(|| {
                    refused(
                        format_smolstr!("{at}.order"),
                        "ascending or descending",
                        order_value,
                    )
                })?;
            Ok(AxisField { field, order })
        })
        .collect()
}

fn value_fields(value: &Scalar) -> Result<Vec<ValueField>> {
    let rows = sequence(value, "$.values")?;
    rows.iter()
        .enumerate()
        .map(|(index, value)| {
            let at = format_smolstr!("$.values[{index}]");
            let fields = record(value, &at, VALUE_FIELDS)?;
            let field = text(
                required(fields, "field", &at)?,
                &format_smolstr!("{at}.field"),
            )?;
            let aggregate_value = required(fields, "aggregate", &at)?;
            let aggregate = aggregate_value
                .as_str()
                .and_then(Aggregate::from_name)
                .ok_or_else(|| {
                    refused(
                        format_smolstr!("{at}.aggregate"),
                        "a pivot aggregate",
                        aggregate_value,
                    )
                })?;
            let caption = optional_text(fields.get("caption"), &format_smolstr!("{at}.caption"))?;
            let number_format = optional_text(
                fields.get("numberFormat"),
                &format_smolstr!("{at}.numberFormat"),
            )?;
            Ok(ValueField {
                field,
                aggregate,
                caption,
                number_format,
            })
        })
        .collect()
}
