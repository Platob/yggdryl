//! The fields a message implies but did not carry.
//!
//! A venue sends what its counterparty needs and nothing more, so a row is
//! routinely missing values the message itself already determines: an order
//! stating `OrderQty` and `CumQty` has said what `LeavesQty` is, a fill
//! stating `LastQty` and `LastPx` has said what it was worth, and a message
//! naming its instrument by an ISIN has said which country issued it. Every
//! consumer then derives those independently, which is how two systems come
//! to disagree about one message.
//!
//! The specification tabulates these rather than stating them as arithmetic:
//! FIX 4.4's Appendix D walks an order's whole life and shows what each
//! report carries at every step, FIX 4.2's Appendix O does the same for the
//! fields a foreign exchange trade settles on, and Appendix 6-D lists every
//! `SecurityType` under the ISO 10962 category its CFI code opens with. The
//! code sets say the rest: `SecurityIDSource` names the standard each code
//! stands for, and each standard closes its identifiers with a check digit;
//! the dictionary files every `SecurityType` under the `Product` group it
//! belongs to; `OrdStatus` and `ExecType` spell most of their values alike;
//! `TimeInForce` defines its own absence as a day order; and ISO 6166 opens a
//! number with the two letters ISO 3166 gives its issuing country. This
//! module is those tables read as the implications they are.
//!
//! # It is a table, not code per field
//!
//! A rule is a tag, the message types it speaks for, the conditions that must
//! hold, and how the value is derived. Adding one is a row, exactly as adding
//! a [lift](super::lift) is. There is no expression layer and no venue
//! condition: a rule the specification does not state is not a rule.
//!
//! # Enrichment never touches the entries
//!
//! The row is the interpretation and the entries are what arrived, so a
//! derived value goes to the row alone. Re-emitting an enriched message
//! therefore reproduces the received line byte for byte, which is the whole
//! reason the two facts are held apart. It also means enrichment is
//! idempotent: a stated value is never overwritten, so a value derived once
//! is a stated value the second time and derives to itself.
//!
//! # A rule answers only when the answer is certain
//!
//! Every input must be present and typed. A missing input, an untyped one, or
//! a condition that does not hold answers nothing rather than a guess, and a
//! rule that would contradict a stated value never runs at all. An identifier
//! that validates as no standard, a CFI whose category several security types
//! share, and a security type outside every group the specification files are
//! all silence. The cost of silence is a null column; the cost of a guess is
//! a wrong number nobody can tell from a sent one.

use crate::Scalar;
use crate::types::{Code, Isin, State, StringEnum};

use super::msg::FixMsg;
use super::registry::FixRegistry;
use super::{ISINCODE_TAG, MICCODE_TAG, STATE_TAG};

/// A condition one rule requires, read from the row.
struct FixWhen {
    /// The tag consulted.
    tag: i32,
    /// The values it must carry, as the row spells them. Empty means the tag
    /// need only be present.
    values: &'static [&'static str],
}

impl FixWhen {
    /// Whether the row satisfies this condition.
    fn holds(&self, msg: &FixMsg) -> bool {
        let Some(held) = msg.get_by_tag(self.tag) else {
            return false;
        };
        if held == &Scalar::Null {
            return false;
        }
        if self.values.is_empty() {
            return true;
        }
        // A condition is written in the codes the specification's own
        // matrices are written in. A state column holds the ranked value
        // rather than the code, so the code is read the way the column read
        // it before the two are compared; every other column holds the code.
        if let Scalar::Code(Code::State(state)) = held {
            return self
                .values
                .iter()
                .any(|value| State::from_spelling(value).as_ref() == Some(state));
        }
        let Some(rendered) = held.as_str() else {
            return false;
        };
        self.values
            .iter()
            .any(|value| value.eq_ignore_ascii_case(rendered))
    }
}

