//! Bind a PivotTable's source columns once before the sparse row walk.

use std::collections::{BTreeMap, HashMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result};

use super::super::formula::aggregate::{Accumulator, Aggregate};
use super::super::formula::value::{Operand, Outcome};
use super::{PivotFieldInfo, PivotSource, PivotSpec};
use crate::Scalar;
use crate::excel::{Cell, CellKind, CellRef, DateSystem, ExcelError, NumberFormat, Sheet};

/// Columns are zero-based physical source coordinates in request order.
#[derive(Debug)]
pub struct BoundSource {
    /// Original header spellings in physical source-column order.
    pub headers: Vec<SmolStr>,
    pub rows: Vec<u32>,
    pub columns: Vec<u32>,
    pub values: Vec<u32>,
}

fn invalid(path: impl Into<SmolStr>, expected: &str, actual: impl std::fmt::Debug) -> Error {
    Error::InvalidRecord {
        path: path.into(),
        reason: format_smolstr!("expected {expected}, got {actual:?}"),
    }
}

fn source_headers(source: &PivotSource, sheet: &Sheet) -> Result<(Vec<SmolStr>, BTreeMap<SmolStr, u32>)> {
        if !sheet.name().eq_ignore_ascii_case(&source.sheet) {
            return Err(invalid(
                "$.source.sheet",
                "the selected source sheet",
                sheet.name(),
            ));
        }
        let range = source.range;
        range.start().require_in_grid().map_err(|error| {
            invalid("$.source.range", "an in-grid source start", error)
        })?;
        range.end().require_in_grid().map_err(|error| {
            invalid("$.source.range", "an in-grid source end", error)
        })?;
        if range.row_size() < 2 {
            return Err(invalid(
                "$.source.range",
                "a header and at least one body row",
                range,
            ));
        }
        let mut headers = BTreeMap::<SmolStr, u32>::new();
        let mut labels = Vec::with_capacity(range.column_size() as usize);
        for column in range.start().column()..=range.end().column() {
            let at = CellRef::new(range.start().row(), column);
            let label = sheet
                .cell(at)
                .and_then(|cell| cell.value().as_str())
                .ok_or_else(|| {
                    invalid(
                        format_smolstr!("{}!{at}", sheet.name()),
                        "a text source header",
                        "blank or non-text",
                    )
                })?;
            if label.trim().is_empty() {
                return Err(invalid(
                    format_smolstr!("{}!{at}", sheet.name()),
                    "a nonempty source header",
                    label,
                ));
            }
            // Header normalization is one intake fact. Keep the original
            // spelling for the public field while rejecting aliases.
            let folded = SmolStr::new(label.to_ascii_lowercase());
            if let Some(first) = headers.insert(folded, column) {
                return Err(invalid(
                    format_smolstr!("{}!{at}", sheet.name()),
                    "a distinct source header",
                    CellRef::new(range.start().row(), first),
                ));
            }
            labels.push(SmolStr::new(label));
        }
    Ok((labels, headers))
}

impl BoundSource {
    /// Validate the one physical header row and resolve all requested fields.
    /// The cell walk that follows borrows these column indexes, not names.
    pub fn bind(spec: &PivotSpec, sheet: &Sheet) -> Result<Self> {
        let (labels, headers) = source_headers(&spec.source, sheet)?;
        let locate = |name: &str, path: SmolStr| -> Result<u32> {
            headers
                .get(&SmolStr::new(name.to_ascii_lowercase()))
                .copied()
                .ok_or_else(|| invalid(path, "an existing source header", name))
        };
        let rows = spec
            .rows
            .iter()
            .enumerate()
            .map(|(index, axis)| locate(&axis.field, format_smolstr!("$.rows[{index}].field")))
            .collect::<Result<Vec<_>>>()?;
        let columns = spec
            .columns
            .iter()
            .enumerate()
            .map(|(index, axis)| locate(&axis.field, format_smolstr!("$.columns[{index}].field")))
            .collect::<Result<Vec<_>>>()?;
        let values = spec
            .values
            .iter()
            .enumerate()
            .map(|(index, field)| locate(&field.field, format_smolstr!("$.values[{index}].field")))
            .collect::<Result<Vec<_>>>()?;
        // Source names resolve once above. Physical identity, not input spelling,
        // owns axis uniqueness for both from_scalar and caller-built specs.
        let mut axes = BTreeMap::<u32, SmolStr>::new();
        for (index, &column) in rows.iter().enumerate() {
            let path = format_smolstr!("$.rows[{index}].field");
            if let Some(first) = axes.insert(column, path.clone()) {
                return Err(invalid(path, "a distinct source axis", first));
            }
        }
        for (index, &column) in columns.iter().enumerate() {
            let path = format_smolstr!("$.columns[{index}].field");
            if let Some(first) = axes.insert(column, path.clone()) {
                return Err(invalid(path, "a distinct source axis", first));
            }
        }
        Ok(Self {
            headers: labels,
            rows,
            columns,
            values,
        })
    }
}

