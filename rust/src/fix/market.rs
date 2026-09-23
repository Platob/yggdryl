//! The one FIX boundary into typed graph market operations.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use smol_str::{SmolStr, format_smolstr};

use super::{FixEntry, FixMsg};
use crate::graph::{
    Element, Event, Execution, MarketElement, MarketEventData, MarketOperation, Order, Quote,
};
use crate::{DataType, Decimal18, Error, Result, Scalar, Side, State, TimeUnit};

const MD_ENTRIES: i32 = 268;
const NANOS_PER_DAY: i64 = 86_400_000_000_000;

#[derive(Clone, Default)]
struct Facts {
    action: Option<SmolStr>,
    entry_type: Option<SmolStr>,
    entry_id: Option<SmolStr>,
    entry_ref_id: Option<SmolStr>,
    price: Option<SmolStr>,
    size: Option<SmolStr>,
    date: Option<SmolStr>,
    time: Option<SmolStr>,
    order_id: Option<SmolStr>,
    symbol: Option<SmolStr>,
    side: Option<SmolStr>,
    position: Option<SmolStr>,
    level: Option<SmolStr>,
    book_type: Option<SmolStr>,
    sub_book_type: Option<SmolStr>,
    feed_type: Option<SmolStr>,
    stream_id: Option<SmolStr>,
    market_id: Option<SmolStr>,
    market_segment_id: Option<SmolStr>,
    request_id: Option<SmolStr>,
    depth: Option<SmolStr>,
    report_sequence: Option<SmolStr>,
    application_id: Option<SmolStr>,
    application_sequence: Option<SmolStr>,
    application_begin_sequence: Option<SmolStr>,
    application_end_sequence: Option<SmolStr>,
    application_last_sequence: Option<SmolStr>,
}

impl Facts {
    fn record(&mut self, tag: i32, value: &SmolStr) {
        let slot = match tag {
            279 => &mut self.action,
            269 => &mut self.entry_type,
            278 => &mut self.entry_id,
            280 => &mut self.entry_ref_id,
            270 => &mut self.price,
            271 => &mut self.size,
            272 => &mut self.date,
            273 => &mut self.time,
            37 => &mut self.order_id,
            55 => &mut self.symbol,
            54 => &mut self.side,
            290 => &mut self.position,
            1023 => &mut self.level,
            1021 => &mut self.book_type,
            1173 => &mut self.sub_book_type,
            1022 => &mut self.feed_type,
            1500 => &mut self.stream_id,
            1301 => &mut self.market_id,
            1300 => &mut self.market_segment_id,
            262 => &mut self.request_id,
            264 => &mut self.depth,
            83 => &mut self.report_sequence,
            1180 => &mut self.application_id,
            1181 => &mut self.application_sequence,
            1182 => &mut self.application_begin_sequence,
            1183 => &mut self.application_end_sequence,
            1350 => &mut self.application_last_sequence,
            _ => return,
        };
        if slot.is_none() && !value.is_empty() {
            *slot = Some(value.clone());
        }
    }

    fn overlay(&mut self, other: Self) {
        macro_rules! overlay {
            ($($member:ident),+ $(,)?) => {
                $(if other.$member.is_some() { self.$member = other.$member; })+
            };
        }
        overlay!(
            action,
            entry_type,
            entry_id,
            entry_ref_id,
            price,
            size,
            date,
            time,
            order_id,
            symbol,
            side,
            position,
            level,
            book_type,
            sub_book_type,
            feed_type,
            stream_id,
            market_id,
            market_segment_id,
            request_id,
            depth,
            report_sequence,
            application_id,
            application_sequence,
            application_begin_sequence,
            application_end_sequence,
            application_last_sequence,
        );
    }

