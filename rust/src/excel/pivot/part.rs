//! OOXML pivot parts over the one resolved display plan. Parent subtotals
//! consume source-order groups; unresolved locale-specific labels still refuse.

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result};

use super::super::formula::aggregate::Aggregate;
use super::compute::{BoundSource, PivotComputed, PivotItem, PivotMeasure};
use super::layout::{Geometry, PivotDisplay, PivotEvent};
use super::{BLANK_CAPTION, PivotSpec};
use crate::Scalar;
use crate::excel::NumberFormat;
use crate::excel::cell::serial_text;
use crate::excel::package::escape_attribute;
use crate::excel::shared_strings::encode;
use crate::excel::{Cell, CellRef, DateSystem};

pub(crate) const HIDE_VALUES_URI: &str = "{962EF5D1-5CA2-4c93-8EF4-DBF5C05439D2}";
pub(crate) const HIDE_VALUES_NAMESPACE: &str =
    "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";

/// Three semantic XML members; OPC relationships/overrides stay with package.
pub(crate) struct PivotPartBytes {
    pub table: Vec<u8>,
    pub cache: Vec<u8>,
    pub records: Vec<u8>,
}

fn refusal(path: &str, reason: impl Into<SmolStr>) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new(path),
        reason: reason.into(),
    }
}

fn pivot_label(path: &str, label: &str) -> Result<()> {
    // ECMA-376 pivot captions and names are bounded to 255 characters.
    let length = label.chars().count();
    if length > 255 {
        return Err(refusal(
            path,
            format_smolstr!("expected at most 255 characters, got {length}"),
        ));
    }
    Ok(())
}

const VALUES_CAPTION: &str = "Values";
const GRAND_CAPTION: &str = "Grand Total";

/// Resolved once from a pivot's imported root; both XML and visible cells use it.
pub(crate) struct PivotCaptions {
    values: SmolStr,
    grand: SmolStr,
}

impl PivotCaptions {
    pub(crate) fn new(values: Option<String>, grand: Option<String>) -> Result<Self> {
        let values = values.unwrap_or_else(|| VALUES_CAPTION.to_owned());
        let grand = grand.unwrap_or_else(|| GRAND_CAPTION.to_owned());
        pivot_label("$.pivot.dataCaption", &values)?;
        pivot_label("$.pivot.grandTotalCaption", &grand)?;
        Ok(Self {
            values: values.into(),
            grand: grand.into(),
        })
    }
}
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const RELS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn shared_items(
    items: &super::compute::PivotItems,
    facts: super::compute::PivotItemFacts,
) -> Result<String> {
    let mut xml = format!("<sharedItems count=\"{}\"", items.items().len());
    if facts.blank {
        xml.push_str(" containsBlank=\"1\"");
    }
    if facts.date {
        // Do not mix date attributes with numeric bound attributes.
        if !facts.blank {
            xml.push_str(" containsSemiMixedTypes=\"0\"");
        }
        if !facts.mixed {
            xml.push_str(" containsNonDate=\"0\"");
        }
        xml.push_str(" containsDate=\"1\"");
        xml.push_str(if facts.string {
            " containsString=\"1\""
        } else {
            " containsString=\"0\""
        });
    }
    if facts.mixed {
        xml.push_str(" containsMixedTypes=\"1\"");
    }
    if facts.number && !facts.date {
        if !facts.mixed {
            xml.push_str(" containsSemiMixedTypes=\"0\" containsString=\"0\"");
        }
        xml.push_str(" containsNumber=\"1\"");
        if facts.integer {
            xml.push_str(" containsInteger=\"1\"");
        }
        if let (Some(min), Some(max)) = (facts.min_number, facts.max_number) {
            xml.push_str(&format!(
                " minValue=\"{}\" maxValue=\"{}\"",
                serial_text(min),
                serial_text(max)
            ));
        }
    }
    // The writer authors no fieldGroup. Date bounds are optional OOXML
    // metadata, and native bounds do not equal min/max cache <d> items.
    xml.push('>');
    for (index, item) in items.items().iter().enumerate() {
        match item {
            PivotItem::Text(value) => {
                xml.push_str(&format!("<s v=\"{}\"/>", escape_attribute(&encode(value))))
            }
            PivotItem::Number(value) => {
                xml.push_str(&format!("<n v=\"{}\"/>", serial_text(*value)))
            }
            PivotItem::Date(_) => xml.push_str(&format!("<d v=\"{}\"/>", items.cache_date(index)?)),
            PivotItem::Blank => xml.push_str("<m/>"),
            PivotItem::Boolean(value) => xml.push_str(if *value {
                "<b v=\"1\"/>"
            } else {
                "<b v=\"0\"/>"
            }),
            PivotItem::Error(value) => xml.push_str(&format!("<e v=\"{}\"/>", value.as_str())),
        }
    }
    xml.push_str("</sharedItems>");
    Ok(xml)
}

