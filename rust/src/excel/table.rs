//! The identity, extent and row bands stated by one OOXML table part.

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result};

use super::cell::CellRange;
use super::package::{Edits, NamespaceFamily, Tag, edit_document, element_attributes};

/// What a table part states of its extent: its id, its range, and how
/// many header and totals rows it has.
pub(crate) struct Table {
    pub(crate) id: Option<u32>,
    pub(crate) name: SmolStr,
    pub(crate) range: CellRange,
    pub(crate) header_rows: u32,
    pub(crate) totals_rows: u32,
}

impl Table {
    /// Read the table part `bytes`, which `part` names.
    ///
    /// # Errors
    ///
    /// Returns the refusal of a part that is not well-formed or states no
    /// range.
    pub(crate) fn read(bytes: &[u8], part: &str) -> Result<Self> {
        let attributes = element_attributes(bytes, &["table"], part)?;
        Self::from_attributes(&attributes, part, false)
    }

    /// Resolve one consumed attribute once; duplicate exact QNames are
    /// ambiguous even when the preservation parser retained their bytes.
    fn attribute<'a>(
        attributes: &'a [(SmolStr, String)],
        name: &str,
        location: impl FnOnce() -> SmolStr,
    ) -> Result<Option<&'a str>> {
        let mut matching = attributes.iter().filter(|(key, _)| key == name);
        let first = matching.next();
        if matching.next().is_some() {
            return Err(Error::InvalidRecord {
                path: location(),
                reason: format_smolstr!("expected one {name} attribute, got duplicate attributes"),
            });
        }
        Ok(first.map(|(_, value)| value.as_str()))
    }

    /// A table's root attributes, already read by a document edit.
    pub(crate) fn from_attributes(
        attributes: &[(SmolStr, String)],
        part: &str,
        require_name: bool,
    ) -> Result<Self> {
        let get =
            |name: &str| Self::attribute(attributes, name, || format_smolstr!("{part}#{name}"));
        let count = |name: &str, default: u32| -> Result<u32> {
            match get(name)? {
                Some(value) => value.trim().parse().map_err(|_| Error::InvalidRecord {
                    path: format_smolstr!("{part}#{name}"),
                    reason: format_smolstr!("expected an unsigned 32-bit row count, got {value:?}"),
                }),
                None => Ok(default),
            }
        };
        let range = get("ref")?
            .and_then(|range| range.parse::<CellRange>().ok())
            .ok_or_else(|| Error::InvalidRecord {
                path: SmolStr::new(part),
                reason: SmolStr::new_static("expected a table stating its ref, got none"),
            })?;
        let name = get("displayName")?
            .or(get("name")?)
            .map(super::shared_strings::decode);
        if require_name && !name.as_ref().is_some_and(|name| !name.trim().is_empty()) {
            return Err(Error::InvalidRecord {
                path: SmolStr::new(part),
                reason: SmolStr::new_static("expected a nonempty table displayName or name"),
            });
        }
        Ok(Self {
            id: get("id")?.and_then(|id| id.trim().parse().ok()),
            name: name.map_or_else(|| SmolStr::new(part), |name| SmolStr::new(name.as_ref())),
            range,
            header_rows: count("headerRowCount", 1)?,
            totals_rows: count("totalsRowCount", 0)?,
        })
    }
}

impl Table {
    /// Read the record names and extent from one registered table part.
    pub(crate) fn read_named(bytes: &[u8], part: &str) -> Result<(Self, Vec<SmolStr>)> {
        let mut named = Named {
            part,
            table: None,
            family: None,
            columns: false,
            names: Vec::new(),
            seen: std::collections::HashMap::new(),
            declared: None,
            saw_columns: false,
        };
        edit_document(bytes, &mut named)?;
        let table = named
            .table
            .take()
            .ok_or_else(|| named.refuse("expected a SpreadsheetML table root"))?;
        let expected = table.range.column_size() as usize;
        if named.names.len() != expected || named.declared.is_some_and(|count| count != expected) {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("{part}#tableColumns"),
                reason: format_smolstr!(
                    "expected {expected} named table columns with matching count for {}, got {} columns and count {:?}",
                    table.range,
                    named.names.len(),
                    named.declared
                ),
            });
        }
        Ok((table, named.names))
    }

    /// Resize this registered table's body without moving worksheet cells.
    /// Only its own extent and matching filter extent change; opaque children
    /// retain their original bytes through the package editor.
    pub(crate) fn resized_part(&self, bytes: &[u8], part: &str, after: u32) -> Result<Vec<u8>> {
        let original_after = self.range.end().row() + 1 - self.totals_rows;
        if self.totals_rows > 1
            || original_after <= self.range.start().row() + self.header_rows
            || after <= self.range.start().row() + self.header_rows
            || after
                .checked_add(self.totals_rows)
                .is_none_or(|end| end > super::cell::MAX_ROWS)
        {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("$.table[{}]", self.name),
                reason: SmolStr::new_static(
                    "expected a positive body and at most one totals row within the worksheet grid",
                ),
            });
        }
        let range = CellRange::new(
            self.range.start(),
            super::cell::CellRef::new(after + self.totals_rows - 1, self.range.end().column()),
        );
        if range == self.range {
            return Ok(bytes.to_vec());
        }
        let mut edit = Resize {
            part,
            old: self.range,
            new: range,
            totals_rows: self.totals_rows,
            family: None,
            saw_root: false,
            saw_filter: false,
            main: Vec::new(),
        };
        let changed = edit_document(bytes, &mut edit)?;
        if !edit.saw_root {
            return Err(edit.refuse("expected one SpreadsheetML table root"));
        }
        Ok(changed.unwrap_or_else(|| bytes.to_vec()))
    }
}

