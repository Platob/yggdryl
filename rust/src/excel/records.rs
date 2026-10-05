//! Physical columns, header labels and pairing for Excel records.
//!
//! The record field is the semantic schema; this file only pairs it with
//! coordinates. Wire and held-cell decoding stay in their respective readers.

use std::borrow::Cow;

use smol_str::{SmolStr, format_smolstr};

use crate::{DataType, Error, Field, Result, Scalar, Serie, StructType};

use super::cell::{CellRange, CellRef, MAX_COLUMNS, MAX_ROWS};

/// The resolved flat header of a selected sheet or table.
pub(crate) struct Header {
    /// Sparse streamed headers carry their physical columns. A held Sheet
    /// supplies a dense start without constructing a second column vector.
    columns: PhysicalColumns,
    pub(super) names: Vec<SmolStr>,
    pub(super) from_header: Vec<bool>,
}

enum PhysicalColumns {
    Sparse(Vec<u32>),
    Dense { start: u32 },
}

/// Resolved selection facts borrowed for one field/column pairing.
pub(crate) struct FlatBinding<'a> {
    pub(super) sheet: &'a str,
    pub(super) range: CellRange,
    pub(super) by_name: bool,
    pub(super) authoritative: bool,
}

/// Whether selected columns are explicitly bounded, sparsely observed, or
/// dense in an already-held Sheet. No cell representation crosses this edge.
#[derive(Clone, Copy)]
pub(crate) enum FlatExtent<'a> {
    Table(Option<&'a [SmolStr]>),
    Sparse,
    Dense,
}

impl Header {
    /// Resolve decoded cell labels. The caller only decodes its own cell
    /// representation; names, fallback letters and extent live here.
    pub(super) fn resolve<'a>(
        range: CellRange,
        extent: FlatExtent<'_>,
        has_header: bool,
        cells: impl IntoIterator<Item = Result<(u32, Option<Cow<'a, str>>)>>,
    ) -> Result<Self> {
        if let FlatExtent::Table(table_names) = extent {
            let columns: Vec<u32> = (range.start().column()..=range.end().column()).collect();
            let names = match table_names {
                Some(names) => names.to_vec(),
                None => columns.iter().copied().map(CellRef::column_name).collect(),
            };
            // The table ref states its width even when every cell is absent.
            let from_header = vec![true; columns.len()];
            return Ok(Self {
                columns: PhysicalColumns::Sparse(columns),
                names,
                from_header,
            });
        }
        let dense = matches!(extent, FlatExtent::Dense);
        let mut columns = Vec::new();
        let mut names: Vec<SmolStr> = if dense {
            (range.start().column()..=range.end().column())
                .map(CellRef::column_name)
                .collect()
        } else {
            Vec::new()
        };
        let mut from_header = vec![false; names.len()];
        for cell in cells {
            let (column, label) = cell?;
            if !range.contains_column(column) {
                continue;
            }
            let at = if dense {
                (column - range.start().column()) as usize
            } else {
                columns.push(column);
                from_header.push(false);
                names.push(CellRef::column_name(column));
                names.len() - 1
            };
            if has_header {
                let text = label.as_deref().unwrap_or_default().trim();
                if !text.is_empty() {
                    names[at] = SmolStr::new(text);
                    from_header[at] = true;
                }
            }
        }
        Ok(Self {
            columns: if dense {
                PhysicalColumns::Dense {
                    start: range.start().column(),
                }
            } else {
                PhysicalColumns::Sparse(columns)
            },
            names,
            from_header,
        })
    }

    pub(super) fn len(&self) -> usize {
        self.names.len()
    }

    fn physical(&self, at: usize) -> u32 {
        match &self.columns {
            PhysicalColumns::Sparse(columns) => columns[at],
            PhysicalColumns::Dense { start } => start + at as u32,
        }
    }

    /// One pairing for both streamed and held record readers. No Field or
    /// label is cloned for a leaf; this only allocates the existing mapping.
    pub(super) fn pairing(
        &self,
        root: &Field,
        selection: FlatBinding<'_>,
    ) -> Result<Vec<Option<u32>>> {
        let mut pairing = Vec::with_capacity(root.fields().len());
        for (index, child) in root.fields().iter().enumerate() {
            let column = if selection.by_name {
                self.names
                    .iter()
                    .position(|name| name == child.name())
                    .map(|at| self.physical(at))
                    .or_else(|| {
                        if selection.authoritative {
                            return None;
                        }
                        CellRef::column_index(child.name())
                            .filter(|column| selection.range.contains_column(*column))
                    })
            } else {
                u32::try_from(index)
                    .ok()
                    .and_then(|offset| selection.range.start().column().checked_add(offset))
                    .filter(|column| selection.range.contains_column(*column))
            };
            if column.is_none() && !child.is_nullable() {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("$.{}", child.name()),
                    reason: format_smolstr!(
                        "expected the column in the header of {}, got [{}]",
                        selection.sheet,
                        self.names
                            .iter()
                            .map(SmolStr::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                });
            }
            pairing.push(column);
        }
        Ok(pairing)
    }
}

