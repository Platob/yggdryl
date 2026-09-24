//! The graph's Arrow rows: one operation shape for every input a book
//! takes, and one book shape, encoded and decoded by the same fields.
//!
//! An operation row is `operationkind` - `order`, `quote`, `execution`,
//! `trade` or `snapshot` - then the sixteen [`EventColumn`]s, the nineteen
//! [`MarketColumn`]s, the eight [`OperationColumn`]s, the five book-control
//! columns a market-data entry states, and a nullable `executions` list a
//! trade fills. A book row is the event and market columns, a `bid` and an
//! `ask` side, its `executions`, and the `snapshotpartitions` its last
//! replacement covered. A side is the six element facts, the market
//! columns, and its `live` and `deltas` operations. Every nested row's
//! stated identities must equal the ones re-derived from its content.

use std::iter::FusedIterator;
use std::sync::Arc;

use arrow_schema::SchemaRef;
use smol_str::{SmolStr, format_smolstr};

use super::book::{BookControl, BookInput, SnapshotPartition};
use super::{
    Book, BookRef, BookSide, Element, Event, EventColumn, Market, MarketColumn, MarketData,
    MarketEventData, MarketOperation, MarketOperationEventData, MdUpdateAction, Operation,
    OperationColumn, OperationKind, Trade,
};
use crate::arrow::BatchReader;
use crate::serie::{Proof, land_batch};
use crate::{DataType, Decimal18, Error, Field, Result, Scalar, Serie, StructType, Uuid};

const KIND: &str = "operationkind";
const OPERATION_ROOT: &str = "marketoperation";
const BOOK_ROOT: &str = "book";
const KIND_COLUMNS: usize = 1;
const SIDE_ELEMENT_COLUMNS: [EventColumn; 6] = [
    EventColumn::CurrUuid,
    EventColumn::CrossUuid,
    EventColumn::CrossCode,
    EventColumn::CurrHashCode,
    EventColumn::CrossHashCode,
    EventColumn::SrcUuids,
];
/// The five book-control columns, in row order.
const BOOK_COLUMNS: [&str; 5] = [
    "mdupdateaction",
    "bookscope",
    "mdentrypositionno",
    "mdentrypx",
    "mdentrysize",
];
const SNAPSHOT_KIND: &str = "snapshot";

impl From<BookInput> for Result<BookInput> {
    fn from(input: BookInput) -> Self {
        Ok(input)
    }
}

impl From<Operation> for Result<BookInput> {
    fn from(operation: Operation) -> Self {
        Ok(BookInput::Operation(operation))
    }
}

impl BookInput {
    /// The canonical Arrow row field for every input a book takes.
    ///
    /// Its first child is `operationkind`, followed by [`EventColumn::ALL`],
    /// [`MarketColumn::ALL`], [`OperationColumn::ALL`], the five book-control
    /// columns and a nullable `executions` list, non-null only when
    /// `operationkind` is `trade`.
    ///
    /// # Errors
    ///
    /// Returns an error when a field cannot be built.
    pub fn field() -> Result<Field> {
        operation_field()
    }

    /// Streams book inputs into bounded Arrow record batches.
    ///
    /// The source is not pulled until the returned reader is pulled. A `None`
    /// row size uses the crate default and zero normalizes to one; a stated
    /// byte size closes a nonempty batch once the rows already in it reach the
    /// bound, so one oversized row is still emitted alone.
    ///
    /// # Errors
    ///
    /// Returns an error when the row field cannot be built.
    pub fn arrow_reader<I>(
        inputs: I,
        batch_row_size: Option<usize>,
        batch_byte_size: Option<u64>,
    ) -> Result<BatchReader>
    where
        I: IntoIterator,
        I::Item: Into<Result<BookInput>>,
        I::IntoIter: Send + 'static,
    {
        let field = operation_field()?;
        let rows = inputs.into_iter().enumerate().map(|(ordinal, input)| {
            input
                .into()
                .and_then(|input| checked_operation_row(&input, ordinal as u64))
        });
        crate::arrow::rows::result_reader(&field, rows, batch_row_size, batch_byte_size, None, None)
            .map_err(Into::into)
    }

    /// Decodes book inputs from canonical operation record batches, one
    /// input per row, validating every stated identity against the one its
    /// content derives.
    ///
    /// # Errors
    ///
    /// Returns an error when the schema is not the canonical operation
    /// schema, or a row's identities or facts do not derive from its content.
    pub fn from_arrow_reader(
        batches: BatchReader,
    ) -> Result<impl FusedIterator<Item = Result<BookInput>> + Send + 'static> {
        let schema = batches.schema();
        let intake = Intake::resolve(&schema)?;
        let root = Arc::clone(&intake.root);
        Ok(landed_rows(
            batches,
            schema,
            move |batch, row, ordinal| intake.input_of(batch, row, ordinal),
            root,
        ))
    }
}

impl Book {
    /// The canonical nested Arrow row field for a book.
    ///
    /// # Errors
    ///
    /// Returns an error when a field cannot be built.
    pub fn field() -> Result<Field> {
        book_field()
    }

    /// Streams books into bounded Arrow record batches; see
    /// [`BookInput::arrow_reader`] for the batch bounds.
    ///
    /// # Errors
    ///
    /// Returns an error when the row field cannot be built.
    pub fn arrow_reader<I>(
        books: I,
        batch_row_size: Option<usize>,
        batch_byte_size: Option<u64>,
    ) -> Result<BatchReader>
    where
        I: IntoIterator,
        I::Item: Into<Result<Book>>,
        I::IntoIter: Send + 'static,
    {
        let field = book_field()?;
        let rows = books.into_iter().enumerate().map(|(ordinal, book)| {
            book.into()
                .and_then(|book| checked_book_row(&book, ordinal as u64))
        });
        crate::arrow::rows::result_reader(&field, rows, batch_row_size, batch_byte_size, None, None)
            .map_err(Into::into)
    }

