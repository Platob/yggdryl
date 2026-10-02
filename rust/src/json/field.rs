//! JSON documents read straight into the canonical values one field holds,
//! with no natural value built between the bytes and the row.
//!
//! [`FieldReader::compile`] plans a nested field once - each struct's names,
//! each sequence's item, each map's key and value, each leaf's datatype -
//! and [`FieldReader::read`] walks one document through the crate's one JSON
//! grammar ([`Cursor`]) along that plan: a struct's cells are written where
//! its row stores them, a leaf's token is read by the leaf's own value door
//! ([`dtype_canonical`](crate::value::dtype_canonical)), and nothing is
//! checked twice.
//!
//! It is the fast half of [`from_bytes_with_field`](super::from_bytes_with_field),
//! never a second meaning: a document it reads, it reads to exactly the value
//! that door answers, and a document it is not sure of - any refusal, any
//! shape the plan does not take - it answers `None` for, so the door reads it
//! and says what it says. The plan takes the shapes whose reading is local to
//! one value: structs, the five serie layouts, maps keyed by a leaf, and every
//! leaf whose value is its token's. A union, a run-end or dictionary
//! encoding, a variant, an interval and the null datatype are not planned,
//! and neither is a map that holds bytes, whose document is prepared whole.

use std::borrow::Cow;

use super::parser::{Cursor, Number, Token};
use crate::text::Limits;
use crate::{DataType, DataTypeKind, Field, Scalar};

/// The deepest nesting a plan takes: the contract refuses a value nested
/// past its own hard limit, which a plan this shallow never reaches.
const MAX_PLANNED_DEPTH: usize = 32;

/// A nested field planned for reading documents: `None` from
/// [`Self::compile`] where any part of it is not one the plan takes.
#[derive(Debug)]
pub(crate) struct FieldReader {
    root: Node,
    limits: Limits,
}

impl FieldReader {
    /// Plan reading documents as values of `field`: a nullable struct,
    /// serie layout or map, as the door a cast reads through holds - a
    /// required root, or a leaf one, reads `null` and text by rules of its
    /// own, which the door keeps.
    pub(crate) fn compile(field: &Field) -> Option<Self> {
        if !field.is_nullable() || !super::reads_json(field.dtype()) {
            return None;
        }
        Some(Self {
            root: Node::compile(field, 0)?,
            limits: Limits::default(),
        })
    }

    /// The canonical value `document` holds under the planned field, or
    /// `None` where the plan cannot answer for it - the document is then the
    /// door's to read or to refuse.
    ///
    /// `pool` lends the buffers a sequence's items are gathered in, so a
    /// caller reading many documents allocates them once.
    pub(crate) fn read(&self, document: &[u8], pool: &mut Pool) -> Option<Scalar> {
        if document.len() > self.limits.max_input_bytes() {
            return None;
        }
        let mut cursor = Cursor::new(document, self.limits);
        cursor.begin_document().ok()?;
        let mut walk = Walk {
            cursor: &mut cursor,
            pool,
        };
        let value = match walk.cursor.value(0).ok()? {
            // A nullable root reads `null` as absence; any other root
            // spelling - text included - is the door's.
            Token::Null => Scalar::Null,
            token => self.root.read(token, &mut walk, 0)?,
        };
        cursor.end_document().ok()?;
        Some(value)
    }

    /// Whether the planned root is a struct, whose documents read into one
    /// column per child ([`Self::read_cells`]).
    pub(crate) const fn reads_cells(&self) -> bool {
        matches!(self.root, Node::Struct { .. })
    }

