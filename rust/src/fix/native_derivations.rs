//! The derivations every parse runs: the one implementation of the fields a
//! message implies but did not carry.
//!
//! Twenty-nine rules, each keyed by the tag it fills and read off the
//! message's own children by tag - `secaltids` by its canonical name - so
//! they are the same rules whatever registry the message is typed by. A
//! registry states no rule of its own: its field for a target types the
//! answer, and a registry lacking that field answers nothing there.
//!
//! | target | fills with |
//! | --- | --- |
//! | `AvgPx(6)` | `LastPx`, on a report whose `CumQty` equals a positive `LastQty` |
//! | `CumQty(14)` | `OrderQty - LeavesQty` on a report, never negative |
//! | `Currency(15)` / `SettlCurrency(120)` | each other, except on a currency product (`Product` 4) |
//! | `SecurityIDSource(22)` | `4`, `1` or `2` where `SecurityID` closes as an ISIN, a CUSIP or a SEDOL |
//! | `LastPx(31)`, `BidPx(132)`, `OfferPx(133)` | spot rate plus forward points |
//! | `OrderQty(38)` | `CumQty + CxlQty` on a canceled report, else `CumQty + LeavesQty`, else `CumQty + CxlQty` |
//! | `OrdStatus(39)` | the `ExecType` both spell alike, or a trade's filled or partial status |
//! | `SecurityID(48)` | the first ISIN `secaltids` states |
//! | `Symbol(55)` | `SecurityID` under source `8` or `A`, else the `secaltids` identifier under `8` |
//! | `TimeInForce(59)` | `0`, a day order, on a `D`, `G` or `8` message |
//! | `SettlCurrAmt(119)` | `GrossTradeAmt * SettlCurrFxRate` at scale nine |
//! | `OrigSendingTime(122)` | `SendingTime` on a possible duplicate |
//! | `LeavesQty(151)` | `0` on a closed report, `OrderQty - CumQty` on a working one |
//! | `SecurityType(167)` | the type the CFI code's category names (Appendix 6-D) |
//! | `PutOrCall(201)` | an option CFI code's call or put |
//! | `GrossTradeAmt(381)` | `LastQty * LastPx` at scale nine on a report |
//! | `Product(460)` | the product group the security type is filed under, else the CFI category's |
//! | `CFICode(461)` | the CFI code a security type names |
//! | `CountryOfIssue(470)` | an ISIN's ISO 3166 prefix |
//! | `PeggedPrice(839)` | `PeggedRefPrice + PegOffsetValue` |
//! | `MinPriceIncrementAmount(1146)` | `MinPriceIncrement * ContractMultiplier` |
//! | `TotalTradeQty(2367)` | `LastQty * TradingUnitPeriodMultiplier` |
//! | `LastMultipliedQty(2368)` | `LastQty * ContractMultiplier` at scale nine |
//! | `TotalGrossTradeAmt(2369)` | `LastPx * TotalTradeQty` at scale nine |
//! | `TotalTradeMultipliedQty(2370)` | `TotalTradeQty * ContractMultiplier` at scale nine |
//! | `CurrencyCodeSource(2897)` | `6`, ISO 4217, wherever `Currency` is stated |
//!
//! A report is `MsgType` `8` or `9`. Arithmetic that overflows, a value the
//! target's field refuses and an input the message does not state are all
//! silence, never a guess.

use crate::{Cusip, DataType, Decimal, Isin, Result, Scalar, Sedol, StringEnum};

use super::msg::FixMsg;

/// How many targets [`derive_once`] fills: the bound on the sweeps a
/// fixpoint takes and the most answers one pass lands.
const RULE_COUNT: usize = 29;
const DECIMAL9: DataType = DataType::Decimal128 {
    precision: 38,
    scale: 9,
};
const DECIMAL18: DataType = DataType::DECIMAL;

/// Fill every derivation to a fixpoint and rebuild the message once.
pub(super) fn fill_all(msg: &mut FixMsg) -> Result<()> {
    let landed = NativeRow::new(msg).settle();
    if !landed.is_empty() {
        msg.set_each(landed)?;
    }
    Ok(())
}