pub(super) fn merge_proven_depth(
    sheet: &str,
    selected: CellRange,
    merges: &[CellRange],
) -> Result<Option<u32>> {
    use std::collections::{BTreeMap, BTreeSet, VecDeque};

    let first = selected.start().row();
    let mut starts = BTreeMap::<u32, Vec<usize>>::new();
    for (index, merge) in merges.iter().enumerate() {
        if merge.intersects(selected)
            && merge.start().row() == first
            && (!selected.contains(merge.start()) || !selected.contains(merge.end()))
        {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("{sheet}!{}", merge.start()),
                reason: format_smolstr!(
                    "expected inferred header merge {merge} within selected {selected}"
                ),
            });
        }
        // The same top span can be either a report title or the root of a
        // hierarchy. Child merges cannot distinguish those readings.
        if merge.start().row() == first
            && merge.start().column() == selected.start().column()
            && merge.end().column() == selected.end().column()
            && merge.start().column() < merge.end().column()
        {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("{sheet}!{}", merge.start()),
                reason: format_smolstr!(
                    "ambiguous full-width merge {merge}: title or parent header; choose Rows(n) explicitly"
                ),
            });
        }
        // Keep crossings for RowsWindow's located selected-edge validation;
        // only origins inside the selected region can establish a level.
        if selected.contains(merge.start()) {
            starts.entry(merge.start().row()).or_default().push(index);
        }
    }
    let mut seen = BTreeSet::new();
    let mut required_last = None::<u32>;
    let mut has_nonfull_horizontal_group = false;
    // Root siblings may start anywhere in the selected column span. A child
    // group can start only on the first row after its parent's merge ends,
    // inside that parent's column span. This ignores unrelated body merges.
    let mut frontier =
        VecDeque::from([(first, selected.start().column(), selected.end().column())]);
    while let Some((row, left, right)) = frontier.pop_front() {
        for &index in starts.get(&row).into_iter().flatten() {
            let merge = merges[index];
            if merge.start().column() < left || merge.end().column() > right {
                continue;
            }
            if !seen.insert(index) {
                continue;
            }
            if merge.end().row() > selected.end().row() {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("{sheet}!{}", merge.start()),
                    reason: format_smolstr!(
                        "expected inferred header merge {merge} within selected {selected}"
                    ),
                });
            }
            let horizontal = merge.start().column() < merge.end().column();
            let vertical = merge.start().row() < merge.end().row();
            if vertical {
                required_last = Some(
                    required_last.map_or(merge.end().row(), |last| last.max(merge.end().row())),
                );
            }
            if horizontal {
                // A group needs one physical child row after its covered
                // span. A million-row vertical group is one frontier step.
                let child = merge
                    .end()
                    .row()
                    .checked_add(1)
                    .filter(|child| *child <= selected.end().row())
                    .ok_or_else(|| Error::InvalidRecord {
                        path: format_smolstr!("{sheet}!{}", merge.start()),
                        reason: format_smolstr!(
                            "expected a child row after merged group {merge} inside {selected}"
                        ),
                    })?;
                required_last = Some(required_last.map_or(child, |last| last.max(child)));
                has_nonfull_horizontal_group |= merge.start().column() > selected.start().column()
                    || merge.end().column() < selected.end().column();
                frontier.push_back((child, merge.start().column(), merge.end().column()));
            }
        }
    }
    // The longest structurally linked span fixes one physical end. A shorter
    // Rows depth would cut that span across the header/body edge. Distinct
    // semantic path levels are checked by RowsWindow, not physical height.
    Ok(required_last
        .filter(|_| has_nonfull_horizontal_group)
        .map(|last| last - first + 1))
}

impl RowsWindow {
    /// A merge proves geometry, but a present numeric/date cell does not
    /// positively prove that geometry is a header. Explicit Rows accepts it.
    pub(super) fn require_inferred_label(
        sheet: &str,
        at: CellRef,
        present: bool,
        text: bool,
    ) -> Result<()> {
        if present && !text {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("{sheet}!{at}"),
                reason: SmolStr::new_static(
                    "expected a text label for inferred merged header; choose Rows(n) explicitly",
                ),
            });
        }
        Ok(())
    }

    pub(super) fn require_inferred_body(body_rows: u64, typed: bool) -> Result<()> {
        if body_rows == 0 || !typed {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.header"),
                reason: SmolStr::new_static(
                    "ambiguous merged header without a typed body transition; choose Rows(n) or None explicitly",
                ),
            });
        }
        Ok(())
    }
}

#[derive(Default)]
struct TypeLane {
    dtype: Option<DataType>,
    present: u64,
    mismatch: Option<(CellRef, Error)>,
}

impl TypeLane {
    fn observe(&mut self, dtype: &DataType, sheet: &str, at: CellRef) {
        self.present += 1;
        match &self.dtype {
            None => self.dtype = Some(dtype.clone()),
            Some(first) if first != dtype && self.mismatch.is_none() => {
                self.mismatch = Some((
                    at,
                    Error::InvalidRecord {
                        path: format_smolstr!("{sheet}!{at}"),
                        reason: format_smolstr!(
                            "expected {first} like the column's first value, got {dtype}; declare a field to read the column as one datatype"
                        ),
                    },
                ));
            }
            _ => {}
        }
    }
}

#[derive(Default)]
struct ProbeColumn {
    all: TypeLane,
    body: TypeLane,
    first_label: Option<SmolStr>,
    first_text: bool,
}

/// One typed pass compares None with Source without re-reading a row. The
/// adapters lend already-decoded dtype and literal label facts. No per-cell
/// Field or Scalar is constructed, and a mismatch is kept once per column.
pub(super) struct HeaderProbe {
    sheet: SmolStr,
    range: CellRange,
    requested: crate::RecordHeader,
    expected_rows: Option<u64>,
    first_row: Option<u32>,
    rows: u64,
    columns: std::collections::BTreeMap<u32, ProbeColumn>,
}

pub(super) struct HeaderResolution {
    pub(super) policy: crate::RecordHeader,
    pub(super) field: Field,
    pub(super) header: Header,
    pub(super) record_count: u64,
}

impl HeaderProbe {
    pub(super) fn new(
        sheet: impl Into<SmolStr>,
        range: CellRange,
        requested: crate::RecordHeader,
        expected_rows: Option<u64>,
    ) -> Self {
        Self {
            sheet: sheet.into(),
            range,
            requested,
            expected_rows,
            first_row: None,
            rows: 0,
            columns: std::collections::BTreeMap::new(),
        }
    }

