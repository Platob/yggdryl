//! The store a registry is bound to: the holder its table was loaded from
//! and is committed back to as one snapshot, under the holder's own record
//! options resolved once at the binding.

#[cfg(feature = "iceberg")]
use smol_str::format_smolstr;

use crate::holder::Holder;
use crate::logging::warning::warned;
use crate::media::{IORecordOptions, RecordOptions};
use crate::{Error, IOBase, IOMedia, IOMode, IOResult, MimeType, Result, Url};

use super::{IsinEntry, IsinRegistry, IsinTable};

/// Where a registry's rows are kept: the holder, and the holder's own record
/// options, resolved once when the registry was bound - what the rows are
/// read under; a write declares the registry's row over them.
#[derive(Debug)]
pub(crate) struct Store {
    holder: Holder,
    options: RecordOptions,
    /// A catalog table owns its metadata pointer and publication path.
    native_table: bool,
    /// Whether the holder is a container, read once at the binding.
    container: bool,
    /// Whether a commit replaces the rows under the row the store already
    /// holds - a leaf's, an Iceberg table's schema - rather than laying the
    /// store out afresh as a plain folder's parts are; read once at the
    /// binding.
    keeps_row: bool,
    /// The registry columns, by their place in [`IsinEntry::field`], the
    /// row a store that keeps its row was laid out with has none of - a
    /// store written before the column was the registry's - read off the
    /// load at the binding; emptied where a commit lays the store out
    /// afresh.
    lacking: Vec<usize>,
}

impl Store {
    /// Binds `holder` under its own record options - its encoding, or the
    /// encoding of the record leaves under it, an Iceberg table's its data
    /// files' - a folder listing no record leaf, or plain text alone (a
    /// README beside the parts), laid out as Arrow IPC by the first commit.
    /// A leaf names its encoding by its name, so one this build has no
    /// record encoding for is refused by it; a location inside an Iceberg
    /// table - one partition of it - is refused by name, since a registry
    /// is replaced whole.
    fn bind(holder: Holder) -> Result<Self> {
        let native_table = matches!(&holder, Holder::Table(_));
        let container = holder.is_container();
        #[cfg_attr(not(feature = "iceberg"), allow(unused_mut))]
        let mut keeps_row = native_table || !container;
        #[cfg(feature = "iceberg")]
        if !native_table
            && container
            && let Some(located) = crate::iceberg::located(&holder)?
        {
            if !located.is_whole() {
                return Err(Error::InvalidRecord {
                    path: smol_str::SmolStr::new_static("$.holder"),
                    reason: format_smolstr!(
                        "expected an Iceberg table whole for the instrument registry, got a partition of one at {}",
                        location(&holder)
                    ),
                });
            }
            keeps_row = true;
        }
        let options = match holder.record_options() {
            Ok(RecordOptions::Text(_)) | Err(_) if container => {
                RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?
            }
            resolved => resolved?,
        };
        Ok(Self {
            holder,
            options,
            native_table,
            container,
            keeps_row,
            lacking: Vec::new(),
        })
    }

    /// The options a commit writes under: the holder's own, declaring the
    /// registry's row - its country partition and its order where the store
    /// keeps its row, a leaf or an Iceberg table; a plain folder is laid out
    /// by the layout it spells, partitioning by columns alone, so it takes
    /// the row declaring nothing, which no `column=value` layout contradicts.
    fn write_options(&self) -> RecordOptions {
        let field = if self.container && !self.keeps_row {
            super::ROW.clone()
        } else {
            IsinEntry::field()
        };
        self.options.clone().with_field(field)
    }

    /// Reads off the row `stored` - what the load read - the registry
    /// columns it has none of, where the store keeps its row: a column it
    /// names under any spelling the load reads is one it has. A store
    /// holding no row yet is laid out by the first commit and lacks none.
    fn read_lacking(&mut self, stored: &arrow_schema::Schema) {
        self.lacking.clear();
        if !self.keeps_row || stored.fields().is_empty() {
            return;
        }
        let held: Vec<usize> = stored
            .fields()
            .iter()
            .filter_map(|column| super::registry_column(column.name()))
            .collect();
        let columns = IsinEntry::field().fields().len();
        self.lacking
            .extend((0..columns).filter(|at| !held.contains(at)));
    }