/// Render the resolved one/two-row, zero/one-column tabular matrix.
/// `PivotDisplay` is the sole cache-id to visible-ordinal map for XML/cells.
pub(crate) fn render(
    spec: &PivotSpec,
    bound: &BoundSource,
    computed: &PivotComputed,
    display: &PivotDisplay,
    geometry: Geometry,
    cache_id: u32,
    captions: &PivotCaptions,
    formats: &[Option<u32>],
) -> Result<PivotPartBytes> {
    pivot_label("$.name", &spec.name)?;
    pivot_label("$.pivot.blankCaption", BLANK_CAPTION)?;
    let expected_height = geometry.header_rows + display.row_events.len() as u32;
    let groups = if spec.columns.is_empty() {
        1
    } else {
        display.column_events.len() as u32
    };
    let expected_width = spec.rows.len() as u32 + groups * spec.values.len() as u32;
    if geometry.height != expected_height || geometry.width != expected_width {
        return Err(refusal(
            "$.pivot.location",
            format_smolstr!(
                "expected {expected_width} by {expected_height}, got {} by {}",
                geometry.width,
                geometry.height
            ),
        ));
    }
    let first = spec.source.range.start().column();
    let row_cols: Vec<u32> = bound.rows.iter().map(|&column| column - first).collect();
    let column_cols: Vec<u32> = bound.columns.iter().map(|&column| column - first).collect();
    let mut cache = format!(
        "<pivotCacheDefinition xmlns=\"{MAIN}\" xmlns:r=\"{RELS}\" r:id=\"rId1\" saveData=\"0\" refreshOnLoad=\"1\" createdVersion=\"6\" refreshedVersion=\"6\" minRefreshableVersion=\"3\" recordCount=\"0\"><cacheSource type=\"worksheet\"><worksheetSource ref=\"{}\" sheet=\"{}\"/></cacheSource><cacheFields count=\"{}\">",
        spec.source.range,
        escape_attribute(&encode(&spec.source.sheet)),
        bound.headers.len()
    );
    for (index, name) in bound.headers.iter().enumerate() {
        let column = u32::try_from(index)
            .map_err(|_| refusal("$.source.range", "too many source fields"))?;
        let items = row_cols
            .iter()
            .position(|&axis| axis == column)
            .map(|axis| &computed.row_items[axis])
            .or_else(|| {
                column_cols
                    .iter()
                    .position(|&axis| axis == column)
                    .map(|axis| &computed.column_items[axis])
            });
        let facts = items.map(|items| items.facts());
        let num_fmt_id = if facts.is_some_and(|facts| facts.date_only) {
            14
        } else {
            0
        };
        cache.push_str(&format!(
            "<cacheField name=\"{}\" numFmtId=\"{num_fmt_id}\">",
            escape_attribute(&encode(name))
        ));
        if let (Some(items), Some(facts)) = (items, facts) {
            cache.push_str(&shared_items(items, facts)?);
        } else {
            cache.push_str("<sharedItems/>");
        }
        cache.push_str("</cacheField>");
    }
    cache.push_str("</cacheFields></pivotCacheDefinition>");

    let grid_drop_zones = u8::from(!spec.columns.is_empty());
    let mut table = format!(
        "<pivotTableDefinition xmlns=\"{MAIN}\" name=\"{}\" cacheId=\"{cache_id}\" applyNumberFormats=\"0\" applyBorderFormats=\"0\" applyFontFormats=\"0\" applyPatternFormats=\"0\" applyAlignmentFormats=\"0\" applyWidthHeightFormats=\"1\" dataCaption=\"{}\" grandTotalCaption=\"{}\" updatedVersion=\"6\" minRefreshableVersion=\"3\" createdVersion=\"6\" useAutoFormatting=\"1\" itemPrintTitles=\"1\" indent=\"0\" compact=\"0\" compactData=\"0\" outline=\"0\" outlineData=\"0\" gridDropZones=\"{grid_drop_zones}\" multipleFieldFilters=\"0\" rowGrandTotals=\"{}\" colGrandTotals=\"{}\"><location ref=\"{}\" firstHeaderRow=\"{}\" firstDataRow=\"{}\" firstDataCol=\"{}\"/><pivotFields count=\"{}\">",
        escape_attribute(&encode(&spec.name)),
        escape_attribute(&encode(&captions.values)),
        escape_attribute(&encode(&captions.grand)),
        u8::from(spec.row_grand_totals),
        u8::from(spec.column_grand_totals),
        geometry.range,
        geometry.first_header_row,
        geometry.first_data_row,
        geometry.first_data_col,
        bound.headers.len()
    );
    for index in 0..bound.headers.len() {
        let column = u32::try_from(index)
            .map_err(|_| refusal("$.source.range", "too many source fields"))?;
        let axis = if let Some(row) = row_cols.iter().position(|&axis| axis == column) {
            Some((
                "axisRow",
                &display.row_fields[row],
                spec.rows[row].order,
                computed.row_items[row].items(),
            ))
        } else if let Some(axis) = column_cols.iter().position(|&axis| axis == column) {
            Some((
                "axisCol",
                &display.column_fields[axis],
                spec.columns[axis].order,
                computed.column_items[axis].items(),
            ))
        } else {
            None
        };
        if let Some((axis, visible, order, items)) = axis {
            table.push_str(&format!("<pivotField axis=\"{axis}\" compact=\"0\" outline=\"0\" showAll=\"0\" sortType=\"{}\"><items count=\"{}\">",
                order.as_str(), visible.len()+1));
            for &cache_index in visible {
                if matches!(items[cache_index], PivotItem::Boolean(_) | PivotItem::Blank) {
                    let label = item_caption(&items[cache_index], "$.pivot.itemCaption")?;
                    table.push_str(&format!(
                        "<item x=\"{cache_index}\" n=\"{}\"/>",
                        escape_attribute(&label)
                    ));
                } else if let Some(label) = items[cache_index].authored_error_label() {
                    table.push_str(&format!(
                        "<item x=\"{cache_index}\" n=\"{}\"/>",
                        escape_attribute(label)
                    ));
                } else {
                    table.push_str(&format!("<item x=\"{cache_index}\"/>"));
                }
            }
            table.push_str("<item t=\"default\"/></items></pivotField>");
        } else if bound.values.contains(&column) {
            table.push_str(
                "<pivotField dataField=\"1\" compact=\"0\" outline=\"0\" showAll=\"0\"/>",
            );
        } else {
            table.push_str("<pivotField compact=\"0\" outline=\"0\" showAll=\"0\"/>");
        }
    }
    table.push_str(&format!(
        "</pivotFields><rowFields count=\"{}\">",
        row_cols.len()
    ));
    for field in &row_cols {
        table.push_str(&format!("<field x=\"{field}\"/>"));
    }
    table.push_str(&format!(
        "</rowFields><rowItems count=\"{}\">",
        display.row_events.len()
    ));
    for event in &display.row_events {
        match *event {
            PivotEvent::Leaf { id, first_new } => {
                if first_new == 0 {
                    table.push_str("<i>");
                } else {
                    table.push_str(&format!("<i r=\"{first_new}\">"));
                }
                for field in first_new..spec.rows.len() {
                    let rank = display.row_rank[field][computed.row_tuples[id][field]];
                    if rank == 0 {
                        table.push_str("<x/>");
                    } else {
                        table.push_str(&format!("<x v=\"{rank}\"/>"));
                    }
                }
                table.push_str("</i>");
            }
            PivotEvent::Subtotal { level, from, .. } => {
                let id = display.rows[from];
                table.push_str("<i t=\"default\">");
                for field in 0..=level {
                    let rank = display.row_rank[field][computed.row_tuples[id][field]];
                    if rank == 0 {
                        table.push_str("<x/>");
                    } else {
                        table.push_str(&format!("<x v=\"{rank}\"/>"));
                    }
                }
                table.push_str("</i>");
            }
            PivotEvent::Grand => table.push_str("<i t=\"grand\"><x/></i>"),
        }
    }
    table.push_str("</rowItems>");
    if spec.columns.is_empty() {
        if spec.values.len() == 1 {
            table.push_str("<colItems count=\"1\"><i/></colItems>");
        } else {
            table.push_str(&format!(
                "<colFields count=\"1\"><field x=\"-2\"/></colFields><colItems count=\"{}\">",
                spec.values.len()
            ));
            for value in 0..spec.values.len() {
                if value == 0 {
                    table.push_str("<i><x/></i>");
                } else {
                    table.push_str(&format!("<i i=\"{value}\"><x v=\"{value}\"/></i>"));
                }
            }
            table.push_str("</colItems>");
        }
    } else {
        let fields = spec.columns.len();
        let values = spec.values.len();
        table.push_str(&format!(
            "<colFields count=\"{}\">",
            fields + usize::from(values > 1)
        ));
        for field in &column_cols {
            table.push_str(&format!("<field x=\"{field}\"/>"));
        }
        if values > 1 {
            table.push_str("<field x=\"-2\"/>");
        }
        table.push_str(&format!(
            "</colFields><colItems count=\"{}\">",
            display.column_events.len() * values
        ));
        for event in &display.column_events {
            for value in 0..values {
                match *event {
                    PivotEvent::Leaf { id, first_new } => {
                        if value == 0 {
                            if first_new == 0 {
                                table.push_str("<i>");
                            } else {
                                table.push_str(&format!("<i r=\"{first_new}\">"));
                            }
                            for field in first_new..fields {
                                let rank =
                                    display.column_rank[field][computed.column_tuples[id][field]];
                                if rank == 0 {
                                    table.push_str("<x/>");
                                } else {
                                    table.push_str(&format!("<x v=\"{rank}\"/>"));
                                }
                            }
                            if values > 1 {
                                table.push_str("<x/>");
                            }
                            table.push_str("</i>");
                        } else {
                            table.push_str(&format!(
                                "<i r=\"{fields}\" i=\"{value}\"><x v=\"{value}\"/></i>"
                            ));
                        }
                    }
                    PivotEvent::Subtotal { level, from, .. } => {
                        let id = display.columns[from];
                        if value == 0 {
                            table.push_str("<i t=\"default\">");
                        } else {
                            table.push_str(&format!("<i t=\"default\" i=\"{value}\">"));
                        }
                        for field in 0..=level {
                            let rank =
                                display.column_rank[field][computed.column_tuples[id][field]];
                            if rank == 0 {
                                table.push_str("<x/>");
                            } else {
                                table.push_str(&format!("<x v=\"{rank}\"/>"));
                            }
                        }
                        table.push_str("</i>");
                    }
                    PivotEvent::Grand => {
                        if value == 0 {
                            table.push_str("<i t=\"grand\"><x/></i>");
                        } else {
                            table.push_str(&format!("<i t=\"grand\" i=\"{value}\"><x/></i>"));
                        }
                    }
                }
            }
        }
        table.push_str("</colItems>");
    }
    table.push_str(&format!("<dataFields count=\"{}\">", spec.values.len()));
    for (index, value) in spec.values.iter().enumerate() {
        let source = bound.values[index] - first;
        let caption = value.caption.as_ref().map_or_else(
            || default_caption(value.aggregate, &bound.headers[source as usize]).to_string(),
            ToString::to_string,
        );
        pivot_label(&format!("$.values[{index}].caption"), &caption)?;
        let subtotal = if value.aggregate == Aggregate::Sum {
            String::new()
        } else {
            format!(" subtotal=\"{}\"", value.aggregate.subtotal())
        };
        table.push_str(&format!(
            "<dataField name=\"{}\" fld=\"{source}\"{subtotal}",
            escape_attribute(&encode(&caption))
        ));
        if let Some(id) = formats[index] {
            table.push_str(&format!(" numFmtId=\"{id}\""));
        }
        table.push_str(" baseField=\"0\" baseItem=\"0\"/>");
    }
    table.push_str("</dataFields><pivotTableStyleInfo name=\"PivotStyleLight16\" showRowHeaders=\"1\" showColHeaders=\"1\" showRowStripes=\"0\" showColStripes=\"0\" showLastColumn=\"1\"/>");
    if spec.columns.is_empty() {
        table.push_str(&format!("<extLst><ext uri=\"{HIDE_VALUES_URI}\" xmlns:x14=\"{HIDE_VALUES_NAMESPACE}\"><x14:pivotTableDefinition hideValuesRow=\"1\" xmlns:xm=\"http://schemas.microsoft.com/office/excel/2006/main\"/></ext></extLst>"));
    }
    table.push_str("</pivotTableDefinition>");
    Ok(PivotPartBytes {
        table: table.into_bytes(),
        cache: cache.into_bytes(),
        records: format!("<pivotCacheRecords xmlns=\"{MAIN}\" xmlns:r=\"{RELS}\" count=\"0\"/>")
            .into_bytes(),
    })
}