/// The one retained item fact. Date holds the source serial, not the lossy
/// worksheet date projection; Text keeps the first observed spelling.
#[derive(Clone, Debug, PartialEq)]
pub enum PivotItem {
    Blank,
    Number(f64),
    Date(f64),
    Text(SmolStr),
    Boolean(bool),
    Error(ExcelError),
}

impl PivotItem {
    /// Explicit item@n fixes visible spelling across Excel UI locales.
    pub(crate) fn authored_error_label(&self) -> Option<&'static str> {
        match self {
            Self::Error(error @ (ExcelError::Null | ExcelError::Div0 | ExcelError::Value |
                                  ExcelError::Ref | ExcelError::Name | ExcelError::Num | ExcelError::NA)) =>
                Some(error.as_str()),
            _ => None,
        }
    }
}

/// Typed metadata derived from the interned axis dictionary.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PivotItemFacts {
    pub blank: bool,
    pub number: bool,
    pub date: bool,
    pub string: bool,
    pub mixed: bool,
    pub integer: bool,
    pub date_only: bool,
    pub min_number: Option<f64>,
    pub max_number: Option<f64>,
}

/// One dictionary for an axis field, in first-seen cache insertion order.
#[derive(Debug)]
pub struct PivotItems {
    system: DateSystem,
    entries: Vec<PivotItem>,
    buckets: HashMap<u64, Vec<usize>>,
}

impl PivotItems {
    pub fn new(system: DateSystem) -> Self {
        Self {
            system,
            entries: Vec::new(),
            buckets: HashMap::new(),
        }
    }

    pub fn intern(
        &mut self,
        cell: Option<&Cell>,
        raw_serial: Option<f64>,
        at: CellRef,
    ) -> Result<usize> {
        let candidate = Candidate::from_cell(cell, raw_serial, self.system, at)?;
        let key = candidate.fingerprint();
        if let Some(existing) = self.buckets.get(&key) {
            if let Some(index) = existing
                .iter()
                .copied()
                .find(|&index| candidate.matches(&self.entries[index]))
            {
                return Ok(index);
            }
        }
        let index = self.entries.len();
        self.entries.push(candidate.owned());
        self.buckets.entry(key).or_default().push(index);
        Ok(index)
    }

    pub fn items(&self) -> &[PivotItem] {
        &self.entries
    }

    /// One pass over unique typed items; source rows are never rescanned.
    pub(crate) fn facts(&self) -> PivotItemFacts {
        let mut facts = PivotItemFacts { integer: true, date_only: true, ..PivotItemFacts::default() };
        let mut types = 0_u8;
        for item in &self.entries {
            match item {
                PivotItem::Blank => { facts.blank = true; facts.date_only = false; }
                PivotItem::Number(value) => {
                    facts.number = true;
                    facts.date_only = false;
                    types |= 1;
                    facts.integer &= value.fract() == 0.0;
                    facts.min_number = Some(facts.min_number.map_or(*value, |min| min.min(*value)));
                    facts.max_number = Some(facts.max_number.map_or(*value, |max| max.max(*value)));
                }
                PivotItem::Date(_) => { facts.date = true; types |= 2; }
                PivotItem::Text(_) => { facts.string = true; facts.date_only = false; types |= 4; }
                PivotItem::Boolean(_) => { facts.date_only = false; types |= 8; }
                // Missing/error items do not establish another data type.
                PivotItem::Error(_) => facts.date_only = false,
            }
        }
        facts.integer &= facts.number;
        facts.date_only &= facts.date;
        facts.mixed = types.count_ones() > 1;
        facts
    }