    /// Warns, once per commit and column, where `table` holds a value in a
    /// column the store's row has none of: a commit replaces the rows under
    /// the row the store holds, so the value stays the registry's and not
    /// the store's - no migration rewrites the store - until the store is
    /// laid out afresh. The column and the store are the warning's key.
    fn warn_lacking(&self, table: &IsinTable) {
        for &at in &self.lacking {
            let held = table
                .rows
                .values()
                .filter(|row| row.states_column(at))
                .count();
            if held == 0 {
                continue;
            }
            let column = super::column_name(at);
            let store = location(&self.holder);
            warned!(
                "instrument registry column not stored: the store's row lacks it",
                &format!("{column} at {store}"),
                "the {column} of {held} of the registry's rows is not stored: the store, laid \
                 out before the column, has its rows replaced under its own row, so the registry \
                 alone holds the value until the store is laid out afresh"
            );
        }
    }
}

/// Where `holder` is, as a warning or a refusal names it.
fn location(holder: &Holder) -> String {
    holder
        .url()
        .map_or_else(|| "an unlocated handle".to_owned(), |url| url.to_string())
}

impl IsinRegistry {
    /// A registry bound to `holder` and loaded from it: the holder's own
    /// record stream - an Arrow IPC leaf, a Parquet one, a folder of parts,
    /// an Iceberg table, an object store - read once through
    /// [`Self::extend_from_arrow_reader`] under options resolved once
    /// ([`Self::set_holder`]); a store holding nothing yet is an empty first
    /// run, laid out by the first [`Self::commit`]. Clean after the load.
    ///
    /// Unseeded: the registry holds the store's rows and nothing else.
    /// [`Self::seeded_from_holder`] lays them over the seed instead.
    ///
    /// # Errors
    ///
    /// What the holder's read or [`Self::extend_from_arrow_reader`] refuses.
    pub fn from_holder(holder: impl Into<Holder>) -> Result<Self> {
        Self::new().try_with_holder(holder)
    }

    /// [`Self::from_holder`] over the holder `url` names under `properties`
    /// ([`Holder::from_url`]): any scheme this build holds, a `with (...)`
    /// clause's pairs beside it.
    ///
    /// Unseeded, as [`Self::from_holder`] is: [`Self::seeded_from_url`]
    /// lays the store's rows over the seed instead.
    ///
    /// # Errors
    ///
    /// What [`Holder::from_url`] or [`Self::from_holder`] refuses.
    pub fn from_url<K, V>(url: &Url, properties: impl IntoIterator<Item = (K, V)>) -> Result<Self>
    where
        K: AsRef<str>,
        V: AsRef<str>,
    {
        Self::from_holder(Holder::from_url(url, properties)?)
    }

    /// The seed ([`Self::seeded`]) with the rows `holder` stores laid over
    /// it, bound to that store: the layering [`Self::from_env`] gives the
    /// store the environment names, for a store the caller names. The
    /// store's rows fold over the seed's by the update rule, so a value the
    /// store states wins and a fact only the seed states stands beside it,
    /// a seed row the store has no row of stands, and a row only the store
    /// holds is the store's; a store holding nothing yet loads as the seed
    /// bound to it. The store is read once, as [`Self::from_holder`] reads
    /// it, and the seed costs it no call.
    ///
    /// Clean after the load, so nothing is written until something moves:
    /// the first [`Self::commit`] after a learn or a merge that moved a row
    /// writes the whole snapshot, the seed's rows with the store's.
    ///
    /// # Errors
    ///
    /// What the holder's read or [`Self::extend_from_arrow_reader`] refuses.
    pub fn seeded_from_holder(holder: impl Into<Holder>) -> Result<Self> {
        let mut registry = Self::seeded();
        registry.set_holder_over(holder.into())?;
        Ok(registry)
    }