fn default_caption(aggregate: Aggregate, field: &str) -> SmolStr {
    let name = aggregate.as_str();
    let mut chars = name.chars();
    format_smolstr!(
        "{}{} of {field}",
        chars.next().unwrap().to_ascii_uppercase(),
        chars.as_str()
    )
}

fn caption(spec: &PivotSpec, bound: &BoundSource, index: usize) -> SmolStr {
    spec.values[index].caption.clone().unwrap_or_else(|| {
        let source = bound.values[index] - spec.source.range.start().column();
        default_caption(
            spec.values[index].aggregate,
            &bound.headers[source as usize],
        )
    })
}

fn axis_label(item: &PivotItem, at: &str) -> Result<Scalar> {
    match item {
        PivotItem::Text(text) => Ok(Scalar::from(text.clone())),
        PivotItem::Blank => Ok(Scalar::from(BLANK_CAPTION)),
        PivotItem::Number(value) => Ok(Scalar::from(*value)),
        PivotItem::Boolean(_) => Ok(Scalar::from(item_caption(item, at)?)),
        PivotItem::Error(_) => item
            .authored_error_label()
            .map(Scalar::from)
            .ok_or_else(|| refusal(at, "an authored classic error heading")),
        _ => Err(refusal(
            at,
            "a text, blank or numeric display item in this writer phase",
        )),
    }
}