    fn inherit_context(&mut self, root: &Self) {
        self.symbol.clone_from(&root.symbol);
        self.book_type.clone_from(&root.book_type);
        self.sub_book_type.clone_from(&root.sub_book_type);
        self.feed_type.clone_from(&root.feed_type);
        self.stream_id.clone_from(&root.stream_id);
        self.market_id.clone_from(&root.market_id);
        self.market_segment_id.clone_from(&root.market_segment_id);
        self.request_id.clone_from(&root.request_id);
        self.depth.clone_from(&root.depth);
        self.date.clone_from(&root.date);
        self.time.clone_from(&root.time);
        self.report_sequence.clone_from(&root.report_sequence);
        self.application_id.clone_from(&root.application_id);
        self.application_sequence
            .clone_from(&root.application_sequence);
        self.application_begin_sequence
            .clone_from(&root.application_begin_sequence);
        self.application_end_sequence
            .clone_from(&root.application_end_sequence);
        self.application_last_sequence
            .clone_from(&root.application_last_sequence);
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Order,
    Quote,
    Execution,
}

struct BookEntry {
    facts: Facts,
    position: usize,
    date_days: Option<i64>,
    time_nanos: Option<i64>,
    price: Option<Decimal18>,
    size: Option<Decimal18>,
    empty_snapshot: bool,
}

impl FixMsg {
    /// Reads this message as graph market operations.
    ///
    /// Order, quote, execution and trade messages are one operation. A FIX
    /// `W` or `X` book message is expanded in `NoMDEntries(268)` source order;
    /// an empty `W` is one scoped snapshot control. The structured entry tree
    /// is derived at most once and then walked once.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] naming the FIX tag and occurrence for
    /// an unsupported message, update action or market-data entry type.
    pub fn market_operations(&self) -> Result<Vec<MarketOperation>> {
        operations(self, self.event().clone())
    }

    /// Moves this message into graph market operations.
    ///
    /// A direct order, quote, execution or trade moves its
    /// [`MarketEventData`] without cloning it, then finalizes the generic
    /// operation identity from those projected facts. Book messages
    /// necessarily make one owned event per `NoMDEntries(268)` occurrence, or
    /// one scoped snapshot control for an empty `W`.
    ///
    /// # Errors
    ///
    /// Returns the same typed refusals as [`Self::market_operations`].
    pub fn into_market_operations(self) -> Result<Vec<MarketOperation>> {
        let category = category(&self)?;
        if let Some(kind) = direct_kind(category, self.is_execution()) {
            return Ok(vec![operation(kind, MarketEventData::from(self))]);
        }
        let msgtype = self.header().msgtype();
        if category != "BOOK" || !matches!(msgtype, "W" | "X") {
            return Err(unsupported_message(&self));
        }
        let msgtype = SmolStr::new(msgtype);
        let entries = book_entries(&self)?;
        build_book_operations(MarketEventData::from(self), &msgtype, &entries)
    }
}

impl TryFrom<FixMsg> for MarketOperation {
    type Error = Error;