    /// Decodes books from canonical book record batches, validating every
    /// stated identity against the one its content derives.
    ///
    /// # Errors
    ///
    /// Returns an error when the schema is not the canonical book schema, or
    /// a row's identities or facts do not derive from its content.
    pub fn from_arrow_reader(
        batches: BatchReader,
    ) -> Result<impl FusedIterator<Item = Result<Book>> + Send + 'static> {
        let schema = batches.schema();
        let intake = BookIntake::resolve(&schema)?;
        let root = Arc::clone(&intake.root);
        Ok(landed_rows(
            batches,
            schema,
            move |batch, row, ordinal| intake.book_of(batch, row, ordinal),
            root,
        ))
    }
}

/// Every row of every batch, landed once per batch and read by `read`; a
/// batch of another schema, a landing refusal or a row refusal ends the
/// stream with that error.
fn landed_rows<T, F>(
    mut batches: BatchReader,
    schema: SchemaRef,
    read: F,
    root: Arc<Field>,
) -> impl FusedIterator<Item = Result<T>> + Send + 'static
where
    T: Send + 'static,
    F: Fn(&Serie, usize, u64) -> Result<T> + Send + 'static,
{
    let mut pending: Option<(Serie, usize)> = None;
    let mut ordinal = 0_u64;
    let mut done = false;
    std::iter::from_fn(move || {
        if done {
            return None;
        }
        loop {
            if let Some((batch, row)) = pending.as_mut() {
                if *row < batch.len() {
                    let result = read(batch, *row, ordinal);
                    *row += 1;
                    ordinal += 1;
                    if result.is_err() {
                        done = true;
                        pending = None;
                    }
                    return Some(result);
                }
            }
            pending = None;
            let batch = match batches.next() {
                Some(Ok(batch)) => batch,
                Some(Err(error)) => {
                    done = true;
                    return Some(Err(crate::arrow::from_reader_error(error).into()));
                }
                None => {
                    done = true;
                    return None;
                }
            };
            if batch.schema_ref() != &schema {
                done = true;
                return Some(Err(invalid(
                    format_smolstr!("$[{ordinal}]"),
                    "expected the reader's declared Arrow schema, got a different batch schema",
                )));
            }
            // The batch lands once, proving what its layout does not; a
            // row it refuses is named by the batch's first ordinal and
            // its own row.
            match land_batch(&root, &batch, &Proof::Unproven) {
                Ok(records) => pending = Some((records, 0)),
                Err(error) => {
                    done = true;
                    return Some(Err(invalid(
                        format_smolstr!("$[{ordinal}]"),
                        format_smolstr!("{error}"),
                    )));
                }
            }
        }
    })
    .fuse()
}

impl From<Book> for Result<Book> {
    fn from(book: Book) -> Self {
        Ok(book)
    }
}

/// The canonical row field shared by the encoder and decoder.
fn operation_field() -> Result<Field> {
    let mut fields = Vec::with_capacity(
        KIND_COLUMNS
            + EventColumn::ALL.len()
            + MarketColumn::ALL.len()
            + OperationColumn::ALL.len()
            + BOOK_COLUMNS.len()
            + 1,
    );
    let mut kind = DataType::utf8().required_field(KIND);
    kind.set_display("Operation Kind")?;
    fields.push(kind);
    fields.extend(EventColumn::fields()?);
    fields.extend(MarketColumn::fields()?);
    fields.extend(OperationColumn::fields()?);
    fields.extend(book_control_fields()?);
    fields.push(DataType::serie(execution_field()?).nullable_field("executions"));
    Ok(DataType::from(StructType::from_fields(fields)?).required_field(OPERATION_ROOT))
}

/// The row of one execution inside a trade or a book: every operation
/// column but the kind, which is known, and the executions, which it has
/// none of.
fn execution_field() -> Result<Field> {
    let mut fields = Vec::with_capacity(
        EventColumn::ALL.len()
            + MarketColumn::ALL.len()
            + OperationColumn::ALL.len()
            + BOOK_COLUMNS.len(),
    );
    fields.extend(EventColumn::fields()?);
    fields.extend(MarketColumn::fields()?);
    fields.extend(OperationColumn::fields()?);
    fields.extend(book_control_fields()?);
    Ok(DataType::from(StructType::from_fields(fields)?).required_field("execution"))
}

fn book_control_fields() -> Result<Vec<Field>> {
    let mut fields = vec![
        DataType::utf8().nullable_field(BOOK_COLUMNS[0]),
        DataType::utf8().nullable_field(BOOK_COLUMNS[1]),
        DataType::UInt32.nullable_field(BOOK_COLUMNS[2]),
        DataType::DECIMAL.nullable_field(BOOK_COLUMNS[3]),
        DataType::DECIMAL.nullable_field(BOOK_COLUMNS[4]),
    ];
    for (field, display) in fields.iter_mut().zip([
        "MD Update Action",
        "Book Scope",
        "MD Entry Position",
        "MD Entry Price",
        "MD Entry Size",
    ]) {
        field.set_display(display)?;
    }
    Ok(fields)
}