    /// Begin one physical selected row, including an explicit empty row.
    pub(super) fn row(&mut self, index: u32) -> bool {
        if self.first_row.is_none() {
            self.first_row = Some(index);
        }
        self.rows += 1;
        self.rows == 1
    }

    /// `first_label` is supplied only for cells in the first selected row.
    pub(super) fn cell(
        &mut self,
        at: CellRef,
        dtype: Option<DataType>,
        first_label: Option<(SmolStr, bool)>,
    ) -> Result<()> {
        let explicit_source = self.requested == crate::RecordHeader::Source;
        let explicit_none = self.requested == crate::RecordHeader::None;
        let column = self.columns.entry(at.column()).or_default();
        if let Some((label, text)) = first_label {
            column.first_label = Some(label);
            column.first_text = text;
        }
        if let Some(dtype) = dtype {
            column.all.observe(&dtype, &self.sheet, at);
            if self.rows > 1 {
                column.body.observe(&dtype, &self.sheet, at);
            }
        }
        let mismatch = if explicit_source {
            &mut column.body.mismatch
        } else if explicit_none {
            &mut column.all.mismatch
        } else {
            return Ok(());
        };
        if let Some((_, error)) = mismatch.take() {
            return Err(error);
        }
        Ok(())
    }

    fn at(&self, column: u32) -> CellRef {
        CellRef::new(self.first_row.unwrap_or(self.range.start().row()), column)
    }

    /// The parser visits rows before columns. Keep that first refusal even
    /// though the compact probe stores facts grouped by physical column.
    pub(super) fn first_mismatch(&mut self, source: bool) -> Option<Error> {
        let column = self
            .columns
            .iter()
            .filter_map(|(column, facts)| {
                let lane = if source { &facts.body } else { &facts.all };
                lane.mismatch
                    .as_ref()
                    .map(|(at, _)| ((*at).row(), (*at).column(), *column))
            })
            .min()
            .map(|(_, _, column)| column)?;
        let facts = self.columns.get_mut(&column)?;
        let lane = if source {
            &mut facts.body
        } else {
            &mut facts.all
        };
        lane.mismatch.take().map(|(_, error)| error)
    }

    /// Validate literal, unique labels for every column with any value.
    fn source_labels(&self) -> Result<()> {
        let mut seen = std::collections::BTreeMap::<SmolStr, CellRef>::new();
        for (column, facts) in &self.columns {
            if facts.all.present == 0 {
                continue;
            }
            let at = self.at(*column);
            let Some(label) = facts
                .first_label
                .as_deref()
                .filter(|label| !label.trim().is_empty() && facts.first_text)
            else {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("{}!{at}", self.sheet),
                    reason: format_smolstr!(
                        "expected a text label for the occupied column {column}, got no complete first-row label"
                    ),
                });
            };
            let name = SmolStr::new(label.trim());
            if let Some(other) = seen.insert(name.clone(), at) {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("{}!{at}", self.sheet),
                    reason: format_smolstr!(
                        "expected distinct inferred labels, got {name:?} at {other} and {at}"
                    ),
                });
            }
        }
        Ok(())
    }

    /// The declared data reading of the first row uses the same positional
    /// pairing as a None reader. A failed pairing cannot be that candidate.
    pub(super) fn first_row_pairing(&self, declared: &Field) -> Option<Vec<Option<u32>>> {
        let header =
            Header::resolve(self.range, FlatExtent::Sparse, false, std::iter::empty()).ok()?;
        header
            .pairing(
                declared,
                FlatBinding {
                    sheet: &self.sheet,
                    range: self.range,
                    by_name: false,
                    authoritative: false,
                },
            )
            .ok()
    }

    pub(super) fn resolve(
        mut self,
        name: &str,
        extent: FlatExtent<'_>,
        declared: Option<&Field>,
        first_row_as_data: Option<bool>,
    ) -> Result<HeaderResolution> {
        use crate::RecordHeader;

        let mut source_header = None;
        let policy = if self.requested == RecordHeader::Infer {
            if self.rows == 0 || self.columns.values().all(|facts| facts.all.present == 0) {
                return Err(super::options::ExcelOptions::no_evidence_error());
            }
            let all_consistent = self
                .columns
                .values()
                .all(|facts| facts.all.mismatch.is_none());
            let labels = self.source_labels();
            // A declared Field owns conversion of heterogeneous body values.
            // Source facts decide only whether the first row is a label row;
            // conversion failures are located by the existing row reader.
            let declared_read = declared.is_some();
            let mut source_binding_error = None;
            if labels.is_ok()
                && let Some(field) = declared
            {
                let bound = Header::resolve(
                    self.range,
                    extent,
                    true,
                    self.columns.iter().map(|(column, facts)| {
                        Ok((
                            *column,
                            Some(std::borrow::Cow::Borrowed(
                                facts.first_label.as_deref().unwrap_or_default(),
                            )),
                        ))
                    }),
                )
                .and_then(|header| {
                    header.pairing(
                        field,
                        FlatBinding {
                            sheet: &self.sheet,
                            range: self.range,
                            by_name: true,
                            authoritative: matches!(extent, FlatExtent::Table(_)),
                        },
                    )?;
                    Ok(header)
                });
                match bound {
                    Ok(header) => source_header = Some(header),
                    Err(error) => source_binding_error = Some(error),
                }
            }
            let none_ok = if declared_read {
                labels.is_err() || first_row_as_data == Some(true)
            } else {
                all_consistent
            };
            let source_ok = self.rows > 1
                && labels.is_ok()
                && (source_header.is_some()
                    || (!declared_read
                        && self
                            .columns
                            .values()
                            .all(|facts| facts.body.mismatch.is_none())));
            // A lone text row can be a header-only sheet or one data row.
            if self.rows == 1 && self.columns.values().any(|facts| facts.first_text) {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!(
                        "{}!{}",
                        self.sheet,
                        self.at(self.range.start().column())
                    ),
                    reason: SmolStr::new_static(
                        "ambiguous text-only first row without a body; choose Source or None explicitly",
                    ),
                });
            }
            match (none_ok, source_ok) {
                (true, true) => {
                    return Err(Error::InvalidRecord {
                        path: format_smolstr!(
                            "{}!{}",
                            self.sheet,
                            self.at(self.range.start().column())
                        ),
                        reason: SmolStr::new_static(
                            "ambiguous first row: both a header and data fit; choose Source or None explicitly",
                        ),
                    });
                }
                (false, true) => RecordHeader::Source,
                (true, false) => RecordHeader::None,
                (false, false) => {
                    labels?;
                    if let Some(error) = source_binding_error {
                        return Err(error);
                    }
                    if let Some(error) = self
                        .first_mismatch(true)
                        .or_else(|| self.first_mismatch(false))
                    {
                        return Err(error);
                    }
                    return Err(super::options::ExcelOptions::ambiguous_error());
                }
            }
        } else {
            self.requested
        };
        let source = policy == RecordHeader::Source;
        if declared.is_none()
            && let Some(error) = self.first_mismatch(source)
        {
            return Err(error);
        }
        let header = if source {
            source_header.map(Ok).unwrap_or_else(|| {
                Header::resolve(
                    self.range,
                    extent,
                    source,
                    self.columns.iter().map(|(column, facts)| {
                        Ok((
                            *column,
                            Some(std::borrow::Cow::Borrowed(
                                facts.first_label.as_deref().unwrap_or_default(),
                            )),
                        ))
                    }),
                )
            })?
        } else {
            Header::resolve(
                self.range,
                extent,
                source,
                self.columns.iter().map(|(column, facts)| {
                    Ok((
                        *column,
                        source.then(|| {
                            std::borrow::Cow::Borrowed(
                                facts.first_label.as_deref().unwrap_or_default(),
                            )
                        }),
                    ))
                }),
            )?
        };
        let record_count = self
            .expected_rows
            .unwrap_or_else(|| self.rows.saturating_sub(if source { 1 } else { 0 }));
        if let Some(field) = declared {
            return Ok(HeaderResolution {
                policy,
                field: field.clone(),
                header,
                record_count,
            });
        }
        let mut fields = Vec::with_capacity(header.len());
        for at in 0..header.len() {
            let column = header.physical(at);
            let facts = self.columns.get(&column);
            let lane = facts.map(|facts| if source { &facts.body } else { &facts.all });
            let present = lane.map_or(0, |lane| lane.present);
            if present == 0 && !header.from_header[at] {
                continue;
            }
            let dtype = lane
                .and_then(|lane| lane.dtype.clone())
                .unwrap_or(DataType::Null);
            fields.push(Field::new(
                header.names[at].clone(),
                dtype,
                present < record_count || present == 0,
            ));
        }
        let fields = StructType::from_fields(fields).map_err(|error| Error::InvalidRecord {
            path: format_smolstr!("{}!{}", self.sheet, self.range.start()),
            reason: format_smolstr!("the header names no valid columns: {error}"),
        })?;
        let field = Field::new(name, DataType::from(fields), false);
        Ok(HeaderResolution {
            policy,
            field,
            header,
            record_count,
        })
    }
}