    /// Read `document`, under a planned struct root, straight into `cells`:
    /// one column per child, each of which a row the plan answers for
    /// pushes one cell onto. `Some(true)` for such a row, `Some(false)` for
    /// a `null` one - nothing pushed - and `None` where the plan is not
    /// sure, every column as it was.
    pub(crate) fn read_cells(
        &self,
        document: &[u8],
        pool: &mut Pool,
        cells: &mut [Vec<Scalar>],
    ) -> Option<bool> {
        let Node::Struct { children } = &self.root else {
            return None;
        };
        if document.len() > self.limits.max_input_bytes() {
            return None;
        }
        let rows = cells.first().map(Vec::len);
        let mut cursor = Cursor::new(document, self.limits);
        let read = cursor.begin_document().ok().and_then(|()| {
            let mut walk = Walk {
                cursor: &mut cursor,
                pool,
            };
            match walk.cursor.value(0).ok()? {
                Token::Null => Some(false),
                Token::Object => {
                    read_object(children, &mut walk, 0, |index, value| {
                        cells[index].push(value)
                    })?;
                    Some(true)
                }
                _ => None,
            }
        });
        let read = read.filter(|_| cursor.end_document().is_ok());
        if read.is_none()
            && let Some(rows) = rows
        {
            for column in cells.iter_mut() {
                column.truncate(rows);
            }
        }
        read
    }

    /// Whether the planned root is a map, whose documents read into one run
    /// of entries ([`Self::read_entries`]).
    pub(crate) const fn reads_entries(&self) -> bool {
        matches!(self.root, Node::Map { .. })
    }

    /// Read `document`, under a planned map root, straight onto `entries`:
    /// `Some(Some(n))` for a row of `n` entries pushed, `Some(None)` for a
    /// `null` one - nothing pushed - and `None` where the plan is not sure,
    /// `entries` as it was.
    pub(crate) fn read_entries(
        &self,
        document: &[u8],
        pool: &mut Pool,
        entries: &mut Vec<(Scalar, Scalar)>,
    ) -> Option<Option<usize>> {
        let Node::Map {
            key, value, sorted, ..
        } = &self.root
        else {
            return None;
        };
        if document.len() > self.limits.max_input_bytes() {
            return None;
        }
        let held = entries.len();
        let mut cursor = Cursor::new(document, self.limits);
        let read = cursor.begin_document().ok().and_then(|()| {
            let mut walk = Walk {
                cursor: &mut cursor,
                pool,
            };
            match walk.cursor.value(0).ok()? {
                Token::Null => Some(None),
                Token::Object => {
                    read_entries(key, value, *sorted, &mut walk, 0, entries)?;
                    Some(Some(entries.len() - held))
                }
                _ => None,
            }
        });
        let read = read.filter(|_| cursor.end_document().is_ok());
        if read.is_none() {
            entries.truncate(held);
        }
        read
    }

    /// Whether the planned root is one of the five serie layouts, whose
    /// documents read into one run of items ([`Self::read_items`]).
    pub(crate) const fn reads_items(&self) -> bool {
        matches!(self.root, Node::Sequence { .. })
    }

    /// Read `document`, under a planned serie root, straight onto `items`:
    /// `Some(Some(n))` for a row of `n` items pushed, `Some(None)` for a
    /// `null` one - nothing pushed - and `None` where the plan is not sure,
    /// `items` as it was.
    pub(crate) fn read_items(
        &self,
        document: &[u8],
        pool: &mut Pool,
        items: &mut Vec<Scalar>,
    ) -> Option<Option<usize>> {
        let Node::Sequence { item, width, .. } = &self.root else {
            return None;
        };
        if document.len() > self.limits.max_input_bytes() {
            return None;
        }
        let held = items.len();
        let mut cursor = Cursor::new(document, self.limits);
        let read = cursor.begin_document().ok().and_then(|()| {
            let mut walk = Walk {
                cursor: &mut cursor,
                pool,
            };
            match walk.cursor.value(0).ok()? {
                Token::Null => Some(None),
                Token::Array => {
                    read_items(item, &mut walk, 0, items)?;
                    let count = items.len() - held;
                    width
                        .is_none_or(|width| width == count)
                        .then_some(Some(count))
                }
                _ => None,
            }
        });
        let read = read.filter(|_| cursor.end_document().is_ok());
        if read.is_none() {
            items.truncate(held);
        }
        read
    }
}

/// The buffers a sequence's items are gathered in before they move into
/// the row that stores them: one per nesting level in use, kept between
/// documents.
#[derive(Debug, Default)]
pub(crate) struct Pool(Vec<Vec<Scalar>>);

