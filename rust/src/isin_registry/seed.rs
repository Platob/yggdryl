//! The seed: the common instruments every process default starts from.
//!
//! `config/isin/instruments.json` is one JSON array of listing rows sorted
//! by ISIN then market - an instrument listed on several markets one row
//! per market, as the registry holds it - each keyed as the registry's
//! columns are - `isin`, `ticker`, `miccode` (absent for an index, which
//! trades on no market), `currency`, `countrycode`, `cficode` and, where
//! one is known, `fisn` - and nothing else. That file is the one maintained
//! by hand; `seed.json` beside this module is its copy inside the crate's
//! package, written byte for byte by
//! `python scripts/check_isin_seed.py --sync`, so the published crate and a
//! source distribution embed it too. It is embedded at build time and read
//! once per process: the
//! document is parsed by the crate's JSON codec under the seed's own row,
//! each value proven by its column's datatype, and the rows folded through
//! [`IsinRegistry::extend_from_arrow_reader`]'s column rule, so a seed row is
//! an ordinary statement and the facts it implies are derived as for any
//! other. `scripts/check_isin_seed.py` checks the file whenever it is edited
//! and fails while the copy differs from it; `rust/tests/isin_registry/seed.rs`
//! pins what the embedded copy holds and that it is the file's bytes.

use std::sync::{LazyLock, OnceLock};

use super::{IsinRegistry, IsinTable};
use crate::{DataType, Field, StructType};

/// The seed document, embedded: the crate's copy of
/// `config/isin/instruments.json`.
const DOCUMENT: &str = include_str!("seed.json");

/// The row a seed entry is: the registry's own column names, each typed by
/// its column's datatype so a value it refuses is refused where the
/// document is read rather than cast to null, the ISIN required.
static ROW: LazyLock<Field> = LazyLock::new(|| {
    Field::new(
        "instrument",
        DataType::Struct(StructType::from_unique_fields(vec![
            Field::new("isin", DataType::isin(), false),
            Field::new("ticker", DataType::utf8(), true),
            Field::new("miccode", DataType::Mic, true),
            Field::new("currency", DataType::ccy(), true),
            Field::new("countrycode", DataType::country(), true),
            Field::new("cficode", DataType::cfi(), true),
            Field::new("fisn", DataType::fisn(), true),
        ])),
        false,
    )
});

/// The seed's table, folded on first use.
static TABLE: OnceLock<IsinTable> = OnceLock::new();

/// The seed's table: every row of the embedded document folded into an
/// empty registry by the update rule.
///
/// # Panics
///
/// Where the embedded document does not read - a build defect, which
/// `rust/tests/isin_registry/seed.rs` fails on before it ships.
pub(super) fn table() -> &'static IsinTable {
    TABLE.get_or_init(|| {
        let mut registry = IsinRegistry::new();
        registry
            .extend_from_arrow_reader(rows().expect("the embedded seed parses"))
            .expect("the embedded seed folds");
        registry.table
    })
}

/// The embedded document's rows, laid out under [`ROW`].
fn rows() -> crate::arrow::Result<crate::arrow::BatchReader> {
    let document = crate::from_json_scalar(DOCUMENT)?;
    let rows = document
        .sequence_rows()
        .ok_or_else(|| crate::Error::InvalidRecord {
            path: smol_str::SmolStr::new_static("$"),
            reason: crate::text::expected_got("a JSON array of instruments", document.kind()),
        })?
        .into_owned();
    crate::arrow::rows::reader(&ROW, rows, None, None, None)
}
