//! Streaming Arrow interchange for graph market operations and books.
//!
//! The operation schema is one leading kind discriminant, the canonical graph
//! event and market columns, then the nullable execution list carried only by
//! a composite trade. Encoding holds only the current bounded Arrow batch.
//! Decoding resolves and validates the reader schema once, then holds only one
//! batch and row cursor.

use std::iter::FusedIterator;

use arrow_array::{Array, ListArray, RecordBatch, StructArray};
use arrow_schema::SchemaRef;
use smol_str::{SmolStr, format_smolstr};

use super::{
    Book, BookSide, Element, EventColumn, Execution, MarketColumn, MarketElementData,
    MarketEventData, MarketOperation, Order, Quote, Trade,
};
use crate::arrow::BatchReader;
use crate::{DataType, Error, Field, Result, Scalar, SequenceType, StructType, Uuid};

const KIND: &str = "operationkind";
const OPERATION_ROOT: &str = "marketoperation";
const BOOK_ROOT: &str = "book";
const KIND_COLUMNS: usize = 1;
const SIDE_ELEMENT_COLUMNS: [EventColumn; 8] = [
    EventColumn::CurrUuid,
    EventColumn::CrossUuid,
    EventColumn::CrossCode,
    EventColumn::CurrHashCode,
    EventColumn::CrossHashCode,
    EventColumn::ParentUuids,
    EventColumn::SrcUuids,
    EventColumn::Identifiers,
];

impl From<MarketOperation> for Result<MarketOperation> {
    fn from(operation: MarketOperation) -> Self {
        Ok(operation)
    }
}

impl MarketOperation {
    /// The canonical Arrow row field for heterogeneous market operations.
    ///
    /// Its first child is `operationkind`, followed by [`EventColumn::ALL`],
    /// [`MarketColumn::ALL`], and nullable `executions`. The execution list is
    /// non-null only when `operationkind` is `trade`.
    pub fn field() -> Result<Field> {
        operation_field()
    }

    /// Streams operations into bounded Arrow record batches.
    ///
    /// The source is not pulled until the returned reader is pulled. A `None`
    /// row size uses the crate default and zero normalizes to one; a stated
    /// byte size closes a nonempty batch once the rows already in it reach the
    /// bound, so one oversized row is still emitted alone.
    pub fn arrow_reader<I>(
        operations: I,
        batch_row_size: Option<usize>,
        batch_byte_size: Option<u64>,
    ) -> Result<BatchReader>
    where
        I: IntoIterator,
        I::Item: Into<Result<Self>>,
        I::IntoIter: Send + 'static,
    {
        let field = operation_field()?;
        let rows = operations
            .into_iter()
            .enumerate()
            .map(|(ordinal, operation)| {
                operation
                    .into()
                    .and_then(|operation| checked_operation_row(&operation, ordinal as u64))
            });
        crate::arrow::rows::result_reader(&field, rows, batch_row_size, batch_byte_size, None, None)
            .map_err(Into::into)
    }

    /// Decodes a streamed Arrow reader into market operations.
    ///
    /// The declared schema is validated once before any batch is pulled. A
    /// reader error, changed batch schema, null required cell, unknown kind or
    /// identity fact inconsistent with the row is returned once and then
    /// fuses the iterator.
    pub fn from_arrow_reader(
        batches: BatchReader,
    ) -> Result<impl FusedIterator<Item = Result<Self>> + Send + 'static> {
        let schema = batches.schema();
        let intake = Intake::resolve(&schema)?;
        let mut batches = batches;
        let mut pending: Option<(RecordBatch, usize)> = None;
        let mut ordinal = 0_u64;
        let mut done = false;
        Ok(std::iter::from_fn(move || {
            if done {
                return None;
            }
            loop {
                if let Some((batch, row)) = pending.as_mut() {
                    if *row < batch.num_rows() {
                        let result = intake.operation_of(batch, *row, ordinal);
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
                pending = Some((batch, 0));
            }
        })
        .fuse())
    }
}

impl From<Book> for Result<Book> {
    fn from(book: Book) -> Self {
        Ok(book)
    }
}

impl Book {
    /// The canonical nested Arrow row field for a book.
    ///
    /// The book event and market facts lead, followed by `bid` and `ask`
    /// structs containing their element facts, ordered live operations and
    /// deltas, then the executions observed at the book instant.
    pub fn field() -> Result<Field> {
        book_field()
    }