fn numeric(measure: Option<&PivotMeasure>, path: &str) -> Result<Option<f64>> {
    match measure {
        Some(PivotMeasure::Number(value)) => Ok(Some(*value)),
        Some(PivotMeasure::SourceError(error)) => Err(refusal(
            path,
            format_smolstr!("expected source-error preflight, got {error:?}"),
        )),
        Some(PivotMeasure::Error(error)) => Err(refusal(
            path,
            format_smolstr!("expected numeric pivot result, got {error:?}"),
        )),
        Some(PivotMeasure::Empty) => Ok(None),
        None => Ok(None),
    }
}

// Traverse the visible matrix once for each displayed total. A source error
// outranks a derived empty-domain result; row/column order is the displayed
// order proved by PivotDisplay, not the source insertion order.
fn first_source_error(
    computed: &PivotComputed,
    rows: impl Iterator<Item = usize> + Clone,
    columns: impl Iterator<Item = usize> + Clone,
    value: usize,
) -> Option<PivotMeasure> {
    for row in rows {
        for column in columns.clone() {
            if let Some(PivotMeasure::SourceError(error)) =
                computed.values.get(&(row, column, value))
            {
                return Some(PivotMeasure::SourceError(*error));
            }
        }
    }
    None
}

fn sum_groups(
    computed: &PivotComputed,
    rows: impl Iterator<Item = usize> + Clone,
    columns: impl Iterator<Item = usize> + Clone,
    value: usize,
    path: &str,
) -> Result<Option<f64>> {
    let mut sum = super::super::formula::aggregate::Accumulator::default();
    let mut seen = false;
    for row in rows {
        for column in columns.clone() {
            if let Some(number) = numeric(computed.values.get(&(row, column, value)), path)? {
                sum.push_number(number).map_err(|error| {
                    refusal(path, format_smolstr!("expected finite fold, got {error:?}"))
                })?;
                seen = true;
            }
        }
    }
    if !seen {
        return Ok(None);
    }
    sum.finish_sum()
        .map_err(|error| {
            refusal(
                path,
                format_smolstr!("expected finite total, got {error:?}"),
            )
        })?
        .map(Some)
        .ok_or_else(|| {
            refusal(
                path,
                "expected a settled numeric total, got uncertain binary64 fold",
            )
        })
}

