//! Native evaluation of the derivations carried by the shipped dictionary.
//!
//! This module is entered only after the registry has proven that all twenty-
//! nine derivation terms and their fields are the shipped ones. A registry
//! with any added, removed or edited rule stays on the generic expression
//! evaluator, so `FIX:derivation` remains the public customization surface.

use crate::{
    CusipCode, DataType, Decimal18, FixCategory, IsinCode, Result, Scalar, SedolCode, StringEnum,
    TimeUnit, Timezone,
};

use super::msg::FixMsg;
use super::registry::FixRegistry;

const RULE_COUNT: usize = super::constants::SHIPPED_DERIVATIONS.len();
// Updated only with a reviewed native evaluator. The generator emits the
// dictionary term signature separately, so a rule text change selects the
// generic path until its hard-coded implementation is updated too.
const NATIVE_DERIVATIONS_SHA256: &str =
    "953409e8bfabe9a3e26b2272346bffbb470d70ce29820a06a60520160cf5a104";
const DECIMAL9: DataType = DataType::Decimal128 {
    precision: 38,
    scale: 9,
};
const DECIMAL18: DataType = DataType::DECIMAL;
const DATETIME_NS_UTC: DataType = DataType::DateTime64 {
    unit: TimeUnit::Nanosecond,
    timezone: Timezone::UTC,
};

#[derive(Clone, Copy)]
enum NativeKind {
    Boolean,
    Country,
    Currency,
    DateTimeNsUtc,
    Decimal18,
    Float64,
    Int32,
    String,
}

impl NativeKind {
    fn accepts(self, dtype: &DataType) -> bool {
        match self {
            Self::Boolean => dtype == &DataType::Boolean,
            Self::Country => dtype == &DataType::Country,
            Self::Currency => dtype == &DataType::Currency,
            Self::DateTimeNsUtc => dtype == &DATETIME_NS_UTC,
            Self::Decimal18 => dtype == &DECIMAL18,
            Self::Float64 => dtype == &DataType::Float64,
            Self::Int32 => dtype == &DataType::Int32,
            Self::String => dtype == &DataType::utf8(),
        }
    }
}

/// Every scalar the native rules read or fill. Names are checked as well as
/// tags so an alternate-tag lookup cannot make a different field look native.
const FIELDS: &[(i32, &str, NativeKind)] = &[
    (6, "avgpx", NativeKind::Decimal18),
    (14, "cumqty", NativeKind::Decimal18),
    (15, "currency", NativeKind::Currency),
    (22, "securityidsource", NativeKind::String),
    (31, "lastpx", NativeKind::Decimal18),
    (32, "lastqty", NativeKind::Decimal18),
    (35, "msgtype", NativeKind::String),
    (38, "orderqty", NativeKind::Decimal18),
    (39, "ordstatus", NativeKind::String),
    (43, "possdupflag", NativeKind::Boolean),
    (48, "securityid", NativeKind::String),
    (52, "sendingtime", NativeKind::DateTimeNsUtc),
    (55, "symbol", NativeKind::String),
    (59, "timeinforce", NativeKind::String),
    (84, "cxlqty", NativeKind::Decimal18),
    (119, "settlcurramt", NativeKind::Decimal18),
    (120, "settlcurrency", NativeKind::Currency),
    (122, "origsendingtime", NativeKind::DateTimeNsUtc),
    (132, "bidpx", NativeKind::Decimal18),
    (133, "offerpx", NativeKind::Decimal18),
    (150, "exectype", NativeKind::String),
    (151, "leavesqty", NativeKind::Decimal18),
    (155, "settlcurrfxrate", NativeKind::Float64),
    (167, "securitytype", NativeKind::String),
    (188, "bidspotrate", NativeKind::Decimal18),
    (189, "bidforwardpoints", NativeKind::Decimal18),
    (190, "offerspotrate", NativeKind::Decimal18),
    (191, "offerforwardpoints", NativeKind::Decimal18),
    (194, "lastspotrate", NativeKind::Decimal18),
    (195, "lastforwardpoints", NativeKind::Decimal18),
    (201, "putorcall", NativeKind::Int32),
    (211, "pegoffsetvalue", NativeKind::Float64),
    (231, "contractmultiplier", NativeKind::Float64),
    (381, "grosstradeamt", NativeKind::Decimal18),
    (454, "nosecurityaltid", NativeKind::Int32),
    (455, "securityaltid", NativeKind::String),
    (456, "securityaltidsource", NativeKind::String),
    (460, "product", NativeKind::Int32),
    (461, "cficode", NativeKind::String),
    (470, "countryofissue", NativeKind::Country),
    (839, "peggedprice", NativeKind::Decimal18),
    (969, "minpriceincrement", NativeKind::Float64),
    (1095, "peggedrefprice", NativeKind::Decimal18),
    (1146, "minpriceincrementamount", NativeKind::Decimal18),
    (2353, "tradingunitperiodmultiplier", NativeKind::Int32),
    (2367, "totaltradeqty", NativeKind::Decimal18),
    (2368, "lastmultipliedqty", NativeKind::Decimal18),
    (2369, "totalgrosstradeamt", NativeKind::Decimal18),
    (2370, "totaltrademultipliedqty", NativeKind::Decimal18),
    (2897, "currencycodesource", NativeKind::String),
    (2957, "symbolpositionnumber", NativeKind::Int32),
];