/// One document's walk: the cursor over its bytes and the pool its
/// sequences gather in.
struct Walk<'w, 'a> {
    cursor: &'w mut Cursor<'a>,
    pool: &'w mut Pool,
}

/// One planned level of the field.
#[derive(Debug)]
enum Node {
    /// A struct: its children by name, each with the cell an absent name
    /// holds, where the contract gives one.
    Struct { children: Children },
    /// One of the five serie layouts: the item, and the width a fixed
    /// layout requires.
    Sequence {
        dtype: DataType,
        item: Box<Slot>,
        width: Option<usize>,
    },
    /// A map keyed by a leaf: entries in the order of their names' text,
    /// or of their keys where the map sorts them.
    Map {
        dtype: DataType,
        key: DataType,
        value: Box<Slot>,
        sorted: bool,
    },
    /// A leaf whose value is its token's, read by the leaf's own door;
    /// `base64` where a document spells it as base64 text.
    Leaf { field: Field, base64: bool },
}

/// A planned child, item or value: what it reads, and whether it holds
/// absence.
#[derive(Debug)]
struct Slot {
    node: Node,
    nullable: bool,
}

/// A planned struct's children found by name: the few by a scan, the many
/// by a search over their names in order.
#[derive(Debug)]
struct Children {
    children: Box<[Child]>,
    /// The children's indices in the order of their names, where there are
    /// enough to search.
    by_name: Box<[usize]>,
}

impl Children {
    /// Past this many children a name is searched for rather than scanned.
    const SCANNED: usize = 16;

    fn new(children: Box<[Child]>) -> Self {
        let mut by_name = Vec::new();
        if children.len() > Self::SCANNED {
            by_name = (0..children.len()).collect();
            by_name.sort_by(|left, right| children[*left].name.cmp(&children[*right].name));
        }
        Self {
            children,
            by_name: by_name.into_boxed_slice(),
        }
    }

    /// The index of the child named exactly `name`.
    fn index_of(&self, name: &str) -> Option<usize> {
        if self.by_name.is_empty() {
            return self.children.iter().position(|child| *child.name == *name);
        }
        self.by_name
            .binary_search_by(|index| (*self.children[*index].name).cmp(name))
            .ok()
            .map(|found| self.by_name[found])
    }
}

/// A planned struct child.
#[derive(Debug)]
struct Child {
    name: Box<str>,
    slot: Slot,
    /// The cell a record that names no child holds: `None` where the
    /// contract refuses that absence, which the door then reports.
    absent: Option<Scalar>,
}

impl Node {
    /// Plan `field`, whose datatype the contract checks at `depth`.
    fn compile(field: &Field, depth: usize) -> Option<Self> {
        if depth > MAX_PLANNED_DEPTH {
            return None;
        }
        let dtype = field.dtype();
        Some(match dtype {
            DataType::Struct(fields) => Self::Struct {
                children: Children::new(
                    fields
                        .iter()
                        .map(|child| {
                            Some(Child {
                                name: child.name().into(),
                                slot: Slot::compile(child, depth + 1)?,
                                absent: crate::value::absent_record_cell(child, depth + 1).ok(),
                            })
                        })
                        .collect::<Option<_>>()?,
                ),
            },
            DataType::Serie(item)
            | DataType::SerieView(item)
            | DataType::LargeSerie(item)
            | DataType::LargeSerieView(item) => Self::Sequence {
                dtype: dtype.clone(),
                item: Box::new(Slot::compile(item, depth + 1)?),
                width: None,
            },
            DataType::FixedSizeSerie(item, width) => Self::Sequence {
                dtype: dtype.clone(),
                item: Box::new(Slot::compile(item, depth + 1)?),
                width: Some(usize::try_from(*width).ok()?),
            },
            // A map holding bytes is prepared whole before it is read, so
            // it is the door's.
            DataType::Map(_) | DataType::SortedMap(_)
                if !crate::text::typed::holds_byte_leaf(dtype) =>
            {
                let map = dtype.as_mapping()?;
                let [key, value] = map.entries().fields() else {
                    return None;
                };
                if key.is_nullable() || !is_planned_leaf(key.dtype()) {
                    return None;
                }
                Self::Map {
                    dtype: dtype.clone(),
                    key: key.dtype().clone(),
                    value: Box::new(Slot::compile(value, depth + 1)?),
                    sorted: map.keys_sorted(),
                }
            }
            leaf if is_planned_leaf(leaf) => Self::Leaf {
                field: field.clone(),
                base64: matches!(
                    leaf,
                    crate::bytes_dtypes!() | DataType::Geometry(_) | DataType::Geography(_)
                ),
            },
            _ => return None,
        })
    }