/// One physical header label as decoded by a wire or held-cell adapter.
pub(super) struct RowLabel {
    pub(super) at: CellRef,
    pub(super) text: SmolStr,
}

/// Facts selected before semantic schema construction. `columns` is sorted,
/// unique and sparse for whole-sheet selection; an explicit finite range
/// supplies every stated column. Merge spans add all columns they cover.
pub(super) struct RowsWindow {
    pub(super) sheet: SmolStr,
    pub(super) range: CellRange,
    pub(super) levels: u32,
    pub(super) columns: Vec<u32>,
    pub(super) labels: Vec<RowLabel>,
    pub(super) merges: Vec<CellRange>,
}

#[derive(Clone)]
struct PathLabel {
    name: SmolStr,
    at: CellRef,
}

struct PhysicalPath {
    column: u32,
    segments: Vec<PathLabel>,
}

/// A source field is cloned once at plan compilation, never once per row.
pub(super) struct RowLeaf {
    pub(super) column: Option<u32>,
    pub(super) field: Field,
}

enum RowNode {
    Leaf(usize),
    Struct {
        children: Vec<RowNode>,
        leaves: std::ops::Range<usize>,
        nullable: bool,
        first_column: u32,
        name: SmolStr,
    },
}

/// The one compiled geometry for a nested Excel record. Field remains the
/// only semantic schema, and each node stores index/column facts only.
pub(super) struct RowsLayout {
    root: Field,
    leaves: Vec<RowLeaf>,
    row: RowNode,
    range: CellRange,
    body_start: u32,
}

impl RowsWindow {
    fn refusal(&self, at: CellRef, reason: impl Into<SmolStr>) -> Error {
        Error::InvalidRecord {
            path: format_smolstr!("{}!{at}", self.sheet),
            reason: reason.into(),
        }
    }