fn total(
    spec: &PivotSpec,
    computed: &PivotComputed,
    display: &PivotDisplay,
    row: Option<usize>,
    column: Option<usize>,
    value: usize,
) -> Result<Option<PivotMeasure>> {
    let errors = match (row, column) {
        (Some(_), Some(_)) => None,
        (Some(row), None) => first_source_error(
            computed,
            std::iter::once(row),
            display.columns.iter().copied(),
            value,
        ),
        (None, Some(column)) => first_source_error(
            computed,
            display.rows.iter().copied(),
            std::iter::once(column),
            value,
        ),
        (None, None) => first_source_error(
            computed,
            display.rows.iter().copied(),
            display.columns.iter().copied(),
            value,
        ),
    };
    if errors.is_some() {
        return Ok(errors);
    }
    if spec.values[value].aggregate != Aggregate::Sum {
        return match (row, column) {
            (Some(row), Some(column)) => Ok(computed.values.get(&(row, column, value)).copied()),
            (Some(row), None) => Ok(computed.row_rollups.get(&(row, value)).copied()),
            (None, Some(column)) => Ok(computed.column_rollups.get(&(column, value)).copied()),
            (None, None) => Ok(computed.grand_rollups[value]),
        };
    }
    match (row, column) {
        (Some(row), Some(column)) => Ok(computed.values.get(&(row, column, value)).copied()),
        (Some(row), None) => sum_groups(
            computed,
            std::iter::once(row),
            display.columns.iter().copied(),
            value,
            "$.pivot.rowTotal",
        )
        .map(|value| value.map(PivotMeasure::Number)),
        (None, Some(column)) => sum_groups(
            computed,
            display.rows.iter().copied(),
            std::iter::once(column),
            value,
            "$.pivot.columnTotal",
        )
        .map(|value| value.map(PivotMeasure::Number)),
        (None, None) => computed
            .sum_grand_from_rows(value)
            .map(|value| value.map(PivotMeasure::Number)),
    }
}