    fn try_from(message: FixMsg) -> Result<Self> {
        let operations = message.into_market_operations()?;
        if operations.len() != 1 {
            return Err(invalid(
                "$.NoMDEntries(268)",
                format_smolstr!(
                    "expected exactly one market operation, got {}",
                    operations.len()
                ),
            ));
        }
        Ok(operations.into_iter().next().expect("one operation"))
    }
}

fn operations(message: &FixMsg, base: MarketEventData) -> Result<Vec<MarketOperation>> {
    let category = category(message)?;
    if let Some(kind) = direct_kind(category, message.is_execution()) {
        return Ok(vec![operation(kind, base)]);
    }
    let msgtype = message.header().msgtype();
    if category != "BOOK" || !matches!(msgtype, "W" | "X") {
        return Err(unsupported_message(message));
    }
    let entries = book_entries(message)?;
    build_book_operations(base, msgtype, &entries)
}

fn category(message: &FixMsg) -> Result<&str> {
    message.lifted().msgcat().ok_or_else(|| {
        invalid(
            "$.MsgType(35)",
            format_smolstr!(
                "expected a market message category, got {:?}",
                message.header().msgtype()
            ),
        )
    })
}

fn direct_kind(category: &str, is_execution: bool) -> Option<Kind> {
    match category {
        "ORDR" => Some(Kind::Order),
        "QUOT" => Some(Kind::Quote),
        "EXEC" | "TRAD" if is_execution => Some(Kind::Execution),
        _ => None,
    }
}

fn unsupported_message(message: &FixMsg) -> Error {
    invalid(
        "$.MsgType(35)",
        format_smolstr!(
            "expected ORDR, QUOT, an actual EXEC/TRAD, W or X, got {:?} ({})",
            message.header().msgtype(),
            message.lifted().msgcat().unwrap_or("UNKN")
        ),
    )
}

fn operation(kind: Kind, mut event: MarketEventData) -> MarketOperation {
    event.finalize();
    match kind {
        Kind::Order => MarketOperation::Order(Order::from(event)),
        Kind::Quote => MarketOperation::Quote(Quote::from(event)),
        Kind::Execution => MarketOperation::Execution(Execution::from(event)),
    }
}

fn book_entries(message: &FixMsg) -> Result<Vec<BookEntry>> {
    let entries = message.entries();
    let mut root = Facts {
        request_id: message.lifted().mdreqid().map(SmolStr::new),
        ..Facts::default()
    };
    let mut group = None;
    for entry in entries {
        if entry.tag() == MD_ENTRIES {
            if group.replace(entry).is_some() {
                return Err(invalid(
                    "$.NoMDEntries(268)",
                    "expected one repeating group, got multiple",
                ));
            }
        } else {
            gather(entry, &mut root);
        }
    }
    let group = group
        .ok_or_else(|| invalid("$.NoMDEntries(268)", "expected a repeating group, got none"))?;

    let values = message
        .as_value()
        .as_sequence()
        .ok_or_else(|| invalid("$", "expected the FIX message row to be a sequence"))?;
    let group_at = message.index_of_group(MD_ENTRIES).ok_or_else(|| {
        invalid(
            "$.NoMDEntries(268)",
            "expected one typed repeating group, got none or multiple",
        )
    })?;
    let Some(sequence) = message
        .as_field()
        .fields()
        .get(group_at)
        .ok_or_else(|| invalid("$.NoMDEntries(268)", "typed group field is absent"))?
        .dtype()
        .as_serie_type()
    else {
        return Err(invalid(
            "$.NoMDEntries(268)",
            "expected the typed group field to be a sequence",
        ));
    };
    let members = sequence.item().fields();
    let date_at = member_path(message, members, 272);
    let time_at = member_path(message, members, 273);
    let price_at = member_path(message, members, 270);
    let size_at = member_path(message, members, 271);
    let typed_entries = values
        .get(group_at)
        .and_then(Scalar::sequence_rows)
        .ok_or_else(|| {
            invalid(
                "$.NoMDEntries(268)",
                "expected the typed group value to be a sequence",
            )
        })?;
    if typed_entries.len() != group.entries().len() {
        return Err(invalid(
            "$.NoMDEntries(268)",
            format_smolstr!(
                "typed group has {} entries but the FIX tree has {}",
                typed_entries.len(),
                group.entries().len()
            ),
        ));
    }

    let root_date = root_temporal_count(message, values, 272, TimeUnit::Day)?;
    let root_time = root_temporal_count(message, values, 273, TimeUnit::Nanosecond)?;

    let mut answer = Vec::with_capacity(group.entries().len());
    if group.entries().is_empty() && message.header().msgtype() == "W" {
        answer.push(BookEntry {
            facts: root,
            position: 0,
            date_days: root_date,
            time_nanos: root_time,
            price: None,
            size: None,
            empty_snapshot: true,
        });
        return Ok(answer);
    }
    for (position, occurrence) in group.entries().iter().enumerate() {
        let mut facts = Facts::default();
        facts.inherit_context(&root);
        let mut own = Facts::default();
        gather(occurrence, &mut own);
        facts.overlay(own);
        let typed = typed_entries[position].as_sequence().ok_or_else(|| {
            invalid(
                format_smolstr!("$.NoMDEntries(268)[{position}]"),
                "expected the typed occurrence to be a sequence",
            )
        })?;
        let date_days = temporal_count(
            member_value(typed, date_at.as_deref()),
            TimeUnit::Day,
            format_smolstr!("$.NoMDEntries(268)[{position}].MDEntryDate(272)"),
        )?
        .or(root_date);
        let time_nanos = temporal_count(
            member_value(typed, time_at.as_deref()),
            TimeUnit::Nanosecond,
            format_smolstr!("$.NoMDEntries(268)[{position}].MDEntryTime(273)"),
        )?
        .or(root_time);
        let price = decimal(
            member_value(typed, price_at.as_deref()),
            facts.price.as_deref(),
            format_smolstr!("$.NoMDEntries(268)[{position}].MDEntryPx(270)"),
        )?;
        let size = decimal(
            member_value(typed, size_at.as_deref()),
            facts.size.as_deref(),
            format_smolstr!("$.NoMDEntries(268)[{position}].MDEntrySize(271)"),
        )?;
        answer.push(BookEntry {
            facts,
            position,
            date_days,
            time_nanos,
            price,
            size,
            empty_snapshot: false,
        });
    }
    Ok(answer)
}

fn member_path(message: &FixMsg, fields: &[crate::Field], tag: i32) -> Option<Vec<usize>> {
    for (index, field) in fields.iter().enumerate() {
        if message
            .registry()
            .identity_of(field)
            .is_some_and(|(held, _)| held == tag)
        {
            return Some(vec![index]);
        }
        if matches!(field.dtype(), DataType::Struct(_)) {
            if let Some(mut nested) = member_path(message, field.fields(), tag) {
                nested.insert(0, index);
                return Some(nested);
            }
        }
    }
    None
}

fn member_value<'a>(values: &'a [Scalar], path: Option<&[usize]>) -> Option<&'a Scalar> {
    let (first, nested) = path?.split_first()?;
    let mut value = values.get(*first)?;
    for index in nested {
        value = value.as_sequence()?.get(*index)?;
    }
    Some(value)
}

