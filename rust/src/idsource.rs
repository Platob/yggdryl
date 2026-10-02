//! Who gave an identifier: the issuer or namespace of its value.
//!
//! [`IdSource`] holds the sources the crate names as members - `base` where
//! nothing names one, `derived` where the crate derived the value, `fix`
//! for what a FIX field or group states, and the sources FIX's
//! `PartyIDSource(447)` and `AcctIDSource(660)` code sets commonly name -
//! each a static word, and any other word, a venue's or a bridge's own
//! prefix included, as [`IdSource::Other`].

use crate::identifier::id_vocabulary;

id_vocabulary! {
    /// Who gave an identifier: `fix`, `proprietary`, `base`, `firm.x`.
    ///
    /// A spelling folds as an [`IdType`](crate::IdType) does, to lower case
    /// without the `_`, `-`, space and `#` a spelling breaks it with. Any
    /// other folded word is an [`IdSource::Other`].
    ///
    /// ```
    /// use yggdryl::IdSource;
    ///
    /// assert_eq!("BASE".parse::<IdSource>().unwrap(), IdSource::Base);
    /// assert_eq!("Proprietary".parse::<IdSource>().unwrap(), IdSource::Proprietary);
    /// assert_eq!(IdSource::Derived.as_str(), "derived");
    /// assert!(!"firm.x".parse::<IdSource>().unwrap().is_known());
    /// ```
    IdSource, "an identifier source" {
        /// Nothing names the source: the base an identifier stands on where
        /// no issuer is stated.
        Base => "base",
        /// The crate derived the value rather than read it - an ISIN's
        /// national number, what a ticker's shape names, a currency pair
        /// detected off a symbol, what a lifecycle learned. A lookup answers
        /// a stated source before it.
        Derived => "derived",
        /// A FIX field or group states it.
        Fix => "fix",
        /// A Bank Identifier Code, `PartyIDSource(447)` `B` and
        /// `AcctIDSource(660)` `1`.
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
        /// A legal entity identifier, `PartyIDSource(447)` `N`.
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
    /// where it folds to a source the crate reserves - `base`, `derived`,
    /// `fix` - which names no venue, so the reader's own source stands.
    ///
    /// # Errors
    ///
    /// A namespace no word holds.
    pub(crate) fn from_namespace(folded: &str) -> crate::Result<Option<Self>> {
        let namespace = folded.trim_matches('.');
        if namespace.is_empty() {
            return Ok(None);
        }
        Ok(match namespace.parse()? {
            Self::Base | Self::Derived | Self::Fix => None,
            named => Some(named),
        })
    }
}