    pub fn cache_date(&self, index: usize) -> Result<SmolStr> {
        let Some(PivotItem::Date(serial)) = self.entries.get(index) else {
            return Err(invalid(
                format_smolstr!("$.items[{index}]"),
                "a date cache item",
                self.entries.get(index),
            ));
        };
        self.system.pivot_cache_datetime(*serial)
    }
}

enum Candidate<'a> {
    Blank,
    Number(f64),
    Date(f64),
    Text(&'a str),
    Boolean(bool),
    Error(ExcelError),
}

impl<'a> Candidate<'a> {
    fn from_cell(
        cell: Option<&'a Cell>,
        raw: Option<f64>,
        system: DateSystem,
        at: CellRef,
    ) -> Result<Self> {
        let Some(cell) = cell else {
            return Ok(Self::Blank);
        };
        if let Some(error) = cell.error() {
            if error == ExcelError::Unrecognized {
                return Err(invalid(
                    at.to_string(),
                    "a recognized pivot error item",
                    cell.error_text(),
                ));
            }
            return Ok(Self::Error(error));
        }
        if cell.value() == &Scalar::Null {
            return Ok(Self::Blank);
        }
        match cell.kind() {
            CellKind::SharedString | CellKind::InlineString | CellKind::FormulaString => {
                let Some(text) = cell.value().as_str() else {
                    return Err(invalid(at.to_string(), "a text pivot item", cell.value()));
                };
                if text.is_empty() {
                    return Ok(Self::Blank);
                }
                if !text.is_ascii() {
                    return Err(invalid(
                        at.to_string(),
                        "an ASCII text pivot item until locale collation is known",
                        text,
                    ));
                }
                Ok(Self::Text(text))
            }
            CellKind::Boolean => cell
                .value()
                .as_bool()
                .map(Self::Boolean)
                .ok_or_else(|| invalid(at.to_string(), "a Boolean pivot item", cell.value())),
            CellKind::Number if cell.format() == NumberFormat::Date => {
                let serial = match raw {
                    Some(serial) => serial,
                    None => system
                        .serial_of(cell.value())
                        .map_err(|error| invalid(at.to_string(), "a date source serial", error))?
                        .map(|(serial, _)| serial)
                        .ok_or_else(|| {
                            invalid(at.to_string(), "a date source serial", cell.value())
                        })?,
                };
                system
                    .pivot_cache_millis_from_serial(serial)
                    .map_err(|error| invalid(at.to_string(), "a valid pivot date serial", error))?;
                Ok(Self::Date(serial))
            }
            CellKind::Number if cell.format() == NumberFormat::General => {
                let value = cell.value();
                let number = if let Some(value) = value.as_f64() {
                    value
                } else if let Some(value) = value.as_i64() {
                    let number = value as f64;
                    if number as i128 != i128::from(value) {
                        return Err(invalid(
                            at.to_string(),
                            "an exactly representable pivot integer",
                            value,
                        ));
                    }
                    number
                } else if let Some(value) = value.as_u64() {
                    let number = value as f64;
                    if number as i128 != i128::from(value) {
                        return Err(invalid(
                            at.to_string(),
                            "an exactly representable pivot integer",
                            value,
                        ));
                    }
                    number
                } else {
                    return Err(invalid(at.to_string(), "a numeric pivot item", value));
                };
                if !number.is_finite() || (number == 0.0 && number.is_sign_negative()) {
                    return Err(invalid(
                        at.to_string(),
                        "a finite nonnegative-zero pivot number",
                        number,
                    ));
                }
                Ok(Self::Number(number))
            }
            _ => Err(invalid(
                at.to_string(),
                "a supported pivot item kind and format",
                (cell.kind(), cell.format()),
            )),
        }
    }

