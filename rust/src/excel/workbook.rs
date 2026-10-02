//! The workbook: the one model of a package's parts and sheets.
//!
//! A [`Workbook`] opened over a handle mounts the archive and reads the
//! package documents once - `_rels/.rels` to the office document, the
//! workbook part for its sheets and date system, its relationships for each
//! sheet's part and for the shared strings and styles. Sheets are then read
//! on demand: the first access to a sheet parses its part into a
//! [`Sheet`] held until the workbook is dropped; the shared strings and the
//! styles are read once, on the first sheet that needs them. A workbook built
//! in memory holds only the sheets it was given.
//!
//! Writing produces the package again: a sheet held in memory is written
//! from its cells, every other member of an opened package is carried over
//! unchanged - its formatting, its other parts, its shared strings - and the
//! styles part gains the crate's own formats after the ones already there.
//! A workbook with no visible worksheet is refused, as Excel refuses it.

use std::io::Read;
use std::sync::{Arc, OnceLock};

use smol_str::{SmolStr, format_smolstr};

use crate::holder::{Buffer, Holder};
use crate::zip::ZipArchive;
use crate::{Codec, Error, IOBase, Result};

use super::cell::DateSystem;
use super::package::{self, RelationshipKind, Relationships};
use super::parser::SheetRows;
use super::shared_strings::{SharedStringTable, SharedStrings};
use super::sheet::{Sheet, SheetState, validate_sheet_name};
use super::styles::Styles;

/// What a `<sheet>` entry names, by the relationship its `r:id` resolves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SheetKind {
    /// A grid of cells: the one kind with rows to read.
    Worksheet,
    /// A chart on a tab of its own; it holds no cells.
    Chartsheet,
    /// A dialog on a tab of its own; it holds no cells.
    Dialogsheet,
}

impl SheetKind {
    /// The kind as the refusals name it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Worksheet => "worksheet",
            Self::Chartsheet => "chartsheet",
            Self::Dialogsheet => "dialogsheet",
        }
    }
}

/// One `<sheet>` of the workbook, its part, and the [`Sheet`] once parsed.
#[derive(Debug)]
struct Slot {
    name: SmolStr,
    kind: SheetKind,
    state: SheetState,
    /// The member holding the sheet, `None` for a sheet added in memory.
    part: Option<SmolStr>,
    parsed: OnceLock<Sheet>,
}

impl Slot {
    /// The sheet's state: the parsed sheet's once it is held, else what the
    /// workbook part stated.
    fn state(&self) -> SheetState {
        self.parsed.get().map_or(self.state, Sheet::state)
    }

    fn held(name: SmolStr, kind: SheetKind, state: SheetState, part: Option<SmolStr>) -> Self {
        Self {
            name,
            kind,
            state,
            part,
            parsed: OnceLock::new(),
        }
    }

    fn of(sheet: Sheet) -> Self {
        let slot = Self::held(
            SmolStr::new(sheet.name()),
            SheetKind::Worksheet,
            sheet.state(),
            None,
        );
        let _ = slot.parsed.set(sheet);
        slot
    }
}

/// A workbook: its sheets, reachable by name, over the package they came
/// from or built from nothing.
///
/// Held state: the sheet list and the resolved part names from open, one
/// parsed [`Sheet`] per sheet asked for, and the shared strings and styles
/// once a sheet needed them - all until drop.
///
/// ```
/// use yggdryl::excel::Workbook;
/// use yggdryl::holder::Buffer;
/// use yggdryl::Scalar;
///
/// let mut workbook = Workbook::new();
/// let trades = workbook.add_sheet("Trades")?;
/// trades.set_cell("A1".parse()?, "symbol")?;
/// trades.set_cell("A2".parse()?, "AAPL")?;
/// let bytes = workbook.into_bytes()?;
///
/// // The package opens again over any handle, and only the sheet asked
/// // for is parsed.
/// let opened = Workbook::open(Buffer::from_bytes(bytes))?;
/// assert_eq!(opened.sheet_names(), ["Trades"]);
/// assert_eq!(opened.sheet("trades")?.scalar("A2".parse()?), Scalar::from("AAPL"));
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Debug)]
pub struct Workbook {
    /// The package this workbook was opened over; `None` for one built.
    archive: Option<Arc<ZipArchive>>,
    system: DateSystem,
    slots: Vec<Slot>,
    /// The workbook part, `xl/workbook.xml` unless the package says otherwise.
    workbook_part: SmolStr,
    strings_part: Option<SmolStr>,
    styles_part: Option<SmolStr>,
    strings: OnceLock<Arc<SharedStrings>>,
    styles: OnceLock<Arc<Styles>>,
}

impl Default for Workbook {
    fn default() -> Self {
        Self::new()
    }
}