/// Whether any rule answers for `msg` anew: false where the fixpoint
/// already stands, which one sweep proves.
pub(super) fn lands_anything(msg: &FixMsg) -> bool {
    !NativeRow::new(msg).settle().is_empty()
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
    /// stated. A stated null remains fillable.
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
        self.landed.iter().any(|(held, _)| *held == tag) || self.msg.states_indexed_tag(tag)
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

    fn decimal(&self, tag: i32) -> Option<Decimal> {
        Decimal::from_scalar(&self.get(tag)?)
    }

    /// The first alternate identifier under `source`: the first occurrence
    /// stating that source answers, so an absent member there is absence
    /// rather than permission to inspect a later occurrence.
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

    /// Type one answer through the registry's field for its tag before
    /// another rule can read it. Invalid identifiers, code values and
    /// overflows are silence, and so is a tag the registry holds no field for.
    fn put(&mut self, tag: i32, answer: Option<Scalar>) {
        let Some(answer) = answer.filter(|value| !value.is_null()) else {
            return;
        };
        let Some(field) = self.msg.registry().get_field_by_tag(tag) else {
            return;
        };
        // A decimal target takes the exact market decimal a numeric answer
        // restates as before the field is asked. In particular,
        // MinPriceIncrement * ContractMultiplier multiplies two floats whose
        // certain result belongs in decimal128(38,18).
        let answer = if field.dtype() == &DataType::DECIMAL {
            let Some(answer) = Decimal::from_scalar(&answer) else {
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
    fill!(15, the_other_currency(row, 120));
    fill!(22, security_id_source(row));
    fill!(31, add(row, 194, 195));
    fill!(38, order_quantity(row));
    fill!(39, order_status(row));
    fill!(48, alternate_isin(row).map(Scalar::from));
    fill!(55, symbol(row));
    fill!(59, time_in_force(row));
    fill!(119, scaled_product(row, 381, 155, 9));
    fill!(120, the_other_currency(row, 15));
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

/// `Currency(15)` and `SettlCurrency(120)` each state the other, except on a
/// message stated as a currency product (`product is distinct from 4`),
/// where the two are the two legs of the pair.
fn the_other_currency(row: &NativeRow<'_>, other: i32) -> Option<Scalar> {
    let currency_product = row.get(460).and_then(|value| value.as_i128()) == Some(4);
    (!currency_product).then(|| row.get(other)).flatten()
}

fn cumulative_quantity(row: &NativeRow<'_>) -> Option<Scalar> {
    if !row.text_in(35, REPORTS) {
        return None;
    }
    let difference = subtract(row, 38, 151)?;
    (!Decimal::from_scalar(&difference)?.is_negative()).then_some(difference)
}

fn security_id_source(row: &NativeRow<'_>) -> Option<Scalar> {
    let identifier = row.get(48)?;
    let text = identifier.as_str()?;
    if Isin::new(text).is_ok() {
        Some(Scalar::from("4"))
    } else if Cusip::new(text).is_ok() {
        Some(Scalar::from("1"))
    } else if Sedol::new(text).is_ok() {
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
    if canceled.is_some_and(Decimal::is_positive) && leaves.unwrap_or(Decimal::ZERO).is_zero() {
        return add(row, 14, 84);
    }
    // Both sums are evaluated: even when the first is stated, an overflow
    // in the fallback silences the rule, so an answer is never read off
    // arithmetic that failed beside it.
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

fn alternate_isin(row: &NativeRow<'_>) -> Option<Isin> {
    let alternate = row.alternate("4")?;
    Isin::new(alternate.as_str()?).ok()
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
    (!Decimal::from_scalar(&difference)?.is_negative()).then_some(difference)
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
    } else if starts_case(cfi, "O?F") {
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
    } else if starts_case(cfi, "SF???N") {
        "FXNDS"
    } else if starts_case(cfi, "SF") {
        "FXSWAP"
    } else if starts_case(cfi, "IF") {
        "FXSPOT"
    } else if starts_case(cfi, "JF???N") || starts_case(cfi, "JF??FC") {
        "FXNDF"
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
        "SFXXXP"
    } else if member_upper(Some(security), &["FXSPOT"]) {
        "IFXXXP"
    } else if member_upper(Some(security), &["FXFWD"]) {
        "JFTXFP"
    } else if member_upper(Some(security), &["FXNDF"]) {
        "JFTXFN"
    } else if member_upper(Some(security), &["FXNDS"]) {
        "SFXXXN"
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

fn stated_isin(row: &NativeRow<'_>) -> Option<Isin> {
    let primary = row
        .text_is(22, "4")
        .then(|| row.get(48))
        .flatten()
        .and_then(|value| value.as_str().and_then(|text| Isin::new(text).ok()));
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
        // Scale the float and truncate toward zero through the integer
        // cast; the target below then checks decimal128's thirty-eight
        // digits.
        let floating = value.as_f64()?;
        scaled_float(floating, decimal_factor(scale)?)
    };
    let dtype = match scale {
        9 => &DECIMAL9,
        18 => &DECIMAL18,
        _ => return None,
    };
    dtype.scalar(Scalar::decimal128(unscaled, scale)).ok()
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

/// Whether `value` opens with the ASCII `pattern`, case not counting and `?`
/// any one character: a CFI code's prefix, read as SQL's `ilike` reads one.
fn starts_case(value: &str, pattern: &str) -> bool {
    if value.is_ascii() {
        return value.len() >= pattern.len()
            && value
                .bytes()
                .zip(pattern.bytes())
                .all(|(held, expected)| expected == b'?' || held.eq_ignore_ascii_case(&expected));
    }
    let folded = value.to_lowercase();
    let mut characters = folded.chars();
    pattern.chars().all(|expected| {
        characters
            .next()
            .is_some_and(|held| expected == '?' || held == expected.to_ascii_lowercase())
    })
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