/// Whether `registry` has exactly the field shapes the hard-coded evaluator
/// assumes. The caller separately proves the complete canonical term set.
pub(super) fn supports(registry: &FixRegistry) -> bool {
    super::constants::SHIPPED_DERIVATIONS_SHA256 == NATIVE_DERIVATIONS_SHA256
        && FIELDS.iter().all(|(tag, name, kind)| {
            registry.get_field_by_tag(*tag).is_some_and(|field| {
                field.name() == *name
                    && field.as_fix().tag().ok().flatten() == Some(*tag)
                    && kind.accepts(field.dtype())
            })
        })
        && supports_secaltids(registry)
}

fn supports_secaltids(registry: &FixRegistry) -> bool {
    let Some(group) = registry.get_definition(FixCategory::Groups, "secaltids") else {
        return false;
    };
    if group.name() != "secaltids" || group.as_fix().counter().ok().flatten() != Some(454) {
        return false;
    }
    let DataType::List(item) = group.dtype() else {
        return false;
    };
    if item.name() != "secaltid" || !matches!(item.dtype(), DataType::Struct(_)) {
        return false;
    }
    let members = item.fields();
    members.len() == 3
        && members[0].name() == "securityaltid"
        && NativeKind::String.accepts(members[0].dtype())
        && members[1].name() == "securityaltidsource"
        && NativeKind::String.accepts(members[1].dtype())
        && members[2].name() == "symbolpositionnumber"
        && NativeKind::Int32.accepts(members[2].dtype())
}

/// Fill every shipped derivation to a fixpoint and rebuild the message once.
pub(super) fn fill_all(msg: &mut FixMsg) -> Result<()> {
    let landed = NativeRow::new(msg).settle();
    if !landed.is_empty() {
        msg.set_each(landed)?;
    }
    Ok(())
}

/// The message plus answers landed during this pass. Reading landed answers
/// before the message is what gives a dependency chain its fixpoint without a
/// materialized working row.
struct NativeRow<'message> {
    msg: &'message FixMsg,
    landed: Vec<(i32, Scalar)>,
}

impl<'message> NativeRow<'message> {
    const fn new(msg: &'message FixMsg) -> Self {
        Self {
            msg,
            landed: Vec::new(),
        }
    }

    fn settle(mut self) -> Vec<(i32, Scalar)> {
        // A productive sweep fills at least one target, and a filled target is
        // never visited again. The target count therefore bounds the fixpoint.
        for _ in 0..=RULE_COUNT {
            let before = self.landed.len();
            derive_once(&mut self);
            if self.landed.len() == before {
                break;
            }
        }
        self.landed
    }

    /// A non-null answer already landed in this pass, else what the message
    /// stated. A stated null remains fillable, exactly like the generic row.
    fn get(&self, tag: i32) -> Option<Scalar> {
        self.landed
            .iter()
            .find_map(|(held, value)| (*held == tag).then(|| value.clone()))
            .or_else(|| self.msg.indexed_by_tag(tag))
            .filter(|value| !value.is_null())
    }