    /// The canonical value of the planned datatype `token` opens, reading
    /// the rest of it off `cursor`; `None` where the plan is not sure.
    fn read(&self, token: Token<'_>, walk: &mut Walk<'_, '_>, depth: usize) -> Option<Scalar> {
        match self {
            Self::Struct { children } => {
                let Token::Object = token else {
                    return None;
                };
                read_struct(children, walk, depth)
            }
            Self::Sequence { dtype, item, width } => {
                let Token::Array = token else {
                    return None;
                };
                let mut items = walk.pool.0.pop().unwrap_or_default();
                let read = read_items(item, walk, depth, &mut items).and_then(|()| {
                    width
                        .is_none_or(|width| width == items.len())
                        .then(|| dtype.declared_layout(Scalar::from_sequence(items.drain(..))))
                });
                items.clear();
                walk.pool.0.push(items);
                read
            }
            Self::Map {
                dtype,
                key,
                value,
                sorted,
            } => {
                let Token::Object = token else {
                    return None;
                };
                read_map(dtype, key, value, *sorted, walk, depth)
            }
            Self::Leaf { field, base64 } => {
                let value = match token {
                    Token::Bool(value) => Scalar::from(value),
                    Token::Number(number) => number_scalar(number),
                    Token::String(Cow::Borrowed(text)) => Scalar::from(text),
                    Token::String(Cow::Owned(text)) => Scalar::from(text),
                    Token::Null | Token::Array | Token::Object => return None,
                };
                let value = if *base64 {
                    crate::text::typed::base64_payload(value, field).ok()?
                } else {
                    value
                };
                crate::value::dtype_canonical(field.dtype(), value)
                    .ok()
                    .filter(|value| !value.is_null())
            }
        }
    }
}

impl Slot {
    fn compile(field: &Field, depth: usize) -> Option<Self> {
        Some(Self {
            node: Node::compile(field, depth)?,
            nullable: field.is_nullable(),
        })
    }

    /// A `null` is the absence a nullable slot holds and a required one
    /// refuses; anything else is the node's.
    fn read(&self, token: Token<'_>, walk: &mut Walk<'_, '_>, depth: usize) -> Option<Scalar> {
        match token {
            Token::Null => self.nullable.then_some(Scalar::Null),
            token => self.node.read(token, walk, depth),
        }
    }
}

/// A struct row from the object the cursor just opened: every cell written
/// where the row stores it, a name the struct does not have - or names
/// twice - leaving the row to the door.
fn read_struct(children: &Children, walk: &mut Walk<'_, '_>, depth: usize) -> Option<Scalar> {
    // A struct of no child builds no cell, but its object is still read.
    if children.children.is_empty() {
        read_object(children, walk, depth, |_, _| {})?;
        return Some(Scalar::from_sequence([]));
    }
    Scalar::try_build_sequence(children.children.len(), |cells| {
        read_object(children, walk, depth, |index, value| cells[index] = value).ok_or_else(unsure)
    })
    .ok()
}

/// The cells of a struct row from the object the cursor just opened, each
/// handed to `put` with its child's index - once per child, a name the
/// object does not state given the cell its absence holds - where the plan
/// is sure of all of them.
fn read_object(
    children: &Children,
    walk: &mut Walk<'_, '_>,
    depth: usize,
    mut put: impl FnMut(usize, Scalar),
) -> Option<()> {
    let mut stated = Stated::new(children.children.len());
    let mut first = true;
    while let Some((_, name)) = walk.cursor.next_key(first).ok()? {
        first = false;
        let index = children.index_of(&name)?;
        if !stated.insert(index) {
            return None;
        }
        let token = walk.cursor.value(depth + 1).ok()?;
        put(
            index,
            children.children[index].slot.read(token, walk, depth + 1)?,
        );
    }
    for (index, child) in children.children.iter().enumerate() {
        if !stated.contains(index) {
            put(index, child.absent.clone()?);
        }
    }
    Some(())
}

