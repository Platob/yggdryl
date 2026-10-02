//! The type of name an identifier is: one vocabulary for security,
//! alternate and party identifiers.
//!
//! [`IdType`] holds the words the crate names as members - every source
//! FIX's `SecurityIDSource(22)` code set names, the operation identifiers a
//! FIX message states, the party roles a FIX flow commonly carries - each a
//! static word that costs nothing to hold or compare, and any other word as
//! [`IdType::Other`]. A security type carries the rule its codes follow: an
//! ISIN closes on its check digit, a pair is stored canonical.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::LazyLock;

use smol_str::format_smolstr;

use crate::bbg::BBG_WIDTH;
use crate::identifier::{IDENTIFIER_KEY_WIDTH, IDENTIFIER_VALUE_WIDTH, id_vocabulary};
use crate::ric::RIC_WIDTH;
use crate::{Ccy, Cfi, Country, Cusip, Error, Figi, Forex, Isin, Result, Ric, Sedol};

id_vocabulary! {
    /// The type of name an identifier is: `isin`, `clordid`,
    /// `executingtrader`.
    ///
    /// A spelling folds to lower case without the `_`, `-`, space and `#` a
    /// spelling breaks it with, so `ISIN_Number` and `isinnumber` are both
    /// [`IdType::Isin`]; each member also reads by the code-set names FIX
    /// gives it. Any other folded word is an [`IdType::Other`].
    ///
    /// ```
    /// use yggdryl::IdType;
    ///
    /// assert_eq!("ISIN_Number".parse::<IdType>().unwrap(), IdType::Isin);
    /// assert_eq!("bloombergsymbol".parse::<IdType>().unwrap(), IdType::Bloomberg);
    /// assert_eq!(IdType::ClOrdId.as_str(), "clordid");
    /// let house = "House Code".parse::<IdType>().unwrap();
    /// assert!(!house.is_known());
    /// assert_eq!(house.as_str(), "housecode");
    /// ```
    IdType, "an identifier type" {
        /// A CUSIP, FIX `SecurityIDSource(22)` code `1`.
        Cusip => "cusip",
        /// A SEDOL, code `2`.
        Sedol => "sedol",
        /// A QUIK code, code `3`.
        Quik => "quik",
        /// An ISIN, code `4`.
        Isin => "isin" | "isinnumber",
        /// A Reuters instrument code, code `5`.
        Ric => "ric" | "riccode",
        /// An ISO 4217 currency code, code `6`.
        IsoCcy => "isoccy" | "isocurrencycode",
        /// An ISO 3166 country code, code `7`.
        IsoCtry => "isoctry" | "isocountrycode",
        /// An exchange symbol, code `8`.
        ExchSymb => "exchsymb" | "exchangesymbol",
        /// A Consolidated Tape Association symbol, code `9`.
        Cta => "cta" | "consolidatedtapeassociation" | "ctasymbol" | "consolidatedtapeassociationsymbol",
        /// A Bloomberg symbol, code `A`.
        Bloomberg => "bloomberg" | "bbgsymb" | "bloombergsymbol",
        /// A Wertpapierkennnummer, code `B`.
        Wkn => "wkn" | "wertpapier",
        /// A Dutch security code, code `C`.
        Dutch => "dutch",
        /// A Valor number, code `D`.
        Valor => "valor" | "valoren",
        /// A SICOVAM code, code `E`.
        Sicovam => "sicovam",
        /// A Belgian security code, code `F`.
        Belgian => "belgian",
        /// A Common Code of Clearstream and Euroclear, code `G`.
        Common => "common" | "commoncode",
        /// A clearing house's or clearing organization's own code, code `H`.
        /// `clearingorganization` alone is the party role `PartyRole(452)`
        /// `21` names.
        ClearingHouse => "clearinghouse" | "clearinghouseclearingorganization",
        /// An ISDA FpML product specification, code `I`.
        FpmlSpec => "fpmlspec" | "isdafpmlspecification" | "isdafpmlproductspecification",
        /// An OPRA option symbol, code `J`.
        Opra => "opra" | "optionpricereportingauthority" | "optionspricereportingauthority",
        /// An ISDA FpML product URL, code `K`.
        FpmlUrl => "fpmlurl" | "isdafpmlurl" | "isdafpmlproducturl",
        /// A letter of credit, code `L`.
        Loc => "loc" | "letterofcredit",
        /// A marketplace's own identifier, code `M`.
        MktAssigned => "mktassigned" | "marketplaceassignedidentifier",
        /// A Markit RED entity CLIP, code `N`.
        RedEntity => "redentity" | "markitredentityclip",
        /// A Markit RED pair CLIP, code `P`.
        RedPair => "redpair" | "markitredpairclip",
        /// A CFTC commodity code, code `Q`.
        Cftc => "cftc" | "cftccommoditycode",
        /// An ISDA commodity reference price, code `R`.
        IsdaCommodity => "isdacommodity" | "isdacommodityreferenceprice",
        /// A FIGI, code `S`.
        Figi => "figi" | "financialinstrumentglobalidentifier",
        /// A legal entity identifier, code `T`.
        Lei => "lei" | "legalentityidentifier",
        /// A synthetic instrument, code `U`.
        Synthetic => "synthetic",
        /// A Fidessa instrument mnemonic, code `V`.
        Fim => "fim" | "fidessainstrumentmnemonic",
        /// An index name, code `W`.
        Index => "index" | "indexname",
        /// A uniform symbol, code `X`.
        Umtf => "umtf" | "uniformsymbol",
        /// A digital token identifier, code `Y`.
        Dti => "dti" | "digitaltokenidentifier",
        /// A currency pair, canonical `CCY/CCY` - the crate's own, which FIX
        /// gives no code.
        Forex => "forex" | "forexcode" | "ccypair" | "currencypair",
        /// An ISO 10962 classification.
        Cfi => "cfi" | "cficode",
        /// A venue's or a bridge's own instrument key.
        InstrumentId => "instrumentid",
        /// `OrderID(37)`.
        OrderId => "orderid",
        /// `ClOrdID(11)`.
        ClOrdId => "clordid",
        /// `OrigClOrdID(41)`, `clordid`'s one parent, which a bridge also
        /// spells `parentclordid`.
        OrigClOrdId => "origclordid" | "parentclordid",
        /// `SecondaryOrderID(198)`.
        SecondaryOrderId => "secondaryorderid",
        /// `SecondaryClOrdID(526)`.
        SecondaryClOrdId => "secondaryclordid",
        /// `SecondaryExecID(527)`.
        SecondaryExecId => "secondaryexecid",
        /// `SecondaryQuoteID(1751)`.
        SecondaryQuoteId => "secondaryquoteid",
        /// `SecondaryTradeID(1040)`.
        SecondaryTradeId => "secondarytradeid",
        /// `SecondaryFirmTradeID(1042)`.
        SecondaryFirmTradeId => "secondaryfirmtradeid",
        /// `SecondaryAllocID(793)`.
        SecondaryAllocId => "secondaryallocid",
        /// `SecondaryIndividualAllocID(989)`.
        SecondaryIndividualAllocId => "secondaryindividualallocid",
        /// `ExecID(17)`.
        ExecId => "execid",
        /// `QuoteID(117)`.
        QuoteId => "quoteid",
        /// `QuoteReqID(131)`.
        QuoteReqId => "quotereqid",
        /// `MDReqID(262)`.
        MdReqId => "mdreqid",
        /// `TrdMatchID(880)`.
        TrdMatchId => "trdmatchid",
        /// `TradeID(1003)`.
        TradeId => "tradeid",
        /// `TradeReportID(571)`.
        TradeReportId => "tradereportid",
        /// `MDEntryID(278)`.
        MdEntryId => "mdentryid",
        /// `MDEntryRefID(280)`.
        MdEntryRefId => "mdentryrefid",
        /// The current regulatory trade identifier, `RegulatoryTradeIDType(1906)`
        /// `0`.
        RegTradeId => "regtradeid",
        /// The previous regulatory trade identifier, type `1`.
        PrevRegTradeId => "prevregtradeid",
        /// A block's regulatory trade identifier, type `2`.
        BlockRegTradeId => "blockregtradeid",
        /// A related regulatory trade identifier, type `3`.
        RelatedRegTradeId => "relatedregtradeid",
        /// A cleared block's regulatory trade identifier, type `4`.
        ClearedRegTradeId => "clearedregtradeid",
        /// A trading venue transaction identification code, type `5`.
        Tvtic => "tvtic" | "tradingvenuetransactionidentifier",
        /// A report tracking number, type `6`.
        ReportTrackingNumber => "reporttrackingnumber",
        /// The account an order is booked to, `Account(1)`.
        Account => "account",
        /// A party whose role nothing states.
        Party => "party",
        /// A user of a system.
        UserId => "userid",
        /// `PartyRole(452)` `1`.
        ExecutingFirm => "executingfirm",
        /// `PartyRole(452)` `3`.
        ClientId => "clientid",
        /// `PartyRole(452)` `4`.
        ClearingFirm => "clearingfirm",
        /// `PartyRole(452)` `5`.
        InvestorId => "investorid",
        /// `PartyRole(452)` `7`.
        EnteringFirm => "enteringfirm",
        /// `PartyRole(452)` `11`.
        OrderOriginationTrader => "orderoriginationtrader",
        /// `PartyRole(452)` `12`.
        ExecutingTrader => "executingtrader",
        /// `PartyRole(452)` `13`.
        OrderOriginationFirm => "orderoriginationfirm",
        /// `PartyRole(452)` `16`.
        ExecutingSystem => "executingsystem",
        /// `PartyRole(452)` `17`.
        ContraFirm => "contrafirm",
        /// `PartyRole(452)` `21`.
        ClearingOrganization => "clearingorganization",
        /// `PartyRole(452)` `22`.
        Exchange => "exchange",
        /// `PartyRole(452)` `24`.
        CustomerAccount => "customeraccount",
        /// `PartyRole(452)` `36`.
        EnteringTrader => "enteringtrader",
        /// `PartyRole(452)` `37`.
        ContraTrader => "contratrader",
        /// `PartyRole(452)` `38`.
        PositionAccount => "positionaccount",
        /// `PartyRole(452)` `44`.
        OrderEntryOperatorId => "orderentryoperatorid",
        /// `PartyRole(452)` `73`.
        ExecutionVenue => "executionvenue",
        /// `PartyRole(452)` `76`.
        DeskId => "deskid",
        /// `PartyRole(452)` `122`.
        InvestmentDecisionMaker => "investmentdecisionmaker",
        /// `PartyRole(452)` `131`.
        Algorithm => "algorithm",
    }
}