    /// Whether the target is already stated or landed. Checked before an
    /// answer is evaluated, so later fixpoint sweeps do no work for settled
    /// rules.
    fn has(&self, tag: i32) -> bool {
        self.landed.iter().any(|(held, _)| *held == tag)
            || self
                .msg
                .indexed_by_tag(tag)
                .is_some_and(|value| !value.is_null())
    }

    fn with_text<T>(&self, tag: i32, read: impl FnOnce(&str) -> T) -> Option<T> {
        let value = self.get(tag)?;
        value.as_str().map(read)
    }

    fn text_is(&self, tag: i32, expected: &str) -> bool {
        self.with_text(tag, |value| value == expected)
            .unwrap_or(false)
    }

    fn text_in(&self, tag: i32, expected: &[&str]) -> bool {
        self.with_text(tag, |value| expected.contains(&value))
            .unwrap_or(false)
    }

    fn decimal(&self, tag: i32) -> Option<Decimal18> {
        Decimal18::from_scalar(&self.get(tag)?)
    }

    /// The first alternate identifier under `source`. Group filtering precedes
    /// `[0]` in the canonical terms, so an absent member on that first matching
    /// occurrence is absence rather than permission to inspect a later one.
    fn alternate(&self, source_value: &str) -> Option<Scalar> {
        let at = self.msg.as_field().index_of("secaltids")?;
        let column = self.msg.as_field().fields().get(at)?;
        let sequence = (column.dtype()).as_serie_type()?;
        let item = sequence.item();
        let identifier = item.index_of("securityaltid")?;
        let source = item.index_of("securityaltidsource")?;
        let group = self.msg.as_value().as_sequence()?.get(at)?.as_serie()?;
        for occurrence in group.iter() {
            let held = occurrence.as_sequence()?;
            if held.get(source).and_then(Scalar::as_str) == Some(source_value) {
                return held
                    .get(identifier)
                    .filter(|value| !value.is_null())
                    .cloned();
            }
        }
        None
    }

    /// Type one answer through its shipped registry field before another rule
    /// can read it. Invalid identifiers, code values and overflows are silence.
    fn put(&mut self, tag: i32, answer: Option<Scalar>) {
        let Some(answer) = answer.filter(|value| !value.is_null()) else {
            return;
        };
        let Some(field) = self.msg.registry().get_field_by_tag(tag) else {
            return;
        };
        // The generic evaluator gives a decimal target the exact market
        // decimal a numeric expression restates as before asking the field.
        // In particular, MinPriceIncrement * ContractMultiplier is a float
        // expression whose certain result belongs in decimal128(38,18).
        let answer = if field.dtype() == &DataType::DECIMAL {
            let Some(answer) = Decimal18::from_scalar(&answer) else {
                return;
            };
            Scalar::from(answer)
        } else {
            answer
        };
        let Ok(answer) = field.scalar(answer) else {
            return;
        };
        if answer.is_null() {
            return;
        }
        if self.landed.capacity() == 0 {
            self.landed.reserve_exact(RULE_COUNT);
        }
        self.landed.push((tag, answer));
    }
}

fn derive_once(row: &mut NativeRow<'_>) {
    macro_rules! fill {
        ($tag:literal, $answer:expr) => {{
            if !row.has($tag) {
                let answer = $answer;
                row.put($tag, answer);
            }
        }};
    }

    fill!(6, average_price(row));
    fill!(14, cumulative_quantity(row));
    fill!(15, row.get(120));
    fill!(22, security_id_source(row));
    fill!(31, add(row, 194, 195));
    fill!(38, order_quantity(row));
    fill!(39, order_status(row));
    fill!(48, alternate_isin(row).map(Scalar::from));
    fill!(55, symbol(row));
    fill!(59, time_in_force(row));
    fill!(119, scaled_product(row, 381, 155, 9));
    fill!(120, row.get(15));
    fill!(122, original_sending_time(row));
    fill!(132, add(row, 188, 189));
    fill!(133, add(row, 190, 191));
    fill!(151, leaves_quantity(row));
    fill!(167, security_type(row));
    fill!(201, put_or_call(row));
    fill!(381, gross_trade_amount(row));
    fill!(460, product(row));
    fill!(461, cfi_code(row));
    fill!(470, country_of_issue(row));
    fill!(839, pegged_price(row));
    fill!(1146, multiply(row, 969, 231));
    fill!(2367, multiply(row, 32, 2353));
    fill!(2368, scaled_product(row, 32, 231, 9));
    fill!(2369, scaled_product(row, 31, 2367, 9));
    fill!(2370, scaled_product(row, 2367, 231, 9));
    fill!(2897, row.get(15).map(|_| Scalar::from("6")));
}