/// The items of the array the cursor just opened, gathered into `items`.
fn read_items(
    item: &Slot,
    walk: &mut Walk<'_, '_>,
    depth: usize,
    items: &mut Vec<Scalar>,
) -> Option<()> {
    let mut first = true;
    while walk.cursor.next_item(first).ok()? {
        first = false;
        let token = walk.cursor.value(depth + 1).ok()?;
        items.push(item.read(token, walk, depth + 1)?);
    }
    Some(())
}

/// A map from the object the cursor just opened, its entries in the order
/// [`read_entries`] gives them.
fn read_map(
    dtype: &DataType,
    key: &DataType,
    value: &Slot,
    sorted: bool,
    walk: &mut Walk<'_, '_>,
    depth: usize,
) -> Option<Scalar> {
    let mut entries = Vec::new();
    read_entries(key, value, sorted, walk, depth, &mut entries)?;
    // The one duplicate-key rule a mapping is held to.
    Scalar::from_mapping(entries)
        .ok()
        .map(|mapping| dtype.declared_layout(mapping))
}

/// The entries of the object the cursor just opened, pushed onto `entries`:
/// in the order of their names' text - the order a record holds them in -
/// or, where the map sorts its keys, of the keys they read as. Names or keys
/// that collide leave the map to the door, as does anything else the plan is
/// not sure of; what was pushed is the caller's to take back.
fn read_entries(
    key: &DataType,
    value: &Slot,
    sorted: bool,
    walk: &mut Walk<'_, '_>,
    depth: usize,
    entries: &mut Vec<(Scalar, Scalar)>,
) -> Option<()> {
    let start = entries.len();
    let mut first = true;
    while let Some((_, name)) = walk.cursor.next_key(first).ok()? {
        first = false;
        let token = walk.cursor.value(depth + 1).ok()?;
        let held = value.read(token, walk, depth + 1)?;
        entries.push((Scalar::from(name.as_ref()), held));
    }
    let row = &mut entries[start..];
    // A name is text, so the names order as their text does.
    row.sort_by(|left, right| left.0.as_str().cmp(&right.0.as_str()));
    if row.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return None;
    }
    for entry in row.iter_mut() {
        let name = std::mem::replace(&mut entry.0, Scalar::Null);
        entry.0 = if sorted {
            // A sorted map orders its entries by the keys the names read
            // as through the key's door, which read again as themselves.
            let read = key.scalar(name).ok()?;
            let again = crate::value::dtype_canonical(key, read.clone()).ok()?;
            // Equal and of one leaf, so a restatement stores what this
            // stores: equality alone spans leaves and widths.
            (again.id() == read.id() && again == read).then_some(read)?
        } else {
            crate::value::dtype_canonical(key, name).ok()?
        };
        if entry.0.is_null() {
            return None;
        }
    }
    if sorted {
        row.sort_by(|left, right| left.0.cmp(&right.0));
    }
    // The one duplicate-key rule a mapping is held to.
    crate::scalar::unique_keys(row).ok()
}

/// A JSON number as the value the grammar types it as.
fn number_scalar(number: Number) -> Scalar {
    match number {
        Number::UInt64(value) => Scalar::from(value),
        Number::Int64(value) => Scalar::from(value),
        Number::UInt128(value) => Scalar::from(value),
        Number::Int128(value) => Scalar::from(value),
        Number::Float(value) => Scalar::from(value),
    }
}