    fn fingerprint(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        match self {
            Self::Blank => 0u8.hash(&mut hasher),
            Self::Number(value) => {
                1u8.hash(&mut hasher);
                value.to_bits().hash(&mut hasher);
            }
            Self::Date(value) => {
                2u8.hash(&mut hasher);
                value.to_bits().hash(&mut hasher);
            }
            Self::Text(value) => {
                3u8.hash(&mut hasher);
                for byte in value.bytes() {
                    hasher.write_u8(byte.to_ascii_lowercase());
                }
            }
            Self::Boolean(value) => {
                4u8.hash(&mut hasher);
                value.hash(&mut hasher);
            }
            Self::Error(value) => {
                5u8.hash(&mut hasher);
                value.hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    fn matches(&self, item: &PivotItem) -> bool {
        match (self, item) {
            (Self::Blank, PivotItem::Blank) => true,
            (Self::Number(a), PivotItem::Number(b)) | (Self::Date(a), PivotItem::Date(b)) => {
                a.to_bits() == b.to_bits()
            }
            (Self::Text(a), PivotItem::Text(b)) => a.eq_ignore_ascii_case(b),
            (Self::Boolean(a), PivotItem::Boolean(b)) => a == b,
            (Self::Error(a), PivotItem::Error(b)) => a == b,
            _ => false,
        }
    }

    fn owned(self) -> PivotItem {
        match self {
            Self::Blank => PivotItem::Blank,
            Self::Number(value) => PivotItem::Number(value),
            Self::Date(value) => PivotItem::Date(value),
            Self::Text(value) => PivotItem::Text(SmolStr::new(value)),
            Self::Boolean(value) => PivotItem::Boolean(value),
            Self::Error(value) => PivotItem::Error(value),
        }
    }
}

/// One populated source tuple. A missing row/column combination has no group;
/// an authored blank value still creates a group with no numeric input.
struct Group {
    aggregate: Aggregate,
    values: Accumulator,
    error: Option<ExcelError>,
    source_error: Option<ExcelError>,
    present: bool,
}

/// The source cell crosses the pivot numeric boundary once, then each needed
/// leaf/row/column/grand accumulator receives the same resolved fact.
#[derive(Clone, Copy)]
enum ValueInput {
    Absent,
    Present,
    Number(f64),
    Error(ExcelError),
}

impl Group {
    fn new(aggregate: Aggregate) -> Self {
        let values = if matches!(
            aggregate,
            Aggregate::StdDev | Aggregate::StdDevP | Aggregate::Var | Aggregate::VarP
        ) {
            Accumulator::variance()
        } else {
            Accumulator::default()
        };
        Self { aggregate, values, error: None, source_error: None, present: false }
    }

    fn input(
        cell: Option<&Cell>,
        raw: Option<f64>,
        system: DateSystem,
        at: CellRef,
    ) -> Result<ValueInput> {
        let Some(cell) = cell else {
            return Ok(ValueInput::Absent);
        };
        if let Some(error) = cell.error() {
            if error == ExcelError::Unrecognized {
                return Err(invalid(
                    at.to_string(),
                    "a recognized pivot error",
                    cell.error_text(),
                ));
            }
            return Ok(ValueInput::Error(error));
        }
        // Openpyxl's empty inline-string source is an authored physical cell
        // that Excel treats as a blank pivot input. A formula-string empty
        // result remains present, as the native edge workbook proves.
        if cell.value() == &Scalar::Null
            || (matches!(cell.kind(), CellKind::SharedString | CellKind::InlineString)
                && cell.formula().is_none()
                && cell.value().as_str() == Some(""))
        {
            return Ok(ValueInput::Absent);
        }
        // Presence is a typed fact for Count; numeric folds ignore it.
        if cell.kind() != CellKind::Number {
            return Ok(ValueInput::Present);
        }
        match cell.calculation_operand(system, raw) {
            Outcome::Computed(Operand::Number(number)) => Ok(ValueInput::Number(number)),
            Outcome::Computed(Operand::Error(error)) => Ok(ValueInput::Error(error)),
            other => Err(invalid(
                at.to_string(),
                "a resolved numeric pivot value",
                other,
            )),
        }
    }

    fn push(&mut self, input: ValueInput) {
        if let ValueInput::Error(error) = input {
            if !matches!(self.aggregate, Aggregate::Count | Aggregate::CountNumbers) {
                self.source_error.get_or_insert(error);
            }
        }
        if self.error.is_some() {
            return;
        }
        self.present |= !matches!(input, ValueInput::Absent);
        match input {
            ValueInput::Absent => {}
            ValueInput::Present if self.aggregate == Aggregate::Count => {
                if let Err(error) = self.values.push_present() {
                    self.error = Some(error);
                }
            }
            ValueInput::Present => {}
            ValueInput::Number(number) => {
                let accepted = match self.aggregate {
                    Aggregate::Count => self.values.push_present(),
                    Aggregate::Product => self.values.push_product(number),
                    _ => self.values.push_number(number),
                };
                if let Err(error) = accepted {
                    self.error = Some(error);
                }
            }
            ValueInput::Error(error) => match self.aggregate {
                Aggregate::Count => {
                    if let Err(error) = self.values.push_present() {
                        self.error = Some(error);
                    }
                }
                Aggregate::CountNumbers => {}
                _ => self.error = Some(error),
            },
        }
    }

    fn finish(self, path: SmolStr) -> Result<PivotMeasure> {
        if !self.present {
            return Ok(PivotMeasure::Empty);
        }
        if let Some(source) = self.source_error {
            return Ok(PivotMeasure::SourceError(source));
        }
        if let Some(error) = self.error {
            return Ok(PivotMeasure::Error(error));
        }
        let result = match self.aggregate {
            Aggregate::Sum => self.values.finish_sum(),
            Aggregate::Average => self.values.finish_average(),
            Aggregate::Count => Ok(self.values.finish_counta()),
            Aggregate::CountNumbers => Ok(self.values.finish_count()),
            Aggregate::Min => Ok(Some(self.values.finish_min())),
            Aggregate::Max => Ok(Some(self.values.finish_max())),
            Aggregate::Product => Ok(Some(self.values.finish_product())),
            Aggregate::Var => self.values.finish_variance(true),
            Aggregate::VarP => self.values.finish_variance(false),
            Aggregate::StdDev | Aggregate::StdDevP => {
                self.values
                    .finish_variance(self.aggregate == Aggregate::StdDev)
                    .and_then(|variance| variance
                        .map(|variance| {
                            Scalar::from(variance)
                                .checked_sqrt()
                                .map_err(|_| ExcelError::Num)
                                .and_then(|root| root.as_f64().ok_or(ExcelError::Num))
                        })
                        .transpose())
            }
        };
        match result {
            Ok(Some(number)) => Ok(PivotMeasure::Number(number)),
            Ok(None) => Err(invalid(
                path,
                "a settled pivot numeric result",
                "uncertain binary64 fold",
            )),
            Err(error) => Ok(PivotMeasure::Error(error)),
        }
    }
}

/// A populated pivot result, distinct from an absent row/column combination.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PivotMeasure {
    Empty,
    Number(f64),
    /// Authored source error, distinct from a derived empty-domain error.
    SourceError(ExcelError),
    Error(ExcelError),
}

/// Group ids use first-observed cache insertion order. Visible ordering is a
/// separate layout fact; no source row stores a tuple or creates a Scalar.
pub struct PivotComputed {
    pub row_items: Vec<PivotItems>,
    pub column_items: Vec<PivotItems>,
    pub row_tuples: Vec<Vec<usize>>,
    pub column_tuples: Vec<Vec<usize>>,
    pub values: HashMap<(usize, usize, usize), PivotMeasure>,
    /// Non-SUM rollups consume original source cells in source order. SUM's
    /// observed grand is folded from completed item groups instead.
    pub row_rollups: HashMap<(usize, usize), PivotMeasure>,
    pub column_rollups: HashMap<(usize, usize), PivotMeasure>,
    pub grand_rollups: Vec<Option<PivotMeasure>>,
    /// For each leaf tuple, the interned parent ID at every shallower depth.
    pub row_parents: Vec<Vec<usize>>,
    pub column_parents: Vec<Vec<usize>>,
    /// Source-order non-SUM folds for parent intersections. Depth zero is a
    /// grand axis, full depth is a leaf, and an interior depth is a parent.
    pub parent_rollups: HashMap<(usize, usize, usize, usize, usize), PivotMeasure>,
}

impl PivotComputed {
    /// Walk the source rectangle once, resolving header names only in `bound`.
    /// Each value field chooses one shared accumulator mode before the row walk.
    pub fn build(spec: &PivotSpec, bound: &BoundSource, sheet: &Sheet) -> Result<Self> {
        let system = sheet.date_system();
        let mut row_items: Vec<_> = bound.rows.iter().map(|_| PivotItems::new(system)).collect();
        let mut column_items: Vec<_> = bound
            .columns
            .iter()
            .map(|_| PivotItems::new(system))
            .collect();
        let mut row_tuples = Vec::<Vec<usize>>::new();
        let mut column_tuples = Vec::<Vec<usize>>::new();
        let mut row_ids = HashMap::<Vec<usize>, usize>::new();
        let mut column_ids = HashMap::<Vec<usize>, usize>::new();
        let needs_parent_rollups = spec.subtotals
            && spec.values.iter().any(|value| value.aggregate != Aggregate::Sum)
            && (bound.rows.len() > 1 || bound.columns.len() > 1);
        let mut row_prefixes: Vec<HashMap<Vec<usize>, usize>> = if needs_parent_rollups {
            (1..bound.rows.len()).map(|_| HashMap::new()).collect()
        } else { Vec::new() };
        let mut column_prefixes: Vec<HashMap<Vec<usize>, usize>> = if needs_parent_rollups {
            (1..bound.columns.len()).map(|_| HashMap::new()).collect()
        } else { Vec::new() };
        let mut row_parents = Vec::<Vec<usize>>::new();
        let mut column_parents = Vec::<Vec<usize>>::new();
        let mut row_key = Vec::with_capacity(bound.rows.len());
        let mut column_key = Vec::with_capacity(bound.columns.len());
        let mut row_scopes = if needs_parent_rollups {
            Vec::with_capacity(bound.rows.len() + 1)
        } else { Vec::new() };
        let mut column_scopes = if needs_parent_rollups {
            Vec::with_capacity(bound.columns.len() + 1)
        } else { Vec::new() };
        // Only an emitted grand-axis event consumes the corresponding
        // source-order rollup. An unrequested variance total must not turn a
        // settled leaf into an unrelated NumericPolicy refusal.
        let needed_row_rollups = !spec.columns.is_empty() && spec.row_grand_totals;
        let needed_column_rollups = !spec.columns.is_empty() && spec.column_grand_totals;
        let needed_grand_rollups = spec.column_grand_totals
            && (spec.columns.is_empty() || spec.row_grand_totals);
        let mut groups = HashMap::<(usize, usize, usize), Group>::new();
        let mut row_rollups = HashMap::<(usize, usize), Group>::new();
        let mut column_rollups = HashMap::<(usize, usize), Group>::new();
        let mut parent_rollups = HashMap::<(usize, usize, usize, usize, usize), Group>::new();
        let mut grand_rollups: Vec<Group> = spec
            .values
            .iter()
            .map(|field| Group::new(field.aggregate))
            .collect();
        let start = spec.source.range.start().row() + 1;
        let end = spec.source.range.end().row();
        let mut physical = sheet.rows().peekable();
        for row_index in start..=end {
            while physical.peek().is_some_and(|row| row.index() < row_index) {
                physical.next();
            }
            let row = if physical.peek().is_some_and(|row| row.index() == row_index) {
                physical.next()
            } else {
                None
            };
            row_key.clear();
            for (items, &column) in row_items.iter_mut().zip(&bound.rows) {
                let at = CellRef::new(row_index, column);
                row_key.push(items.intern(
                    row.and_then(|row| row.cell(column)),
                    sheet.retained_serial(at),
                    at,
                )?);
            }
            column_key.clear();
            for (items, &column) in column_items.iter_mut().zip(&bound.columns) {
                let at = CellRef::new(row_index, column);
                column_key.push(items.intern(
                    row.and_then(|row| row.cell(column)),
                    sheet.retained_serial(at),
                    at,
                )?);
            }
            let previous_rows = row_tuples.len();
            let row_id = if let Some(&id) = row_ids.get(row_key.as_slice()) {
                id
            } else {
                let id = row_tuples.len();
                if needs_parent_rollups {
                    let parents = row_prefixes.iter_mut().enumerate().map(|(level, prefixes)| {
                        let prefix = &row_key[..=level];
                        if let Some(&id) = prefixes.get(prefix) { id } else {
                            let id = prefixes.len();
                            prefixes.insert(prefix.to_vec(), id);
                            id
                        }
                    }).collect();
                    row_parents.push(parents);
                }
                let tuple = row_key.clone();
                row_ids.insert(tuple.clone(), id);
                row_tuples.push(tuple);
                id
            };
            let previous_columns = column_tuples.len();
            let column_id = if let Some(&id) = column_ids.get(column_key.as_slice()) {
                id
            } else {
                let id = column_tuples.len();
                if needs_parent_rollups {
                    let parents = column_prefixes.iter_mut().enumerate().map(|(level, prefixes)| {
                        let prefix = &column_key[..=level];
                        if let Some(&id) = prefixes.get(prefix) { id } else {
                            let id = prefixes.len();
                            prefixes.insert(prefix.to_vec(), id);
                            id
                        }
                    }).collect();
                    column_parents.push(parents);
                }
                let tuple = column_key.clone();
                column_ids.insert(tuple.clone(), id);
                column_tuples.push(tuple);
                id
            };
            // Distinct leaf pairs are a lower bound on rendered geometry.
            // Subtotal/grand events can only enlarge it.
            if row_tuples.len() != previous_rows || column_tuples.len() != previous_columns {
                let rows = u32::try_from(row_tuples.len()).map_err(|_| Error::InvalidRecord {
                    path: "$.pivot.rows".into(), reason: "expected bounded row items".into(),
                })?;
                let columns = u32::try_from(column_tuples.len()).map_err(|_| Error::InvalidRecord {
                    path: "$.pivot.columns".into(), reason: "expected bounded column items".into(),
                })?;
                super::layout::geometry(spec, CellRef::new(0, 0), rows, columns)?;
            }
            if needs_parent_rollups {
                row_scopes.clear();
                row_scopes.push((bound.rows.len(), row_id));
                row_scopes.extend(row_parents[row_id].iter().enumerate()
                    .map(|(level, &id)| (level + 1, id)));
                if spec.column_grand_totals && !bound.rows.is_empty() {
                    row_scopes.push((0, 0));
                }
                column_scopes.clear();
                column_scopes.push((bound.columns.len(), column_id));
                column_scopes.extend(column_parents[column_id].iter().enumerate()
                    .map(|(level, &id)| (level + 1, id)));
                if spec.row_grand_totals && !spec.columns.is_empty() {
                    column_scopes.push((0, 0));
                }
            }
            for (value_index, &column) in bound.values.iter().enumerate() {
                let at = CellRef::new(row_index, column);
                let input = Group::input(
                    row.and_then(|row| row.cell(column)),
                    sheet.retained_serial(at),
                    system,
                    at,
                )?;
                let aggregate = spec.values[value_index].aggregate;
                groups
                    .entry((row_id, column_id, value_index))
                    .or_insert_with(|| Group::new(aggregate))
                    .push(input);
                if needs_parent_rollups && aggregate != Aggregate::Sum {
                    for &(row_depth, row_group) in &row_scopes {
                        for &(column_depth, column_group) in &column_scopes {
                            let row_parent = row_depth > 0 && row_depth < bound.rows.len();
                            let column_parent = column_depth > 0 && column_depth < bound.columns.len();
                            if row_parent || column_parent {
                                parent_rollups.entry((row_depth, row_group, column_depth,
                                    column_group, value_index))
                                    .or_insert_with(|| Group::new(aggregate)).push(input);
                            }
                        }
                    }
                }
                if aggregate != Aggregate::Sum {
                    if needed_row_rollups {
                        row_rollups
                            .entry((row_id, value_index))
                            .or_insert_with(|| Group::new(aggregate))
                            .push(input);
                    }
                    if needed_column_rollups {
                        column_rollups
                            .entry((column_id, value_index))
                            .or_insert_with(|| Group::new(aggregate))
                            .push(input);
                    }
                    if needed_grand_rollups {
                        grand_rollups[value_index].push(input);
                    }
                }
            }
        }
        let mut values = HashMap::with_capacity(groups.len());
        for (key, group) in groups {
            let path = format_smolstr!("$.pivot.groups[{},{},{}]", key.0, key.1, key.2);
            values.insert(key, group.finish(path)?);
        }
        let mut settled_rows = HashMap::with_capacity(row_rollups.len());
        for (key, group) in row_rollups {
            settled_rows.insert(
                key,
                group.finish(format_smolstr!("$.pivot.rowTotals[{},{}]", key.0, key.1))?,
            );
        }
        let mut settled_columns = HashMap::with_capacity(column_rollups.len());
        for (key, group) in column_rollups {
            settled_columns.insert(
                key,
                group.finish(format_smolstr!("$.pivot.columnTotals[{},{}]", key.0, key.1))?,
            );
        }
        let mut settled_grand = Vec::with_capacity(grand_rollups.len());
        for (index, group) in grand_rollups.into_iter().enumerate() {
            settled_grand.push(
                (needed_grand_rollups && spec.values[index].aggregate != Aggregate::Sum)
                    .then(|| group.finish(format_smolstr!("$.pivot.grandTotal[{index}]")))
                    .transpose()?,
            );
        }
        let mut settled_parents = HashMap::with_capacity(parent_rollups.len());
        for (key, group) in parent_rollups {
            settled_parents.insert(key, group.finish(format_smolstr!(
                "$.pivot.parentTotals[{},{},{},{},{}]", key.0, key.1, key.2, key.3, key.4
            ))?);
        }
        Ok(Self {
            row_items,
            column_items,
            row_tuples,
            column_tuples,
            values,
            row_rollups: settled_rows,
            column_rollups: settled_columns,
            grand_rollups: settled_grand,
            row_parents,
            column_parents,
            parent_rollups: settled_parents,
        })
    }

    /// Reconcile the observed SUM grand from completed row groups in cache
    /// insertion order. Layout must first prove its visible order agrees with
    /// this order, or perform its own visible-order fold over `values`.
    pub fn sum_grand_from_rows(&self, value_index: usize) -> Result<Option<f64>> {
        let mut total = Accumulator::default();
        for row_id in 0..self.row_tuples.len() {
            let mut row = Accumulator::default();
            for column_id in 0..self.column_tuples.len() {
                match self.values.get(&(row_id, column_id, value_index)) {
                    Some(PivotMeasure::Number(value)) => {
                        row.push_number(*value).map_err(|error| {
                            invalid("$.pivot.total", "a finite numeric fold", error)
                        })?
                    }
                    Some(PivotMeasure::Error(error)) => {
                        return Err(invalid("$.pivot.total", "a numeric row group", error));
                    }
                    Some(PivotMeasure::SourceError(error)) => {
                        return Err(invalid("$.pivot.total", "a source-error row group", error));
                    }
                    Some(PivotMeasure::Empty) => {}
                    None => {}
                }
            }
            let Some(value) = row
                .finish_sum()
                .map_err(|error| invalid("$.pivot.total", "a finite row total", error))?
            else {
                return Err(invalid(
                    "$.pivot.total",
                    "a settled row total",
                    "uncertain binary64 fold",
                ));
            };
            total
                .push_number(value)
                .map_err(|error| invalid("$.pivot.total", "a finite grand fold", error))?;
        }
        total
            .finish_sum()
            .map_err(|error| invalid("$.pivot.grandTotal", "a finite grand total", error))?
            .map(Some)
            .ok_or_else(|| {
                invalid(
                    "$.pivot.grandTotal",
                    "a settled numeric grand total",
                    "uncertain binary64 fold",
                )
            })
    }
}

/// Scan one selected physical source rectangle using the same header and
/// dictionary owners as PivotComputed. No PivotTable or package part is made.
pub(crate) fn field_info(source: &PivotSource, sheet: &Sheet) -> Result<Vec<PivotFieldInfo>> {
    let (labels, _) = source_headers(source, sheet)?;
    let system = sheet.date_system();
    let mut items: Vec<_> = labels.iter().map(|_| PivotItems::new(system)).collect();
    let mut numeric = vec![true; labels.len()];
    let range = source.range;
    let mut physical = sheet.rows().peekable();
    for row_index in range.start().row() + 1..=range.end().row() {
        while physical.peek().is_some_and(|row| row.index() < row_index) {
            physical.next();
        }
        let row = if physical.peek().is_some_and(|row| row.index() == row_index) {
            physical.next()
        } else {
            None
        };
        for (offset, dictionary) in items.iter_mut().enumerate() {
            let column = range.start().column() + offset as u32;
            let at = CellRef::new(row_index, column);
            let cell = row.and_then(|row| row.cell(column));
            let raw = sheet.retained_serial(at);
            numeric[offset] &= matches!(
                Group::input(cell, raw, system, at)?,
                ValueInput::Absent | ValueInput::Number(_)
            );
            dictionary.intern(
                cell,
                raw,
                at,
            )?;
        }
    }
    labels.into_iter().zip(items).zip(numeric).map(|((name, items), numeric)| {
        Ok(PivotFieldInfo {
            name,
            numeric,
            aggregate: if numeric { Aggregate::Sum } else { Aggregate::Count },
            items: u64::try_from(items.items().len())
                .map_err(|_| invalid("$.source.range", "a bounded item count", items.items().len()))?,
        })
    }).collect()
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! Source-header binding controls for the one pivot compute owner.

    pub use super::BoundSource;
    pub use super::{PivotComputed, PivotItem, PivotItems, PivotMeasure};
}