fn root_temporal_count(
    message: &FixMsg,
    values: &[Scalar],
    tag: i32,
    unit: TimeUnit,
) -> Result<Option<i64>> {
    temporal_count(
        message
            .unique_index_of_tag(tag)
            .and_then(|at| values.get(at)),
        unit,
        format_smolstr!(
            "$.{}({tag})",
            if tag == 272 {
                "MDEntryDate"
            } else {
                "MDEntryTime"
            }
        ),
    )
}

fn temporal_count(value: Option<&Scalar>, unit: TimeUnit, path: SmolStr) -> Result<Option<i64>> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    value.temporal_count_at(unit).map(Some).ok_or_else(|| {
        invalid(
            path,
            format_smolstr!(
                "expected a temporal value exactly representable as {unit}, got {value:?}"
            ),
        )
    })
}

fn gather(entry: &FixEntry, facts: &mut Facts) {
    if let Some(value) = entry.held_value() {
        facts.record(entry.tag(), value);
    }
    for child in entry.entries() {
        gather(child, facts);
    }
}

fn build_book_operations(
    base: MarketEventData,
    msgtype: &str,
    entries: &[BookEntry],
) -> Result<Vec<MarketOperation>> {
    let mut answer = Vec::with_capacity(entries.len());
    let mut base = Some(base);
    for (index, entry) in entries.iter().enumerate() {
        let event = if index + 1 == entries.len() {
            base.take().expect("the final entry takes the base")
        } else {
            base.as_ref().expect("the base remains").clone()
        };
        answer.push(build_book_operation(event, msgtype, entry)?);
    }
    Ok(answer)
}