fn snapshot_partitions_field() -> Result<Field> {
    let entry = DataType::from(StructType::from_fields(vec![
        DataType::utf8().nullable_field("symbol"),
        DataType::utf8().required_field("scope"),
    ])?)
    .required_field("snapshotpartition");
    Ok(DataType::serie(entry).nullable_field("snapshotpartitions"))
}

/// The nested book row field shared by both directions.
fn book_field() -> Result<Field> {
    let mut fields = Vec::with_capacity(EventColumn::ALL.len() + MarketColumn::ALL.len() + 4);
    fields.extend(EventColumn::fields()?);
    fields.extend(MarketColumn::fields()?);
    fields.push(side_field("bid")?);
    fields.push(side_field("ask")?);
    fields.push(DataType::serie(execution_field()?).required_field("executions"));
    fields.push(snapshot_partitions_field()?);
    Ok(DataType::from(StructType::from_fields(fields)?).required_field(BOOK_ROOT))
}

fn side_field(name: &'static str) -> Result<Field> {
    let mut fields = Vec::with_capacity(SIDE_ELEMENT_COLUMNS.len() + MarketColumn::ALL.len() + 2);
    fields.extend(
        SIDE_ELEMENT_COLUMNS
            .into_iter()
            .map(EventColumn::field)
            .collect::<Result<Vec<_>>>()?,
    );
    fields.extend(MarketColumn::fields()?);
    fields.push(DataType::serie(operation_field()?).required_field("live"));
    fields.push(DataType::serie(operation_field()?).required_field("deltas"));
    Ok(DataType::from(StructType::from_fields(fields)?).required_field(name))
}

fn book_control_facts(book: Option<&BookRef>) -> [Scalar; 5] {
    let Some(book) = book else {
        return [
            Scalar::Null,
            Scalar::Null,
            Scalar::Null,
            Scalar::Null,
            Scalar::Null,
        ];
    };
    [
        book.action
            .map_or(Scalar::Null, |action| Scalar::from(action.as_str())),
        book.scope.as_deref().map_or(Scalar::Null, Scalar::from),
        book.position.map_or(Scalar::Null, Scalar::from),
        book.entry_px.map_or(Scalar::Null, Scalar::from),
        book.entry_size.map_or(Scalar::Null, Scalar::from),
    ]
}

fn book_control_of(cells: &[Scalar]) -> Option<BookRef> {
    let book = BookRef {
        action: cells[0].as_str().and_then(MdUpdateAction::read),
        scope: cells[1].as_str().map(SmolStr::new),
        position: cells[2].as_u64().and_then(|held| u32::try_from(held).ok()),
        entry_px: Decimal18::from_scalar(&cells[3]),
        entry_size: Decimal18::from_scalar(&cells[4]),
    };
    book.is_stated().then_some(book)
}

fn kind_of(input: &BookInput) -> &'static str {
    match input {
        BookInput::Operation(operation) => operation.kind().as_str(),
        BookInput::Trade(_) => OperationKind::Trade.as_str(),
        BookInput::Snapshot(_) => SNAPSHOT_KIND,
    }
}

/// The market and operation facts of one input: an operation's or a trade
/// root's own, and for a snapshot control the event's market facts with no
/// operation fact.
fn operation_columns(input: &BookInput) -> Vec<Scalar> {
    let mut cells = Vec::with_capacity(MarketColumn::ALL.len() + OperationColumn::ALL.len());
    match input {
        BookInput::Operation(operation) => {
            cells.extend(market_cells(operation.data()));
            cells.extend(operation_cells(operation.data()));
        }
        BookInput::Trade(trade) => {
            cells.extend(market_cells(trade.data()));
            cells.extend(operation_cells(trade.data()));
        }
        BookInput::Snapshot(control) => {
            cells.extend(market_cells(&control.event));
            cells.extend(OperationColumn::ALL.iter().map(|_| Scalar::Null));
        }
    }
    cells
}

fn event_cells(event: &(impl Event + ?Sized)) -> impl Iterator<Item = Scalar> + '_ {
    EventColumn::ALL
        .into_iter()
        .map(move |column| column.fact(event).unwrap_or(Scalar::Null))
}

fn market_cells(market: &(impl Market + ?Sized)) -> impl Iterator<Item = Scalar> + '_ {
    MarketColumn::ALL
        .into_iter()
        .map(move |column| column.fact(market).unwrap_or(Scalar::Null))
}

fn operation_cells(
    operation: &(impl MarketOperation + ?Sized),
) -> impl Iterator<Item = Scalar> + '_ {
    OperationColumn::ALL
        .into_iter()
        .map(move |column| column.fact(operation).unwrap_or(Scalar::Null))
}

fn row_of(input: &BookInput) -> Scalar {
    let executions = match input {
        BookInput::Trade(trade) => {
            Scalar::from_sequence(trade.executions().iter().map(execution_row))
        }
        _ => Scalar::Null,
    };
    Scalar::from_sequence(
        std::iter::once(Scalar::from(kind_of(input)))
            .chain(event_cells(input.event()))
            .chain(operation_columns(input))
            .chain(book_control_facts(input.book()))
            .chain([executions]),
    )
}

fn execution_row(operation: &Operation) -> Scalar {
    Scalar::from_sequence(
        event_cells(operation)
            .chain(market_cells(operation))
            .chain(operation_cells(operation))
            .chain(book_control_facts(operation.book())),
    )
}