    pub(super) fn header_range(range: CellRange, levels: u32) -> Result<CellRange> {
        let Some(last) = levels
            .checked_sub(1)
            .and_then(|height| range.start().row().checked_add(height))
        else {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.header"),
                reason: format_smolstr!(
                    "expected a positive Rows depth inside the sheet, got {levels}"
                ),
            });
        };
        if last > range.end().row() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.header"),
                reason: format_smolstr!(
                    "expected {} header rows inside selected {}, got end row {}",
                    levels,
                    range,
                    last + 1
                ),
            });
        }
        Ok(CellRange::new(
            range.start(),
            CellRef::new(last, range.end().column()),
        ))
    }

    /// Expand an implicit cell extent only by the requested header depth and
    /// merges touching its anchored header probe. Explicit ranges never use
    /// this door: their stated edges are authoritative.
    pub(super) fn implicit_range(
        cells: CellRange,
        levels: u32,
        merges: impl IntoIterator<Item = CellRange>,
    ) -> Result<CellRange> {
        let Some(last) = levels
            .checked_sub(1)
            .and_then(|height| cells.start().row().checked_add(height))
            .filter(|last| *last < MAX_ROWS)
        else {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.header"),
                reason: format_smolstr!(
                    "expected {levels} physical header rows inside the Excel grid from {}, got an out-of-grid end",
                    cells.start()
                ),
            });
        };
        let probe = CellRange::new(cells.start(), CellRef::new(last, cells.end().column()));
        let mut end_row = cells.end().row().max(last);
        let mut end_column = cells.end().column();
        for merge in merges {
            if merge.intersects(probe) {
                end_row = end_row.max(merge.end().row());
                end_column = end_column.max(merge.end().column());
            }
        }
        Ok(CellRange::new(
            cells.start(),
            CellRef::new(end_row, end_column),
        ))
    }

    /// Resolve literal label paths using only declared merges. Every covered
    /// cell is accounted for once; no forward fill or delimiter flattening.
    fn paths(mut self) -> Result<(Vec<PhysicalPath>, u32)> {
        use std::collections::BTreeMap;

        let header = Self::header_range(self.range, self.levels)?;
        let body_start = header.end().row() + 1;
        let mut labels = BTreeMap::<CellRef, SmolStr>::new();
        for label in std::mem::take(&mut self.labels) {
            if header.contains(label.at) {
                labels.insert(label.at, label.text);
            }
        }
        self.columns.extend(
            labels
                .iter()
                .filter_map(|(at, text)| (!text.trim().is_empty()).then_some(at.column())),
        );
        // Index only declared spans by the columns they actually cover.
        // A million-row vertical merge is one interval, never a million
        // occupied-cell entries. Excel bounds its width to 16,384 columns.
        let mut spans = BTreeMap::<u32, Vec<usize>>::new();
        for (index, merge) in self.merges.iter().enumerate() {
            if !merge.intersects(header) {
                continue;
            }
            if !header.contains(merge.start()) || !header.contains(merge.end()) {
                return Err(self.refusal(merge.start(), format_smolstr!(
                    "expected merged header {merge} within selected {header}, got a span crossing the header or range edge")));
            }
            self.columns
                .extend(merge.start().column()..=merge.end().column());
            for column in merge.start().column()..=merge.end().column() {
                spans.entry(column).or_default().push(index);
            }
        }
        for (column, indices) in &mut spans {
            indices.sort_unstable_by_key(|index| self.merges[*index].start().row());
            for pair in indices.windows(2) {
                let first = self.merges[pair[0]];
                let next = self.merges[pair[1]];
                if first.end().row() >= next.start().row() {
                    let at = CellRef::new(next.start().row(), *column);
                    return Err(self.refusal(at, format_smolstr!(
                        "expected nonoverlapping merged header spans, got {first} and {next} at {at}")));
                }
            }
        }
        self.columns.sort_unstable();
        self.columns.dedup();
        self.columns
            .retain(|column| header.contains_column(*column));
        for (at, text) in &labels {
            if let Some(indices) = spans.get(&at.column()) {
                let end =
                    indices.partition_point(|index| self.merges[*index].start().row() <= at.row());
                if end > 0
                    && self.merges[indices[end - 1]].contains(*at)
                    && *at != self.merges[indices[end - 1]].start()
                    && !text.trim().is_empty()
                {
                    return Err(self.refusal(*at, format_smolstr!(
                        "expected only the top-left of merged header {} to carry a label, got {text:?} at {at}",
                        self.merges[indices[end - 1]])));
                }
            }
        }
        let mut paths = Vec::with_capacity(self.columns.len());
        for column in std::mem::take(&mut self.columns) {
            let mut segments = Vec::new();
            let mut row = header.start().row();
            let mut merge_at = 0;
            let indices = spans.get(&column).map(Vec::as_slice).unwrap_or_default();
            while row <= header.end().row() {
                let at = CellRef::new(row, column);
                while merge_at < indices.len() && self.merges[indices[merge_at]].end().row() < row {
                    merge_at += 1;
                }
                let origin = if merge_at < indices.len()
                    && self.merges[indices[merge_at]].start().row() == row
                {
                    let merge = self.merges[indices[merge_at]];
                    row = merge.end().row() + 1; // skip covered lower levels
                    merge.start()
                } else {
                    row += 1;
                    at
                };
                let text = labels.get(&origin).map(SmolStr::as_str).unwrap_or_default();
                if text.trim().is_empty() {
                    return Err(self.refusal(
                        at,
                        format_smolstr!(
                            "expected a literal header label or a declared merge, got blank at {at}"
                        ),
                    ));
                }
                segments.push(PathLabel {
                    name: SmolStr::new(text),
                    at: origin,
                });
                // The Field tree is recursive. Refuse at its existing schema
                // depth boundary before building one from an Excel grid.
                if segments.len() >= DataType::PARSE_RECURSION_LIMIT {
                    return Err(self.refusal(origin, format_smolstr!(
                        "expected schema nesting below the Field limit of {}, got {} header levels",
                        DataType::PARSE_RECURSION_LIMIT, segments.len())));
                }
            }
            paths.push(PhysicalPath { column, segments });
        }
        self.validate_paths(&paths)?;
        Ok((paths, body_start))
    }

    fn validate_paths(&self, paths: &[PhysicalPath]) -> Result<()> {
        use std::collections::BTreeMap;
        let mut complete = BTreeMap::<Vec<SmolStr>, CellRef>::new();
        let mut prefixes = BTreeMap::<Vec<SmolStr>, CellRef>::new();
        let mut groups = BTreeMap::<Vec<SmolStr>, (u32, CellRef)>::new();
        for path in paths {
            let names: Vec<SmolStr> = path.segments.iter().map(|s| s.name.clone()).collect();
            let at = path
                .segments
                .last()
                .map_or(CellRef::new(self.range.start().row(), path.column), |s| {
                    s.at
                });
            if let Some(other) = complete.insert(names.clone(), at) {
                return Err(self.refusal(
                    at,
                    format_smolstr!(
                        "expected distinct complete header paths, got duplicate at {other} and {at}"
                    ),
                ));
            }
            if let Some(other) = prefixes.get(&names) {
                return Err(self.refusal(
                    at,
                    format_smolstr!(
                        "expected a leaf or group at one path, got both at {other} and {at}"
                    ),
                ));
            }
            for depth in 1..names.len() {
                let prefix = names[..depth].to_vec();
                if let Some(other) = complete.get(&prefix) {
                    return Err(self.refusal(
                        at,
                        format_smolstr!(
                            "expected a leaf or group at one path, got both at {other} and {at}"
                        ),
                    ));
                }
                prefixes.entry(prefix.clone()).or_insert(at);
                match groups.get_mut(&prefix) {
                    Some((last, other)) => {
                        if *last + 1 != path.column {
                            return Err(self.refusal(at, format_smolstr!(
                                "expected contiguous group columns, got the group at {other} and {at}")));
                        }
                        *last = path.column;
                    }
                    None => {
                        groups.insert(prefix, (path.column, at));
                    }
                }
            }
        }
        Ok(())
    }
}