/// The security types FIX's `SecurityIDSource(22)` code set names, each with
/// its one-character code, in the code set's order.
const FIX_SECURITY_SOURCES: [(IdType, char); 33] = [
    (IdType::Cusip, '1'),
    (IdType::Sedol, '2'),
    (IdType::Quik, '3'),
    (IdType::Isin, '4'),
    (IdType::Ric, '5'),
    (IdType::IsoCcy, '6'),
    (IdType::IsoCtry, '7'),
    (IdType::ExchSymb, '8'),
    (IdType::Cta, '9'),
    (IdType::Bloomberg, 'A'),
    (IdType::Wkn, 'B'),
    (IdType::Dutch, 'C'),
    (IdType::Valor, 'D'),
    (IdType::Sicovam, 'E'),
    (IdType::Belgian, 'F'),
    (IdType::Common, 'G'),
    (IdType::ClearingHouse, 'H'),
    (IdType::FpmlSpec, 'I'),
    (IdType::Opra, 'J'),
    (IdType::FpmlUrl, 'K'),
    (IdType::Loc, 'L'),
    (IdType::MktAssigned, 'M'),
    (IdType::RedEntity, 'N'),
    (IdType::RedPair, 'P'),
    (IdType::Cftc, 'Q'),
    (IdType::IsdaCommodity, 'R'),
    (IdType::Figi, 'S'),
    (IdType::Lei, 'T'),
    (IdType::Synthetic, 'U'),
    (IdType::Fim, 'V'),
    (IdType::Index, 'W'),
    (IdType::Umtf, 'X'),
    (IdType::Dti, 'Y'),
];