struct Resize<'a> {
    part: &'a str,
    old: CellRange,
    new: CellRange,
    totals_rows: u32,
    family: Option<NamespaceFamily>,
    saw_root: bool,
    saw_filter: bool,
    main: Vec<bool>,
}

impl Resize<'_> {
    fn refuse(&self, reason: &str) -> Error {
        Error::InvalidRecord {
            path: SmolStr::new(self.part),
            reason: SmolStr::new(reason),
        }
    }

    fn family(namespace: quick_xml::name::ResolveResult<'_>) -> Option<NamespaceFamily> {
        match namespace {
            quick_xml::name::ResolveResult::Bound(uri) => std::str::from_utf8(uri.as_ref())
                .ok()
                .and_then(NamespaceFamily::from_namespace),
            _ => None,
        }
    }
}

impl Edits for Resize<'_> {
    fn start(
        &mut self,
        path: &[SmolStr],
        attributes: &[(SmolStr, String)],
        namespace: quick_xml::name::ResolveResult<'_>,
    ) -> Result<Tag> {
        let family = Self::family(namespace);
        if path.len() == 1 {
            if path[0] != "table" || family.is_none() || self.saw_root {
                return Err(self.refuse("expected one SpreadsheetML table root"));
            }
            let old = Table::attribute(attributes, "ref", || format_smolstr!("{}#ref", self.part))?;
            if old.and_then(|value| value.parse::<CellRange>().ok()) != Some(self.old) {
                return Err(self.refuse("expected the selected table's original ref"));
            }
            self.saw_root = true;
            self.family = family;
            self.main.push(true);
            return Ok(Tag::Set(vec![(
                SmolStr::new_static("ref"),
                Some(super::shift::range_text(self.new)),
            )]));
        }
        let own = self.main.last().copied().unwrap_or(false) && family == self.family;
        self.main.push(own);
        if !own || path.first().is_none_or(|name| name != "table") {
            return Ok(Tag::Keep);
        }
        if path.len() == 2 && path[1] == "autoFilter" {
            if self.saw_filter {
                return Err(self.refuse("expected one table autoFilter"));
            }
            self.saw_filter = true;
            let old = Table::attribute(attributes, "ref", || {
                format_smolstr!("{}#autoFilter@ref", self.part)
            })?;
            let old_filter = CellRange::new(
                self.old.start(),
                super::cell::CellRef::new(
                    self.old.end().row() - self.totals_rows,
                    self.old.end().column(),
                ),
            );
            if old.and_then(|value| value.parse::<CellRange>().ok()) != Some(old_filter) {
                return Err(self.refuse("expected the table autoFilter to span its original ref"));
            }
            let new_filter = CellRange::new(
                self.new.start(),
                super::cell::CellRef::new(
                    self.new.end().row() - self.totals_rows,
                    self.new.end().column(),
                ),
            );
            return Ok(Tag::Set(vec![(
                SmolStr::new_static("ref"),
                Some(super::shift::range_text(new_filter)),
            )]));
        }
        if path.last().is_some_and(|name| name == "sortState") {
            return Err(self.refuse("expected no table sortState while changing body height"));
        }
        Ok(Tag::Keep)
    }

    fn end(&mut self, _: &[SmolStr], _: usize, _: usize, _: &[(SmolStr, String)]) -> Tag {
        self.main.pop();
        Tag::Keep
    }
}

/// The package editor supplies normalized namespace and attribute values.
/// Only selected metadata asks for column names; structural edits keep their
/// existing extent-only contract and use the same root attribute intake.
struct Named<'a> {
    part: &'a str,
    table: Option<Table>,
    family: Option<NamespaceFamily>,
    columns: bool,
    names: Vec<SmolStr>,
    seen: std::collections::HashMap<String, usize>,
    declared: Option<usize>,
    saw_columns: bool,
}