const REPORTS: &[&str] = &["8", "9"];
const TIMED: &[&str] = &["D", "G", "8"];
const WORKING: &[&str] = &["0", "1", "6", "E", "5", "7", "9"];
const CLOSED: &[&str] = &["2", "3", "4", "8", "C"];
const AGREED: &[&str] = &["0", "3", "4", "5", "6", "7", "8", "9", "A", "B", "C", "E"];
const TRADES: &[&str] = &["F", "G"];

fn average_price(row: &NativeRow<'_>) -> Option<Scalar> {
    if !row.text_in(35, REPORTS) {
        return None;
    }
    let cumulative = row.decimal(14)?;
    let last = row.decimal(32)?;
    (last.is_positive() && cumulative == last)
        .then(|| row.get(31))
        .flatten()
}

fn cumulative_quantity(row: &NativeRow<'_>) -> Option<Scalar> {
    if !row.text_in(35, REPORTS) {
        return None;
    }
    let difference = subtract(row, 38, 151)?;
    (!Decimal18::from_scalar(&difference)?.is_negative()).then_some(difference)
}

fn security_id_source(row: &NativeRow<'_>) -> Option<Scalar> {
    let identifier = row.get(48)?;
    let text = identifier.as_str()?;
    if IsinCode::new(text).is_ok() {
        Some(Scalar::from("4"))
    } else if CusipCode::new(text).is_ok() {
        Some(Scalar::from("1"))
    } else if SedolCode::new(text).is_ok() {
        Some(Scalar::from("2"))
    } else {
        None
    }
}

fn order_quantity(row: &NativeRow<'_>) -> Option<Scalar> {
    if !row.text_in(35, REPORTS) {
        return None;
    }
    let canceled = row.decimal(84);
    let leaves = row.decimal(151);
    if canceled.is_some_and(Decimal18::is_positive) && leaves.unwrap_or(Decimal18::ZERO).is_zero() {
        return add(row, 14, 84);
    }
    // Expression function arguments are eager: even when the first sum is
    // non-null, an overflow in the fallback silences the generic rule. Keep
    // that exact error boundary while avoiding its working scalar array.
    let leaves = evaluated_add(row, 14, 151).ok()?;
    let canceled = evaluated_add(row, 14, 84).ok()?;
    leaves.or(canceled)
}

fn order_status(row: &NativeRow<'_>) -> Option<Scalar> {
    if !row.text_in(35, REPORTS) {
        return None;
    }
    let execution = row.get(150)?;
    let code = execution.as_str()?;
    if AGREED.contains(&code) {
        return Some(execution);
    }
    if !TRADES.contains(&code) {
        return None;
    }
    let leaves = row.decimal(151)?;
    if leaves.is_zero() {
        return Some(Scalar::from("2"));
    }
    (leaves.is_positive() && row.decimal(14)?.is_positive()).then(|| Scalar::from("1"))
}

fn alternate_isin(row: &NativeRow<'_>) -> Option<IsinCode> {
    let alternate = row.alternate("4")?;
    IsinCode::new(alternate.as_str()?).ok()
}

fn symbol(row: &NativeRow<'_>) -> Option<Scalar> {
    if row.text_in(22, &["8", "A"]) {
        if let Some(identifier) = row.get(48) {
            return Some(identifier);
        }
    }
    row.alternate("8")
}

fn time_in_force(row: &NativeRow<'_>) -> Option<Scalar> {
    row.text_in(35, TIMED).then(|| Scalar::from("0"))
}