fn event_total(
    spec: &PivotSpec,
    computed: &PivotComputed,
    display: &PivotDisplay,
    row: PivotEvent,
    column: PivotEvent,
    value: usize,
) -> Result<Option<PivotMeasure>> {
    if !matches!(row, PivotEvent::Subtotal { .. }) && !matches!(column, PivotEvent::Subtotal { .. })
    {
        let row_id = match row {
            PivotEvent::Leaf { id, .. } => Some(id),
            _ => None,
        };
        let column_id = if spec.columns.is_empty() && row_id.is_none() {
            None
        } else {
            match column {
                PivotEvent::Leaf { id, .. } => Some(id),
                _ => None,
            }
        };
        return total(spec, computed, display, row_id, column_id, value);
    }
    let row_single;
    let rows: &[usize] = match row {
        PivotEvent::Leaf { id, .. } => {
            row_single = id;
            std::slice::from_ref(&row_single)
        }
        PivotEvent::Subtotal { from, to, .. } => &display.rows[from..to],
        PivotEvent::Grand => &display.rows,
    };
    let column_single;
    let columns: &[usize] = match column {
        PivotEvent::Leaf { id, .. } => {
            column_single = id;
            std::slice::from_ref(&column_single)
        }
        PivotEvent::Subtotal { from, to, .. } => &display.columns[from..to],
        PivotEvent::Grand => &display.columns,
    };
    if let Some(error) = first_source_error(
        computed,
        rows.iter().copied(),
        columns.iter().copied(),
        value,
    ) {
        return Ok(Some(error));
    }
    if spec.values[value].aggregate != Aggregate::Sum {
        let row_scope = match row {
            PivotEvent::Leaf { id, .. } => (spec.rows.len(), id),
            PivotEvent::Subtotal { level, from, .. } => {
                (level + 1, computed.row_parents[display.rows[from]][level])
            }
            PivotEvent::Grand => (0, 0),
        };
        let column_scope = match column {
            PivotEvent::Leaf { id, .. } => (spec.columns.len(), id),
            PivotEvent::Subtotal { level, from, .. } => (
                level + 1,
                computed.column_parents[display.columns[from]][level],
            ),
            PivotEvent::Grand => (0, 0),
        };
        return Ok(computed
            .parent_rollups
            .get(&(
                row_scope.0,
                row_scope.1,
                column_scope.0,
                column_scope.1,
                value,
            ))
            .copied());
    }
    sum_groups(
        computed,
        rows.iter().copied(),
        columns.iter().copied(),
        value,
        "$.pivot.subtotal",
    )
    .map(|number| number.map(PivotMeasure::Number))
}