    /// Streams books into bounded nested Arrow record batches.
    ///
    /// The source is not pulled until the returned reader is pulled. Owned
    /// books and their fallible counterparts are accepted directly; row and
    /// byte bounds close the current nonempty batch independently.
    pub fn arrow_reader<I>(
        books: I,
        batch_row_size: Option<usize>,
        batch_byte_size: Option<u64>,
    ) -> Result<BatchReader>
    where
        I: IntoIterator,
        I::Item: Into<Result<Self>>,
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

    /// Decodes nested Arrow batches into canonical books without replaying
    /// serialized deltas as new mutations.
    ///
    /// Schema resolution happens once. One batch and row cursor are held, and
    /// the first source, row, derived-summary, symbol or identity error fuses
    /// the iterator.
    pub fn from_arrow_reader(
        batches: BatchReader,
    ) -> Result<impl FusedIterator<Item = Result<Self>> + Send + 'static> {
        let schema = batches.schema();
        let intake = BookIntake::resolve(&schema)?;
        let mut batches = batches;
        let mut pending: Option<(RecordBatch, usize)> = None;
        let mut ordinal = 0_u64;
        let mut done = false;
        Ok(std::iter::from_fn(move || {
            if done {
                return None;
            }
            loop {
                if let Some((batch, row)) = pending.as_mut() {
                    if *row < batch.num_rows() {
                        let result = intake.book_of(batch, *row, ordinal);
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
                pending = Some((batch, 0));
            }
        })
        .fuse())
    }
}

/// The canonical row field shared by the encoder and decoder.
fn operation_field() -> Result<Field> {
    let mut fields =
        Vec::with_capacity(KIND_COLUMNS + EventColumn::ALL.len() + MarketColumn::ALL.len() + 1);
    let mut kind = DataType::utf8().required_field(KIND);
    kind.set_display("Operation Kind")?;
    fields.push(kind);
    fields.extend(EventColumn::fields()?);
    fields.extend(MarketColumn::fields()?);
    fields.push(DataType::list(execution_field()?).nullable_field("executions"));
    Ok(DataType::from(StructType::from_fields(fields)?).required_field(OPERATION_ROOT))
}

fn execution_field() -> Result<Field> {
    let mut fields = Vec::with_capacity(EventColumn::ALL.len() + MarketColumn::ALL.len());
    fields.extend(EventColumn::fields()?);
    fields.extend(MarketColumn::fields()?);
    Ok(DataType::from(StructType::from_fields(fields)?).required_field("execution"))
}

/// The nested book row field shared by both directions.
fn book_field() -> Result<Field> {
    let mut fields = Vec::with_capacity(EventColumn::ALL.len() + MarketColumn::ALL.len() + 3);
    fields.extend(EventColumn::fields()?);
    fields.extend(MarketColumn::fields()?);
    fields.push(side_field("bid")?);
    fields.push(side_field("ask")?);
    fields.push(DataType::list(execution_field()?).required_field("executions"));
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
    fields.push(DataType::list(operation_field()?).required_field("live"));
    fields.push(DataType::list(operation_field()?).required_field("deltas"));
    Ok(DataType::from(StructType::from_fields(fields)?).required_field(name))
}

/// One operation in canonical schema order. The shared row adapter validates
/// this ordered sequence against the field before materializing it.
fn row_of(operation: &MarketOperation) -> Scalar {
    Scalar::from_sequence(
        std::iter::once(Scalar::from(operation.kind().as_str()))
            .chain(
                EventColumn::ALL
                    .into_iter()
                    .map(|column| column.fact(operation).unwrap_or(Scalar::Null)),
            )
            .chain(
                MarketColumn::ALL
                    .into_iter()
                    .map(|column| column.fact(operation).unwrap_or(Scalar::Null)),
            )
            .chain([match operation {
                MarketOperation::Trade(trade) => {
                    Scalar::from_sequence(trade.executions().iter().map(execution_row))
                }
                _ => Scalar::Null,
            }]),
    )
}

fn checked_operation_row(operation: &MarketOperation, ordinal: u64) -> Result<Scalar> {
    match operation {
        MarketOperation::Trade(trade) => validate_trade_for_write(trade, ordinal)?,
        _ => validate_event_for_write(operation, |name| format_smolstr!("$[{ordinal}].{name}"))?,
    }
    Ok(row_of(operation))
}

fn checked_book_row(book: &Book, ordinal: u64) -> Result<Scalar> {
    for (name, side) in [("bid", book.bid()), ("ask", book.ask())] {
        for (index, operation) in side.live().enumerate() {
            validate_event_for_write(operation, |field| {
                NestedList::SideLive(name).field_path(ordinal, index, field)
            })?;
        }
        for (index, operation) in side.deltas().iter().enumerate() {
            validate_event_for_write(operation, |field| {
                NestedList::SideDeltas(name).field_path(ordinal, index, field)
            })?;
        }
    }
    for (index, execution) in book.executions().iter().enumerate() {
        validate_event_for_write(execution, |field| {
            NestedList::Executions.field_path(ordinal, index, field)
        })?;
    }

    validate_book_parts_for_write(book, ordinal)?;
    let bid = validate_side_for_write(book.bid(), ordinal, "bid")?;
    let ask = validate_side_for_write(book.ask(), ordinal, "ask")?;
    let canonical = canonical_book_event(book, &bid, &ask);

    let mut stated: MarketEventData = book.as_ref().clone();
    IdentityClaims::from_element(book)
        .validate(&canonical, |name| format_smolstr!("$[{ordinal}].{name}"))?;
    normalize_identity(&mut stated, &canonical);
    validate_market_event(&stated, &canonical, |name| {
        format_smolstr!("$[{ordinal}].{name}")
    })?;
    Ok(book_row(book))
}

fn validate_event_for_write<E, P>(event: &E, path: P) -> Result<()>
where
    E: super::MarketEvent + ?Sized,
    P: Fn(&str) -> SmolStr + Copy,
{
    let mut canonical = MarketEventData::from(event);
    let mut stated = canonical.clone();
    canonical.finalize();
    IdentityClaims::from_element(event).validate(&canonical, path)?;
    normalize_identity(&mut stated, &canonical);
    validate_market_event(&stated, &canonical, path)
}

fn validate_trade_for_write(trade: &Trade, ordinal: u64) -> Result<()> {
    for (index, execution) in trade.executions().iter().enumerate() {
        validate_event_for_write(execution, |name| {
            format_smolstr!("$[{ordinal}].executions[{index}].{name}")
        })?;
    }
    let canonical = Trade::from_parts(trade.as_ref().clone(), trade.executions().to_vec())
        .map_err(|error| prefix_invalid(error, || format_smolstr!("$[{ordinal}]")))?;
    IdentityClaims::from_element(trade)
        .validate(&canonical, |name| format_smolstr!("$[{ordinal}].{name}"))?;
    let mut stated = trade.as_ref().clone();
    normalize_identity(&mut stated, &canonical);
    validate_market_event(&stated, canonical.as_ref(), |name| {
        format_smolstr!("$[{ordinal}].{name}")
    })
}

fn validate_side_for_write(
    stated_side: &BookSide,
    ordinal: u64,
    name: &'static str,
) -> Result<MarketElementData> {
    let canonical = canonical_side(stated_side)
        .map_err(|error| prefix_invalid(error, || format_smolstr!("$[{ordinal}].{name}")))?;
    IdentityClaims::from_element(stated_side)
        .validate(&canonical, |field| side_field_path(ordinal, name, field))?;
    let mut stated: MarketElementData = stated_side.as_ref().clone();
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

fn canonical_side(side: &BookSide) -> Result<MarketElementData> {
    side.canonical_element()
}

fn canonical_book_event(
    book: &Book,
    bid: &MarketElementData,
    ask: &MarketElementData,
) -> MarketEventData {
    book.canonical_event(bid, ask)
}

fn book_row(book: &Book) -> Scalar {
    Scalar::from_sequence(
        EventColumn::ALL
            .into_iter()
            .map(|column| column.fact(book).unwrap_or(Scalar::Null))
            .chain(
                MarketColumn::ALL
                    .into_iter()
                    .map(|column| column.fact(book).unwrap_or(Scalar::Null)),
            )
            .chain([
                side_row(book.bid()),
                side_row(book.ask()),
                Scalar::from_sequence(book.executions().iter().map(execution_row)),
            ]),
    )
}

fn side_row(side: &BookSide) -> Scalar {
    Scalar::from_sequence(
        SIDE_ELEMENT_COLUMNS
            .into_iter()
            .map(|column| element_fact(column, side).unwrap_or(Scalar::Null))
            .chain(
                MarketColumn::ALL
                    .into_iter()
                    .map(|column| column.fact(side).unwrap_or(Scalar::Null)),
            )
            .chain([
                Scalar::from_sequence(side.live().map(row_of)),
                Scalar::from_sequence(side.deltas().iter().map(row_of)),
            ]),
    )
}

fn execution_row(event: &(impl super::MarketEvent + ?Sized)) -> Scalar {
    Scalar::from_sequence(
        EventColumn::ALL
            .into_iter()
            .map(|column| column.fact(event).unwrap_or(Scalar::Null))
            .chain(
                MarketColumn::ALL
                    .into_iter()
                    .map(|column| column.fact(event).unwrap_or(Scalar::Null)),
            ),
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
        EventColumn::ParentUuids => (!element.get_parentuuids().is_empty()).then(|| {
            Scalar::from_sequence(element.get_parentuuids().iter().copied().map(Scalar::Uuid))
        }),
        EventColumn::SrcUuids => (!element.get_srcuuids().is_empty()).then(|| {
            Scalar::from_sequence(element.get_srcuuids().iter().copied().map(Scalar::Uuid))
        }),
        EventColumn::Identifiers => {
            (!element.get_identifiers().is_empty()).then(|| {
                Scalar::from_mapping(element.get_identifiers().iter().map(
                    |(scheme, identifier)| {
                        (
                            Scalar::from(scheme.as_str()),
                            Scalar::from(identifier.as_str()),
                        )
                    },
                ))
                .expect("a sorted identifier map has unique keys")
            })
        }
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

fn validate_market_element(
    stated: &MarketElementData,
    canonical: &MarketElementData,
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

/// Schema facts resolved once for a streamed decode.
struct Intake {
    fields: Vec<Field>,
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
            fields: field.fields().to_vec(),
        })
    }

    fn operation_of(
        &self,
        batch: &RecordBatch,
        row: usize,
        ordinal: u64,
    ) -> Result<MarketOperation> {
        let kind = self.cell(batch, row, ordinal, 0)?;
        let Some(kind) = kind.as_str() else {
            return Err(invalid(
                format_smolstr!("$[{ordinal}].{KIND}"),
                "expected order, quote, execution, trade, or snapshot, got null",
            ));
        };

        let mut event = MarketEventData::default();
        let mut claims = IdentityClaims::default();
        let mut at = KIND_COLUMNS;
        for column in EventColumn::ALL {
            let value = self.cell(batch, row, ordinal, at)?;
            claims.record(column, &value);
            column.record(&mut event, &value);
            at += 1;
        }
        for column in MarketColumn::ALL {
            let value = self.cell(batch, row, ordinal, at)?;
            column.record(&mut event, &value);
            at += 1;
        }
        let payload = self.cell(batch, row, ordinal, at)?;
        let executions = optional_executions_from_value(
            &payload,
            || format_smolstr!("$[{ordinal}].executions"),
            |index| format_smolstr!("$[{ordinal}].executions[{index}]"),
            |index, name| format_smolstr!("$[{ordinal}].executions[{index}].{name}"),
        )?;
        finish_operation(kind, event, claims, executions, |name| {
            format_smolstr!("$[{ordinal}].{name}")
        })
    }

    fn cell(&self, batch: &RecordBatch, row: usize, ordinal: u64, column: usize) -> Result<Scalar> {
        let field = &self.fields[column];
        let path = format_smolstr!("$[{ordinal}].{}", field.name());
        let value = value_from_array_at(
            field.dtype(),
            batch.column(column).as_ref(),
            row,
            path.clone(),
        )?;
        if matches!(value, Scalar::Null) && !field.is_nullable() {
            return Err(invalid(path, "expected a non-null value, got null"));
        }
        Ok(value)
    }
}

struct BookIntake {
    fields: Vec<Field>,
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
            fields: field.fields().to_vec(),
        })
    }