    /// [`Self::seeded_from_holder`] over the holder `url` names under
    /// `properties` ([`Holder::from_url`]): the seed with the store's rows
    /// laid over it, bound to the store, clean after the load.
    ///
    /// ```
    /// use yggdryl::local::LocalFolder;
    /// use yggdryl::{Ccy, IdType, Isin, IsinEntry, IsinRegistry, Mic, Url};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let root = LocalFolder::temporary()?
    ///     .path()?
    ///     .join(format!("yggdryl-isin-seeded-doc-{}", std::process::id()));
    /// let url = Url::from_path(root.join("instruments.arrows"))?;
    /// let none: [(&str, &str); 0] = [];
    /// // A store stating Apple, a seed instrument, under another currency, and
    /// // one instrument the seed has no row of.
    /// let mut store = IsinRegistry::from_url(&url, none)?;
    /// store.merge(
    ///     IsinEntry::new(Isin::new("US0378331005")?)
    ///         .with_miccode(Some(Mic::new("XNAS")?))
    ///         .with_ticker(Some("AAPL".into()))
    ///         .with_currency(Some(Ccy::new("CHF")?)),
    /// )?;
    /// store.merge(IsinEntry::new(Isin::new("GB0002634946")?).with_miccode(Some(Mic::new("XLON")?)))?;
    /// store.commit()?;
    ///
    /// let seed = IsinRegistry::seeded();
    /// let mut registry = IsinRegistry::seeded_from_url(&url, none)?;
    /// assert_eq!(registry.len(), seed.len() + 1, "the seed, and the store's other row");
    /// assert!(!registry.is_dirty());
    /// let apple = registry.get("US0378331005").expect("the seed's and the store's");
    /// assert_eq!(apple.currency().map(Ccy::as_str), Some("CHF"), "the store's value wins");
    /// assert_eq!(apple.fisn(), seed.get("US0378331005").and_then(IsinEntry::fisn), "the seed's stands");
    ///
    /// // The first commit that moves anything writes every row.
    /// registry.merge(IsinEntry::new(Isin::new("CH0012214059")?).try_with_code(IdType::Ric, "HOLN.S")?)?;
    /// assert_eq!(registry.commit()?.written_rows, registry.len() as u64);
    /// assert!(IsinRegistry::from_url(&url, none)?.iter().eq(registry.iter()));
    /// std::fs::remove_dir_all(&root)?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// What [`Holder::from_url`] or [`Self::seeded_from_holder`] refuses.
    pub fn seeded_from_url<K, V>(
        url: &Url,
        properties: impl IntoIterator<Item = (K, V)>,
    ) -> Result<Self>
    where
        K: AsRef<str>,
        V: AsRef<str>,
    {
        Self::seeded_from_holder(Holder::from_url(url, properties)?)
    }

    /// Binds the registry to `holder`: the holder's rows are loaded, the
    /// rows the registry held fold over them by the update rule
    /// ([`Self::merge`]), and the registry is dirty exactly where a held
    /// row moved something. The options the rows cross under are resolved
    /// here, once. Answers how many rows the holder held.
    ///
    /// # Errors
    ///
    /// What the holder's read, [`Self::extend_from_arrow_reader`] or a
    /// fold of a held row refuses; the registry then stands as it was,
    /// bound to the store it was.
    pub fn set_holder(&mut self, holder: impl Into<Holder>) -> Result<usize> {
        let mut store = Store::bind(holder.into())?;
        let held = std::mem::take(&mut self.table);
        let was_dirty = self.dirty;
        let loaded = (|| -> Result<usize> {
            let reader = store.holder.read_arrow_reader(&store.options)?;
            store.read_lacking(&reader.schema());
            let read = self.extend_from_arrow_reader(reader)?;
            self.dirty = false;
            for row in held.rows.values() {
                self.merge(row.clone())?;
            }
            Ok(read)
        })();
        match loaded {
            Ok(read) => {
                self.store = Some(Box::new(store));
                Ok(read)
            }
            Err(error) => {
                self.table = held;
                self.dirty = was_dirty;
                Err(error)
            }
        }
    }

    /// Binds the registry to `holder` with the precedence of
    /// [`Self::set_holder`] turned over: the holder's rows fold over the
    /// rows the registry holds by the update rule, so a value the store
    /// states wins, and the registry is clean after - the seed beneath a
    /// store ([`Self::seeded_from_holder`], what [`Self::from_env`] loads).
    /// Answers how many rows the holder held.
    ///
    /// # Errors
    ///
    /// What the holder's read or [`Self::extend_from_arrow_reader`]
    /// refuses; the registry then stands as it was, bound to the store it
    /// was.
    pub(crate) fn set_holder_over(&mut self, holder: Holder) -> Result<usize> {
        let mut store = Store::bind(holder)?;
        let held = self.table.clone();
        let was_dirty = self.dirty;
        let loaded = (|| -> Result<usize> {
            let reader = store.holder.read_arrow_reader(&store.options)?;
            store.read_lacking(&reader.schema());
            Ok(self.extend_from_arrow_reader(reader)?)
        })();
        match loaded {
            Ok(read) => {
                self.dirty = false;
                self.store = Some(Box::new(store));
                Ok(read)
            }
            Err(error) => {
                self.table = held;
                self.dirty = was_dirty;
                Err(error)
            }
        }
    }