pub(crate) fn item_caption(item: &PivotItem, path: &str) -> Result<SmolStr> {
    match item {
        PivotItem::Text(text) => Ok(text.clone()),
        PivotItem::Blank => Ok(BLANK_CAPTION.into()),
        PivotItem::Number(value) => Ok(serial_text(*value)),
        PivotItem::Boolean(value) => Ok(crate::excel::FormatCode::general()
            .render(&Scalar::from(*value), DateSystem::Year1900)
            .text),
        PivotItem::Date(_) | PivotItem::Error(_) => Err(refusal(
            path,
            "a native-proven locale-sensitive or temporal parent caption",
        )),
    }
}

/// Display cells plus only the temporal bits their typed values cannot restate.
/// The sheet's existing CellExtra owner installs these bits after insertion.
pub(crate) struct PivotDisplayCells {
    pub cells: Vec<Cell>,
    pub serials: Vec<(CellRef, u64)>,
}

fn put_axis(
    cells: &mut Vec<Cell>,
    serials: &mut Vec<(CellRef, u64)>,
    at: CellRef,
    item: &PivotItem,
    path: &str,
    system: DateSystem,
) -> Result<()> {
    let cell = match item {
        PivotItem::Date(serial) => {
            let value = system
                .scalar_from_serial(*serial, NumberFormat::Date)
                .map_err(|error| {
                    refusal(
                        path,
                        format_smolstr!("expected a representable date item: {error}"),
                    )
                })?;
            let cell = Cell::from_scalar(at, value, system)?;
            if let Some(bits) =
                super::super::sheet::CellExtra::exceptional_serial(&cell, *serial, system)
            {
                serials.push((at, bits));
            }
            cell
        }
        _ => Cell::from_scalar(at, axis_label(item, path)?, system)?,
    };
    cells.push(cell);
    Ok(())
}