    fn book_of(&self, batch: &RecordBatch, row: usize, ordinal: u64) -> Result<Book> {
        let mut event = MarketEventData::default();
        let mut claims = IdentityClaims::default();
        let mut at = 0;
        for column in EventColumn::ALL {
            let value = self.cell(batch, row, ordinal, at)?;
            claims.record(column, &value);
            column.record(&mut event, &value);
            at += 1;
        }
        for column in MarketColumn::ALL {
            let value = self.cell(batch, row, ordinal, at)?;
            column.record(&mut event, &value);
            at += 1;
        }
        let mut stated = event.clone();
        let bid = side_from_value(&self.cell(batch, row, ordinal, at)?, ordinal, "bid")?;
        at += 1;
        let ask = side_from_value(&self.cell(batch, row, ordinal, at)?, ordinal, "ask")?;
        at += 1;
        let executions = executions_from_value(
            &self.cell(batch, row, ordinal, at)?,
            || NestedList::Executions.path(ordinal),
            |index| NestedList::Executions.item_path(ordinal, index),
            |index, name| NestedList::Executions.field_path(ordinal, index, name),
        )?;
        let book = Book::from_parts(event, bid, ask, executions)
            .map_err(|error| prefix_invalid(error, || format_smolstr!("$[{ordinal}]")))?;
        claims.validate(&book, |name| format_smolstr!("$[{ordinal}].{name}"))?;
        normalize_identity(&mut stated, &book);
        validate_market_event(&stated, book.as_ref(), |name| {
            format_smolstr!("$[{ordinal}].{name}")
        })?;
        Ok(book)
    }