fn original_sending_time(row: &NativeRow<'_>) -> Option<Scalar> {
    (row.get(43)?.as_bool() == Some(true))
        .then(|| row.get(52))
        .flatten()
}

fn leaves_quantity(row: &NativeRow<'_>) -> Option<Scalar> {
    if !row.text_in(35, REPORTS) {
        return None;
    }
    if row.text_in(39, CLOSED) {
        return Some(Scalar::from(0_i32));
    }
    if !row.text_in(39, WORKING) {
        return None;
    }
    let difference = subtract(row, 38, 14)?;
    (!Decimal18::from_scalar(&difference)?.is_negative()).then_some(difference)
}

fn security_type(row: &NativeRow<'_>) -> Option<Scalar> {
    let cfi = row.get(461)?;
    let cfi = cfi.as_str()?;
    let security = if starts_case(cfi, "ES") {
        "CS"
    } else if starts_case(cfi, "EP") {
        "PS"
    } else if starts_case(cfi, "ED") {
        "DR"
    } else if starts_case(cfi, "EU") || starts_case(cfi, "CI") {
        "MF"
    } else if starts_case(cfi, "CE") {
        "ETF"
    } else if starts_case(cfi, "F") {
        "FUT"
    } else if wildcard_o_f(cfi) {
        "OOF"
    } else if starts_case(cfi, "O") || starts_case(cfi, "H") {
        "OPT"
    } else if starts_case(cfi, "DC") {
        "CB"
    } else if starts_case(cfi, "DT") {
        "MTN"
    } else if starts_case(cfi, "DA") {
        "ABS"
    } else if starts_case(cfi, "DG") {
        "MBS"
    } else if starts_case(cfi, "SR") {
        "IRS"
    } else if starts_case(cfi, "SC") {
        "CDS"
    } else if starts_case(cfi, "ST") {
        "CMDTYSWAP"
    } else if starts_case(cfi, "SF") {
        "FXSWAP"
    } else if starts_case(cfi, "IF") {
        "FXSPOT"
    } else if starts_case(cfi, "JF") {
        "FXFWD"
    } else if starts_case(cfi, "JR") {
        "FRA"
    } else if starts_case(cfi, "JE") {
        "EQFWD"
    } else if starts_case(cfi, "LR") {
        "REPO"
    } else if starts_case(cfi, "LS") {
        "SECLOAN"
    } else if starts_case(cfi, "TI") {
        "INDEX"
    } else {
        return None;
    };
    Some(Scalar::from(security))
}

fn put_or_call(row: &NativeRow<'_>) -> Option<Scalar> {
    let cfi = row.get(461)?;
    let cfi = cfi.as_str()?;
    if starts_case(cfi, "OC") || starts_case(cfi, "HC") {
        Some(Scalar::from(1_i32))
    } else if starts_case(cfi, "OP") || starts_case(cfi, "HP") {
        Some(Scalar::from(0_i32))
    } else {
        None
    }
}

fn gross_trade_amount(row: &NativeRow<'_>) -> Option<Scalar> {
    row.text_in(35, REPORTS)
        .then(|| scaled_product(row, 32, 31, 9))
        .flatten()
}

fn product(row: &NativeRow<'_>) -> Option<Scalar> {
    let security = row.get(167);
    let security = security.as_ref().and_then(Scalar::as_str);
    let product = if member_upper(security, PRODUCT_AGENCY) {
        1
    } else if member_upper(security, PRODUCT_CORPORATE) {
        3
    } else if member_upper(security, PRODUCT_CURRENCY) {
        4
    } else if member_upper(security, PRODUCT_EQUITY) {
        5
    } else if member_upper(security, PRODUCT_GOVERNMENT) {
        6
    } else if member_upper(security, PRODUCT_LOAN) {
        8
    } else if member_upper(security, PRODUCT_MONEY_MARKET) {
        9
    } else if member_upper(security, PRODUCT_MORTGAGE) {
        10
    } else if member_upper(security, PRODUCT_MUNICIPAL) {
        11
    } else if member_upper(security, PRODUCT_FINANCING) {
        13
    } else if row
        .with_text(461, |cfi| starts_case(cfi, "E"))
        .unwrap_or(false)
    {
        5
    } else if row
        .with_text(461, |cfi| starts_case(cfi, "L"))
        .unwrap_or(false)
    {
        13
    } else {
        return None;
    };
    Some(Scalar::from(product))
}