/// Stage occupied cells from the same bounded row/column events used by XML.
pub(crate) fn display_cells(
    spec: &PivotSpec,
    bound: &BoundSource,
    computed: &PivotComputed,
    display: &PivotDisplay,
    captions: &PivotCaptions,
    geometry: Geometry,
    system: DateSystem,
) -> Result<PivotDisplayCells> {
    let at = geometry.range.start();
    let data_col = at.column() + spec.rows.len() as u32;
    let header = at.row();
    let data_start = header + geometry.header_rows;
    let mut cells = Vec::with_capacity(
        display.row_events.len()
            * (spec.rows.len() + display.column_events.len() * spec.values.len())
            + geometry.header_rows as usize * geometry.width as usize,
    );
    let mut serials = Vec::new();
    let put = |cells: &mut Vec<Cell>, row: u32, column: u32, scalar: Scalar| -> Result<()> {
        cells.push(Cell::from_scalar(
            CellRef::new(row, column),
            scalar,
            system,
        )?);
        Ok(())
    };
    let put_measure = |cells: &mut Vec<Cell>,
                       row: u32,
                       column: u32,
                       measure: Option<PivotMeasure>|
     -> Result<()> {
        let at = CellRef::new(row, column);
        match measure {
            Some(PivotMeasure::Number(number)) => {
                cells.push(Cell::from_scalar(at, Scalar::from(number), system)?)
            }
            Some(PivotMeasure::Error(error) | PivotMeasure::SourceError(error)) => {
                cells.push(Cell::from_scalar(at, Scalar::Null, system)?.with_error(error));
            }
            Some(PivotMeasure::Empty) | None => {}
        }
        Ok(())
    };
    if spec.columns.is_empty() {
        for (axis, &column) in bound.rows.iter().enumerate() {
            let label =
                bound.headers[(column - spec.source.range.start().column()) as usize].clone();
            put(
                &mut cells,
                header,
                at.column() + axis as u32,
                Scalar::from(label),
            )?;
        }
        for value in 0..spec.values.len() {
            put(
                &mut cells,
                header,
                data_col + value as u32,
                Scalar::from(caption(spec, bound, value)),
            )?;
        }
    } else {
        if spec.values.len() == 1 {
            put(
                &mut cells,
                header,
                at.column(),
                Scalar::from(caption(spec, bound, 0)),
            )?;
        }
        for (axis, &column) in bound.columns.iter().enumerate() {
            let label =
                bound.headers[(column - spec.source.range.start().column()) as usize].clone();
            put(
                &mut cells,
                header,
                data_col + axis as u32,
                Scalar::from(label),
            )?;
        }
        if spec.values.len() > 1 {
            put(
                &mut cells,
                header,
                data_col + spec.columns.len() as u32,
                Scalar::from(captions.values.clone()),
            )?;
        }
        for (axis, &column) in bound.rows.iter().enumerate() {
            let label =
                bound.headers[(column - spec.source.range.start().column()) as usize].clone();
            put(
                &mut cells,
                data_start - 1,
                at.column() + axis as u32,
                Scalar::from(label),
            )?;
        }
        for (position, &event) in display.column_events.iter().enumerate() {
            let first = data_col + (position * spec.values.len()) as u32;
            match event {
                PivotEvent::Leaf { id, first_new } => {
                    let tuple = &computed.column_tuples[id];
                    for axis in first_new..spec.columns.len() {
                        let item = &computed.column_items[axis].items()[tuple[axis]];
                        put_axis(
                            &mut cells,
                            &mut serials,
                            CellRef::new(header + 1 + axis as u32, first),
                            item,
                            &format!("$.columns[{axis}]"),
                            system,
                        )?;
                    }
                    if spec.values.len() > 1 {
                        for value in 0..spec.values.len() {
                            put(
                                &mut cells,
                                data_start - 1,
                                first + value as u32,
                                Scalar::from(caption(spec, bound, value)),
                            )?;
                        }
                    }
                }
                PivotEvent::Subtotal { level, from, .. } => {
                    let id = display.columns[from];
                    let tuple = &computed.column_tuples[id];
                    let item = &computed.column_items[level].items()[tuple[level]];
                    let label = item_caption(item, &format!("$.columns[{level}]"))?;
                    for value in 0..spec.values.len() {
                        let label = if spec.values.len() == 1 {
                            format_smolstr!("Total {label}")
                        } else {
                            format_smolstr!("{} {label}", caption(spec, bound, value))
                        };
                        put(
                            &mut cells,
                            header + 1 + level as u32,
                            first + value as u32,
                            Scalar::from(label),
                        )?;
                    }
                }
                PivotEvent::Grand => {
                    for value in 0..spec.values.len() {
                        let label = if spec.values.len() == 1 {
                            captions.grand.clone()
                        } else {
                            format_smolstr!("Total {}", caption(spec, bound, value))
                        };
                        put(
                            &mut cells,
                            header + 1,
                            first + value as u32,
                            Scalar::from(label),
                        )?;
                    }
                }
            }
        }
    }
    for (position, &row_event) in display.row_events.iter().enumerate() {
        let row = data_start + position as u32;
        match row_event {
            PivotEvent::Leaf { id, first_new } => {
                let tuple = &computed.row_tuples[id];
                for axis in first_new..spec.rows.len() {
                    let item = &computed.row_items[axis].items()[tuple[axis]];
                    put_axis(
                        &mut cells,
                        &mut serials,
                        CellRef::new(row, at.column() + axis as u32),
                        item,
                        &format!("$.rows[{axis}]"),
                        system,
                    )?;
                }
            }
            PivotEvent::Subtotal { level, from, .. } => {
                let id = display.rows[from];
                let tuple = &computed.row_tuples[id];
                let item = &computed.row_items[level].items()[tuple[level]];
                let label = item_caption(item, &format!("$.rows[{level}]"))?;
                put(
                    &mut cells,
                    row,
                    at.column() + level as u32,
                    Scalar::from(format_smolstr!("Total {label}")),
                )?;
            }
            PivotEvent::Grand => put(
                &mut cells,
                row,
                at.column(),
                Scalar::from(captions.grand.clone()),
            )?,
        }
        for (position, &column_event) in display.column_events.iter().enumerate() {
            for value in 0..spec.values.len() {
                let column = data_col + (position * spec.values.len() + value) as u32;
                put_measure(
                    &mut cells,
                    row,
                    column,
                    event_total(spec, computed, display, row_event, column_event, value)?,
                )?;
            }
        }
    }
    Ok(PivotDisplayCells { cells, serials })
}