/// How one rule answers.
enum FixDerivation {
    /// `left - right`, refused below zero: a quantity cannot be negative, and
    /// a negative result means the two inputs were never about one order.
    Difference(i32, i32),
    /// `left + right`.
    Sum(i32, i32),
    /// `left * right`.
    Product(i32, i32),
    /// Whatever another tag carries, unchanged and re-typed for the column it
    /// lands in.
    Same(i32),
    /// The constant zero.
    Zero,
    /// A code the specification states as the meaning of an absence.
    Constant(&'static str),
    /// Whatever the first stated of several tags carries.
    First(&'static [i32]),
    /// One member of the first occurrence of a group whose other member
    /// states `wanted`: the `SecurityAltID` beside a source of `4`, say.
    Member {
        group: i32,
        member: i32,
        by: i32,
        wanted: &'static str,
    },
    /// The `SecurityIDSource` code of the standard a tag's value validates
    /// as, each read by its own check digit.
    Source(i32),
    /// The country whose two letters open the ISIN a tag carries, where ISO
    /// 3166 lists them as one.
    Country(i32),
    /// A table read by the prefix a tag's value opens with. `?` in a pattern
    /// stands for any one character, and the first pattern to match answers,
    /// so a longer pattern is listed before the shorter one it refines.
    Prefix(i32, &'static [(&'static str, &'static str)]),
    /// The CFI a security type states, with the exercise character an
    /// option's `PutOrCall` supplies.
    Cfi { securitytype: i32, putorcall: i32 },
    /// The group the dictionary's own code set files a tag's value under,
    /// read through a table of what each group means.
    Group(i32, &'static [(&'static str, &'static str)]),
    /// Filled where nothing is left, else partially filled where something
    /// is left and something was done.
    Filled { left: i32, done: i32 },
}

/// One field a message implies, and what implies it.
struct FixRule {
    /// The tag filled.
    tag: i32,
    /// The message types this rule speaks for; empty means all of them.
    msgtypes: &'static [&'static str],
    /// Every condition that must hold before the rule runs.
    when: &'static [FixWhen],
    /// How the value is derived.
    from: FixDerivation,
}

/// The message types that report an order's state.
const REPORTS: &[&str] = &["8", "9"];

/// The message types that carry a `TimeInForce`: an order, a replace and the
/// report on either.
const TIMED: &[&str] = &["D", "G", "8"];

/// The order statuses that leave quantity still working.
///
/// Appendix D's matrices: a `New`, `PartiallyFilled`, `PendingCancel`,
/// `PendingReplace`, `Replaced` or `Stopped` order still has quantity the
/// market can fill, so what is left is what was asked for minus what was
/// done. `Suspended` is here too - the order exists and is not working, but
/// its remainder is unchanged.
const WORKING: &[&str] = &["0", "1", "6", "E", "5", "7", "9"];

/// The order statuses that leave nothing working.
///
/// A `Filled`, `DoneForDay`, `Canceled`, `Rejected` or `Expired` order has no
/// remainder, whatever the arithmetic of the other two would say. Appendix D
/// shows `LeavesQty` at zero on every one of these.
const CLOSED: &[&str] = &["2", "3", "4", "8", "C"];

/// The execution types whose value `OrdStatus` spells with the same meaning.
///
/// The two code sets agree on `New`, `DoneForDay`, `Canceled`, `Replaced`,
/// `PendingCancel`, `Stopped`, `Rejected`, `Suspended`, `PendingNew`,
/// `Calculated`, `Expired` and `PendingReplace`. `D` is left out because it
/// is `Restated` in one and `AcceptedForBidding` in the other, and a trade
/// says what happened rather than what the order is.
const AGREED: &[&str] = &["0", "3", "4", "5", "6", "7", "8", "9", "A", "B", "C", "E"];

/// The execution types that report a trade: a fill, or a correction to one.
const TRADES: &[&str] = &["F", "G"];

/// The `SecurityIDSource` codes under which `SecurityID` is a symbol: an
/// exchange's, or Bloomberg's.
const SYMBOLS: &[&str] = &["8", "A"];

/// The `SecurityIDSource` code of an ISIN.
const ISIN: &str = "4";

/// Appendix 6-D read at its category level: the one `SecurityType` a CFI
/// category or group names.
///
/// `O?F` - an option on a future - is read before the `O` every other listed
/// option opens with. A category several types share, such as the `DB` of
/// every plain bond, is absent because no one type is certain of it.
const SECURITYTYPE_OF_CFI: &[(&str, &str)] = &[
    ("ES", "CS"),
    ("EP", "PS"),
    ("ED", "DR"),
    ("EU", "MF"),
    ("CE", "ETF"),
    ("CI", "MF"),
    ("F", "FUT"),
    ("O?F", "OOF"),
    ("O", "OPT"),
    ("H", "OPT"),
    ("DC", "CB"),
    ("DT", "MTN"),
    ("DA", "ABS"),
    ("DG", "MBS"),
    ("SR", "IRS"),
    ("SC", "CDS"),
    ("ST", "CMDTYSWAP"),
    ("SF", "FXSWAP"),
    ("IF", "FXSPOT"),
    ("JF", "FXFWD"),
    ("JR", "FRA"),
    ("JE", "EQFWD"),
    ("LR", "REPO"),
    ("LS", "SECLOAN"),
    ("TI", "INDEX"),
];

/// The exercise character of a listed or an unlisted option, and what it
/// says about `PutOrCall`.
const PUTORCALL_OF_CFI: &[(&str, &str)] = &[("OC", "1"), ("OP", "0"), ("HC", "1"), ("HP", "0")];

/// The `Product` a CFI category alone decides: an equity, or a financing.
const PRODUCT_OF_CFI: &[(&str, &str)] = &[("E", "5"), ("L", "13")];

/// The mortgage-backed security types.
const MORTGAGE: &[&str] = &[
    "MBS", "CMBS", "CMO", "TBA", "PFAND", "MPT", "IET", "MIO", "MPO", "MPP", "CMB",
];

/// The corporate bond security types at a fixed rate.
const CORPORATE: &[&str] = &[
    "CORP",
    "EUCORP",
    "YANK",
    "PRCORP",
    "DUAL",
    "XLINKD",
    "DIMSUMCORP",
];

/// The floating rate note security types.
const FLOATING: &[&str] = &["FRN", "EUFRN", "TFRN"];

/// The government bond security types.
const GOVERNMENT: &[&str] = &[
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

/// The bill and money market security types.
const MONEY_MARKET: &[&str] = &[
    "TBILL", "TB", "CTB", "CP", "CD", "BA", "BN", "CL", "DN", "EUCD", "EUCP", "LQN", "ONITE", "PN",
    "STN", "TD", "XCN", "YCD", "NCD", "NCP", "JCD", "RCD", "TDR", "TLQN", "SLQN", "CPIB", "CLCP",
    "CAMM", "BAB", "BDN", "BNST", "BOX", "CN", "EUNCP", "EUSTLQN", "EUTD", "MN", "PZFJ",
];

/// The municipal security types.
const MUNICIPAL: &[&str] = &[
    "GO", "REV", "AN", "COFO", "COFP", "MT", "RAN", "SPCLA", "SPCLO", "SPCLT", "TAN", "TAXA",
    "TECP", "TRAN", "VRDN", "VRDO", "TMB", "TMCP", "MCPIB",
];

/// Appendix 6-D read the other way: the CFI a `SecurityType` states, down to
/// the category and group the type names and `X` where it says nothing more.
/// `?` is the exercise character an option's `PutOrCall` supplies.
const CFI_OF_SECURITYTYPE: &[(&[&str], &str)] = &[
    (&["CS"], "ESXXXX"),
    (&["PS"], "EPXXXX"),
    (&["DR"], "EDXXXX"),
    (&["MF", "MMF"], "CIXXXX"),
    (&["ETF"], "CEXXXX"),
    (&["FUT"], "FXXXXX"),
    (&["OPT", "OOP", "OOC"], "O?XXXX"),
    (&["OOF"], "O?FXXX"),
    (&["CB"], "DCXXXX"),
    (&["MTN", "EUMTN"], "DTXXXX"),
    (&["ABS"], "DAXXXX"),
    (MORTGAGE, "DGXXXX"),
    (CORPORATE, "DBXXXX"),
    (FLOATING, "DBVXXX"),
    (GOVERNMENT, "DBXXXX"),
    (MONEY_MARKET, "DYXXXX"),
    (MUNICIPAL, "DNXXXX"),
    (&["IRS"], "SRXXXX"),
    (&["CDS"], "SCXXXX"),
    (&["CMDTYSWAP"], "STXXXX"),
    (&["FXSWAP"], "SFXXXX"),
    (&["FXSPOT"], "IFXXXX"),
    (&["FXFWD"], "JFXXXX"),
    (&["FRA"], "JRXXXX"),
    (&["EQFWD"], "JEXXXX"),
    (&["REPO"], "LRXXXX"),
    (&["SECLOAN"], "LSXXXX"),
    (&["INDEX"], "TIXXXX"),
];

/// The `Product` code each group of the `SecurityType` code set names.
///
/// The dictionary files every security type under the group the
/// specification lists it in, and the `Product` code set spells those
/// groups. `Derivatives` and `Other` are absent: the first spans products
/// the specification codes separately, and the second is where the
/// specification put what it could not place.
const PRODUCT_OF_GROUP: &[(&str, &str)] = &[
    ("Agency", "1"),
    ("Corporate", "3"),
    ("Currency", "4"),
    ("Equity", "5"),
    ("Government", "6"),
    ("Loan", "8"),
    ("Money Market", "9"),
    ("Mortgage", "10"),
    ("Municipal", "11"),
    ("Financing", "13"),
];

/// Every rule, in the order they are applied.
///
/// Order matters only where one rule's output is another's input, and each
/// such chain is laid out so one pass reaches its end: a `SecurityID`'s
/// validation states the source, under which the ISIN column is read, and
/// an ISIN found only among the alternate identifiers becomes the
/// `SecurityID`, whose validation states the source in turn; a
/// `SecurityType` read off a CFI places the `Product`; an `OrdStatus` read
/// off an `ExecType` decides what is left; and `GrossTradeAmt` is derived
/// before `SettlCurrAmt`, which is derived from it.
///
/// The primary identifier is read before the alternate ones, as the crate's
/// own column is defined: a message stating both states its ISIN in
/// `SecurityID`, and the alternate answers only where the primary did not.
static RULES: &[FixRule] = &[
    // The `SecurityIDSource` code set names the standard each code stands
    // for, and ISO 6166, CUSIP and SEDOL each close an identifier with a
    // check digit: a `SecurityID` one of them closes names its own source.
    FixRule {
        tag: 22,
        msgtypes: &[],
        when: &[],
        from: FixDerivation::Source(48),
    },
    // ISO 6166, in the primary identifier: `SecurityID` under an ISIN source.
    FixRule {
        tag: ISINCODE_TAG,
        msgtypes: &[],
        when: &[FixWhen {
            tag: 22,
            values: &[ISIN],
        }],
        from: FixDerivation::Same(48),
    },
    // ISO 6166, in the alternate identifiers: the `SecurityAltID` whose
    // source says ISIN is the instrument's ISIN where the primary was not.
    FixRule {
        tag: ISINCODE_TAG,
        msgtypes: &[],
        when: &[],
        from: FixDerivation::Member {
            group: 454,
            member: 455,
            by: 456,
            wanted: ISIN,
        },
    },
    // A message stating its ISIN and no `SecurityID` - a bridge row's
    // `ISINCODE`, or an alternate identifier alone - has stated its
    // `SecurityID`, and the validation below then states the source. The
    // source is read a second time because the first reading found no
    // `SecurityID` to validate; a `SecurityID` it did read is stated by now,
    // and a source it did state stands.
    FixRule {
        tag: 48,
        msgtypes: &[],
        when: &[],
        from: FixDerivation::Same(ISINCODE_TAG),
    },
    FixRule {
        tag: 22,
        msgtypes: &[],
        when: &[],
        from: FixDerivation::Source(48),
    },
    // ISO 6166 opens a number with the ISO 3166 code of the country whose
    // agency numbered it, where one did: `XS` and `EU` are agencies and not
    // countries, and say nothing about the issue.
    FixRule {
        tag: 470,
        msgtypes: &[],
        when: &[],
        from: FixDerivation::Country(ISINCODE_TAG),
    },
    // A `SecurityID` under an exchange's or Bloomberg's source is the symbol,
    // and so is the `SecurityAltID` an exchange gave.
    FixRule {
        tag: 55,
        msgtypes: &[],
        when: &[FixWhen {
            tag: 22,
            values: SYMBOLS,
        }],
        from: FixDerivation::Same(48),
    },
    FixRule {
        tag: 55,
        msgtypes: &[],
        when: &[],
        from: FixDerivation::Member {
            group: 454,
            member: 455,
            by: 456,
            wanted: "8",
        },
    },
    // Appendix 6-D, both ways, and the exercise character of an option.
    FixRule {
        tag: 167,
        msgtypes: &[],
        when: &[],
        from: FixDerivation::Prefix(461, SECURITYTYPE_OF_CFI),
    },
    FixRule {
        tag: 461,
        msgtypes: &[],
        when: &[],
        from: FixDerivation::Cfi {
            securitytype: 167,
            putorcall: 201,
        },
    },
    FixRule {
        tag: 201,
        msgtypes: &[],
        when: &[],
        from: FixDerivation::Prefix(461, PUTORCALL_OF_CFI),
    },
    // The `SecurityType` code set's own groups, as the `Product` code set
    // spells them; else the two CFI categories that are one product alone.
    FixRule {
        tag: 460,
        msgtypes: &[],
        when: &[],
        from: FixDerivation::Group(167, PRODUCT_OF_GROUP),
    },
    FixRule {
        tag: 460,
        msgtypes: &[],
        when: &[],
        from: FixDerivation::Prefix(461, PRODUCT_OF_CFI),
    },
    // The crate's own market column: the exchange the instrument is listed
    // on, the destination it was routed to, or the market it last traded on.
    FixRule {
        tag: MICCODE_TAG,
        msgtypes: &[],
        when: &[],
        from: FixDerivation::First(&[207, 100, 30]),
    },
    // `TimeInForce` defines its own absence: an order, a replace or a report
    // stating none is a day order.
    FixRule {
        tag: 59,
        msgtypes: TIMED,
        when: &[],
        from: FixDerivation::Constant("0"),
    },
    // A report stating an execution type the two code sets spell alike has
    // stated its order status; a trade has stated it in what is left and
    // what was done.
    FixRule {
        tag: 39,
        msgtypes: REPORTS,
        when: &[FixWhen {
            tag: 150,
            values: AGREED,
        }],
        from: FixDerivation::Same(150),
    },
    FixRule {
        tag: 39,
        msgtypes: REPORTS,
        when: &[FixWhen {
            tag: 150,
            values: TRADES,
        }],
        from: FixDerivation::Filled {
            left: 151,
            done: 14,
        },
    },
    // The crate's own lifecycle column: the order's status, else what the
    // report said happened.
    FixRule {
        tag: STATE_TAG,
        msgtypes: &[],
        when: &[],
        from: FixDerivation::First(&[39, 150]),
    },
    // Appendix D, every matrix: what is left is what was ordered minus what
    // was done, and nothing is left once the order is closed.
    FixRule {
        tag: 151,
        msgtypes: REPORTS,
        when: &[FixWhen {
            tag: 39,
            values: CLOSED,
        }],
        from: FixDerivation::Zero,
    },
    FixRule {
        tag: 151,
        msgtypes: REPORTS,
        when: &[FixWhen {
            tag: 39,
            values: WORKING,
        }],
        from: FixDerivation::Difference(38, 14),
    },
    // The same identity read the other two ways. A report stating what is
    // left and what was done has stated what was ordered.
    FixRule {
        tag: 38,
        msgtypes: REPORTS,
        when: &[],
        from: FixDerivation::Sum(14, 151),
    },
    FixRule {
        tag: 14,
        msgtypes: REPORTS,
        when: &[],
        from: FixDerivation::Difference(38, 151),
    },
    // A fill's worth, which Appendix O settles on and Appendix D's execution
    // reports carry.
    FixRule {
        tag: 381,
        msgtypes: REPORTS,
        when: &[],
        from: FixDerivation::Product(32, 31),
    },
    // Appendix O: the settled amount is the traded amount at the stated rate.
    FixRule {
        tag: 119,
        msgtypes: &[],
        when: &[],
        from: FixDerivation::Product(381, 155),
    },
    // Appendix O: a trade settling in the currency it was dealt in states the
    // dealt currency once. `SettlCurrency` absent means "the same one", which
    // is a default rather than an absence, and a reader joining two captures
    // on the settlement currency needs it stated - and a trade stating only
    // the currency it settles in has said which it was dealt in.
    FixRule {
        tag: 120,
        msgtypes: &[],
        when: &[],
        from: FixDerivation::Same(15),
    },
    FixRule {
        tag: 15,
        msgtypes: &[],
        when: &[],
        from: FixDerivation::Same(120),
    },
    // The average of one fill is that fill's price. Stated only where the
    // report says the whole done quantity is this fill, because an average
    // over two fills is not derivable from one of them.
    FixRule {
        tag: 6,
        msgtypes: REPORTS,
        when: &[],
        from: FixDerivation::Same(31),
    },
];

/// The value one derivation answers, or nothing when it cannot be certain.
fn derive(registry: &FixRegistry, msg: &FixMsg, from: &FixDerivation) -> Option<Scalar> {
    let stated = |tag: i32| msg.get_by_tag(tag).filter(|held| !held.is_null());
    let number = |tag: i32| stated(tag).and_then(Scalar::as_f64);
    let text = |tag: i32| stated(tag).and_then(Scalar::as_str);
    match from {
        FixDerivation::Zero => Some(Scalar::from(0.0_f64)),
        FixDerivation::Constant(code) => Some(Scalar::from(*code)),
        FixDerivation::Same(tag) => stated(*tag).cloned(),
        FixDerivation::First(tags) => tags.iter().find_map(|tag| stated(*tag).cloned()),
        FixDerivation::Sum(left, right) => Some(Scalar::from(number(*left)? + number(*right)?)),
        FixDerivation::Product(left, right) => Some(Scalar::from(number(*left)? * number(*right)?)),
        FixDerivation::Difference(left, right) => {
            let held = number(*left)? - number(*right)?;
            // A negative remainder means the two inputs were never about one
            // order, and answering it would state a quantity that cannot
            // exist.
            (held >= 0.0).then(|| Scalar::from(held))
        }
        FixDerivation::Member {
            group,
            member,
            by,
            wanted,
        } => msg.group_member_where(*group, *member, *by, wanted),
        FixDerivation::Source(tag) => {
            let held = text(*tag)?;
            let code = if Isin::is_valid(held) {
                ISIN
            } else if is_cusip(held) {
                "1"
            } else if is_sedol(held) {
                "2"
            } else {
                return None;
            };
            Some(Scalar::from(code))
        }
        FixDerivation::Country(tag) => {
            let isin = Isin::new(text(*tag)?).ok()?;
            let prefix = isin.prefix();
            StringEnum::COUNTRIES
                .binary_search(&prefix)
                .is_ok()
                .then(|| Scalar::from(prefix))
        }
        FixDerivation::Prefix(tag, table) => {
            let held = text(*tag)?;
            table
                .iter()
                .find(|(pattern, _)| opens_with(held, pattern))
                .map(|(_, answer)| Scalar::from(*answer))
        }
        FixDerivation::Cfi {
            securitytype,
            putorcall,
        } => {
            let held = text(*securitytype)?;
            let (_, pattern) = CFI_OF_SECURITYTYPE
                .iter()
                .find(|(types, _)| types.iter().any(|known| known.eq_ignore_ascii_case(held)))?;
            // `PutOrCall` is `1` for a call and `0` for a put; an option
            // stating neither is an option whose exercise the CFI leaves
            // open, which `X` is the code for.
            let exercise = match stated(*putorcall).and_then(Scalar::as_i128) {
                Some(1) => 'C',
                Some(0) => 'P',
                _ => 'X',
            };
            let rendered: String = pattern
                .chars()
                .map(|held| if held == '?' { exercise } else { held })
                .collect();
            Some(Scalar::from(rendered))
        }
        FixDerivation::Group(tag, table) => {
            let held = text(*tag)?;
            let group = registry
                .get_field_by_tag(*tag)?
                .as_fix()
                .code(held)?
                .group()?;
            table
                .iter()
                .find(|(known, _)| *known == group)
                .map(|(_, answer)| Scalar::from(*answer))
        }
        FixDerivation::Filled { left, done } => {
            let left = number(*left)?;
            if left == 0.0 {
                return Some(Scalar::from("2"));
            }
            (left > 0.0 && number(*done)? > 0.0).then(|| Scalar::from("1"))
        }
    }
}

/// Whether `text` opens with `pattern`, `?` standing for any one character
/// and case not counting.
fn opens_with(text: &str, pattern: &str) -> bool {
    text.len() >= pattern.len()
        && pattern
            .bytes()
            .zip(text.bytes())
            .all(|(wanted, held)| wanted == b'?' || wanted.eq_ignore_ascii_case(&held))
}

/// The value an identifier's check reads one character as: a digit as
/// itself, a letter as ten plus its alphabet position.
fn identifier_digit(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some(u32::from(byte - b'0')),
        b'A'..=b'Z' => Some(u32::from(byte - b'A') + 10),
        b'a'..=b'z' => Some(u32::from(byte - b'a') + 10),
        _ => None,
    }
}

/// Whether `text` is a CUSIP: eight characters closed by their modulus-10
/// "double-add-double" digit, every second character doubled and the digits
/// of each product summed.
fn is_cusip(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() != 9 || !bytes[8].is_ascii_digit() {
        return false;
    }
    let mut sum = 0;
    for (index, byte) in bytes[..8].iter().enumerate() {
        let Some(mut value) = identifier_digit(*byte) else {
            return false;
        };
        if index % 2 == 1 {
            value *= 2;
        }
        sum += value / 10 + value % 10;
    }
    (10 - sum % 10) % 10 == u32::from(bytes[8] - b'0')
}

/// Whether `text` is a SEDOL: six characters weighted `1, 3, 1, 7, 3, 9` and
/// closed by their modulus-10 digit.
fn is_sedol(text: &str) -> bool {
    const WEIGHTS: [u32; 6] = [1, 3, 1, 7, 3, 9];
    let bytes = text.as_bytes();
    if bytes.len() != 7 || !bytes[6].is_ascii_digit() {
        return false;
    }
    let mut sum = 0;
    for (byte, weight) in bytes[..6].iter().zip(WEIGHTS) {
        let Some(value) = identifier_digit(*byte) else {
            return false;
        };
        sum += value * weight;
    }
    (10 - sum % 10) % 10 == u32::from(bytes[6] - b'0')
}

/// Whether the average price this report states is one fill's price.
///
/// `AvgPx` over two fills is not derivable from one of them, so the rule runs
/// only where the report says the whole done quantity arrived in this fill.
fn single_fill(msg: &FixMsg) -> bool {
    let (Some(done), Some(last)) = (msg.get_by_tag(14), msg.get_by_tag(32)) else {
        return false;
    };
    match (done.as_f64(), last.as_f64()) {
        (Some(done), Some(last)) => done == last && last > 0.0,
        _ => false,
    }
}

/// Fills what `msg` implies, leaving what it stated and what arrived alone.
///
/// Every answer lands through [`FixMsg::set`]: a row already holding the tag
/// holds a stated null - a spelling the field reads as nothing sent - and the
/// answer takes that child's place, anything else is appended. The
/// dictionary's own field types the value on the way in, so a derived column
/// is indistinguishable from a stated one and carries the same display,
/// description and `fix:tag` a reader resolves it by - and a value it
/// refuses, an identifier the check digit does not close, is silence: a
/// refused write leaves the row exactly as it was, and there is nothing
/// else here that can fail.
pub(super) fn enrich(registry: &FixRegistry, msg: FixMsg) -> FixMsg {
    let msgtype = msg
        .get_by_tag(35)
        .and_then(Scalar::as_str)
        .unwrap_or_default()
        .to_owned();

    let mut held = msg;
    for rule in RULES {
        if !rule.msgtypes.is_empty() && !rule.msgtypes.iter().any(|known| *known == msgtype) {
            continue;
        }
        // A stated value is never overwritten, which is what makes this
        // idempotent: the second pass finds the first pass's answer stated.
        if held
            .get_by_tag(rule.tag)
            .is_some_and(|value| value != &Scalar::Null)
        {
            continue;
        }
        if !rule.when.iter().all(|when| when.holds(&held)) {
            continue;
        }
        if rule.tag == 6 && !single_fill(&held) {
            continue;
        }
        let Some(value) = derive(registry, &held, &rule.from) else {
            continue;
        };
        // A rule whose output another rule reads has to be visible to it, so
        // each answer lands before the next rule runs rather than once at
        // the end.
        let _ = held.set(rule.tag, value);
    }
    held
}
