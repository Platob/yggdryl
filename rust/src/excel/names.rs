//! The workbook's defined names: `Print_Area`, a named constant, a named
//! range - what the workbook part's `<definedNames>` lists.
//!
//! A [`DefinedName`] is read once, its formula a [`Formula`] held at `A1`
//! as Excel spells a name's references, its scope the [`SheetKey`] of the
//! sheet a `localSheetId` names. A name is written back as the file wrote it
//! until something touches it - a sheet renamed or removed that its formula
//! names, a sheet it is scoped to removed, a tab moved under its scope -
//! and then the list is written again from the model, each name keeping
//! every attribute it was read with.

use std::sync::Arc;

use smol_str::SmolStr;

use crate::Result;

use super::cell::CellRef;
use super::formula::Formula;
use super::workbook::SheetKey;

/// One defined name of a workbook.
///
/// ```
/// use yggdryl::excel::Workbook;
///
/// let workbook = Workbook::new();
/// assert_eq!(workbook.defined_names().count(), 0);
/// ```
#[derive(Clone, Debug)]
pub struct DefinedName {
    name: SmolStr,
    scope: Option<SheetKey>,
    formula: Formula,
    hidden: bool,
    comment: Option<SmolStr>,
    /// The `<definedName>` as read, which is written back while the name
    /// states what it was read with.
    raw: Arc<[u8]>,
    /// The formula and the scope's tab the element was read with.
    read: (Formula, Option<usize>),
}

impl PartialEq for DefinedName {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.scope == other.scope
            && self.formula == other.formula
            && self.hidden == other.hidden
            && self.comment == other.comment
    }
}

impl DefinedName {
    /// A name read from the workbook part: its element as written, its
    /// attributes, its text and its scope's tab and key.
    pub(crate) fn read(
        raw: Arc<[u8]>,
        name: SmolStr,
        text: &str,
        scope: Option<(usize, SheetKey)>,
        hidden: bool,
        comment: Option<SmolStr>,
    ) -> Self {
        let formula = Formula::from_file(text, CellRef::new(0, 0));
        Self {
            name,
            scope: scope.map(|(_, key)| key),
            read: (formula.clone(), scope.map(|(tab, _)| tab)),
            formula,
            hidden,
            comment,
            raw,
        }
    }

    /// The name: `Rate`, `_xlnm.Print_Area`.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The sheet the name is defined on, `None` for a name of the workbook.
    #[must_use]
    pub const fn scope(&self) -> Option<SheetKey> {
        self.scope
    }

    /// What the name stands for, held at `A1`: `Sheet1!$A$1:$C$9`, `0.07`.
    #[must_use]
    pub const fn formula(&self) -> &Formula {
        &self.formula
    }

    /// The name's text as the file spells it.
    #[must_use]
    pub fn text(&self) -> String {
        self.formula.at(CellRef::new(0, 0)).to_string()
    }

    /// Whether the name is hidden from Excel's name manager.
    #[must_use]
    pub const fn is_hidden(&self) -> bool {
        self.hidden
    }

    /// The comment the name carries.
    #[must_use]
    pub fn comment(&self) -> Option<&str> {
        self.comment.as_deref()
    }

    /// Put `formula` in the name.
    pub(crate) fn set_formula(&mut self, formula: Formula) {
        self.formula = formula;
    }

    /// Define the name on the sheet `key`: the one now in the tab it was
    /// defined on.
    pub(crate) fn set_scope(&mut self, key: SheetKey) {
        self.scope = Some(key);
    }

    /// The `<definedName>` for the name scoped to tab `tab`: the element as
    /// read while it states what it was read with, else that element with
    /// its `localSheetId` and its text restated.
    ///
    /// # Errors
    ///
    /// Returns the codec's refusal of an element that is not well-formed.
    pub(crate) fn element(&self, tab: Option<usize>) -> Result<Vec<u8>> {
        if self.formula == self.read.0 && tab == self.read.1 {
            return Ok(self.raw.to_vec());
        }
        let text = self.text();
        super::package::rewrite(
            &self.raw,
            &super::package::Rewrite {
                attributes: &|name| {
                    if name == b"definedName" {
                        vec![("localSheetId", tab.map(|tab| tab.to_string()))]
                    } else {
                        Vec::new()
                    }
                },
                text: Some(&|name, _| (name == b"definedName").then(|| text.clone())),
                ..super::package::Rewrite::default()
            },
        )
    }
}