    fn cell(&self, batch: &RecordBatch, row: usize, ordinal: u64, column: usize) -> Result<Scalar> {
        let field = &self.fields[column];
        let path = format_smolstr!("$[{ordinal}].{}", field.name());
        let value = value_from_array_at(
            field.dtype(),
            batch.column(column).as_ref(),
            row,
            path.clone(),
        )?;
        if matches!(value, Scalar::Null) && !field.is_nullable() {
            return Err(invalid(path, "expected a non-null value, got null"));
        }
        Ok(value)
    }
}

#[derive(Clone, Copy)]
enum NestedList {
    SideLive(&'static str),
    SideDeltas(&'static str),
    Executions,
}

impl NestedList {
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
    let mut element = MarketElementData::from(carrier);
    for column in MarketColumn::ALL {
        require_nested(&values[at], column.nullable(), || {
            side_field_path(ordinal, side_name, column.name())
        })?;
        column.record(&mut element, &values[at]);
        at += 1;
    }
    let mut stated = element.clone();
    let live = operations_from_value(&values[at], ordinal, NestedList::SideLive(side_name))?;
    at += 1;
    let deltas = operations_from_value(&values[at], ordinal, NestedList::SideDeltas(side_name))?;
    let side = BookSide::from_parts(element, live, deltas)
        .map_err(|error| prefix_invalid(error, || side_path(ordinal, side_name)))?;
    claims.validate(&side, |name| side_field_path(ordinal, side_name, name))?;
    let canonical: &MarketElementData = side.as_ref();
    normalize_identity(&mut stated, canonical);
    validate_market_element(&stated, canonical, |name| {
        side_field_path(ordinal, side_name, name)
    })?;
    Ok(side)
}

fn operations_from_value(
    value: &Scalar,
    ordinal: u64,
    path: NestedList,
) -> Result<Vec<MarketOperation>> {
    sequence(value, || path.path(ordinal), "a market-operation list")?
        .iter()
        .enumerate()
        .map(|(index, value)| operation_from_value(value, ordinal, path, index))
        .collect()
}

fn optional_executions_from_value<P, I, F>(
    value: &Scalar,
    list_path: P,
    item_path: I,
    field_path: F,
) -> Result<Option<Vec<Execution>>>
where
    P: FnOnce() -> SmolStr,
    I: Fn(usize) -> SmolStr + Copy,
    F: Fn(usize, &str) -> SmolStr + Copy,
{
    if value.is_null() {
        Ok(None)
    } else {
        executions_from_value(value, list_path, item_path, field_path).map(Some)
    }
}

fn executions_from_value<P, I, F>(
    value: &Scalar,
    list_path: P,
    item_path: I,
    field_path: F,
) -> Result<Vec<Execution>>
where
    P: FnOnce() -> SmolStr,
    I: Fn(usize) -> SmolStr + Copy,
    F: Fn(usize, &str) -> SmolStr + Copy,
{
    sequence(value, list_path, "an execution list")?
        .iter()
        .enumerate()
        .map(|(index, value)| execution_from_value(value, index, item_path, field_path))
        .collect()
}

fn execution_from_value<I, F>(
    value: &Scalar,
    index: usize,
    item_path: I,
    field_path: F,
) -> Result<Execution>
where
    I: Fn(usize) -> SmolStr + Copy,
    F: Fn(usize, &str) -> SmolStr + Copy,
{
    let values = sequence(value, || item_path(index), "an execution struct")?;
    let expected = EventColumn::ALL.len() + MarketColumn::ALL.len();
    if values.len() != expected {
        return Err(invalid(
            item_path(index),
            format_smolstr!("expected {expected} execution fields, got {}", values.len()),
        ));
    }
    let mut event = MarketEventData::default();
    let mut claims = IdentityClaims::default();
    let mut at = 0;
    for column in EventColumn::ALL {
        require_nested(&values[at], column.nullable(), || {
            field_path(index, column.name())
        })?;
        claims.record(column, &values[at]);
        column.record(&mut event, &values[at]);
        at += 1;
    }
    for column in MarketColumn::ALL {
        require_nested(&values[at], column.nullable(), || {
            field_path(index, column.name())
        })?;
        column.record(&mut event, &values[at]);
        at += 1;
    }
    let mut stated = event.clone();
    event.finalize();
    claims.validate(&event, |name| field_path(index, name))?;
    normalize_identity(&mut stated, &event);
    validate_market_event(&stated, &event, |name| field_path(index, name))?;
    Ok(Execution::from(event))
}

fn finish_operation<P>(
    kind: &str,
    mut event: MarketEventData,
    claims: IdentityClaims,
    executions: Option<Vec<Execution>>,
    path: P,
) -> Result<MarketOperation>
where
    P: Fn(&str) -> SmolStr + Copy,
{
    let mut stated = event.clone();
    let operation = match kind {
        "trade" => {
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
            Trade::from_parts(event, executions)
                .map(MarketOperation::Trade)
                .map_err(|error| prefix_invalid(error, || prefix))?
        }
        "order" | "quote" | "execution" | "snapshot" => {
            if executions.is_some() {
                return Err(invalid(
                    path("executions"),
                    format_smolstr!("expected null for {kind}, got an execution list"),
                ));
            }
            event.finalize();
            match kind {
                "order" => Order::from(event).into(),
                "quote" => Quote::from(event).into(),
                "execution" => Execution::from(event).into(),
                "snapshot" => MarketOperation::Snapshot(event),
                _ => unreachable!("the operation kind was matched"),
            }
        }
        other => {
            return Err(invalid(
                path(KIND),
                format_smolstr!(
                    "expected order, quote, execution, trade, or snapshot, got {other:?}"
                ),
            ));
        }
    };
    claims.validate(&operation, path)?;
    normalize_identity(&mut stated, &operation);
    validate_market_event(&stated, operation.as_ref(), path)?;
    Ok(operation)
}

fn operation_from_value(
    value: &Scalar,
    ordinal: u64,
    path: NestedList,
    index: usize,
) -> Result<MarketOperation> {
    let values = sequence(
        value,
        || path.item_path(ordinal, index),
        "a market-operation struct",
    )?;
    let expected = KIND_COLUMNS + EventColumn::ALL.len() + MarketColumn::ALL.len() + 1;
    if values.len() != expected {
        return Err(invalid(
            path.item_path(ordinal, index),
            format_smolstr!(
                "expected {expected} market-operation fields, got {}",
                values.len()
            ),
        ));
    }
    let Some(kind) = values[0].as_str() else {
        return Err(invalid(
            path.field_path(ordinal, index, KIND),
            "expected order, quote, execution, trade, or snapshot, got null",
        ));
    };
    let mut event = MarketEventData::default();
    let mut claims = IdentityClaims::default();
    let mut at = KIND_COLUMNS;
    for column in EventColumn::ALL {
        require_nested(&values[at], column.nullable(), || {
            path.field_path(ordinal, index, column.name())
        })?;
        claims.record(column, &values[at]);
        column.record(&mut event, &values[at]);
        at += 1;
    }
    for column in MarketColumn::ALL {
        require_nested(&values[at], column.nullable(), || {
            path.field_path(ordinal, index, column.name())
        })?;
        column.record(&mut event, &values[at]);
        at += 1;
    }
    let executions = optional_executions_from_value(
        &values[at],
        || path.field_path(ordinal, index, "executions"),
        |child| format_smolstr!("{}[{child}]", path.field_path(ordinal, index, "executions")),
        |child, name| {
            format_smolstr!(
                "{}[{child}].{name}",
                path.field_path(ordinal, index, "executions")
            )
        },
    )?;
    finish_operation(kind, event, claims, executions, |name| {
        path.field_path(ordinal, index, name)
    })
}

fn value_from_array_at(
    dtype: &DataType,
    array: &dyn Array,
    index: usize,
    path: SmolStr,
) -> Result<Scalar> {
    if index >= array.len() {
        return Err(invalid(
            path,
            format_smolstr!("array index {index} exceeds length {}", array.len()),
        ));
    }
    if array.is_null(index) {
        return Ok(Scalar::Null);
    }
    match dtype {
        DataType::Struct(fields) => {
            let array = array
                .as_any()
                .downcast_ref::<StructArray>()
                .ok_or_else(|| {
                    invalid(
                        path.clone(),
                        format_smolstr!("expected a struct array, got {}", array.data_type()),
                    )
                })?;
            fields
                .iter()
                .zip(array.columns())
                .map(|(field, child)| {
                    value_from_array_at(
                        field.dtype(),
                        child.as_ref(),
                        index,
                        format_smolstr!("{path}.{}", field.name()),
                    )
                })
                .collect::<Result<Vec<_>>>()
                .map(Scalar::from_sequence)
        }
        DataType::Sequence(SequenceType::List(item)) => {
            let array = array.as_any().downcast_ref::<ListArray>().ok_or_else(|| {
                invalid(
                    path.clone(),
                    format_smolstr!("expected a list array, got {}", array.data_type()),
                )
            })?;
            let values = array.value(index);
            (0..values.len())
                .map(|item_index| {
                    value_from_array_at(
                        item.dtype(),
                        values.as_ref(),
                        item_index,
                        format_smolstr!("{path}[{item_index}]"),
                    )
                })
                .collect::<Result<Vec<_>>>()
                .map(Scalar::from_sequence)
        }
        _ => crate::arrow::value::value_from_array(dtype, array, index)
            .map_err(|error| invalid(path, format_smolstr!("{error}"))),
    }
}

fn sequence<'a>(
    value: &'a Scalar,
    path: impl FnOnce() -> SmolStr,
    expected: &str,
) -> Result<&'a [Scalar]> {
    value
        .as_sequence()
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