impl RowsLayout {
    /// Physical leaf width of a declared record, without reading cells.
    pub(super) fn leaf_width(root: &Field) -> Result<usize> {
        crate::preflight_schema_shape(root.dtype(), "Field")?;
        fn count(field: &Field) -> Result<usize> {
            if !matches!(field.dtype(), DataType::Struct(_)) {
                return Ok(1);
            }
            field.fields().iter().try_fold(0_usize, |width, child| {
                width
                    .checked_add(count(child)?)
                    .ok_or_else(|| Error::InvalidRecord {
                        path: SmolStr::new_static("$.header"),
                        reason: SmolStr::new_static(
                            "the physical leaf width overflowed the platform size",
                        ),
                    })
            })
        }
        root.fields().iter().try_fold(0_usize, |width, child| {
            width
                .checked_add(count(child)?)
                .ok_or_else(|| Error::InvalidRecord {
                    path: SmolStr::new_static("$.header"),
                    reason: SmolStr::new_static(
                        "the physical leaf width overflowed the platform size",
                    ),
                })
        })
    }

    /// Infer the existing Field tree from exact physical paths. Stats are one
    /// datatype/nullability tally per physical body column from reader/Sheet.
    pub(super) fn inferred(
        window: RowsWindow,
        name: &str,
        stats: &std::collections::BTreeMap<u32, (DataType, bool)>,
    ) -> Result<Self> {
        let range = window.range;
        let (paths, body_start) = window.paths()?;
        fn fields(
            paths: &[PhysicalPath],
            level: usize,
            stats: &std::collections::BTreeMap<u32, (DataType, bool)>,
        ) -> Result<Vec<Field>> {
            let mut result = Vec::new();
            let mut first = 0;
            while first < paths.len() {
                let name = paths[first].segments[level].name.clone();
                let mut after = first + 1;
                while after < paths.len() && paths[after].segments[level].name == name {
                    after += 1;
                }
                let group = &paths[first..after];
                if group[0].segments.len() == level + 1 {
                    let (dtype, nullable) = stats
                        .get(&group[0].column)
                        .cloned()
                        .unwrap_or((DataType::Null, true));
                    result.push(Field::new(name, dtype, nullable));
                } else {
                    result.push(Field::new(
                        name,
                        DataType::from(StructType::from_fields(fields(group, level + 1, stats)?)?),
                        false,
                    ));
                }
                first = after;
            }
            Ok(result)
        }
        let root = Field::new(
            name,
            DataType::from(StructType::from_fields(fields(&paths, 0, stats)?)?),
            false,
        );
        crate::preflight_schema_shape(root.dtype(), "Field")?;
        Self::bind(root, paths, range, body_start)
    }

    /// Bind a declared Field's exact leaf-name paths to physical columns.
    /// The source schema remains Field; this only compiles coordinates.
    pub(super) fn declared(window: RowsWindow, root: &Field) -> Result<Self> {
        crate::preflight_schema_shape(root.dtype(), "Field")?;
        let range = window.range;
        let (paths, body_start) = window.paths()?;
        Self::bind(root.clone().with_nullable(false), paths, range, body_start)
    }