fn build_book_operation(
    mut event: MarketEventData,
    msgtype: &str,
    entry: &BookEntry,
) -> Result<MarketOperation> {
    let path = |tag: i32, name: &str| {
        format_smolstr!("$.NoMDEntries(268)[{}].{name}({tag})", entry.position)
    };
    if entry.empty_snapshot {
        let scope = book_scope(&entry.facts, event.get_symbolticker());
        event.set_symbolticker(
            entry
                .facts
                .symbol
                .as_deref()
                .map(str::to_owned)
                .or_else(|| event.get_symbolticker().map(str::to_owned)),
        );
        let mut crosscode = scope.clone();
        push_scope(&mut crosscode, "BookSnapshot", "empty");
        event.set_crosscode(crosscode);
        event.set_state(State::read("New").expect("the shipped new state"));
        let mut identifiers = operation_identifiers(&event);
        identifiers.insert("BookScope".to_owned(), scope);
        identifiers.insert("MDUpdateAction".to_owned(), "SNAPSHOT".to_owned());
        event.set_identifiers(identifiers);
        event.finalize();
        return Ok(MarketOperation::Snapshot(event));
    }
    let entry_type = entry.facts.entry_type.as_deref().ok_or_else(|| {
        invalid(
            path(269, "MDEntryType"),
            "expected 0 (bid), 1 (offer) or 2 (trade), got no value",
        )
    })?;
    let kind = match entry_type {
        "0" | "1" if entry.facts.order_id.is_some() => Kind::Order,
        "0" | "1" => Kind::Quote,
        "2" => Kind::Execution,
        other => {
            return Err(invalid(
                path(269, "MDEntryType"),
                format_smolstr!("expected 0 (bid), 1 (offer) or 2 (trade), got {other:?}"),
            ));
        }
    };
    let action = if msgtype == "W" {
        "SNAPSHOT"
    } else {
        entry.facts.action.as_deref().ok_or_else(|| {
            invalid(
                path(279, "MDUpdateAction"),
                "expected an incremental action from 0 through 5, got no value",
            )
        })?
    };
    let state = match action {
        "SNAPSHOT" | "0" => State::read("New").expect("the shipped new state"),
        "1" | "5" => State::read("Replaced").expect("the shipped replaced state"),
        "2" | "3" | "4" => State::read("Canceled").expect("the shipped canceled state"),
        other => {
            return Err(invalid(
                path(279, "MDUpdateAction"),
                format_smolstr!("expected an incremental action from 0 through 5, got {other:?}"),
            ));
        }
    };

    let side = match entry_type {
        "0" => Side::read("Buy").expect("the shipped buy side"),
        "1" => Side::read("Sell").expect("the shipped sell side"),
        _ => entry
            .facts
            .side
            .as_deref()
            .and_then(Side::from_spelling)
            .unwrap_or_else(Side::unknown),
    };
    if let Some(unix) = entry_unix(entry, event.get_currunix(), &path)? {
        if msgtype == "W" {
            if matches!(kind, Kind::Execution) {
                event.set_execunix(Some(unix));
            } else {
                event.set_creaunix(Some(unix));
            }
        } else {
            event.set_currunix(unix);
        }
        if msgtype != "W" && matches!(kind, Kind::Execution) {
            event.set_execunix(Some(unix));
        }
    }
    let scope = book_scope(&entry.facts, event.get_symbolticker());
    let crosscode = if let Some(identifier) = entry
        .facts
        .entry_id
        .as_deref()
        .or(entry.facts.entry_ref_id.as_deref())
    {
        let mut crosscode = scope.clone();
        push_scope(&mut crosscode, "MDEntryID", identifier);
        crosscode
    } else {
        fallback_crosscode(&scope, entry_type, action, &entry.facts, &path)?
    };

    event.set_px(entry.price.unwrap_or(Decimal18::ZERO));
    event.set_qty(entry.size.unwrap_or(Decimal18::ZERO));
    event.set_side(side);
    event.set_state(state);
    event.set_symbolticker(
        entry
            .facts
            .symbol
            .as_deref()
            .map(str::to_owned)
            .or_else(|| event.get_symbolticker().map(str::to_owned)),
    );
    event.set_crosscode(crosscode);

    let mut identifiers = operation_identifiers(&event);
    identifiers.insert("BookScope".to_owned(), scope);
    insert(
        &mut identifiers,
        "MDEntryID",
        entry.facts.entry_id.as_deref(),
    );
    insert(
        &mut identifiers,
        "MDEntryRefID",
        entry.facts.entry_ref_id.as_deref(),
    );
    identifiers.insert("MDUpdateAction".to_owned(), action.to_owned());
    insert(
        &mut identifiers,
        "MDEntryPositionNo",
        entry.facts.position.as_deref(),
    );
    insert(
        &mut identifiers,
        "MDPriceLevel",
        entry.facts.level.as_deref(),
    );
    insert(&mut identifiers, "MDEntryPx", entry.facts.price.as_deref());
    insert(&mut identifiers, "MDEntrySize", entry.facts.size.as_deref());
    insert(&mut identifiers, "MDEntryDate", entry.facts.date.as_deref());
    insert(&mut identifiers, "MDEntryTime", entry.facts.time.as_deref());
    insert(&mut identifiers, "OrderID", entry.facts.order_id.as_deref());
    insert(
        &mut identifiers,
        "RptSeq",
        entry.facts.report_sequence.as_deref(),
    );
    insert(
        &mut identifiers,
        "ApplID",
        entry.facts.application_id.as_deref(),
    );
    insert(
        &mut identifiers,
        "ApplSeqNum",
        entry.facts.application_sequence.as_deref(),
    );
    insert(
        &mut identifiers,
        "ApplBegSeqNum",
        entry.facts.application_begin_sequence.as_deref(),
    );
    insert(
        &mut identifiers,
        "ApplEndSeqNum",
        entry.facts.application_end_sequence.as_deref(),
    );
    insert(
        &mut identifiers,
        "ApplLastSeqNum",
        entry.facts.application_last_sequence.as_deref(),
    );
    event.set_identifiers(identifiers);
    Ok(operation(kind, event))
}

fn decimal(
    typed: Option<&Scalar>,
    rendered: Option<&str>,
    path: SmolStr,
) -> Result<Option<Decimal18>> {
    let Some(value) = typed.filter(|value| !value.is_null()) else {
        return rendered
            .map(|value| {
                value.parse::<Decimal18>().map_err(|_| {
                    invalid(
                        path.clone(),
                        format_smolstr!("expected an exact decimal, got {value:?}"),
                    )
                })
            })
            .transpose();
    };
    Decimal18::from_scalar(value).map(Some).ok_or_else(|| {
        invalid(
            path,
            format_smolstr!("expected an exact decimal, got {value:?}"),
        )
    })
}