impl Workbook {
    /// An empty workbook under the 1900 date system, with no sheet yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            archive: None,
            system: DateSystem::Year1900,
            slots: Vec::new(),
            workbook_part: SmolStr::new_static(package::WORKBOOK_PART),
            strings_part: None,
            styles_part: None,
            strings: OnceLock::new(),
            styles: OnceLock::new(),
        }
    }

    /// Open the package `handle` holds: the archive is mounted and the
    /// workbook documents read; no sheet is parsed yet.
    ///
    /// A handle holding nothing opens as an empty workbook, per the laziness
    /// contract every read follows.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for a BIFF (`.xls`) or encrypted
    /// workbook, [`Error::Codec`] for a handle whose name declares a content
    /// coding (`trades.xlsx.gz`), for bytes that are not a ZIP package or for a
    /// part that is not well-formed, and [`Error::InvalidRecord`] naming the
    /// part for a package with no workbook.
    pub fn open(handle: impl Into<Holder>) -> Result<Self> {
        let handle = handle.into();
        super::reject_outer_coding(&handle)?;
        if handle.size() == 0 {
            return Ok(Self::new());
        }
        let head = handle.read_range_bytes(0, 4)?;
        if head == [0xD0, 0xCF, 0x11, 0xE0] {
            return Err(Error::unsupported(
                "reading a BIFF or encrypted workbook",
                handle.url().map_or_else(
                    || SmolStr::new_static("<buffer>"),
                    |url| format_smolstr!("{url}"),
                ),
            ));
        }
        let archive = Arc::new(ZipArchive::new(handle));
        let mut workbook = Self::new();
        workbook.load(archive)?;
        Ok(workbook)
    }

    /// Open the package `bytes` hold.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::open`] returns.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        Self::open(Holder::buffer(Buffer::from_bytes(bytes)))
    }

    /// Read the package documents off `archive`.
    fn load(&mut self, archive: Arc<ZipArchive>) -> Result<()> {
        let members = archive.entries().map_err(|error| match error {
            Error::Codec { position, reason, .. } => Error::Codec {
                format: "xlsx",
                position,
                reason: format_smolstr!(
                    "expected a ZIP package (application/vnd.openxmlformats-officedocument.spreadsheetml.sheet), got: {reason}"
                ),
            },
            other => other,
        })?;
        let held = |name: &str| members.iter().any(|entry| entry.name() == name);
        let listing = || {
            members
                .iter()
                .map(|entry| entry.name().to_owned())
                .collect::<Vec<_>>()
                .join(", ")
        };
        // The office document is where the package's own relationships say,
        // and at the conventional part where they say nothing.
        let workbook_part = if held(package::ROOT_RELATIONSHIPS_PART) {
            let root = archive.read_member(package::ROOT_RELATIONSHIPS_PART)?;
            Relationships::from_xml(&root, "")?
                .first_of(RelationshipKind::OfficeDocument)
                .and_then(|relationship| relationship.target.clone())
                .unwrap_or_else(|| SmolStr::new_static(package::WORKBOOK_PART))
        } else {
            SmolStr::new_static(package::WORKBOOK_PART)
        };
        if !held(&workbook_part) {
            return Err(Error::InvalidRecord {
                path: workbook_part,
                reason: format_smolstr!(
                    "expected the workbook part in the package, got the members [{}]",
                    listing()
                ),
            });
        }
        let relationships_part = relationships_part_of(&workbook_part);
        let relationships = if held(&relationships_part) {
            Relationships::from_xml(&archive.read_member(&relationships_part)?, &workbook_part)?
        } else {
            Relationships::default()
        };
        let document = archive.read_member(&workbook_part)?;
        let (entries, system) = read_workbook(&document, &workbook_part)?;
        let mut slots = Vec::with_capacity(entries.len());
        for entry in entries {
            // A sheet is its part: an `r:id` no relationship names, or one
            // naming an external target, is a package no reader can honour.
            let part = relationships
                .by_id(&entry.rid)
                .and_then(|relationship| relationship.target.clone())
                .ok_or_else(|| Error::InvalidRecord {
                    path: workbook_part.clone(),
                    reason: format_smolstr!(
                        "expected the relationship {} of sheet {:?} to name a part in {}, got [{}]",
                        entry.rid,
                        entry.name,
                        relationships_part,
                        relationships
                            .entries()
                            .iter()
                            .map(|relationship| relationship.id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                })?;
            let kind = match relationships
                .by_id(&entry.rid)
                .map(|relationship| relationship.kind)
            {
                Some(RelationshipKind::Chartsheet) => SheetKind::Chartsheet,
                Some(RelationshipKind::Dialogsheet) => SheetKind::Dialogsheet,
                _ => SheetKind::Worksheet,
            };
            slots.push(Slot::held(entry.name, kind, entry.state, Some(part)));
        }
        self.archive = Some(archive);
        self.system = system;
        self.slots = slots;
        self.workbook_part = workbook_part;
        self.strings_part = relationships
            .first_of(RelationshipKind::SharedStrings)
            .and_then(|relationship| relationship.target.clone())
            .filter(|part| held(part));
        self.styles_part = relationships
            .first_of(RelationshipKind::Styles)
            .and_then(|relationship| relationship.target.clone())
            .filter(|part| held(part));
        Ok(())
    }

    /// The date system the workbook's serials count from.
    #[must_use]
    pub const fn date_system(&self) -> DateSystem {
        self.system
    }

    /// Count serial dates from `system` in every sheet written, the ones
    /// already parsed included.
    pub fn set_date_system(&mut self, system: DateSystem) {
        self.system = system;
        for sheet in self
            .slots
            .iter_mut()
            .filter_map(|slot| slot.parsed.get_mut())
        {
            sheet.set_date_system(system);
        }
    }

    /// The sheets, in tab order, worksheets and chart sheets alike.
    #[must_use]
    pub fn sheet_names(&self) -> Vec<&str> {
        self.slots.iter().map(|slot| slot.name.as_str()).collect()
    }

    /// What the sheet `name` is, `None` for a name no sheet has.
    #[must_use]
    pub fn sheet_kind(&self, name: &str) -> Option<SheetKind> {
        self.resolve(name).map(|at| self.slots[at].kind)
    }

    /// How many sheets the workbook lists.
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether the workbook lists no sheet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// The position of the sheet `name`, compared without case as Excel
    /// compares names.
    fn resolve(&self, name: &str) -> Option<usize> {
        self.slots
            .iter()
            .position(|slot| slot.name.eq_ignore_ascii_case(name) || slot.name == name)
    }

    /// The worksheet `name`, parsed on first access.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Absent`] listing the sheets when none has the name,
    /// [`Error::InvalidRecord`] naming the kind for a chart or dialog sheet,
    /// or the part's refusal.
    pub fn sheet(&self, name: &str) -> Result<&Sheet> {
        let at = self.resolve(name).ok_or_else(|| self.absent(name))?;
        self.parsed(at)
    }

    /// The worksheet `name`, when the workbook has it.
    ///
    /// # Errors
    ///
    /// Returns a chart or dialog sheet's refusal, or the part's.
    pub fn get_sheet(&self, name: &str) -> Result<Option<&Sheet>> {
        match self.resolve(name) {
            Some(at) => self.parsed(at).map(Some),
            None => Ok(None),
        }
    }

    /// The sheet at zero-based `index` in tab order, when there is one.
    ///
    /// # Errors
    ///
    /// Returns a chart or dialog sheet's refusal, or the part's.
    pub fn sheet_at(&self, index: usize) -> Result<Option<&Sheet>> {
        if index >= self.slots.len() {
            return Ok(None);
        }
        self.parsed(index).map(Some)
    }

    /// The worksheet `name`, mutably, parsed on first access.
    ///
    /// The tab's name is the workbook's fact: [`Self::rename_sheet`] changes
    /// it, and a name set on the sheet itself through this borrow is not
    /// what the package is written under.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::sheet`] returns.
    pub fn sheet_mut(&mut self, name: &str) -> Result<&mut Sheet> {
        let at = self.resolve(name).ok_or_else(|| self.absent(name))?;
        self.parsed(at)?;
        Ok(self.slots[at]
            .parsed
            .get_mut()
            .expect("the sheet was parsed by the call above"))
    }

    /// Rename the sheet `name` to `new_name`, keeping its place and part.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Absent`] listing the sheets when none has the name,
    /// the new name's refusal ([`validate_sheet_name`]), or
    /// [`Error::Conflict`] when another sheet already has it, compared
    /// without case; a sheet renamed to its own name in another case takes
    /// it.
    pub fn rename_sheet(&mut self, name: &str, new_name: impl Into<SmolStr>) -> Result<()> {
        let at = self.resolve(name).ok_or_else(|| self.absent(name))?;
        let new_name = new_name.into();
        validate_sheet_name(&new_name)?;
        if self.resolve(&new_name).is_some_and(|other| other != at) {
            return Err(self.taken(&new_name));
        }
        if let Some(sheet) = self.slots[at].parsed.get_mut() {
            sheet.set_name(new_name.clone())?;
        }
        self.slots[at].name = new_name;
        Ok(())
    }

    /// Add an empty worksheet named `name` after the last tab.
    ///
    /// # Errors
    ///
    /// Returns the name's refusal ([`validate_sheet_name`]), or
    /// [`Error::Conflict`] when a sheet already has the name, compared
    /// without case.
    pub fn add_sheet(&mut self, name: impl Into<SmolStr>) -> Result<&mut Sheet> {
        let name = name.into();
        let sheet = Sheet::new(name)?.with_date_system(self.system);
        if self.resolve(sheet.name()).is_some() {
            return Err(self.taken(sheet.name()));
        }
        self.insert_sheet(sheet)?;
        let at = self.slots.len() - 1;
        Ok(self.slots[at]
            .parsed
            .get_mut()
            .expect("the sheet was just inserted"))
    }

    /// Put `sheet` in the workbook: in place of the sheet of the same name,
    /// answering it, or after the last tab.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] naming the kind when the name is a
    /// chart or dialog sheet's, which holds no cells to replace.
    pub fn insert_sheet(&mut self, sheet: Sheet) -> Result<Option<Sheet>> {
        let sheet = sheet.with_date_system(self.system);
        match self.resolve(sheet.name()) {
            Some(at) => {
                if self.slots[at].kind != SheetKind::Worksheet {
                    return Err(self.not_a_worksheet(at));
                }
                let previous = self.parsed(at).ok().cloned();
                let mut slot = Slot::of(sheet);
                slot.part = self.slots[at].part.clone();
                self.slots[at] = slot;
                Ok(previous)
            }
            None => {
                self.slots.push(Slot::of(sheet));
                Ok(None)
            }
        }
    }

    /// Take the sheet `name` out of the workbook, answering it parsed.
    ///
    /// # Errors
    ///
    /// Returns the part's refusal when the sheet had to be parsed to be
    /// answered; a chart or dialog sheet is removed and answers `None`.
    pub fn remove_sheet(&mut self, name: &str) -> Result<Option<Sheet>> {
        let Some(at) = self.resolve(name) else {
            return Ok(None);
        };
        if self.slots[at].kind == SheetKind::Worksheet {
            self.parsed(at)?;
        }
        let slot = self.slots.remove(at);
        Ok(slot.parsed.into_inner())
    }

    /// The position of the sheet `name` in tab order, compared without case.
    pub(crate) fn position(&self, name: &str) -> Option<usize> {
        self.resolve(name)
    }

    /// The first worksheet in tab order, which a read addresses when its
    /// options name no sheet.
    pub(crate) fn first_worksheet(&self) -> Option<&str> {
        self.slots
            .iter()
            .find(|slot| slot.kind == SheetKind::Worksheet)
            .map(|slot| slot.name.as_str())
    }

    /// The parsed sheet at `at`, parsing its part on first access.
    fn parsed(&self, at: usize) -> Result<&Sheet> {
        let slot = &self.slots[at];
        if slot.kind != SheetKind::Worksheet {
            return Err(self.not_a_worksheet(at));
        }
        if let Some(sheet) = slot.parsed.get() {
            return Ok(sheet);
        }
        let sheet = self.parse(slot)?;
        Ok(slot.parsed.get_or_init(|| sheet))
    }

    /// Parse the part of `slot` into a sheet.
    fn parse(&self, slot: &Slot) -> Result<Sheet> {
        let Some(part) = &slot.part else {
            return Ok(Sheet::new(slot.name.clone())?
                .with_date_system(self.system)
                .with_state(slot.state));
        };
        let member = self.member(part)?;
        let rows = SheetRows::new(std::io::BufReader::new(member), slot.name.clone());
        let strings = self.strings()?;
        let styles = self.styles()?;
        Sheet::from_rows(
            slot.name.clone(),
            slot.state,
            self.system,
            rows,
            &strings,
            &styles,
        )
    }

    /// Stream the part of the worksheet `name`, for the record path.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::sheet`] returns, and an empty part for a sheet
    /// added in memory and never written.
    pub(crate) fn sheet_reader(&self, name: &str) -> Result<Box<dyn Read + Send>> {
        let at = self.resolve(name).ok_or_else(|| self.absent(name))?;
        let slot = &self.slots[at];
        if slot.kind != SheetKind::Worksheet {
            return Err(self.not_a_worksheet(at));
        }
        match &slot.part {
            Some(part) => self.member(part),
            None => Ok(Box::new(std::io::Cursor::new(EMPTY_SHEET.as_bytes()))),
        }
    }

    /// The part name of the worksheet `name`, for the refusals a read names.
    pub(crate) fn sheet_part(&self, name: &str) -> Result<SmolStr> {
        let at = self.resolve(name).ok_or_else(|| self.absent(name))?;
        Ok(self.slots[at]
            .part
            .clone()
            .unwrap_or_else(|| package::worksheet_part(at + 1)))
    }

    /// One member of the package, streamed.
    fn member(&self, part: &str) -> Result<Box<dyn Read + Send>> {
        let Some(archive) = &self.archive else {
            return Ok(Box::new(std::io::empty()));
        };
        archive
            .member_reader(part)?
            .ok_or_else(|| Error::absent("workbook part", part))
    }

    /// The shared strings, read on first use; none for a package without
    /// the part.
    pub(crate) fn strings(&self) -> Result<Arc<SharedStrings>> {
        if let Some(strings) = self.strings.get() {
            return Ok(Arc::clone(strings));
        }
        let strings = match (&self.archive, &self.strings_part) {
            (Some(archive), Some(part)) => SharedStrings::from_xml(&archive.read_member(part)?)?,
            _ => SharedStrings::default(),
        };
        let strings = Arc::new(strings);
        let _ = self.strings.set(Arc::clone(&strings));
        Ok(strings)
    }

    /// The styles, read on first use; General everywhere for a package
    /// without the part.
    pub(crate) fn styles(&self) -> Result<Arc<Styles>> {
        if let Some(styles) = self.styles.get() {
            return Ok(Arc::clone(styles));
        }
        let styles = match (&self.archive, &self.styles_part) {
            (Some(archive), Some(part)) => Styles::from_xml(&archive.read_member(part)?)?,
            _ => Styles::default(),
        };
        let styles = Arc::new(styles);
        let _ = self.styles.set(Arc::clone(&styles));
        Ok(styles)
    }

    /// Calls the package's handle has answered so far: what opening the
    /// workbook and reading its sheets asked of the bytes beneath it, in
    /// [`IOBase`] calls, which the cost pins assert.
    #[must_use]
    pub fn handle_reads(&self) -> u64 {
        self.archive
            .as_ref()
            .map_or(0, |archive| archive.handle_reads())
    }

    /// The refusal of a name another sheet already has.
    fn taken(&self, name: &str) -> Error {
        Error::Conflict {
            expected: "sheet name no other sheet has",
            actual: "sheet of that name, compared without case",
            path: SmolStr::new(name),
        }
    }

    fn absent(&self, name: &str) -> Error {
        Error::Absent {
            expected: "worksheet",
            path: format_smolstr!(
                "{name} (the workbook holds [{}])",
                self.sheet_names().join(", ")
            ),
        }
    }

    fn not_a_worksheet(&self, at: usize) -> Error {
        let slot = &self.slots[at];
        Error::InvalidRecord {
            path: format_smolstr!("$.{}", slot.name),
            reason: format_smolstr!(
                "expected a worksheet, got the {} `{}`, which holds no cells",
                slot.kind.as_str(),
                slot.name
            ),
        }
    }

    /// The package, as bytes: every sheet held in memory written from its
    /// cells, every other member of an opened package carried over.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a workbook with no visible
    /// worksheet, which Excel refuses to open, or a cell no part spells.
    pub fn into_bytes(&self) -> Result<Vec<u8>> {
        self.write_package(None)
    }

    /// Write the package, the worksheet at `replaced` (a slot index and the
    /// stream of its part) taking the place of the one held.
    ///
    /// The archive is built in memory and answered whole: one write of the
    /// handle a caller hands the bytes to.
    pub(crate) fn write_package(&self, replaced: Option<Replaced<'_>>) -> Result<Vec<u8>> {
        if !self
            .slots
            .iter()
            .any(|slot| slot.kind == SheetKind::Worksheet && slot.state() == SheetState::Visible)
        {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: SmolStr::new_static(
                    "expected at least one visible worksheet, which Excel requires of a workbook",
                ),
            });
        }
        let target = ZipArchive::new(Holder::buffer(Buffer::new())).with_restart_stride(0);
        match &self.archive {
            Some(archive) => self.write_preserving(&target, archive, replaced)?,
            None => self.write_fresh(&target, replaced)?,
        }
        target.flush()?;
        match target.into_handle()? {
            Holder::Buffer(buffer) => Ok(buffer.into_bytes()),
            other => other.read_all_bytes(),
        }
    }

    /// The style offset the crate's formats take beside a package's own, and
    /// the styles part to write.
    fn styles_for_write(&self, archive: Option<&Arc<ZipArchive>>) -> Result<(Vec<u8>, u32)> {
        if let (Some(archive), Some(part)) = (archive, &self.styles_part) {
            let bytes = archive.read_member(part)?;
            let styles = self.styles()?;
            return styles.spliced(&bytes);
        }
        let mut bytes = Vec::new();
        Styles::write(&mut bytes)?;
        Ok((bytes, 0))
    }

    /// A package built from nothing but the sheets held.
    fn write_fresh(&self, target: &ZipArchive, replaced: Option<Replaced<'_>>) -> Result<()> {
        let (styles, offset) = self.styles_for_write(None)?;
        let mut strings = SharedStringTable::new();
        let mut content = Vec::new();
        package::write_content_types(&mut content, self.slots.len())?;
        target.write_member_with(package::CONTENT_TYPES_PART, &content, Codec::Deflate)?;
        content.clear();
        package::write_root_relationships(&mut content)?;
        target.write_member_with(package::ROOT_RELATIONSHIPS_PART, &content, Codec::Deflate)?;
        content.clear();
        write_workbook(&mut content, &self.slots, self.system)?;
        target.write_member_with(package::WORKBOOK_PART, &content, Codec::Deflate)?;
        content.clear();
        package::write_workbook_relationships(&mut content, self.slots.len())?;
        target.write_member_with(
            package::WORKBOOK_RELATIONSHIPS_PART,
            &content,
            Codec::Deflate,
        )?;
        let mut replaced = replaced;
        for (at, slot) in self.slots.iter().enumerate() {
            let part = package::worksheet_part(at + 1);
            if replaced.as_ref().is_some_and(|(index, _)| *index == at) {
                let (_, stream) = replaced.take().expect("checked just above");
                target.write_member_from(&part, stream(offset)?, Codec::Deflate)?;
                continue;
            }
            content.clear();
            match slot.parsed.get() {
                Some(sheet) => sheet.write_xml(&mut content, &mut strings, offset)?,
                None => content.extend_from_slice(EMPTY_SHEET.as_bytes()),
            }
            target.write_member_with(&part, &content, Codec::Deflate)?;
        }
        content.clear();
        strings.write(&mut content)?;
        target.write_member_with(package::SHARED_STRINGS_PART, &content, Codec::Deflate)?;
        target.write_member_with(package::STYLES_PART, &styles, Codec::Deflate)?;
        Ok(())
    }

    /// The opened package again, its untouched members carried over.
    fn write_preserving(
        &self,
        target: &ZipArchive,
        archive: &Arc<ZipArchive>,
        replaced: Option<Replaced<'_>>,
    ) -> Result<()> {
        let members = archive.entries()?;
        let mut replaced = replaced;
        // Which parts this write speaks for; every other member is copied.
        let mut written: Vec<SmolStr> = Vec::new();
        let rewrites_any = self.slots.iter().enumerate().any(|(at, slot)| {
            slot.parsed.get().is_some()
                || replaced.as_ref().is_some_and(|(index, _)| *index == at)
                || slot.part.is_none()
        });
        let (styles, offset) = if rewrites_any {
            self.styles_for_write(Some(archive))?
        } else {
            (Vec::new(), 0)
        };
        // Existing strings keep their indexes: the table opens with them.
        let existing = self.strings()?;
        let mut strings = SharedStringTable::from_existing(&existing);
        let existing_parts: Vec<Option<SmolStr>> =
            self.slots.iter().map(|slot| slot.part.clone()).collect();
        let mut next_number = members
            .iter()
            .filter_map(|entry| {
                entry
                    .name()
                    .strip_prefix("xl/worksheets/sheet")
                    .and_then(|rest| rest.strip_suffix(".xml"))
                    .and_then(|number| number.parse::<usize>().ok())
            })
            .max()
            .unwrap_or(0)
            + 1;
        let mut parts: Vec<SmolStr> = Vec::with_capacity(self.slots.len());
        for (at, slot) in self.slots.iter().enumerate() {
            let part = match &slot.part {
                Some(part) => part.clone(),
                None => {
                    let part = package::worksheet_part(next_number);
                    next_number += 1;
                    part
                }
            };
            if replaced.as_ref().is_some_and(|(index, _)| *index == at) {
                let (_, stream) = replaced.take().expect("checked just above");
                target.write_member_from(&part, stream(offset)?, Codec::Deflate)?;
            } else if let Some(sheet) = slot.parsed.get() {
                let mut content = Vec::new();
                sheet.write_xml(&mut content, &mut strings, offset)?;
                target.write_member_with(&part, &content, Codec::Deflate)?;
            } else if slot.part.is_none() {
                target.write_member_with(&part, EMPTY_SHEET.as_bytes(), Codec::Deflate)?;
            } else {
                let bytes = archive.read_member(&part)?;
                target.write_member_with(&part, &bytes, Codec::Deflate)?;
            }
            written.push(part.clone());
            parts.push(part);
        }
        // The sheet list changed when a sheet was added, removed or renamed,
        // or a part is new: then the workbook documents are spliced.
        let original: Vec<(SmolStr, Option<SmolStr>)> = read_workbook(
            &archive.read_member(&self.workbook_part)?,
            &self.workbook_part,
        )?
        .0
        .into_iter()
        .map(|entry| (entry.name, Some(entry.rid)))
        .collect();
        let relationships_part = relationships_part_of(&self.workbook_part);
        let list_changed = original.len() != self.slots.len()
            || original
                .iter()
                .zip(&self.slots)
                .any(|((name, _), slot)| *name != slot.name)
            || existing_parts.iter().any(Option::is_none)
            || self
                .slots
                .iter()
                .zip(&existing_parts)
                .any(|(slot, part)| part.is_none() && slot.part.is_some());
        let strings_part = self
            .strings_part
            .clone()
            .unwrap_or_else(|| SmolStr::new_static(package::SHARED_STRINGS_PART));
        let styles_part = self
            .styles_part
            .clone()
            .unwrap_or_else(|| SmolStr::new_static(package::STYLES_PART));
        if rewrites_any {
            let mut content = Vec::new();
            strings.write(&mut content)?;
            target.write_member_with(&strings_part, &content, Codec::Deflate)?;
            target.write_member_with(&styles_part, &styles, Codec::Deflate)?;
            written.push(strings_part.clone());
            written.push(styles_part.clone());
        }
        let workbook_documents = [
            self.workbook_part.clone(),
            relationships_part.clone(),
            SmolStr::new_static(package::CONTENT_TYPES_PART),
        ];
        if list_changed || self.strings_part.is_none() || self.styles_part.is_none() {
            self.splice_documents(
                target,
                archive,
                &parts,
                &strings_part,
                &styles_part,
                rewrites_any,
            )?;
            written.extend(workbook_documents.iter().cloned());
        }
        // A removed sheet's part, and its own relationships, go with it.
        let orphaned: Vec<SmolStr> = if archive.get_entry(&relationships_part)?.is_some() {
            Relationships::from_xml(
                &archive.read_member(&relationships_part)?,
                &self.workbook_part,
            )?
            .entries()
            .iter()
            .filter(|relationship| {
                matches!(
                    relationship.kind,
                    RelationshipKind::Worksheet
                        | RelationshipKind::Chartsheet
                        | RelationshipKind::Dialogsheet
                )
            })
            .filter_map(|relationship| relationship.target.clone())
            .filter(|part| !parts.contains(part))
            .flat_map(|part| [relationships_part_of(&part), part])
            .collect()
        } else {
            Vec::new()
        };
        // The calculation chain names cells of the sheets this write
        // replaced; a stale one makes Excel repair the file.
        let calc_chain = rewrites_any;
        for entry in &members {
            if entry.is_directory()
                || written.iter().any(|part| part == entry.name())
                || orphaned.iter().any(|part| part == entry.name())
            {
                continue;
            }
            if calc_chain && entry.name() == "xl/calcChain.xml" {
                continue;
            }
            let bytes = archive.read_member(entry.name())?;
            target.write_member_with(entry.name(), &bytes, Codec::Deflate)?;
        }
        Ok(())
    }

    /// Rewrite the workbook part, its relationships and the content types
    /// for the current sheet list, keeping everything else they state.
    fn splice_documents(
        &self,
        target: &ZipArchive,
        archive: &Arc<ZipArchive>,
        parts: &[SmolStr],
        strings_part: &str,
        styles_part: &str,
        rewrites_any: bool,
    ) -> Result<()> {
        let relationships_part = relationships_part_of(&self.workbook_part);
        let base = folder_of(&self.workbook_part);
        // Sheet relationships take ids past every id the part already holds.
        let existing = if archive.get_entry(&relationships_part)?.is_some() {
            Relationships::from_xml(
                &archive.read_member(&relationships_part)?,
                &self.workbook_part,
            )?
        } else {
            Relationships::default()
        };
        let mut next_id = existing
            .entries()
            .iter()
            .filter_map(|relationship| relationship.id.strip_prefix("rId")?.parse::<usize>().ok())
            .max()
            .unwrap_or(0)
            + 1;
        let mut ids: Vec<SmolStr> = Vec::with_capacity(self.slots.len());
        let mut relationship_fragment = String::new();
        for (slot, part) in self.slots.iter().zip(parts) {
            let id = format_smolstr!("rId{next_id}");
            next_id += 1;
            let kind = match slot.kind {
                SheetKind::Worksheet => package::WORKSHEET_RELATIONSHIP,
                SheetKind::Chartsheet => {
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet"
                }
                SheetKind::Dialogsheet => {
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/dialogsheet"
                }
            };
            relationship_fragment.push_str(&package::relationship_element(
                &id,
                kind,
                &relative_to(&base, part),
            ));
            ids.push(id);
        }
        // The shared parts are related only when this write puts them in
        // the package: a relationship to no part is a package to repair.
        if rewrites_any && existing.first_of(RelationshipKind::SharedStrings).is_none() {
            relationship_fragment.push_str(&package::relationship_element(
                &format_smolstr!("rId{next_id}"),
                package::SHARED_STRINGS_RELATIONSHIP,
                &relative_to(&base, strings_part),
            ));
            next_id += 1;
        }
        if rewrites_any && existing.first_of(RelationshipKind::Styles).is_none() {
            relationship_fragment.push_str(&package::relationship_element(
                &format_smolstr!("rId{next_id}"),
                package::STYLES_RELATIONSHIP,
                &relative_to(&base, styles_part),
            ));
        }
        let relationships = if archive.get_entry(&relationships_part)?.is_some() {
            package::rewrite(
                &archive.read_member(&relationships_part)?,
                &package::Rewrite {
                    skip: &|start| {
                        local_is(start, b"Relationship")
                            && attribute_is_kind(
                                start,
                                &["worksheet", "chartsheet", "dialogsheet", "calcChain"],
                            )
                    },
                    patch_count: &|_| None,
                    before_end: &|name| {
                        (name == b"Relationships").then(|| relationship_fragment.clone())
                    },
                    after_start: &|_| None,
                    before_start: &|_| None,
                    set_attribute: &|_| None,
                },
            )?
        } else {
            format!(
                "<Relationships xmlns=\"{}\">{relationship_fragment}</Relationships>",
                package::PACKAGE_RELATIONSHIPS_NAMESPACE
            )
            .into_bytes()
        };
        target.write_member_with(&relationships_part, &relationships, Codec::Deflate)?;

        let mut sheets_fragment = String::new();
        for (index, (slot, id)) in self.slots.iter().zip(&ids).enumerate() {
            sheets_fragment.push_str(&format!(
                "<sheet name=\"{}\" sheetId=\"{}\" r:id=\"{id}\"{}/>",
                escape_attribute(&slot.name),
                index + 1,
                match slot.state() {
                    SheetState::Visible => String::new(),
                    other => format!(" state=\"{}\"", other.as_str()),
                }
            ));
        }
        // The date system is the workbook's: `workbookPr` states it, set
        // on the element there or written before `sheets` where the part
        // has none.
        let workbook_bytes = archive.read_member(&self.workbook_part)?;
        let has_workbook_pr = package::has_element(&workbook_bytes, b"workbookPr")?;
        let date1904 = u8::from(self.system == DateSystem::Year1904).to_string();
        let workbook = package::rewrite(
            &workbook_bytes,
            &package::Rewrite {
                skip: &|start| local_is(start, b"sheet"),
                patch_count: &|_| None,
                before_end: &|name| (name == b"sheets").then(|| sheets_fragment.clone()),
                after_start: &|_| None,
                before_start: &|name| {
                    (name == b"sheets" && !has_workbook_pr && self.system == DateSystem::Year1904)
                        .then(|| "<workbookPr date1904=\"1\"/>".to_owned())
                },
                set_attribute: &|name| {
                    (name == b"workbookPr").then(|| ("date1904", date1904.clone()))
                },
            },
        )?;
        target.write_member_with(&self.workbook_part, &workbook, Codec::Deflate)?;

        let mut overrides = String::new();
        let existing_types = archive
            .get_entry(package::CONTENT_TYPES_PART)?
            .map(|_| archive.read_member(package::CONTENT_TYPES_PART))
            .transpose()?
            .unwrap_or_default();
        let existing_text = String::from_utf8_lossy(&existing_types).into_owned();
        for (slot, part) in self.slots.iter().zip(parts) {
            if slot.kind == SheetKind::Worksheet
                && !existing_text.contains(&format!("PartName=\"/{part}\""))
            {
                overrides.push_str(&package::override_element(
                    part,
                    package::WORKSHEET_CONTENT_TYPE,
                ));
            }
        }
        if rewrites_any && !existing_text.contains(&format!("PartName=\"/{strings_part}\"")) {
            overrides.push_str(&package::override_element(
                strings_part,
                package::SHARED_STRINGS_CONTENT_TYPE,
            ));
        }
        if rewrites_any && !existing_text.contains(&format!("PartName=\"/{styles_part}\"")) {
            overrides.push_str(&package::override_element(
                styles_part,
                package::STYLES_CONTENT_TYPE,
            ));
        }
        let dropped: Vec<String> = archive
            .entries()?
            .iter()
            .filter(|entry| {
                entry.name().starts_with("xl/worksheets/")
                    && !parts.iter().any(|part| part == entry.name())
                    || rewrites_any && entry.name() == "xl/calcChain.xml"
            })
            .map(|entry| format!("/{}", entry.name()))
            .collect();
        let content_types = if existing_types.is_empty() {
            // A package with no content types part gets one naming the
            // parts this package actually holds, wherever they are.
            let mut content = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Types xmlns=\"{}\">\
                 <Default Extension=\"rels\" ContentType=\"{}\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/>",
                package::CONTENT_TYPES_NAMESPACE,
                package::RELATIONSHIPS_CONTENT_TYPE
            );
            content.push_str(&package::override_element(
                &self.workbook_part,
                package::WORKBOOK_CONTENT_TYPE,
            ));
            for (slot, part) in self.slots.iter().zip(parts) {
                if slot.kind == SheetKind::Worksheet {
                    content.push_str(&package::override_element(
                        part,
                        package::WORKSHEET_CONTENT_TYPE,
                    ));
                }
            }
            if rewrites_any {
                content.push_str(&package::override_element(
                    strings_part,
                    package::SHARED_STRINGS_CONTENT_TYPE,
                ));
                content.push_str(&package::override_element(
                    styles_part,
                    package::STYLES_CONTENT_TYPE,
                ));
            }
            content.push_str("</Types>");
            content.into_bytes()
        } else {
            package::rewrite(
                &existing_types,
                &package::Rewrite {
                    skip: &|start| {
                        local_is(start, b"Override")
                            && start.attributes().flatten().any(|attribute| {
                                package::local_name(attribute.key.as_ref()) == b"PartName"
                                    && dropped
                                        .iter()
                                        .any(|part| part.as_bytes() == attribute.value.as_ref())
                            })
                    },
                    patch_count: &|_| None,
                    before_end: &|name| (name == b"Types").then(|| overrides.clone()),
                    after_start: &|_| None,
                    before_start: &|_| None,
                    set_attribute: &|_| None,
                },
            )?
        };
        target.write_member_with(package::CONTENT_TYPES_PART, &content_types, Codec::Deflate)?;
        Ok(())
    }
}

