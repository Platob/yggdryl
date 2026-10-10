//! Who gave an identifier: the issuer or namespace of its value.
//!
//! [`IdSource`] holds the sources the crate names as members - `base` where
//! nothing names one, FIX's own fields included, `derived` where the crate
//! derived the value, and the sources FIX's `PartyIDSource(447)` and
//! `AcctIDSource(660)` code sets commonly name - each a static word, and any
//! other word, a venue's or a bridge's own prefix included, as
//! [`IdSource::Other`].
//!
//! Two sources are standards whose values are a registered code: a value
//! under `bic` is a BIC and one under `legalentityidentifier` an LEI,
//! whatever type of name it is, so such a source answers for its values'
//! shape and rank beside their type.

use smol_str::{SmolStr, format_smolstr};

use crate::IdKey;
use crate::identifier::id_vocabulary;
use yggdryl::{Bic, CodeValue, Error, Lei, Result};

id_vocabulary! {
    /// Who gave an identifier: `base`, `proprietary`, `firm.x`.
    ///
    /// A spelling folds as an [`IdType`](crate::IdType) does, to lower case
    /// without the `_`, `-`, space and `#` a spelling breaks it with. Any
    /// other folded word is an [`IdSource::Other`].
    ///
    /// ```
    /// # yggdryl_market::install().unwrap();
    /// use yggdryl_market::IdSource;
    ///
    /// assert_eq!("BASE".parse::<IdSource>().unwrap(), IdSource::Base);
    /// assert_eq!("FIX".parse::<IdSource>().unwrap(), IdSource::Base, "the standard is the base");
    /// assert_eq!("Proprietary".parse::<IdSource>().unwrap(), IdSource::Proprietary);
    /// assert_eq!(IdSource::Derived.as_str(), "derived");
    /// assert!(!"firm.x".parse::<IdSource>().unwrap().is_known());
    /// ```
    IdSource, "an identifier source" {
        /// Nothing names the source: the base an identifier stands on where
        /// no issuer is stated - what a FIX field or group states, so `fix`
        /// is read as this source and never written. A key from it is
        /// spelled as its type alone ([`IdKey`](crate::IdKey)).
        Base => "base" | "fix",
        /// The crate derived the value rather than read it - an ISIN's
        /// national number, what a ticker's shape names, a currency pair
        /// detected off a symbol, what a lifecycle learned. A lookup answers
        /// a stated source before it.
        Derived => "derived",
        /// A Bank Identifier Code, `PartyIDSource(447)` `B` and
        /// `AcctIDSource(660)` `1`: every value it gives is an ISO 9362 BIC
        /// ([`Bic`](yggdryl::Bic)), held by its shape.
        Bic => "bic",
        /// A generally accepted market participant identifier,
        /// `PartyIDSource(447)` `C`.
        GeneralIdentifier => "generalidentifier" | "generallyacceptedmarketparticipantidentifier",
        /// A proprietary or custom code, `PartyIDSource(447)` `D`.
        Proprietary => "proprietary" | "proprietarycustomcode",
        /// An ISO 3166 country code, `PartyIDSource(447)` `E`.
        IsoCountryCode => "isocountrycode",
        /// A settlement entity location, `PartyIDSource(447)` `F`.
        SettlementEntityLocation => "settlemententitylocation",
        /// A market identifier code, `PartyIDSource(447)` `G`.
        Mic => "mic",
        /// A central securities depository participant, `PartyIDSource(447)`
        /// `H`.
        CsdParticipant => "csdparticipant",
        /// A tax identifier, `PartyIDSource(447)` `J`.
        TaxId => "taxid",
        /// A legal entity identifier, `PartyIDSource(447)` `N`: every value
        /// it gives is an ISO 17442 LEI ([`Lei`](yggdryl::Lei)), held by its
        /// shape.
        LegalEntityIdentifier => "legalentityidentifier",
        /// A short code, `PartyIDSource(447)` `P`.
        ShortCodeIdentifier => "shortcodeidentifier",
        /// A natural person's national identifier, `PartyIDSource(447)` `Q`.
        NationalIdNaturalPerson => "nationalidnaturalperson",
        /// A SID code, `AcctIDSource(660)` `2`.
        SidCode => "sidcode",
        /// A TFM (GSPTA) code, `AcctIDSource(660)` `3`.
        Tfm => "tfm",
        /// An OMGEO code, `AcctIDSource(660)` `4`.
        Omgeo => "omgeo",
        /// A DTCC code, `AcctIDSource(660)` `5`.
        DtccCode => "dtcccode",
        /// A special segregated account identifier, `PartyIDSource(447)` `T`
        /// and `AcctIDSource(660)` `6`.
        Spsaid => "spsaid",
    }
}