fn cfi_code(row: &NativeRow<'_>) -> Option<Scalar> {
    let security = row.get(167)?;
    let security = security.as_str()?;
    let cfi = if member_upper(Some(security), &["CS"]) {
        "ESXXXX"
    } else if member_upper(Some(security), &["PS"]) {
        "EPXXXX"
    } else if member_upper(Some(security), &["DR"]) {
        "EDXXXX"
    } else if member_upper(Some(security), &["MF", "MMF"]) {
        "CIXXXX"
    } else if member_upper(Some(security), &["ETF"]) {
        "CEXXXX"
    } else if member_upper(Some(security), &["FUT"]) {
        "FXXXXX"
    } else if member_upper(Some(security), &["OPT", "OOP", "OOC"]) {
        return Some(Scalar::from(match exercise(row) {
            'C' => "OCXXXX",
            'P' => "OPXXXX",
            _ => "OXXXXX",
        }));
    } else if member_upper(Some(security), &["OOF"]) {
        return Some(Scalar::from(match exercise(row) {
            'C' => "OCFXXX",
            'P' => "OPFXXX",
            _ => "OXFXXX",
        }));
    } else if member_upper(Some(security), &["CB"]) {
        "DCXXXX"
    } else if member_upper(Some(security), &["MTN", "EUMTN"]) {
        "DTXXXX"
    } else if member_upper(Some(security), &["ABS"]) {
        "DAXXXX"
    } else if member_upper(Some(security), CFI_MORTGAGE) {
        "DGXXXX"
    } else if member_upper(Some(security), CFI_CORPORATE) {
        "DBXXXX"
    } else if member_upper(Some(security), CFI_FLOATING) {
        "DBVXXX"
    } else if member_upper(Some(security), CFI_GOVERNMENT) {
        "DBXXXX"
    } else if member_upper(Some(security), CFI_MONEY_MARKET) {
        "DYXXXX"
    } else if member_upper(Some(security), CFI_MUNICIPAL) {
        "DNXXXX"
    } else if member_upper(Some(security), &["IRS"]) {
        "SRXXXX"
    } else if member_upper(Some(security), &["CDS"]) {
        "SCXXXX"
    } else if member_upper(Some(security), &["CMDTYSWAP"]) {
        "STXXXX"
    } else if member_upper(Some(security), &["FXSWAP"]) {
        "SFXXXX"
    } else if member_upper(Some(security), &["FXSPOT"]) {
        "IFXXXX"
    } else if member_upper(Some(security), &["FXFWD"]) {
        "JFXXXX"
    } else if member_upper(Some(security), &["FRA"]) {
        "JRXXXX"
    } else if member_upper(Some(security), &["EQFWD"]) {
        "JEXXXX"
    } else if member_upper(Some(security), &["REPO"]) {
        "LRXXXX"
    } else if member_upper(Some(security), &["SECLOAN"]) {
        "LSXXXX"
    } else if member_upper(Some(security), &["INDEX"]) {
        "TIXXXX"
    } else {
        return None;
    };
    Some(Scalar::from(cfi))
}

fn country_of_issue(row: &NativeRow<'_>) -> Option<Scalar> {
    let isin = stated_isin(row)?;
    let prefix = isin.prefix();
    StringEnum::COUNTRIES
        .binary_search(&prefix)
        .is_ok()
        .then(|| Scalar::from(prefix))
}

fn pegged_price(row: &NativeRow<'_>) -> Option<Scalar> {
    let reference = row.get(1095)?;
    let offset = exact_decimal(row, 211, 18)?;
    reference.checked_add(&offset).ok()
}

fn stated_isin(row: &NativeRow<'_>) -> Option<IsinCode> {
    let primary = row
        .text_is(22, "4")
        .then(|| row.get(48))
        .flatten()
        .and_then(|value| value.as_str().and_then(|text| IsinCode::new(text).ok()));
    primary.or_else(|| alternate_isin(row))
}