    fn bind(
        root: Field,
        paths: Vec<PhysicalPath>,
        range: CellRange,
        body_start: u32,
    ) -> Result<Self> {
        use std::collections::BTreeMap;
        let physical: BTreeMap<Vec<SmolStr>, u32> = paths
            .into_iter()
            .map(|path| {
                (
                    path.segments.into_iter().map(|s| s.name).collect(),
                    path.column,
                )
            })
            .collect();
        let mut leaves = Vec::new();
        fn descend(
            field: &Field,
            names: &mut Vec<SmolStr>,
            physical: &BTreeMap<Vec<SmolStr>, u32>,
            leaves: &mut Vec<RowLeaf>,
        ) -> Result<RowNode> {
            if let DataType::Struct(_) = field.dtype() {
                let first = leaves.len();
                let mut children = Vec::new();
                for child in field.fields() {
                    names.push(SmolStr::new(child.name()));
                    children.push(descend(child, names, physical, leaves)?);
                    names.pop();
                }
                let first_column = leaves[first..]
                    .iter()
                    .filter_map(|leaf| leaf.column)
                    .min()
                    .unwrap_or(0);
                Ok(RowNode::Struct {
                    children,
                    leaves: first..leaves.len(),
                    nullable: field.is_nullable(),
                    first_column,
                    name: SmolStr::new(field.name()),
                })
            } else {
                let column = physical.get(names).copied();
                if column.is_none() && !field.is_nullable() {
                    return Err(Error::InvalidRecord {
                        path: SmolStr::new_static("$.header"),
                        reason: format_smolstr!(
                            "expected required leaf {:?} in the selected header paths, got no matching column",
                            names
                        ),
                    });
                }
                let index = leaves.len();
                leaves.push(RowLeaf {
                    column,
                    field: field.clone(),
                });
                Ok(RowNode::Leaf(index))
            }
        }
        let row = descend(&root, &mut Vec::new(), &physical, &mut leaves)?;
        Ok(Self {
            root,
            leaves,
            row,
            range,
            body_start,
        })
    }

    pub(super) fn root(&self) -> &Field {
        &self.root
    }
    pub(super) fn range(&self) -> CellRange {
        self.range
    }
    pub(super) fn leaves(&self) -> &[RowLeaf] {
        &self.leaves
    }
    pub(super) fn body_start(&self) -> u32 {
        self.body_start
    }

    /// Move exactly one typed leaf value into each resulting Struct sequence.
    pub(super) fn assemble(&self, values: &mut [Scalar], sheet: &str, row: u32) -> Result<Scalar> {
        self.row.assemble(values, sheet, row)
    }
}

impl RowNode {
    fn assemble(&self, values: &mut [Scalar], sheet: &str, row: u32) -> Result<Scalar> {
        match self {
            Self::Leaf(index) => Ok(std::mem::replace(&mut values[*index], Scalar::Null)),
            Self::Struct {
                children,
                leaves,
                nullable,
                first_column,
                name,
            } => {
                if *nullable && values[leaves.clone()].iter().all(Scalar::is_null) {
                    return Err(Error::InvalidRecord {
                        path: format_smolstr!("{sheet}!{}", CellRef::new(row, *first_column)),
                        reason: format_smolstr!(
                            "expected a present leaf to distinguish nullable group {name:?} from a null parent, got all null"
                        ),
                    });
                }
                Scalar::try_sequence(children.len(), |index| {
                    children[index].assemble(values, sheet, row)
                })
            }
        }
    }
}

pub(super) struct RowsWriteLayout {
    root_nullable: bool,
    anchor: CellRef,
    levels: u32,
    leaves: Vec<WriteLeaf>,
    labels: Vec<(CellRef, SmolStr)>,
    merges: Vec<CellRange>,
    nullable_groups: Vec<WriteGroup>,
}

pub(super) struct WriteLeaf {
    pub(super) path: Vec<usize>,
    pub(super) field: Field,
}

pub(super) struct WriteGroup {
    pub(super) path: Vec<usize>,
    pub(super) leaves: std::ops::Range<usize>,
    pub(super) name: SmolStr,
    pub(super) column: u32,
}

/// One bound record batch. The null prefix is reused for every row.
pub(super) struct RowsWriteColumns<'a> {
    root: &'a Serie,
    leaves: Vec<&'a Serie>,
    groups: Vec<&'a Serie>,
    null_prefix: Vec<usize>,
}