impl IdSource {
    /// The source a namespace spelled before a name - a key's, or a
    /// `{NAMESPACE}INSTRUMENTID` security source's - names, already folded:
    /// its dots at either end dropped, and `None` where nothing is left or
    /// where it folds to a source the crate reserves - `base`, which `fix`
    /// spells too, or `derived` - which names no venue, so the reader's own
    /// source stands.
    ///
    /// # Errors
    ///
    /// A namespace no word holds.
    pub(crate) fn from_namespace(folded: &str) -> yggdryl::Result<Option<Self>> {
        let namespace = folded.trim_matches('.');
        if namespace.is_empty() {
            return Ok(None);
        }
        Ok(match namespace.parse()? {
            Self::Base | Self::Derived => None,
            named => Some(named),
        })
    }

    /// The registered code every value this source gives is: a BIC under
    /// [`Self::Bic`] and an LEI under [`Self::LegalEntityIdentifier`],
    /// whatever type of name the value is - a party's role, the account, a
    /// word no member names - and `None` under every other source, whose
    /// values follow their type's rule alone. The one owner of the rule
    /// [`Identifier::new`](crate::Identifier::new) holds a value to beside
    /// its type's, and of the rank it reads beside the type's.
    pub(crate) const fn code(&self) -> Option<SourceCode> {
        match self {
            Self::Bic => Some(SourceCode::Bic),
            Self::LegalEntityIdentifier => Some(SourceCode::Lei),
            _ => None,
        }
    }
}

/// A registered code every value of one source is ([`IdSource::code`]).
#[derive(Clone, Copy, Debug)]
pub(crate) enum SourceCode {
    /// An ISO 9362 business identifier code.
    Bic,
    /// An ISO 17442 legal entity identifier.
    Lei,
}

impl SourceCode {
    /// `value`, stated under `key`, held by the code's shape and
    /// upper-cased. Allocation-free where it is held: both codes are inline.
    ///
    /// # Errors
    ///
    /// A value that is not of the code's shape, located on `key` and naming
    /// the source and the code it expected.
    pub(crate) fn hold(self, key: &IdKey, value: &str) -> Result<SmolStr> {
        let (held, code) = match self {
            Self::Bic => (Bic::new(value).map(|code| code.storage().clone()), "a BIC"),
            Self::Lei => (Lei::new(value).map(|code| code.storage().clone()), "an LEI"),
        };
        held.map_err(|error| {
            let reason = match error {
                Error::InvalidDataType { reason, .. } | Error::InvalidRecord { reason, .. } => {
                    reason
                }
                other => format_smolstr!("{other}"),
            };
            Error::InvalidRecord {
                path: format_smolstr!("{key}"),
                reason: format_smolstr!(
                    "a value under the {} source is {code}: {reason}",
                    key.src()
                ),
            }
        })
    }

    /// How real `value` is as the code: the code's own
    /// [`CodeValue::rank`] - a BIC whose country ISO 3166 lists one, an LEI
    /// whose check digits close one - and zero for a value of neither
    /// shape. Allocation-free.
    pub(crate) fn rank(self, value: &str) -> u8 {
        match self {
            Self::Bic => Bic::new(value).map_or(0, |code| code.rank()),
            Self::Lei => Lei::new(value).map_or(0, |code| code.rank()),
        }
    }
}