fn exercise(row: &NativeRow<'_>) -> char {
    match row.get(201).and_then(|value| value.as_i128()) {
        Some(1) => 'C',
        Some(0) => 'P',
        _ => 'X',
    }
}

fn add(row: &NativeRow<'_>, left: i32, right: i32) -> Option<Scalar> {
    row.get(left)?.checked_add(&row.get(right)?).ok()
}

/// One eagerly evaluated nullable sum. Absence is a null answer; arithmetic
/// failure is an error so a surrounding function cannot hide it by choosing
/// another argument.
fn evaluated_add(
    row: &NativeRow<'_>,
    left: i32,
    right: i32,
) -> std::result::Result<Option<Scalar>, ()> {
    let Some(left) = row.get(left) else {
        return Ok(None);
    };
    let Some(right) = row.get(right) else {
        return Ok(None);
    };
    left.checked_add(&right).map(Some).map_err(|_| ())
}

fn subtract(row: &NativeRow<'_>, left: i32, right: i32) -> Option<Scalar> {
    row.get(left)?.checked_sub(&row.get(right)?).ok()
}

fn multiply(row: &NativeRow<'_>, left: i32, right: i32) -> Option<Scalar> {
    row.get(left)?.checked_mul(&row.get(right)?).ok()
}

fn exact_decimal(row: &NativeRow<'_>, tag: i32, scale: i8) -> Option<Scalar> {
    let value = row.get(tag)?;
    let unscaled = if value.is_decimal() {
        value.decimal_unscaled_at(scale)?
    } else if let Some(integer) = value.as_i128() {
        integer.checked_mul(decimal_factor(scale)?)?
    } else {
        // This is the expression evaluator's numeric cast: scale the
        // float and truncate toward zero through the native integer cast.
        // The target below then checks decimal128's thirty-eight digits.
        let floating = value.as_f64()?;
        scaled_float(floating, decimal_factor(scale)?)
    };
    let dtype = match scale {
        9 => &DECIMAL9,
        18 => &DECIMAL18,
        _ => return None,
    };
    dtype.scalar(Scalar::d128(unscaled, scale)).ok()
}

fn scaled_product(row: &NativeRow<'_>, left: i32, right: i32, scale: i8) -> Option<Scalar> {
    exact_decimal(row, left, scale)?
        .checked_mul(&exact_decimal(row, right, scale)?)
        .ok()
}

fn decimal_factor(scale: i8) -> Option<i128> {
    match scale {
        9 => Some(1_000_000_000),
        18 => Some(1_000_000_000_000_000_000),
        _ => None,
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn scaled_float(value: f64, factor: i128) -> i128 {
    (value * factor as f64) as i128
}

fn starts_case(value: &str, prefix: &str) -> bool {
    if value.is_ascii() {
        return value
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix));
    }
    let folded = value.to_lowercase();
    let mut characters = folded.chars();
    prefix
        .chars()
        .all(|expected| characters.next() == Some(expected.to_ascii_lowercase()))
}

fn wildcard_o_f(value: &str) -> bool {
    if value.is_ascii() {
        let bytes = value.as_bytes();
        return bytes.len() >= 3
            && bytes[0].eq_ignore_ascii_case(&b'O')
            && bytes[2].eq_ignore_ascii_case(&b'F');
    }
    let folded = value.to_lowercase();
    let mut characters = folded.chars();
    characters.next() == Some('o') && characters.nth(1) == Some('f')
}

fn member_upper(value: Option<&str>, members: &[&str]) -> bool {
    value.is_some_and(|value| {
        if value.is_ascii() {
            members
                .iter()
                .any(|member| value.eq_ignore_ascii_case(member))
        } else {
            let upper = value.to_uppercase();
            members.contains(&upper.as_str())
        }
    })
}