/// The words that name another instrument's identifier, never this
/// instrument's: a field name opening with one, and a key spelling one
/// before a security type ([`IdType::from_key_end`]).
const REFUSED_FIELD_PREFIXES: [&str; 5] = ["leg", "underlying", "contra", "related", "benchmark"];

/// The field names that are a ticker, never a security type.
const REFUSED_FIELD_NAMES: [&str; 3] = ["ticker", "symbol", "symbolticker"];

/// The words a type's spelling opens with to name a parent of the type
/// after them, in the order a base's [`IdType::parents`] lists them: the
/// value it held before it last changed, then its chain's first.
const PARENT_PREFIXES: [&str; 2] = ["parent", "orig"];

/// The words that, spelled before an identifier name at the end of a key,
/// stay part of the type it names - `firm.x.ParentOrderID` is a
/// `parentorderid`, `OriginalOrderID` an `originalorderid` - longest first.
const PARENTAGE_WORDS: [&str; 4] = ["original", "parent", "origin", "orig"];

impl IdType {
    /// The types holding this type's parents, nearest first: the value it
    /// held before it last changed, then - where there are two - the value
    /// its chain first stated. `clordid`'s is `origclordid` alone, FIX's
    /// `OrigClOrdID(41)`, the client order identifier a cancel/replace
    /// replaced; any other type the crate names, or a word ending in `id`,
    /// has `parent{type}` then `orig{type}` - `orderid` has
    /// `parentorderid` and `origorderid` - and every other type, a parent
    /// type included, has none, so parentage never nests. A dictionary
    /// states another list on a field with `FIX:parents`
    /// ([`FixRegistry::parents_of`](crate::FixRegistry::parents_of)).
    /// Borrowed for a type the crate names, so a lifecycle asking per
    /// identifier allocates nothing.
    ///
    /// ```
    /// use yggdryl::IdType;
    ///
    /// let parents: Vec<String> = IdType::OrderId.parents().iter().map(|kind| kind.to_string()).collect();
    /// assert_eq!(parents, ["parentorderid", "origorderid"]);
    /// assert_eq!(IdType::ClOrdId.parents().as_ref(), [IdType::OrigClOrdId]);
    /// assert!(IdType::OrigClOrdId.parents().is_empty());
    /// ```
    #[must_use]
    pub fn parents(&self) -> Cow<'static, [Self]> {
        static KNOWN_PARENTS: LazyLock<HashMap<&'static str, Box<[IdType]>>> =
            LazyLock::new(|| {
                IdType::SPELLINGS
                    .into_iter()
                    .zip(IdType::KNOWN)
                    .map(|(spelled, kind)| (spelled, kind.spelled_parents()))
                    .collect()
            });
        match KNOWN_PARENTS.get(self.as_str()) {
            Some(parents) => Cow::Borrowed(parents),
            None => Cow::Owned(self.spelled_parents().into_vec()),
        }
    }

    /// The base this type is a parent of, and its place among the base's
    /// [`Self::parents`]: `origclordid` is `clordid`'s first,
    /// `parentorderid` `orderid`'s first and `origorderid` its second. A
    /// word spelled `origin` or `original` before an identifier is no
    /// parent, and nothing allocates.
    ///
    /// ```
    /// use yggdryl::IdType;
    ///
    /// assert_eq!(IdType::OrigClOrdId.parent_of(), Some((IdType::ClOrdId, 0)));
    /// let parent: IdType = "ParentOrderID".parse().unwrap();
    /// assert_eq!(parent.parent_of(), Some((IdType::OrderId, 0)));
    /// let origin: IdType = "OrigTradeID".parse().unwrap();
    /// assert_eq!(origin.parent_of(), Some((IdType::TradeId, 1)));
    /// assert_eq!("ParentClOrdID".parse::<IdType>().unwrap(), IdType::OrigClOrdId, "one parent, two spellings");
    /// assert_eq!("originalorderid".parse::<IdType>().unwrap().parent_of(), None);
    /// ```
    #[must_use]
    pub fn parent_of(&self) -> Option<(Self, usize)> {
        PARENT_PREFIXES.iter().enumerate().find_map(|(at, prefix)| {
            let rest = self.as_str().strip_prefix(prefix)?;
            // `origin...` and `original...` are words of their own.
            if *prefix == "orig" && rest.starts_with("in") && !rest.parse::<Self>().ok()?.is_known()
            {
                return None;
            }
            let base = rest.parse::<Self>().ok()?;
            let at = match base {
                Self::ClOrdId => (*prefix == "orig").then_some(0)?,
                _ if !base.has_parents() => return None,
                _ => at,
            };
            Some((base, at))
        })
    }

    /// Whether this type has parents of its own: a type the crate names or
    /// a word ending in `id`, that is no parent itself and whose every
    /// parent spelling fits the width a word holds - all its parents or
    /// none, so a parent's place is its place in the list.
    fn has_parents(&self) -> bool {
        let longest = PARENT_PREFIXES
            .iter()
            .map(|prefix| prefix.len())
            .max()
            .unwrap_or(0);
        (self.is_known() || self.as_str().ends_with("id"))
            && self.as_str().len() + longest <= IDENTIFIER_KEY_WIDTH
            && self.parent_of().is_none()
    }

    /// [`Self::parents`] spelled out, every one a type [`Self::parent_of`]
    /// reads back to this one.
    fn spelled_parents(&self) -> Box<[Self]> {
        if matches!(self, Self::ClOrdId) {
            return Box::new([Self::OrigClOrdId]);
        }
        if !self.has_parents() {
            return Box::default();
        }
        let word = self.as_str();
        PARENT_PREFIXES
            .iter()
            .filter_map(|prefix| {
                let mut buffer = [0_u8; IDENTIFIER_KEY_WIDTH];
                let spelled = buffer.get_mut(..prefix.len() + word.len())?;
                spelled[..prefix.len()].copy_from_slice(prefix.as_bytes());
                spelled[prefix.len()..].copy_from_slice(word.as_bytes());
                let parent = std::str::from_utf8(spelled).ok()?.parse::<Self>().ok()?;
                parent
                    .parent_of()
                    .is_some_and(|(base, _)| base == *self)
                    .then_some(parent)
            })
            .collect()
    }

    /// Whether this type names a security: a type FIX's
    /// `SecurityIDSource(22)` code set names, a currency pair, a
    /// classification or an instrument key - what a market's `securityids`
    /// hold.
    ///
    /// ```
    /// use yggdryl::IdType;
    ///
    /// assert!(IdType::Isin.is_security());
    /// assert!(IdType::InstrumentId.is_security());
    /// assert!(!IdType::ClOrdId.is_security());
    /// ```
    #[must_use]
    pub fn is_security(&self) -> bool {
        matches!(self, Self::Forex | Self::Cfi | Self::InstrumentId)
            || self.fix_security_source().is_some()
    }

    /// Whether this type names a party: the account, a party of no role, a
    /// user, or a `PartyRole(452)` role the crate names - what an
    /// operation's `partyids` hold.
    ///
    /// ```
    /// use yggdryl::IdType;
    ///
    /// assert!(IdType::Account.is_party());
    /// assert!(IdType::ExecutingTrader.is_party());
    /// assert!(!IdType::OrderId.is_party());
    /// ```
    #[must_use]
    pub const fn is_party(&self) -> bool {
        matches!(
            self,
            Self::Account
                | Self::Party
                | Self::UserId
                | Self::ExecutingFirm
                | Self::ClientId
                | Self::ClearingFirm
                | Self::InvestorId
                | Self::EnteringFirm
                | Self::OrderOriginationTrader
                | Self::ExecutingTrader
                | Self::OrderOriginationFirm
                | Self::ExecutingSystem
                | Self::ContraFirm
                | Self::ClearingOrganization
                | Self::Exchange
                | Self::CustomerAccount
                | Self::EnteringTrader
                | Self::ContraTrader
                | Self::PositionAccount
                | Self::OrderEntryOperatorId
                | Self::ExecutionVenue
                | Self::DeskId
                | Self::InvestmentDecisionMaker
                | Self::Algorithm
        )
    }

    /// The spellings that name an identifier at the end of a key: each type
    /// the crate names whose spelling ends with `id`, the account, and the
    /// security codes that close on a check digit - an ISIN, a CUSIP, a
    /// SEDOL, a FIGI.
    pub(crate) fn identifier_names<'name>() -> impl Iterator<Item = &'name str> + Clone {
        let spellings: &'name [&'name str] = &Self::SPELLINGS;
        spellings.iter().copied().filter(|spelled| {
            spelled.ends_with("id")
                || matches!(*spelled, "account" | "isin" | "cusip" | "sedol" | "figi")
        })
    }

    /// The type the end of a folded key names, and where in the key it
    /// starts: the longest of `names` the key ends with, stepped back over a
    /// parentage word spelled before it - `parent`, `orig`, `origin`,
    /// `original` - which stays part of the type, so `firm.x.parentorderid`
    /// ends with `parentorderid`. A security type is refused where another
    /// instrument's word - `leg`, `underlying`, `contra`, `related`,
    /// `benchmark` - opens the key or ends what is spelled before the type,
    /// after any namespace (`omsunderlyingisin`, `fix.legisin`,
    /// `firm.x.contracusip`), since it names that instrument's code. `None`
    /// where the key ends with none of `names`.
    pub(crate) fn from_key_end<'name>(
        folded: &str,
        names: impl IntoIterator<Item = &'name str>,
    ) -> Option<(usize, Self)> {
        let longest = names
            .into_iter()
            .filter(|name| !name.is_empty() && folded.ends_with(name))
            .map(str::len)
            .max()?;
        let mut at = folded.len() - longest;
        if let Some(word) = PARENTAGE_WORDS
            .iter()
            .find(|word| folded[..at].ends_with(*word))
        {
            at -= word.len();
        }
        let kind = folded[at..].parse::<Self>().ok()?;
        let security =
            kind.is_security() || kind.parent_of().is_some_and(|(base, _)| base.is_security());
        let before = folded[..at].trim_end_matches('.');
        let other_instrument = REFUSED_FIELD_PREFIXES
            .iter()
            .any(|word| folded.starts_with(word) || before.ends_with(word));
        (!(security && other_instrument)).then_some((at, kind))
    }

    /// The security type one FIX `SecurityIDSource(22)` code names.
    #[must_use]
    pub fn from_fix_security_source(code: char) -> Option<Self> {
        FIX_SECURITY_SOURCES
            .iter()
            .find(|(_, held)| *held == code)
            .map(|(kind, _)| kind.clone())
    }

    /// The FIX `SecurityIDSource(22)` code of a security type FIX names.
    #[must_use]
    pub fn fix_security_source(&self) -> Option<char> {
        FIX_SECURITY_SOURCES
            .iter()
            .find(|(kind, _)| kind == self)
            .map(|(_, code)| *code)
    }

    /// The security type a `SecurityIDSource(22)` value states: its
    /// one-character code as FIX writes it (`4`, `K`), else a name the code
    /// set gives a member, in any case and spacing, its punctuation and its
    /// parenthesized remarks passed over - `ISIN number`, `Clearing House /
    /// Clearing Organization`, `ISDA/FpML Product URL (URL in SecurityID)` -
    /// else the value kept as it was stated, under the one fold every type
    /// word takes: a code FIX names no member for, a private code (`100` and
    /// above) or a venue's own word is the word it folds to - lower case,
    /// its breaks dropped - and never another member, so `100` stays `100`
    /// and `Z` is `z`. A code is case-sensitive: `a` is a word of its own,
    /// never the Bloomberg symbol `A` names. A member naming another kind of
    /// identifier - `ClOrdID`, the party role `Exchange` - is no source.
    ///
    /// ```
    /// use yggdryl::IdType;
    ///
    /// assert_eq!(IdType::from_security_source("4").unwrap(), IdType::Isin);
    /// assert_eq!(IdType::from_security_source("ISIN").unwrap(), IdType::Isin);
    /// assert_eq!(IdType::from_security_source("ISIN number").unwrap(), IdType::Isin);
    /// assert_eq!(IdType::from_security_source("B").unwrap(), IdType::Wkn);
    /// assert_eq!(
    ///     IdType::from_security_source("ISDA/FpML Product URL (URL in SecurityID)").unwrap(),
    ///     IdType::FpmlUrl
    /// );
    /// assert_eq!(IdType::from_security_source("101").unwrap().as_str(), "101");
    /// assert!(IdType::from_security_source("ticker").is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// A value no word folds from - a byte other than an ASCII letter, a
    /// digit, `.` and the breaks a fold drops, or more than
    /// [`IDENTIFIER_KEY_WIDTH`] of them - `ticker` in any spelling, which
    /// is the name a person knows an instrument by rather than a security
    /// type, and a member that names an operation's or a party's
    /// identifier.
    pub fn from_security_source(text: &str) -> Result<Self> {
        let trimmed = text.trim();
        let mut chars = trimmed.chars();
        let code = match (chars.next(), chars.next()) {
            (Some(code), None) => Self::from_fix_security_source(code),
            _ => None,
        };
        if let Some(kind) = code {
            return Ok(kind);
        }
        let kind = match Self::from_security_source_name(trimmed) {
            Some(kind) => kind,
            None => trimmed.parse::<Self>()?,
        };
        kind.check_security()?;
        if kind.is_known() && !kind.is_security() {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("{kind}"),
                reason: crate::text::expected_got(
                    "a security identifier type",
                    format_args!("{kind}, which names another kind of identifier"),
                ),
            });
        }
        Ok(kind)
    }

    /// The member a code set's name for a source names: its ASCII letters,
    /// digits and dots lower-cased - a dot no member spells, so a dotted word
    /// is a word - its other punctuation, its spacing and what it writes
    /// between parentheses passed over - a remark such as `(XML in
    /// SecurityXML(1185))` explains the source and never names one. `None`
    /// where that names no member or the name holds a byte no member
    /// spells.
    fn from_security_source_name(name: &str) -> Option<Self> {
        let mut buffer = [0_u8; IDENTIFIER_KEY_WIDTH];
        let mut len = 0;
        let mut depth = 0_usize;
        for byte in name.bytes() {
            match byte {
                b'(' => depth += 1,
                b')' => depth = depth.saturating_sub(1),
                _ if depth > 0 => {}
                _ if byte.is_ascii_alphanumeric() || byte == b'.' => {
                    *buffer.get_mut(len)? = byte.to_ascii_lowercase();
                    len += 1;
                }
                _ if byte.is_ascii() => {}
                _ => return None,
            }
        }
        Self::from_folded(std::str::from_utf8(&buffer[..len]).ok()?)
    }

    /// Refuses the one word no security type is: `ticker`, the name a person
    /// knows an instrument by, which lives on `set_ticker`.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidRecord`] for `ticker`.
    pub fn check_security(&self) -> Result<()> {
        if self.as_str() == "ticker" {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("{self}"),
                reason: "a ticker is the name a person knows an instrument by, not a security \
                         identifier type: state it through set_ticker"
                    .into(),
            });
        }
        Ok(())
    }

    /// The security type a field is named for, or `None` where the name is
    /// not one instrument's own identifier.
    ///
    /// A leading `#` is dropped and the name folds as every spelling here
    /// folds. The whole name as a security type FIX names - or a currency
    /// pair - names the type; otherwise `[security] + alias + [code | id |
    /// number]`, where the alias is one, does. `ticker`, `symbol` and
    /// `symbolticker` name no type, and neither does a field of another
    /// instrument - `leg*`, `underlying*`, `contra*`, `related*`,
    /// `benchmark*`.
    ///
    /// ```
    /// use yggdryl::IdType;
    ///
    /// assert_eq!(IdType::from_field_name("#ISINCODE"), Some(IdType::Isin));
    /// assert_eq!(IdType::from_field_name("cusip_code"), Some(IdType::Cusip));
    /// assert_eq!(IdType::from_field_name("legisin"), None);
    /// assert_eq!(IdType::from_field_name("ticker"), None);
    /// ```
    #[must_use]
    pub fn from_field_name(name: &str) -> Option<Self> {
        let name = name.strip_prefix('#').unwrap_or(name);
        let mut buffer = [0_u8; crate::identifier::IDENTIFIER_KEY_WIDTH];
        let folded = crate::identifier::fold_into(name, &mut buffer).ok()?;
        let field_source = |text: &str| Self::from_folded(text).filter(Self::is_field_source);
        if let Some(kind) = field_source(folded) {
            return Some(kind);
        }
        if REFUSED_FIELD_NAMES.contains(&folded)
            || REFUSED_FIELD_PREFIXES
                .iter()
                .any(|prefix| folded.starts_with(prefix))
        {
            return None;
        }
        let alias = folded.strip_prefix("security").unwrap_or(folded);
        if alias.is_empty() {
            return None;
        }
        if let Some(kind) = (alias != folded).then(|| field_source(alias)).flatten() {
            return Some(kind);
        }
        ["code", "id", "number"]
            .iter()
            .find_map(|suffix| alias.strip_suffix(suffix))
            .filter(|stem| !stem.is_empty())
            .and_then(field_source)
    }

    /// Whether a field's name may name this type: a security type FIX names,
    /// or a currency pair.
    fn is_field_source(&self) -> bool {
        matches!(self, Self::Forex) || self.fix_security_source().is_some()
    }

    /// The most bytes a value of this type may be: the fixed width of a
    /// checked code, its own code's 32-byte bound for a Bloomberg symbol and
    /// a RIC, and [`IDENTIFIER_VALUE_WIDTH`] for every
    /// other type - an FpML product URL, an index name and a private
    /// source's code among them.
    #[must_use]
    pub fn max_value_width(&self) -> usize {
        match self {
            Self::Isin | Self::Figi => 12,
            Self::Cusip | Self::Valor => 9,
            Self::Sedol | Self::Forex => 7,
            Self::Wkn | Self::Cfi => 6,
            Self::IsoCcy => 3,
            Self::IsoCtry => 2,
            Self::Bloomberg => BBG_WIDTH,
            Self::Ric => RIC_WIDTH,
            _ => IDENTIFIER_VALUE_WIDTH,
        }
    }

    /// Whether values of this type fold to upper case.
    fn folds_case(&self) -> bool {
        matches!(
            self,
            Self::Isin
                | Self::Cusip
                | Self::Sedol
                | Self::Figi
                | Self::Wkn
                | Self::Cfi
                | Self::IsoCcy
                | Self::IsoCtry
        )
    }

    /// `value`, already trimmed and stating something, as this type stores
    /// it, written into `buffer` where it moves: an ISIN, a CUSIP, a SEDOL
    /// and a FIGI close on their check digit, a CFI parses and an ISO
    /// currency or country code is one ISO 4217 or ISO 3166 names, each
    /// upper-cased first; a RIC is one token of printable ASCII, its case
    /// kept; a WKN is six of `[0-9A-HJ-NP-Z]`, a Valor number one to nine
    /// digits without a leading zero; a pair is stored as its one canonical
    /// spelling - `eurusd` is `EUR/USD`; every other security type FIX names
    /// takes printable ASCII within [`Self::max_value_width`], and every
    /// other type any text within it. Nothing allocates.
    pub(crate) fn value_into<'value>(
        &self,
        value: &'value str,
        buffer: &'value mut [u8; IDENTIFIER_VALUE_WIDTH],
    ) -> Result<&'value str> {
        let refusal = |actual: &dyn std::fmt::Display| Error::InvalidRecord {
            path: format_smolstr!("{self}"),
            reason: crate::text::expected_got(format_args!("a {self} value"), actual),
        };
        if matches!(self, Self::Forex) {
            let pair = Forex::new(value)?;
            return copied(pair.as_str(), buffer).ok_or_else(|| refusal(&value));
        }
        let width = self.max_value_width();
        if value.len() > width {
            return Err(refusal(&format_args!(
                "{} bytes, over the {width} the type allows",
                value.len()
            )));
        }
        let value = if self.folds_case() && value.bytes().any(|byte| byte.is_ascii_lowercase()) {
            let slot = &mut buffer[..value.len()];
            slot.copy_from_slice(value.as_bytes());
            slot.make_ascii_uppercase();
            std::str::from_utf8(slot).expect("upper-casing keeps UTF-8")
        } else {
            value
        };
        match self {
            Self::Isin => drop(Isin::new(value)?),
            Self::Cusip => drop(Cusip::new(value)?),
            Self::Sedol => drop(Sedol::new(value)?),
            Self::Figi => drop(Figi::new(value)?),
            Self::Ric => drop(Ric::new(value)?),
            Self::Cfi => drop(Cfi::new(value)?),
            Self::IsoCcy => drop(Ccy::new(value)?),
            Self::IsoCtry => drop(Country::new(value)?),
            Self::Wkn => {
                if value.len() != 6
                    || !value.bytes().all(|byte| {
                        matches!(byte, b'0'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z')
                    })
                {
                    return Err(refusal(&format_args!(
                        "{value:?}, not six of [0-9A-HJ-NP-Z]"
                    )));
                }
            }
            Self::Valor => {
                if !value.bytes().all(|byte| byte.is_ascii_digit()) || value.starts_with('0') {
                    return Err(refusal(&format_args!(
                        "{value:?}, not one to nine digits without a leading zero"
                    )));
                }
            }
            _ if self.fix_security_source().is_some() => {
                if let Some((position, byte)) = value
                    .bytes()
                    .enumerate()
                    .find(|(_, byte)| !byte.is_ascii() || byte.is_ascii_control())
                {
                    return Err(refusal(&format_args!(
                        "the byte 0x{byte:02X} at {position}, not printable ASCII"
                    )));
                }
            }
            _ => {}
        }
        Ok(value)
    }
}

/// `text` copied into `buffer`, where it fits.
fn copied<'buffer>(
    text: &str,
    buffer: &'buffer mut [u8; IDENTIFIER_VALUE_WIDTH],
) -> Option<&'buffer str> {
    let slot = buffer.get_mut(..text.len())?;
    slot.copy_from_slice(text.as_bytes());
    std::str::from_utf8(slot).ok()
}