fn checked_operation_row(input: &BookInput, ordinal: u64) -> Result<Scalar> {
    match input {
        BookInput::Operation(operation) => {
            validate_operation_for_write(operation, |name| format_smolstr!("$[{ordinal}].{name}"))?;
        }
        BookInput::Trade(trade) => validate_trade_for_write(trade, ordinal)?,
        BookInput::Snapshot(control) => {
            validate_control_for_write(control, |name| format_smolstr!("$[{ordinal}].{name}"))?;
        }
    }
    Ok(row_of(input))
}

fn checked_book_row(book: &Book, ordinal: u64) -> Result<Scalar> {
    for (name, side) in [("bid", book.bid()), ("ask", book.ask())] {
        for (index, operation) in side.live().enumerate() {
            validate_operation_for_write(operation, |field| {
                NestedSerie::SideLive(name).field_path(ordinal, index, field)
            })?;
        }
        for (index, operation) in side.deltas().iter().enumerate() {
            validate_operation_for_write(operation, |field| {
                NestedSerie::SideDeltas(name).field_path(ordinal, index, field)
            })?;
        }
    }
    for (index, execution) in book.executions().iter().enumerate() {
        validate_operation_for_write(execution, |field| {
            NestedSerie::Executions.field_path(ordinal, index, field)
        })?;
    }
    validate_book_parts_for_write(book, ordinal)?;
    let bid = validate_side_for_write(book.bid(), ordinal, "bid")?;
    let ask = validate_side_for_write(book.ask(), ordinal, "ask")?;
    let canonical = book.canonical_event(&bid, &ask);
    let mut stated: MarketEventData = book.as_ref().clone();
    IdentityClaims::from_element(book)
        .validate(&canonical, |name| format_smolstr!("$[{ordinal}].{name}"))?;
    normalize_identity(&mut stated, &canonical);
    validate_market_event(&stated, &canonical, |name| {
        format_smolstr!("$[{ordinal}].{name}")
    })?;
    Ok(book_row(book))
}

/// An operation is written only as its canonical self: the identities it
/// states must be the ones its content derives, and its facts must be the
/// ones its finalize settles on.
fn validate_operation_for_write<P>(operation: &Operation, path: P) -> Result<()>
where
    P: Fn(&str) -> SmolStr + Copy,
{
    let mut canonical = operation.clone();
    canonical.finalize();
    IdentityClaims::from_element(operation).validate(&canonical, path)?;
    let mut stated = operation.data().clone();
    normalize_identity(&mut stated, canonical.data());
    validate_operation_event(&stated, canonical.data(), path)
}

fn validate_control_for_write<P>(control: &BookControl, path: P) -> Result<()>
where
    P: Fn(&str) -> SmolStr + Copy,
{
    let mut canonical = control.event.clone();
    canonical.finalize();
    IdentityClaims::from_element(&control.event).validate(&canonical, path)?;
    let mut stated = control.event.clone();
    normalize_identity(&mut stated, &canonical);
    validate_market_event(&stated, &canonical, path)
}

fn validate_trade_for_write(trade: &Trade, ordinal: u64) -> Result<()> {
    for (index, execution) in trade.executions().iter().enumerate() {
        validate_operation_for_write(execution, |name| {
            format_smolstr!("$[{ordinal}].executions[{index}].{name}")
        })?;
    }
    let canonical = Trade::from_parts(trade.data().clone(), trade.executions().to_vec())
        .map_err(|error| prefix_invalid(error, || format_smolstr!("$[{ordinal}]")))?;
    IdentityClaims::from_element(trade)
        .validate(&canonical, |name| format_smolstr!("$[{ordinal}].{name}"))?;
    let mut stated = trade.data().clone();
    normalize_identity(&mut stated, canonical.data());
    validate_operation_event(&stated, canonical.data(), |name| {
        format_smolstr!("$[{ordinal}].{name}")
    })
}

fn validate_side_for_write(
    stated_side: &BookSide,
    ordinal: u64,
    name: &'static str,
) -> Result<MarketData> {
    let canonical = stated_side
        .canonical_element()
        .map_err(|error| prefix_invalid(error, || format_smolstr!("$[{ordinal}].{name}")))?;
    IdentityClaims::from_element(stated_side)
        .validate(&canonical, |field| side_field_path(ordinal, name, field))?;
    let mut stated: MarketData = stated_side.as_ref().clone();
    normalize_identity(&mut stated, &canonical);
    validate_market_element(&stated, &canonical, |field| {
        side_field_path(ordinal, name, field)
    })?;
    Ok(canonical)
}

fn validate_book_parts_for_write(book: &Book, ordinal: u64) -> Result<()> {
    book.validate_parts()
        .map_err(|error| prefix_invalid(error, || format_smolstr!("$[{ordinal}]")))
}

fn book_row(book: &Book) -> Scalar {
    let partitions = book.snapshot_partitions();
    let partitions = if partitions.is_empty() {
        Scalar::Null
    } else {
        Scalar::from_sequence(partitions.iter().map(|partition| {
            Scalar::from_sequence([
                partition
                    .symbol
                    .as_deref()
                    .map_or(Scalar::Null, Scalar::from),
                Scalar::from(partition.scope.as_str()),
            ])
        }))
    };
    Scalar::from_sequence(event_cells(book).chain(market_cells(book)).chain([
        side_row(book.bid()),
        side_row(book.ask()),
        Scalar::from_sequence(book.executions().iter().map(execution_row)),
        partitions,
    ]))
}

fn side_row(side: &BookSide) -> Scalar {
    Scalar::from_sequence(
        SIDE_ELEMENT_COLUMNS
            .into_iter()
            .map(|column| element_fact(column, side).unwrap_or(Scalar::Null))
            .chain(market_cells(side))
            .chain([
                Scalar::from_sequence(
                    side.live()
                        .map(|operation| row_of(&BookInput::Operation(operation.clone()))),
                ),
                Scalar::from_sequence(
                    side.deltas()
                        .iter()
                        .map(|operation| row_of(&BookInput::Operation(operation.clone()))),
                ),
            ]),
    )
}