impl Named<'_> {
    fn refuse(&self, reason: &str) -> Error {
        Error::InvalidRecord {
            path: SmolStr::new(self.part),
            reason: SmolStr::new(reason),
        }
    }
}

impl Edits for Named<'_> {
    fn start(
        &mut self,
        path: &[SmolStr],
        attributes: &[(SmolStr, String)],
        namespace: quick_xml::name::ResolveResult<'_>,
    ) -> Result<Tag> {
        let family = match namespace {
            quick_xml::name::ResolveResult::Bound(uri) => std::str::from_utf8(uri.as_ref())
                .ok()
                .and_then(NamespaceFamily::from_namespace),
            _ => None,
        };
        let part = self.part;
        let column = self.names.len() + 1;
        let get = |name: &str| {
            Table::attribute(attributes, name, || match path.len() {
                1 => format_smolstr!("{part}#{name}"),
                2 => format_smolstr!("{part}#tableColumns@{name}"),
                _ => format_smolstr!("{part}#tableColumn[{column}]@{name}"),
            })
        };
        if path.len() == 1 {
            if path[0] != "table" || family.is_none() || self.table.is_some() {
                return Err(self.refuse("expected one SpreadsheetML table root"));
            }
            let table = Table::from_attributes(attributes, self.part, true)?;
            if u64::from(table.header_rows) + u64::from(table.totals_rows)
                > u64::from(table.range.row_size())
            {
                return Err(
                    self.refuse("expected header and totals rows contained by the table ref")
                );
            }
            self.family = family;
            self.table = Some(table);
        } else if path.len() == 2 && path[1] == "tableColumns" {
            self.columns = family.is_some() && family == self.family;
            if self.columns {
                if self.saw_columns {
                    return Err(self.refuse("expected one tableColumns element"));
                }
                self.saw_columns = true;
                let stated = get("count")?;
                self.declared = stated
                    .map(|count| count.trim().parse::<u32>().map(|count| count as usize))
                    .transpose()
                    .map_err(|_| Error::InvalidRecord {
                        path: format_smolstr!("{}#tableColumns@count", self.part),
                        reason: format_smolstr!(
                            "expected an unsigned 32-bit column count, got {:?}",
                            stated
                        ),
                    })?;
                let width = self
                    .table
                    .as_ref()
                    .expect("table root read")
                    .range
                    .column_size() as usize;
                if self.declared.is_some_and(|count| count != width) {
                    return Err(Error::InvalidRecord {
                        path: format_smolstr!("{}#tableColumns@count", self.part),
                        reason: format_smolstr!(
                            "expected table column count {width}, got {:?}",
                            stated
                        ),
                    });
                }
            }
        } else if path.len() == 3
            && path[1] == "tableColumns"
            && path[2] == "tableColumn"
            && self.columns
            && family.is_some()
            && family == self.family
        {
            let at = self.names.len();
            let width = self
                .table
                .as_ref()
                .expect("table root read")
                .range
                .column_size() as usize;
            if at >= width {
                return Err(self.refuse("expected no tableColumn beyond the table width"));
            }
            let name = get("name")?
                .map(super::shared_strings::decode)
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| Error::InvalidRecord {
                    path: format_smolstr!("{}#tableColumn[{}]@name", self.part, at + 1),
                    reason: SmolStr::new_static("expected a nonempty table column name"),
                })?;
            if let Some(previous) = self.seen.insert(name.to_ascii_lowercase(), at) {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("{}#tableColumn[{}]@name", self.part, at + 1),
                    reason: format_smolstr!(
                        "expected a distinct column name, got {name:?} also at column {}",
                        previous + 1
                    ),
                });
            }
            self.names.push(SmolStr::new(name));
        }
        Ok(Tag::Keep)
    }

    fn end(&mut self, path: &[SmolStr], _: usize, _: usize, _: &[(SmolStr, String)]) -> Tag {
        if path.len() == 2 && path[1] == "tableColumns" {
            self.columns = false;
        }
        Tag::Keep
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! Private table-part rewrite pins for the mirrored Excel table tests.

    /// Pin the table-part sidecar rewrite without fabricating 16,384 named
    /// columns merely to exercise one full-width wire reference.
    pub fn resized_part(bytes: &[u8], after: u32) -> crate::Result<Vec<u8>> {
        let part = "xl/tables/table1.xml";
        let table = super::Table::read(bytes, part)?;
        table.resized_part(bytes, part, after)
    }
}