/// A worksheet written from a stream in place of the one held: the slot it
/// takes, and the part's stream once the style offset the crate's formats
/// take in this package is known.
pub(crate) type Replaced<'a> = (
    usize,
    Box<dyn FnOnce(u32) -> Result<Box<dyn Read + Send>> + 'a>,
);

/// A worksheet part with no cell.
const EMPTY_SHEET: &str = "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData/></worksheet>";

/// One `<sheet>` entry of the workbook part.
struct SheetEntry {
    name: SmolStr,
    rid: SmolStr,
    state: SheetState,
}

/// Read the workbook part: its sheets and its date system.
fn read_workbook(bytes: &[u8], part: &str) -> Result<(Vec<SheetEntry>, DateSystem)> {
    use quick_xml::events::Event;

    let mut reader = super::styles::reader(bytes);
    let mut buffer = Vec::new();
    let mut entries = Vec::new();
    let mut system = DateSystem::Year1900;
    loop {
        let position = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| package::codec_error(position, error.to_string()))?;
        match event {
            Event::Start(ref start) | Event::Empty(ref start) => {
                match package::local_name(start.name().as_ref()) {
                    b"sheet" => {
                        let name =
                            package::attribute(start, b"name", position)?.unwrap_or_default();
                        let rid = package::attribute(start, b"id", position)?.unwrap_or_default();
                        let state = SheetState::from_attribute(
                            package::attribute(start, b"state", position)?.as_deref(),
                        )
                        .map_err(|error| Error::InvalidRecord {
                            path: SmolStr::new(part),
                            reason: format_smolstr!("{error}"),
                        })?;
                        entries.push(SheetEntry {
                            name: SmolStr::new(name),
                            rid: SmolStr::new(rid),
                            state,
                        });
                    }
                    b"workbookPr" => {
                        if let Some(value) = package::attribute(start, b"date1904", position)? {
                            system = match value.trim() {
                                "1" | "true" => DateSystem::Year1904,
                                "0" | "false" => DateSystem::Year1900,
                                other => {
                                    return Err(Error::InvalidRecord {
                                        path: format_smolstr!("{part}#workbookPr/@date1904"),
                                        reason: format_smolstr!(
                                            "expected 1, true, 0 or false for date1904, got {other:?}"
                                        ),
                                    });
                                }
                            };
                        }
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok((entries, system))
}

/// Write a fresh workbook part for `slots`.
fn write_workbook<W: std::io::Write>(
    writer: &mut W,
    slots: &[Slot],
    system: DateSystem,
) -> Result<()> {
    write!(
        writer,
        "<workbook xmlns=\"{}\" xmlns:r=\"{}\"><workbookPr date1904=\"{}\"/><sheets>",
        super::NAMESPACE,
        super::RELATIONSHIPS_NAMESPACE,
        u8::from(system == DateSystem::Year1904)
    )?;
    for (index, slot) in slots.iter().enumerate() {
        write!(
            writer,
            "<sheet name=\"{}\" sheetId=\"{}\" r:id=\"rId{}\"",
            escape_attribute(&slot.name),
            index + 1,
            index + 1
        )?;
        if slot.state() != SheetState::Visible {
            write!(writer, " state=\"{}\"", slot.state().as_str())?;
        }
        write!(writer, "/>")?;
    }
    write!(writer, "</sheets>")?;
    if slots
        .iter()
        .filter_map(|slot| slot.parsed.get())
        .any(|sheet| sheet.cells().any(|cell| cell.formula().is_some()))
    {
        write!(writer, "<calcPr fullCalcOnLoad=\"1\"/>")?;
    }
    write!(writer, "</workbook>")?;
    Ok(())
}

/// The `.rels` part of `part`: `xl/_rels/workbook.xml.rels` for
/// `xl/workbook.xml`.
fn relationships_part_of(part: &str) -> SmolStr {
    match part.rsplit_once('/') {
        Some((folder, name)) => format_smolstr!("{folder}/_rels/{name}.rels"),
        None => format_smolstr!("_rels/{part}.rels"),
    }
}

/// The folder of a part, empty at the package root.
fn folder_of(part: &str) -> String {
    part.rsplit_once('/')
        .map_or(String::new(), |(folder, _)| folder.to_owned())
}

/// A part's name relative to `base`, as a relationship target spells it.
fn relative_to(base: &str, part: &str) -> String {
    if base.is_empty() {
        return part.to_owned();
    }
    match part
        .strip_prefix(base)
        .and_then(|rest| rest.strip_prefix('/'))
    {
        Some(relative) => relative.to_owned(),
        None => format!("/{part}"),
    }
}

/// Escape text for an attribute value.
fn escape_attribute(text: &str) -> String {
    let mut escaped = Vec::with_capacity(text.len());
    if crate::xml::write_attribute_text(&mut escaped, text).is_err() {
        return text.to_owned();
    }
    String::from_utf8(escaped).unwrap_or_else(|_| text.to_owned())
}

/// Whether a start tag's local name is `name`.
fn local_is(start: &quick_xml::events::BytesStart<'_>, name: &[u8]) -> bool {
    package::local_name(start.name().as_ref()) == name
}

/// Whether a `Relationship` start tag's `Type` ends in one of `kinds`.
fn attribute_is_kind(start: &quick_xml::events::BytesStart<'_>, kinds: &[&str]) -> bool {
    start.attributes().flatten().any(|attribute| {
        package::local_name(attribute.key.as_ref()) == b"Type"
            && kinds.iter().any(|kind| {
                std::str::from_utf8(&attribute.value)
                    .ok()
                    .and_then(|value| value.rsplit('/').next())
                    == Some(*kind)
            })
    })
}