fn element_fact(column: EventColumn, element: &(impl Element + ?Sized)) -> Option<Scalar> {
    match column {
        EventColumn::CurrUuid => Some(Scalar::Uuid(element.get_curruuid())),
        EventColumn::CrossUuid => Some(Scalar::Uuid(element.get_crossuuid())),
        EventColumn::CrossCode => {
            let code = element.get_crosscode();
            (!code.is_empty()).then(|| Scalar::from(code))
        }
        EventColumn::CurrHashCode => Some(Scalar::from(element.get_currhashcode())),
        EventColumn::CrossHashCode => Some(Scalar::from(element.get_crosshashcode())),
        EventColumn::SrcUuids => (!element.get_srcuuids().is_empty()).then(|| {
            Scalar::from_sequence(element.get_srcuuids().iter().copied().map(Scalar::Uuid))
        }),
        _ => unreachable!("SIDE_ELEMENT_COLUMNS contains only element facts"),
    }
}

#[derive(Clone, Copy, Default)]
struct IdentityClaims {
    curruuid: Option<Uuid>,
    crossuuid: Option<Uuid>,
    currhashcode: Option<u64>,
    crosshashcode: Option<u64>,
}

impl IdentityClaims {
    fn from_element(element: &(impl Element + ?Sized)) -> Self {
        Self {
            curruuid: Some(element.get_curruuid()),
            crossuuid: Some(element.get_crossuuid()),
            currhashcode: Some(element.get_currhashcode()),
            crosshashcode: Some(element.get_crosshashcode()),
        }
    }

    fn record(&mut self, column: EventColumn, value: &Scalar) {
        match (column, value) {
            (EventColumn::CurrUuid, Scalar::Uuid(value)) => self.curruuid = Some(*value),
            (EventColumn::CrossUuid, Scalar::Uuid(value)) => self.crossuuid = Some(*value),
            (EventColumn::CurrHashCode, value) => self.currhashcode = value.as_u64(),
            (EventColumn::CrossHashCode, value) => self.crosshashcode = value.as_u64(),
            _ => {}
        }
    }

    fn validate<E: Element + ?Sized>(
        self,
        canonical: &E,
        path: impl Fn(&str) -> SmolStr,
    ) -> Result<()> {
        macro_rules! validate {
            ($claim:ident, $actual:expr, $name:literal) => {
                match self.$claim {
                    Some(stated) if stated == $actual => {}
                    Some(stated) => {
                        return Err(invalid(
                            path($name),
                            format_smolstr!(
                                "expected the value derived from the row {:?}, got {stated:?}",
                                $actual
                            ),
                        ));
                    }
                    None => {
                        return Err(invalid(path($name), "expected a non-null identity fact"));
                    }
                }
            };
        }
        validate!(curruuid, canonical.get_curruuid(), "curruuid");
        validate!(crossuuid, canonical.get_crossuuid(), "crossuuid");
        validate!(currhashcode, canonical.get_currhashcode(), "currhashcode");
        validate!(
            crosshashcode,
            canonical.get_crosshashcode(),
            "crosshashcode"
        );
        Ok(())
    }
}

fn normalize_identity<E: Element + ?Sized, C: Element + ?Sized>(value: &mut E, canonical: &C) {
    value.set_curruuid(canonical.get_curruuid());
    value.set_crossuuid(canonical.get_crossuuid());
    value.set_currhashcode(canonical.get_currhashcode());
    value.set_crosshashcode(canonical.get_crosshashcode());
}

fn validate_market_event(
    stated: &MarketEventData,
    canonical: &MarketEventData,
    path: impl Fn(&str) -> SmolStr,
) -> Result<()> {
    if stated == canonical {
        return Ok(());
    }
    for column in MarketColumn::ALL {
        let stated = column.fact(stated);
        let derived = column.fact(canonical);
        if stated != derived {
            return Err(invalid(
                path(column.name()),
                format_smolstr!(
                    "expected the value derived from the row {derived:?}, got {stated:?}"
                ),
            ));
        }
    }
    Err(invalid(
        path("currhashcode"),
        "event differs from its canonical finalized value",
    ))
}

fn validate_operation_event(
    stated: &MarketOperationEventData,
    canonical: &MarketOperationEventData,
    path: impl Fn(&str) -> SmolStr,
) -> Result<()> {
    if stated == canonical {
        return Ok(());
    }
    for column in MarketColumn::ALL {
        let held = column.fact(stated);
        let derived = column.fact(canonical);
        if held != derived {
            return Err(invalid(
                path(column.name()),
                format_smolstr!(
                    "expected the value derived from the row {derived:?}, got {held:?}"
                ),
            ));
        }
    }
    for column in OperationColumn::ALL {
        let held = column.fact(stated);
        let derived = column.fact(canonical);
        if held != derived {
            return Err(invalid(
                path(column.name()),
                format_smolstr!(
                    "expected the value derived from the row {derived:?}, got {held:?}"
                ),
            ));
        }
    }
    Err(invalid(
        path("currhashcode"),
        "operation differs from its canonical finalized value",
    ))
}

