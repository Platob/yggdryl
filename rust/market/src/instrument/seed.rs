//! The seed: the common instruments every process default starts from.
//!
//! `config/instruments/instruments.json` is one JSON array of instruments
//! sorted by ISIN, one object per instrument - `isin`, `cficode`,
//! `countrycode`, `fisn` where one is known, `origccy` where the research
//! names a share class's currency some listing of it trades apart from (the
//! five Irish USD share classes, each named so in its fund's name and its
//! FIRDS short name and listed in GBP or EUR), and `listings`, one object
//! per market - `miccode` (absent for an index, which trades on no market),
//! `ticker`, `currency` - and any other key as a complementary fact of the
//! instrument ([`Instrument::metadata`]), its value text. No origin currency is derived:
//! Tencent's `KY` prefix names its domicile and no currency, so its row
//! states none. That file is the one maintained by hand; `seed.json` beside
//! this module is its copy inside the crate's package, written byte for byte
//! by `python scripts/check_instruments_seed.py --sync`, so the published
//! crate and a source distribution embed it too. It is embedded at build
//! time and read once per process: the document is parsed by the crate's
//! JSON codec, each value proven by its type - the ISIN real, the MIC
//! assigned - and every instrument folded through [`Instruments::merge`], so
//! a seed instrument is an ordinary statement and the facts it implies are
//! derived as for any other. `scripts/check_instruments_seed.py` checks the
//! file whenever it is edited and fails while the copy differs from it;
//! `rust/market/tests/instrument/seed.rs` pins what the embedded copy holds
//! and that it is the file's bytes.

use std::sync::OnceLock;

use smol_str::{SmolStr, format_smolstr};

use super::{Instrument, InstrumentTable, Instruments, StoredRows};
use crate::Listing;
use yggdryl::{Ccy, Cfi, Country, Error, Fisn, Isin, Mic, Result, Scalar};

/// The seed document, embedded: the crate's copy of
/// `config/instruments/instruments.json`.
const DOCUMENT: &str = include_str!("seed.json");

/// The seed's table, folded on first use.
static TABLE: OnceLock<InstrumentTable> = OnceLock::new();

/// What a store holds of the seed's table: the baseline a seeded collection
/// is clean against, recorded once.
static STORED: OnceLock<StoredRows> = OnceLock::new();

/// The seed's table: every instrument of the embedded document folded into
/// an empty collection by the update rule.
///
/// # Panics
///
/// Where the embedded document does not read - a build defect, which
/// `rust/market/tests/instrument/seed.rs` fails on before it ships.
pub(super) fn table() -> &'static InstrumentTable {
    TABLE.get_or_init(|| {
        let mut instruments = Instruments::new();
        for instrument in read().expect("the embedded seed parses") {
            instruments
                .merge(instrument)
                .expect("the embedded seed folds");
        }
        instruments.table
    })
}

/// The seed's table as a store holds it ([`Instruments::is_dirty`]).
pub(super) fn stored() -> &'static StoredRows {
    STORED.get_or_init(|| super::stored_rows(table()))
}

/// The text `row` states under `key`, where it states one.
fn cell<'row>(row: &'row Scalar, key: &str) -> Result<Option<&'row str>> {
    let Some(value) = row.as_struct().and_then(|fields| fields.get(key)) else {
        return Ok(None);
    };
    value
        .as_str()
        .map(Some)
        .ok_or_else(|| Error::InvalidRecord {
            path: format_smolstr!("$.{key}"),
            reason: yggdryl::implementer::expected_got("text", value.kind()),
        })
}

/// A refusal of the seed object at `at`, located on its key.
fn located(at: usize, error: Error) -> Error {
    match error {
        Error::InvalidRecord { path, reason } => Error::InvalidRecord {
            path: format_smolstr!("$[{at}]{}", path.strip_prefix('$').unwrap_or(&path)),
            reason,
        },
        other => Error::InvalidRecord {
            path: format_smolstr!("$[{at}]"),
            reason: format_smolstr!("{other}"),
        },
    }
}

/// The embedded document's instruments, each read by its types.
fn read() -> Result<Vec<Instrument>> {
    let document = yggdryl::from_json_scalar(DOCUMENT)?;
    let rows = document
        .sequence_rows()
        .ok_or_else(|| Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: yggdryl::implementer::expected_got(
                "a JSON array of instruments",
                document.kind(),
            ),
        })?;
    rows.iter()
        .enumerate()
        .map(|(at, row)| read_one(row).map_err(|error| located(at, error)))
        .collect()
}

/// One seed object as the instrument it states.
fn read_one(row: &Scalar) -> Result<Instrument> {
    let isin = cell(row, "isin")?.ok_or_else(|| Error::InvalidRecord {
        path: SmolStr::new_static("$.isin"),
        reason: SmolStr::new_static("expected an ISIN, got none"),
    })?;
    let mut instrument = Instrument::for_security(Isin::new(isin)?)?
        .try_with_cficode(cell(row, "cficode")?.map(Cfi::new).transpose()?)?
        .with_countrycode(cell(row, "countrycode")?.map(Country::new).transpose()?)
        .with_fisn(cell(row, "fisn")?.map(Fisn::new).transpose()?)
        .with_origccy(cell(row, "origccy")?.map(Ccy::new).transpose()?);
    let listings = row
        .as_struct()
        .and_then(|fields| fields.get("listings"))
        .and_then(Scalar::sequence_rows)
        .unwrap_or_default();
    for listing in listings.iter() {
        let listing = Listing::new(cell(listing, "miccode")?.map(Mic::new).transpose()?)
            .with_ticker(cell(listing, "ticker")?.map(SmolStr::new))
            .with_currency(cell(listing, "currency")?.map(Ccy::new).transpose()?);
        instrument = instrument.with_listing(listing)?;
    }
    // A key no field reads is a complementary fact ([`Instrument::metadata`]).
    for (key, value) in row.as_struct().into_iter().flatten() {
        if FIELD_KEYS.contains(&key.as_str()) {
            continue;
        }
        let Some(text) = value.as_str() else {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("$.{key}"),
                reason: yggdryl::implementer::expected_got("text", value.kind()),
            });
        };
        instrument.set_metadata(key, text)?;
    }
    Ok(instrument)
}

/// The keys of a seed object a typed field reads; every other key is a
/// metadata entry.
const FIELD_KEYS: [&str; 6] = [
    "isin",
    "cficode",
    "countrycode",
    "fisn",
    "origccy",
    "listings",
];

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/market/tests/instrument/seed.rs` pins and a caller cannot
    //! reach.

    use yggdryl::{Result, Scalar};

    use crate::Instrument;

    /// One seed object as the instrument it states.
    pub fn read_one(row: &Scalar) -> Result<Instrument> {
        super::read_one(row)
    }
}