impl RowsWriteLayout {
    pub(super) fn compile(
        root: &Field,
        levels: u32,
        anchor: CellRef,
        rows: Option<usize>,
    ) -> Result<Self> {
        anchor.require_in_grid()?;
        crate::preflight_schema_shape(root.dtype(), "Field")?;
        if levels == 0 || levels > MAX_ROWS {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.header"),
                reason: format_smolstr!(
                    "expected 1 to {MAX_ROWS} physical header rows, got {levels}"
                ),
            });
        }
        let after_header = anchor
            .row()
            .checked_add(levels)
            .filter(|after| *after <= MAX_ROWS)
            .ok_or_else(|| Error::InvalidRecord {
                path: SmolStr::new_static("$.header"),
                reason: format_smolstr!(
                    "expected {levels} header rows from {anchor} inside the Excel grid"
                ),
            })?;
        if let Some(rows) = rows
            && rows > (MAX_ROWS - after_header) as usize
        {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.header"),
                reason: format_smolstr!(
                    "expected {levels} header rows and {rows} records from {anchor} inside the Excel grid"
                ),
            });
        }
        let mut layout = Self {
            root_nullable: root.is_nullable(),
            anchor,
            levels,
            leaves: Vec::new(),
            labels: Vec::new(),
            merges: Vec::new(),
            nullable_groups: Vec::new(),
        };
        if !matches!(root.dtype(), DataType::Struct(_)) {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.header"),
                reason: format_smolstr!(
                    "expected a record root for Rows({levels}), got {}",
                    root.dtype()
                ),
            });
        }
        let mut path = Vec::new();
        for (index, child) in root.fields().iter().enumerate() {
            path.push(index);
            layout.visit(child, &mut path, 1)?;
            path.pop();
        }
        if layout.leaves.is_empty() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.header"),
                reason: SmolStr::new_static(
                    "expected at least one header leaf, got an empty record",
                ),
            });
        }
        layout.labels.sort_unstable_by_key(|(at, _)| *at);
        layout.merges.sort_unstable_by_key(|merge| merge.start());
        Ok(layout)
    }

    fn at(&self, depth: u32, leaf_index: usize) -> Result<CellRef> {
        let column = self.anchor.column() as usize + leaf_index;
        if column >= MAX_COLUMNS as usize {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.header"),
                reason: format_smolstr!(
                    "expected at most {} leaf columns from {}, got {}",
                    MAX_COLUMNS - self.anchor.column(),
                    self.anchor,
                    leaf_index + 1
                ),
            });
        }
        Ok(CellRef::new(self.anchor.row() + depth - 1, column as u32))
    }

    fn visit(&mut self, field: &Field, path: &mut Vec<usize>, depth: u32) -> Result<()> {
        let first = self.leaves.len();
        let at = self.at(depth, first)?;
        if field.name().trim().is_empty() {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("$.{}", field.name()),
                reason: format_smolstr!("expected a literal nonblank header label at {at}"),
            });
        }
        self.labels.push((at, SmolStr::new(field.name())));
        if let DataType::Struct(_) = field.dtype() {
            if depth == self.levels || field.fields().is_empty() {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("$.{}", field.name()),
                    reason: format_smolstr!(
                        "expected child labels within Rows({}), got a Struct at {at} without room for them",
                        self.levels
                    ),
                });
            }
            for (index, child) in field.fields().iter().enumerate() {
                path.push(index);
                self.visit(child, path, depth + 1)?;
                path.pop();
            }
            let after = self.leaves.len();
            if after - first > 1 {
                self.merges.push(CellRange::new(
                    at,
                    CellRef::new(at.row(), self.anchor.column() + after as u32 - 1),
                ));
            }
            if field.is_nullable() {
                self.nullable_groups.push(WriteGroup {
                    path: path.clone(),
                    leaves: first..after,
                    name: SmolStr::new(field.name()),
                    column: at.column(),
                });
            }
        } else {
            self.leaves.push(WriteLeaf {
                path: path.clone(),
                field: field.clone(),
            });
            if depth < self.levels {
                self.merges.push(CellRange::new(
                    at,
                    CellRef::new(self.anchor.row() + self.levels - 1, at.column()),
                ));
            }
        }
        Ok(())
    }

    pub(super) fn labels(&self) -> &[(CellRef, SmolStr)] {
        &self.labels
    }
    pub(super) fn merges(&self) -> &[CellRange] {
        &self.merges
    }
    pub(super) fn leaves(&self) -> &[WriteLeaf] {
        &self.leaves
    }
    pub(super) fn first_body_row(&self) -> u32 {
        self.anchor.row() + self.levels
    }

    /// Borrow leaves once per batch or held Serie; never traverse Structs per
    /// output row. `children` are the record root's immediate columns.
    pub(super) fn bind_columns<'a>(
        &self,
        root: &'a Serie,
        children: &'a [Serie],
    ) -> Result<RowsWriteColumns<'a>> {
        let mut leaves = Vec::with_capacity(self.leaves.len());
        for leaf in &self.leaves {
            leaves.push(Self::column_at(children, &leaf.path)?);
        }
        let groups = self
            .nullable_groups
            .iter()
            .map(|group| Self::column_at(children, &group.path))
            .collect::<Result<Vec<_>>>()?;
        let null_prefix = if self.root_nullable || !self.nullable_groups.is_empty() {
            Vec::with_capacity(self.leaves.len() + 1)
        } else {
            Vec::new()
        };
        Ok(RowsWriteColumns {
            root,
            leaves,
            groups,
            null_prefix,
        })
    }

    fn column_at<'a>(children: &'a [Serie], path: &[usize]) -> Result<&'a Serie> {
        let mut column = children.get(path[0]).ok_or_else(|| Error::InvalidRecord {
            path: SmolStr::new_static("$.header"),
            reason: format_smolstr!("expected field path {path:?} in record columns"),
        })?;
        for index in &path[1..] {
            column = column
                .as_struct()
                .and_then(|value| value.children().get(*index))
                .ok_or_else(|| Error::InvalidRecord {
                    path: SmolStr::new_static("$.header"),
                    reason: format_smolstr!("expected Struct at field path {path:?}"),
                })?;
        }
        Ok(column)
    }
}

impl RowsWriteColumns<'_> {
    pub(super) fn leaves(&self) -> &[&Serie] {
        &self.leaves
    }

    /// Nullable Struct validity cannot be carried by blank Excel leaf cells.
    /// Share the same located refusal in both model and streamed writers.
    pub(super) fn check_row(
        &mut self,
        layout: &RowsWriteLayout,
        index: usize,
        row: u32,
        sheet: &str,
    ) -> Result<()> {
        if layout.nullable_groups.is_empty() && !layout.root_nullable {
            return Ok(());
        }
        self.null_prefix.clear();
        self.null_prefix.push(0);
        for leaf in &self.leaves {
            self.null_prefix.push(
                self.null_prefix.last().copied().unwrap() + usize::from(leaf.is_null(index)?),
            );
        }
        if layout.root_nullable
            && (self.root.is_null(index)?
                || self.null_prefix.last().copied() == Some(self.leaves.len()))
        {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("{sheet}!{}", CellRef::new(row, layout.anchor.column())),
                reason: format_smolstr!(
                    "cannot represent nullable root Struct when its parent or every leaf is null in Rows({})",
                    layout.levels
                ),
            });
        }
        for (group, column) in layout.nullable_groups.iter().zip(&self.groups) {
            let all_null = self.null_prefix[group.leaves.end]
                - self.null_prefix[group.leaves.start]
                == group.leaves.len();
            if column.is_null(index)? || all_null {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("{sheet}!{}", CellRef::new(row, group.column)),
                    reason: format_smolstr!(
                        "cannot represent nullable Struct {:?} when its parent or every leaf is null in Rows({})",
                        group.name,
                        layout.levels
                    ),
                });
            }
        }
        Ok(())
    }
}