fn validate_market_element(
    stated: &MarketData,
    canonical: &MarketData,
    path: impl Fn(&str) -> SmolStr,
) -> Result<()> {
    if stated == canonical {
        return Ok(());
    }
    for column in MarketColumn::ALL {
        let stated = column.fact(stated);
        let derived = column.fact(canonical);
        if stated != derived {
            return Err(invalid(
                path(column.name()),
                format_smolstr!(
                    "expected the value derived from live depth {derived:?}, got {stated:?}"
                ),
            ));
        }
    }
    Err(invalid(
        path("currhashcode"),
        "book side differs from its canonical live-depth value",
    ))
}

struct Intake {
    root: Arc<Field>,
}

impl Intake {
    fn resolve(schema: &SchemaRef) -> Result<Self> {
        let field = operation_field()?;
        let expected = field.clone().into_arrow_schema()?;
        if schema != &expected {
            return Err(invalid(
                SmolStr::new_static("$"),
                format_smolstr!(
                    "expected canonical {OPERATION_ROOT} Arrow schema with {} columns, got {} columns",
                    field.field_len(),
                    schema.fields().len()
                ),
            ));
        }
        Ok(Self {
            root: Arc::new(field),
        })
    }

    fn input_of(&self, batch: &Serie, row: usize, ordinal: u64) -> Result<BookInput> {
        let mut cells = Vec::with_capacity(batch.children().len());
        for column in 0..batch.children().len() {
            cells.push(cell(batch, row, ordinal, column)?);
        }
        input_from_cells(
            &cells,
            |name| format_smolstr!("$[{ordinal}].{name}"),
            |index| format_smolstr!("$[{ordinal}].executions[{index}]"),
        )
    }
}

struct BookIntake {
    root: Arc<Field>,
}

impl BookIntake {
    fn resolve(schema: &SchemaRef) -> Result<Self> {
        let field = book_field()?;
        let expected = field.clone().into_arrow_schema()?;
        if schema != &expected {
            return Err(invalid(
                SmolStr::new_static("$"),
                format_smolstr!(
                    "expected canonical {BOOK_ROOT} Arrow schema with {} columns, got {} columns",
                    field.field_len(),
                    schema.fields().len()
                ),
            ));
        }
        Ok(Self {
            root: Arc::new(field),
        })
    }

    fn book_of(&self, batch: &Serie, row: usize, ordinal: u64) -> Result<Book> {
        let mut event = MarketEventData::default();
        let mut claims = IdentityClaims::default();
        let mut at = 0;
        for column in EventColumn::ALL {
            let value = cell(batch, row, ordinal, at)?;
            claims.record(column, &value);
            column.record(&mut event, &value);
            at += 1;
        }
        for column in MarketColumn::ALL {
            let value = cell(batch, row, ordinal, at)?;
            column.record(&mut event, &value);
            at += 1;
        }
        let mut stated = event.clone();
        let bid = side_from_value(&cell(batch, row, ordinal, at)?, ordinal, "bid")?;
        at += 1;
        let ask = side_from_value(&cell(batch, row, ordinal, at)?, ordinal, "ask")?;
        at += 1;
        let executions = executions_from_value(
            &cell(batch, row, ordinal, at)?,
            || NestedSerie::Executions.path(ordinal),
            |index| NestedSerie::Executions.item_path(ordinal, index),
        )?;
        at += 1;
        let snapshots = snapshots_from_value(&cell(batch, row, ordinal, at)?, ordinal)?;
        let book = Book::from_parts(event, bid, ask, executions, snapshots)
            .map_err(|error| prefix_invalid(error, || format_smolstr!("$[{ordinal}]")))?;
        claims.validate(&book, |name| format_smolstr!("$[{ordinal}].{name}"))?;
        normalize_identity(&mut stated, &book);
        validate_market_event(&stated, book.as_ref(), |name| {
            format_smolstr!("$[{ordinal}].{name}")
        })?;
        Ok(book)
    }
}