fn entry_unix(
    entry: &BookEntry,
    fallback_unix: i64,
    path: &impl Fn(i32, &str) -> SmolStr,
) -> Result<Option<i64>> {
    let Some(clock) = entry.time_nanos else {
        return Ok(None);
    };
    let day = entry
        .date_days
        .unwrap_or_else(|| fallback_unix.div_euclid(NANOS_PER_DAY));
    let unix = i128::from(day)
        .checked_mul(i128::from(NANOS_PER_DAY))
        .and_then(|date| date.checked_add(i128::from(clock)))
        .and_then(|unix| i64::try_from(unix).ok())
        .ok_or_else(|| {
            invalid(
                path(273, "MDEntryTime"),
                "market-data entry instant exceeds nanosecond range",
            )
        })?;
    Ok(Some(unix))
}

fn insert(identifiers: &mut BTreeMap<String, String>, name: &str, value: Option<&str>) {
    if let Some(value) = value {
        identifiers.insert(name.to_owned(), value.to_owned());
    }
}

fn operation_identifiers(event: &MarketEventData) -> BTreeMap<String, String> {
    const OWNED: [&str; 17] = [
        "BookScope",
        "MDEntryID",
        "MDEntryRefID",
        "MDUpdateAction",
        "MDEntryPositionNo",
        "MDPriceLevel",
        "MDEntryPx",
        "MDEntrySize",
        "MDEntryDate",
        "MDEntryTime",
        "OrderID",
        "RptSeq",
        "ApplID",
        "ApplSeqNum",
        "ApplBegSeqNum",
        "ApplEndSeqNum",
        "ApplLastSeqNum",
    ];
    let mut identifiers = event.get_identifiers().clone();
    for name in OWNED {
        identifiers.remove(name);
    }
    identifiers
}

fn book_scope(facts: &Facts, fallback_symbol: Option<&str>) -> String {
    let mut scope = String::new();
    push_scope(
        &mut scope,
        "Symbol",
        facts
            .symbol
            .as_deref()
            .or(fallback_symbol)
            .unwrap_or("GLOBAL"),
    );
    for (name, value) in [
        ("MDBookType", facts.book_type.as_deref()),
        ("MDSubBookType", facts.sub_book_type.as_deref()),
        ("MDFeedType", facts.feed_type.as_deref()),
        ("MDStreamID", facts.stream_id.as_deref()),
        ("MarketID", facts.market_id.as_deref()),
        ("MarketSegmentID", facts.market_segment_id.as_deref()),
        ("MDReqID", facts.request_id.as_deref()),
        ("MarketDepth", facts.depth.as_deref()),
    ] {
        if let Some(value) = value {
            push_scope(&mut scope, name, value);
        }
    }
    scope
}

fn push_scope(scope: &mut String, name: &str, value: &str) {
    if !scope.is_empty() {
        scope.push('|');
    }
    write!(scope, "{name}=").expect("writing into a String cannot fail");
    for character in value.chars() {
        match character {
            '%' => scope.push_str("%25"),
            '|' => scope.push_str("%7C"),
            '=' => scope.push_str("%3D"),
            character => scope.push(character),
        }
    }
}

fn fallback_crosscode(
    scope: &str,
    entry_type: &str,
    action: &str,
    facts: &Facts,
    path: &impl Fn(i32, &str) -> SmolStr,
) -> Result<String> {
    let mut crosscode = String::with_capacity(scope.len() + 64);
    crosscode.push_str(scope);
    push_scope(&mut crosscode, "MDEntryType", entry_type);
    if let Some(position) = facts.position.as_deref() {
        push_scope(&mut crosscode, "MDEntryPositionNo", position);
    }
    if let Some(level) = facts.level.as_deref() {
        push_scope(&mut crosscode, "MDPriceLevel", level);
    }
    if facts.position.is_none() && facts.level.is_none() {
        if !matches!(action, "SNAPSHOT" | "0") {
            return Err(invalid(
                path(278, "MDEntryID"),
                "expected MDEntryID, MDEntryRefID, MDEntryPositionNo or MDPriceLevel for an anonymous incremental update",
            ));
        }
        push_scope(
            &mut crosscode,
            "MDEntryPx",
            facts.price.as_deref().unwrap_or(""),
        );
    }
    Ok(crosscode)
}

fn invalid(path: impl Into<SmolStr>, reason: impl Into<SmolStr>) -> Error {
    Error::InvalidRecord {
        path: path.into(),
        reason: reason.into(),
    }
}