    /// [`Self::set_holder`], consuming.
    ///
    /// # Errors
    ///
    /// What [`Self::set_holder`] refuses.
    pub fn try_with_holder(mut self, holder: impl Into<Holder>) -> Result<Self> {
        self.set_holder(holder)?;
        Ok(self)
    }

    /// The holder the registry is bound to, where it is.
    #[must_use]
    pub fn holder(&self) -> Option<&Holder> {
        self.store.as_ref().map(|store| &store.holder)
    }

    /// Writes the table to the holder it is bound to, only where it moved
    /// since it was loaded or last committed, so the store holds exactly
    /// the snapshot ([`Self::into_arrow_reader`]) whatever its layout: a
    /// leaf rewritten whole in one overwrite, an emptied registry
    /// truncating it; an Iceberg table replaced in one atomic snapshot,
    /// every row of every partition, an emptied registry one empty
    /// snapshot that keeps the table a table; a plain folder's record parts
    /// of the store's encoding removed - a leaf of another encoding or a
    /// file that is no record part never touched - then the snapshot laid
    /// out as one part under the layout the folder spells, none where the
    /// registry is empty. A clean registry touches the store with no call
    /// and answers no rows. Clean after.
    ///
    /// A leaf or an Iceberg table keeps the row it was laid out with, so a
    /// store written before a column the registry now has - `eusipacode`,
    /// `fisn` - is written without it, and the commit warns, naming the column and
    /// the store, wherever the registry holds a value there; the store is
    /// never migrated - an emptied leaf, or a new store, is laid out with
    /// the row as it is now.
    ///
    /// # Errors
    ///
    /// A registry bound to no holder, and what the holder's write refuses,
    /// which leaves the registry dirty.
    pub fn commit(&mut self) -> Result<IOResult> {
        let Some(store) = &mut self.store else {
            return Err(Error::absent("isin registry holder", "an unbound registry"));
        };
        if !self.dirty {
            return Ok(IOResult::default());
        }
        let container = store.container;
        if store.native_table {
            let snapshot = Self::snapshot_reader(&self.table)?;
            let written = store.holder.write_serie(
                crate::iomedia::arrow_serie(snapshot)?,
                IOMode::Overwrite,
                Some(&store.write_options()),
            )?;
            store.warn_lacking(&self.table);
            self.dirty = false;
            return Ok(written);
        }
        #[cfg(feature = "iceberg")]
        if container && let Some(mut located) = crate::iceberg::located(&store.holder)? {
            if self.table.is_empty() {
                located.clear()?;
            } else {
                located.overwrite_whole(Self::snapshot_reader(&self.table)?)?;
                store.warn_lacking(&self.table);
            }
            self.dirty = false;
            let rows = self.table.len() as u64;
            return Ok(IOResult::new(rows, rows));
        }
        if container {
            let encoding = store.options.mime_type();
            for part in crate::media::partition::record_parts(&store.holder, encoding)? {
                part?.remove(false)?;
            }
        } else if self.table.is_empty() {
            store.holder.clear()?;
            // An emptied leaf is laid out afresh by the next commit.
            store.lacking.clear();
        }
        let result = if self.table.is_empty() {
            IOResult::default()
        } else {
            let snapshot = Self::snapshot_reader(&self.table)?;
            let written = store.holder.write_arrow_reader(
                snapshot,
                IOMode::Overwrite,
                &store.write_options(),
            )?;
            store.warn_lacking(&self.table);
            written
        };
        self.dirty = false;
        Ok(result)
    }

    /// The rows of `table` as the stream [`Self::into_arrow_reader`] answers.
    fn snapshot_reader(table: &IsinTable) -> Result<crate::arrow::BatchReader> {
        Ok(crate::arrow::rows::reader(
            &super::FIELD,
            super::Snapshot {
                rows: std::sync::Arc::clone(&table.rows),
                after: None,
            },
            None,
            None,
            None,
        )?)
    }
}