/// Whether a leaf datatype's value is its token's alone - nothing below it
/// to plan, no member to choose, no tuple to read - so its own door reads
/// it.
fn is_planned_leaf(dtype: &DataType) -> bool {
    match dtype.kind() {
        DataTypeKind::Boolean
        | DataTypeKind::Integer
        | DataTypeKind::Floating
        | DataTypeKind::Decimal
        | DataTypeKind::Text
        | DataTypeKind::Code
        | DataTypeKind::Enum
        | DataTypeKind::Bytes
        | DataTypeKind::Geospatial
        | DataTypeKind::Uuid => true,
        // An interval reads a tuple, which a token is not.
        DataTypeKind::Temporal => !matches!(dtype, DataType::Interval(_)),
        DataTypeKind::Null | DataTypeKind::Nested => false,
    }
}

/// The plan's word for a row it leaves to the door, inside a build that
/// speaks the crate's errors.
fn unsure() -> crate::Error {
    crate::Error::InvalidRecord {
        path: smol_str::SmolStr::new_static("$"),
        reason: smol_str::SmolStr::new_static("left to the value contract"),
    }
}

/// The children of one struct row its object has named so far: a bit per
/// child, on the stack for up to 256 children and on the heap past them.
struct Stated {
    inline: [u64; 4],
    spilled: Vec<u64>,
}

impl Stated {
    fn new(children: usize) -> Self {
        Self {
            inline: [0; 4],
            spilled: if children > 256 {
                vec![0; children.div_ceil(64)]
            } else {
                Vec::new()
            },
        }
    }

    fn words(&mut self) -> &mut [u64] {
        if self.spilled.is_empty() {
            &mut self.inline
        } else {
            &mut self.spilled
        }
    }

    /// Mark child `index` named, answering whether it was not already.
    fn insert(&mut self, index: usize) -> bool {
        let (word, bit) = (index / 64, 1_u64 << (index % 64));
        let words = self.words();
        let fresh = words[word] & bit == 0;
        words[word] |= bit;
        fresh
    }

    fn contains(&mut self, index: usize) -> bool {
        self.words()[index / 64] & (1_u64 << (index % 64)) != 0
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/json/field.rs` pins and a caller cannot reach.
    use crate::{Field, Scalar};

    /// Whether every part of `field` is one the reader plans.
    pub fn plans(field: &Field) -> bool {
        super::FieldReader::compile(field).is_some()
    }

    /// What the reader planned for `field` reads `document` as: `None`
    /// where it leaves the document to the door, or plans no reading.
    pub fn read(field: &Field, document: &[u8]) -> Option<Scalar> {
        super::FieldReader::compile(field)?.read(document, &mut super::Pool::default())
    }

    /// What the reader planned for a serie `field` pushes onto `items`:
    /// `Some(Some(n))` for `n` items, `Some(None)` for a `null` row, `None`
    /// where it leaves the document to the door.
    pub fn read_items(
        field: &Field,
        document: &[u8],
        items: &mut Vec<Scalar>,
    ) -> Option<Option<usize>> {
        let reader = super::FieldReader::compile(field)?;
        if !reader.reads_items() {
            return None;
        }
        reader.read_items(document, &mut super::Pool::default(), items)
    }

    /// What the reader planned for a map `field` pushes onto `entries`:
    /// `Some(Some(n))` for `n` entries, `Some(None)` for a `null` row, `None`
    /// where it leaves the document to the door.
    pub fn read_entries(
        field: &Field,
        document: &[u8],
        entries: &mut Vec<(Scalar, Scalar)>,
    ) -> Option<Option<usize>> {
        let reader = super::FieldReader::compile(field)?;
        if !reader.reads_entries() {
            return None;
        }
        reader.read_entries(document, &mut super::Pool::default(), entries)
    }

    /// What the reader planned for a struct `field` pushes onto `columns`
    /// for `document`: `Some(true)` one cell onto each, `Some(false)` for a
    /// `null` row, `None` where it leaves the document to the door.
    pub fn read_cells(field: &Field, document: &[u8], columns: &mut [Vec<Scalar>]) -> Option<bool> {
        let reader = super::FieldReader::compile(field)?;
        if !reader.reads_cells() {
            return None;
        }
        reader.read_cells(document, &mut super::Pool::default(), columns)
    }
}