#[derive(Clone, Copy)]
enum NestedSerie {
    SideLive(&'static str),
    SideDeltas(&'static str),
    Executions,
}

impl NestedSerie {
    fn path(self, ordinal: u64) -> SmolStr {
        match self {
            Self::SideLive(side) => format_smolstr!("$[{ordinal}].{side}.live"),
            Self::SideDeltas(side) => format_smolstr!("$[{ordinal}].{side}.deltas"),
            Self::Executions => format_smolstr!("$[{ordinal}].executions"),
        }
    }

    fn item_path(self, ordinal: u64, index: usize) -> SmolStr {
        format_smolstr!("{}[{index}]", self.path(ordinal))
    }

    fn field_path(self, ordinal: u64, index: usize, name: &str) -> SmolStr {
        format_smolstr!("{}[{index}].{name}", self.path(ordinal))
    }
}

fn side_path(ordinal: u64, side: &str) -> SmolStr {
    format_smolstr!("$[{ordinal}].{side}")
}

fn side_field_path(ordinal: u64, side: &str, name: &str) -> SmolStr {
    format_smolstr!("$[{ordinal}].{side}.{name}")
}

fn snapshots_from_value(
    value: &Scalar,
    ordinal: u64,
) -> Result<std::collections::BTreeSet<SnapshotPartition>> {
    if value.is_null() {
        return Ok(std::collections::BTreeSet::new());
    }
    let rows = sequence(
        value,
        || format_smolstr!("$[{ordinal}].snapshotpartitions"),
        "a snapshot partition list",
    )?;
    rows.iter()
        .enumerate()
        .map(|(index, row)| {
            let path = || format_smolstr!("$[{ordinal}].snapshotpartitions[{index}]");
            let cells = sequence(row, path, "a snapshot partition struct")?;
            if cells.len() != 2 {
                return Err(invalid(
                    path(),
                    format_smolstr!("expected 2 snapshot partition fields, got {}", cells.len()),
                ));
            }
            let scope = cells[1]
                .as_str()
                .ok_or_else(|| invalid(path(), "expected a non-null scope, got null"))?;
            Ok(SnapshotPartition {
                symbol: cells[0].as_str().map(SmolStr::new),
                scope: SmolStr::new(scope),
            })
        })
        .collect()
}

fn side_from_value(value: &Scalar, ordinal: u64, side_name: &'static str) -> Result<BookSide> {
    let values = sequence(
        value,
        || side_path(ordinal, side_name),
        "a book-side struct",
    )?;
    let expected = SIDE_ELEMENT_COLUMNS.len() + MarketColumn::ALL.len() + 2;
    if values.len() != expected {
        return Err(invalid(
            side_path(ordinal, side_name),
            format_smolstr!("expected {expected} book-side fields, got {}", values.len()),
        ));
    }
    let mut carrier = MarketEventData::default();
    let mut claims = IdentityClaims::default();
    let mut at = 0;
    for column in SIDE_ELEMENT_COLUMNS {
        require_nested(&values[at], column.nullable(), || {
            side_field_path(ordinal, side_name, column.name())
        })?;
        claims.record(column, &values[at]);
        column.record(&mut carrier, &values[at]);
        at += 1;
    }
    let mut element = carrier.into_market();
    for column in MarketColumn::ALL {
        require_nested(&values[at], column.nullable(), || {
            side_field_path(ordinal, side_name, column.name())
        })?;
        column.record(&mut element, &values[at]);
        at += 1;
    }
    let mut stated = element.clone();
    let live = operations_from_value(&values[at], ordinal, NestedSerie::SideLive(side_name))?;
    at += 1;
    let deltas = operations_from_value(&values[at], ordinal, NestedSerie::SideDeltas(side_name))?;
    let side = BookSide::from_parts(element, live, deltas)
        .map_err(|error| prefix_invalid(error, || side_path(ordinal, side_name)))?;
    claims.validate(&side, |name| side_field_path(ordinal, side_name, name))?;
    let canonical: &MarketData = side.as_ref();
    normalize_identity(&mut stated, canonical);
    validate_market_element(&stated, canonical, |name| {
        side_field_path(ordinal, side_name, name)
    })?;
    Ok(side)
}

/// The operations a side's `live` or `deltas` list holds: every row an
/// order or a quote, never a trade or a snapshot control.
fn operations_from_value(
    value: &Scalar,
    ordinal: u64,
    path: NestedSerie,
) -> Result<Vec<Operation>> {
    sequence(value, || path.path(ordinal), "a market-operation list")?
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let cells = sequence(
                value,
                || path.item_path(ordinal, index),
                "a market-operation struct",
            )?;
            let expected = KIND_COLUMNS
                + EventColumn::ALL.len()
                + MarketColumn::ALL.len()
                + OperationColumn::ALL.len()
                + BOOK_COLUMNS.len()
                + 1;
            if cells.len() != expected {
                return Err(invalid(
                    path.item_path(ordinal, index),
                    format_smolstr!(
                        "expected {expected} market-operation fields, got {}",
                        cells.len()
                    ),
                ));
            }
            let input = input_from_cells(
                &cells,
                |name| path.field_path(ordinal, index, name),
                |child| {
                    format_smolstr!("{}[{child}]", path.field_path(ordinal, index, "executions"))
                },
            )?;
            match input {
                BookInput::Operation(operation) => Ok(operation),
                other => Err(invalid(
                    path.field_path(ordinal, index, KIND),
                    format_smolstr!(
                        "expected an order or quote on a book side, got {}",
                        kind_of(&other)
                    ),
                )),
            }
        })
        .collect()
}

fn executions_from_value<P, I>(value: &Scalar, list_path: P, item_path: I) -> Result<Vec<Operation>>
where
    P: FnOnce() -> SmolStr,
    I: Fn(usize) -> SmolStr + Copy,
{
    sequence(value, list_path, "an execution list")?
        .iter()
        .enumerate()
        .map(|(index, value)| execution_from_value(value, index, item_path))
        .collect()
}

fn execution_from_value<I>(value: &Scalar, index: usize, item_path: I) -> Result<Operation>
where
    I: Fn(usize) -> SmolStr + Copy,
{
    let values = sequence(value, || item_path(index), "an execution struct")?;
    let expected = EventColumn::ALL.len()
        + MarketColumn::ALL.len()
        + OperationColumn::ALL.len()
        + BOOK_COLUMNS.len();
    if values.len() != expected {
        return Err(invalid(
            item_path(index),
            format_smolstr!("expected {expected} execution fields, got {}", values.len()),
        ));
    }
    let field_path = |name: &str| format_smolstr!("{}.{name}", item_path(index));
    let (data, claims, book) = data_from_cells(&values, 0, field_path)?;
    let stated = data.clone();
    let mut operation = Operation::execution(data);
    operation.set_book(book);
    operation.finalize();
    claims.validate(&operation, field_path)?;
    let mut stated = stated;
    normalize_identity(&mut stated, operation.data());
    validate_operation_event(&stated, operation.data(), field_path)?;
    Ok(operation)
}