const PRODUCT_AGENCY: &[&str] = &["EUSUPRA", "FAC", "FADN", "PEF", "SUPRA"];
const PRODUCT_CORPORATE: &[&str] = &[
    "CB",
    "CORP",
    "CPP",
    "DIMSUMCORP",
    "DUAL",
    "EUCORP",
    "EUFRN",
    "FRN",
    "PRCORP",
    "STRUCT",
    "XLINKD",
    "YANK",
];
const PRODUCT_CURRENCY: &[&str] = &[
    "FOR", "FXBN", "FXDN", "FXFWD", "FXNDF", "FXNDS", "FXSPOT", "FXSWAP",
];
const PRODUCT_EQUITY: &[&str] = &["CS", "DR", "PS"];
const PRODUCT_GOVERNMENT: &[&str] = &[
    "BRADY",
    "CAN",
    "CTB",
    "DIMSUMSOV",
    "EUSOV",
    "PROV",
    "SOV",
    "TB",
    "TBILL",
    "TBOND",
    "TCAL",
    "TFRN",
    "TINT",
    "TIPS",
    "TNOTE",
    "TPRN",
];
const PRODUCT_LOAN: &[&str] = &[
    "AMENDED", "BRIDGE", "DEFLTED", "DINP", "LOFC", "MATURED", "REPLACD", "RETIRED", "RVLV",
    "RVLVTRM", "SWING", "TERM", "WITHDRN",
];
const PRODUCT_MONEY_MARKET: &[&str] = &[
    "BA", "BAB", "BDN", "BN", "BNST", "BOX", "CAMM", "CD", "CL", "CLCP", "CN", "CP", "CPIB", "DN",
    "EUCD", "EUCP", "EUMTN", "EUNCP", "EUSTLQN", "EUTD", "JCD", "LQN", "MMF", "MN", "MTN", "NCD",
    "NCP", "ONITE", "PN", "PZFJ", "RCD", "SLQN", "STN", "TD", "TDR", "TLQN", "XCN", "YCD",
];
const PRODUCT_MORTGAGE: &[&str] = &[
    "ABS", "CMB", "CMBS", "CMO", "IET", "MBS", "MIO", "MPO", "MPP", "MPT", "PFAND", "TBA",
];
const PRODUCT_MUNICIPAL: &[&str] = &[
    "AN", "COFO", "COFP", "GO", "MCPIB", "MT", "RAN", "REV", "SPCLA", "SPCLO", "SPCLT", "TAN",
    "TAXA", "TECP", "TMB", "TMCP", "TRAN", "VRDN", "VRDO", "WAR",
];
const PRODUCT_FINANCING: &[&str] = &[
    "BUYSELL",
    "COLLBSKT",
    "DVPLDG",
    "FORWARD",
    "MRGNLOAN",
    "REPO",
    "SECLOAN",
    "SECPLEDGE",
    "SFP",
];

const CFI_MORTGAGE: &[&str] = &[
    "MBS", "CMBS", "CMO", "TBA", "PFAND", "MPT", "IET", "MIO", "MPO", "MPP", "CMB",
];
const CFI_CORPORATE: &[&str] = &[
    "CORP",
    "EUCORP",
    "YANK",
    "PRCORP",
    "DUAL",
    "XLINKD",
    "DIMSUMCORP",
];
const CFI_FLOATING: &[&str] = &["FRN", "EUFRN", "TFRN"];
const CFI_GOVERNMENT: &[&str] = &[
    "TBOND",
    "TNOTE",
    "SOV",
    "EUSOV",
    "BRADY",
    "PROV",
    "CAN",
    "DIMSUMSOV",
    "TIPS",
];
const CFI_MONEY_MARKET: &[&str] = &[
    "TBILL", "TB", "CTB", "CP", "CD", "BA", "BN", "CL", "DN", "EUCD", "EUCP", "LQN", "ONITE", "PN",
    "STN", "TD", "XCN", "YCD", "NCD", "NCP", "JCD", "RCD", "TDR", "TLQN", "SLQN", "CPIB", "CLCP",
    "CAMM", "BAB", "BDN", "BNST", "BOX", "CN", "EUNCP", "EUSTLQN", "EUTD", "MN", "PZFJ",
];
const CFI_MUNICIPAL: &[&str] = &[
    "GO", "REV", "AN", "COFO", "COFP", "MT", "RAN", "SPCLA", "SPCLO", "SPCLT", "TAN", "TAXA",
    "TECP", "TRAN", "VRDN", "VRDO", "TMB", "TMCP", "MCPIB",
];