/// The facts one row states from `start`: the event, market, operation and
/// book-control columns, in order; the identity claims beside them.
fn data_from_cells<P>(
    cells: &[Scalar],
    start: usize,
    path: P,
) -> Result<(MarketOperationEventData, IdentityClaims, Option<BookRef>)>
where
    P: Fn(&str) -> SmolStr + Copy,
{
    let mut data = MarketOperationEventData::default();
    let mut claims = IdentityClaims::default();
    let mut at = start;
    for column in EventColumn::ALL {
        require_nested(&cells[at], column.nullable(), || path(column.name()))?;
        claims.record(column, &cells[at]);
        column.record(&mut data, &cells[at]);
        at += 1;
    }
    for column in MarketColumn::ALL {
        require_nested(&cells[at], column.nullable(), || path(column.name()))?;
        column.record(&mut data, &cells[at]);
        at += 1;
    }
    for column in OperationColumn::ALL {
        column.record(&mut data, &cells[at]);
        at += 1;
    }
    let book = book_control_of(&cells[at..at + BOOK_COLUMNS.len()]);
    Ok((data, claims, book))
}

/// One operation row - the kind, the facts, the book control and the
/// executions - as the input it states.
fn input_from_cells<P, I>(cells: &[Scalar], path: P, execution_path: I) -> Result<BookInput>
where
    P: Fn(&str) -> SmolStr + Copy,
    I: Fn(usize) -> SmolStr + Copy,
{
    let Some(kind) = cells[0].as_str() else {
        return Err(invalid(
            path(KIND),
            "expected order, quote, execution, trade, or snapshot, got null",
        ));
    };
    let (data, claims, book) = data_from_cells(cells, KIND_COLUMNS, path)?;
    let executions_cell = &cells[cells.len() - 1];
    let executions = if executions_cell.is_null() {
        None
    } else {
        Some(executions_from_value(
            executions_cell,
            || path("executions"),
            execution_path,
        )?)
    };
    let stated = data.clone();
    match (OperationKind::read(kind), kind) {
        (Some(OperationKind::Trade), _) => {
            let executions = executions.ok_or_else(|| {
                invalid(
                    path("executions"),
                    "expected a non-null execution list for trade",
                )
            })?;
            let base = path("executions");
            let prefix = base
                .strip_suffix(".executions")
                .map_or_else(|| base.clone(), SmolStr::new);
            let trade = Trade::from_parts(data, executions)
                .map_err(|error| prefix_invalid(error, || prefix))?;
            claims.validate(&trade, path)?;
            let mut stated = stated;
            normalize_identity(&mut stated, trade.data());
            validate_operation_event(&stated, trade.data(), path)?;
            Ok(BookInput::Trade(trade))
        }
        (Some(kind), _) => {
            if executions.is_some() {
                return Err(invalid(
                    path("executions"),
                    format_smolstr!("expected null for {}, got an execution list", kind.as_str()),
                ));
            }
            let mut operation = Operation::new(kind, data).map_err(|error| {
                prefix_invalid(error, || {
                    let base = path(KIND);
                    base.strip_suffix(".operationkind")
                        .map_or_else(|| base.clone(), SmolStr::new)
                })
            })?;
            operation.set_book(book);
            operation.finalize();
            claims.validate(&operation, path)?;
            let mut stated = stated;
            normalize_identity(&mut stated, operation.data());
            validate_operation_event(&stated, operation.data(), path)?;
            Ok(BookInput::Operation(operation))
        }
        (None, SNAPSHOT_KIND) => {
            if executions.is_some() {
                return Err(invalid(
                    path("executions"),
                    "expected null for snapshot, got an execution list",
                ));
            }
            let mut event = data.into_event();
            event.finalize();
            claims.validate(&event, path)?;
            let mut stated = stated.into_event();
            normalize_identity(&mut stated, &event);
            validate_market_event(&stated, &event, path)?;
            let book = book.unwrap_or_default();
            Ok(BookInput::Snapshot(BookControl {
                event,
                book: BookRef {
                    action: Some(MdUpdateAction::Snapshot),
                    ..book
                },
            }))
        }
        (None, other) => Err(invalid(
            path(KIND),
            format_smolstr!("expected order, quote, execution, trade, or snapshot, got {other:?}"),
        )),
    }
}

fn cell(batch: &Serie, row: usize, ordinal: u64, column: usize) -> Result<Scalar> {
    let held = &batch.children()[column];
    held.scalar(row).map_err(|error| {
        let name = held.field().map_or("", Field::name);
        invalid(
            format_smolstr!("$[{ordinal}].{name}"),
            format_smolstr!("{error}"),
        )
    })
}

fn sequence<'a>(
    value: &'a Scalar,
    path: impl FnOnce() -> SmolStr,
    expected: &str,
) -> Result<std::borrow::Cow<'a, [Scalar]>> {
    value
        .sequence_rows()
        .ok_or_else(|| invalid(path(), format_smolstr!("expected {expected}, got scalar")))
}

fn require_nested(value: &Scalar, nullable: bool, path: impl FnOnce() -> SmolStr) -> Result<()> {
    if matches!(value, Scalar::Null) && !nullable {
        return Err(invalid(path(), "expected a non-null value, got null"));
    }
    Ok(())
}

fn prefix_invalid(error: Error, prefix: impl FnOnce() -> SmolStr) -> Error {
    match error {
        Error::InvalidRecord { path, reason } => {
            let suffix = path.strip_prefix('$').unwrap_or(path.as_str());
            Error::InvalidRecord {
                path: format_smolstr!("{}{suffix}", prefix()),
                reason,
            }
        }
        other => other,
    }
}

fn invalid(path: impl Into<SmolStr>, reason: impl Into<SmolStr>) -> crate::Error {
    crate::Error::InvalidRecord {
        path: path.into(),
        reason: reason.into(),
    }
}
