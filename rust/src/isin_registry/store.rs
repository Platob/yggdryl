//! The store a registry is bound to: the holder its table was loaded from
//! and is committed back to as one snapshot, under the holder's own record
//! options resolved once at the binding.

#[cfg(feature = "iceberg")]
use smol_str::format_smolstr;

use crate::holder::Holder;
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
    /// Whether the holder is a container, read once at the binding.
    container: bool,
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
        let container = holder.is_container();
        #[cfg(feature = "iceberg")]
        if container
            && let Some(located) = crate::iceberg::located(&holder)?
            && !located.is_whole()
        {
            return Err(Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$.holder"),
                reason: format_smolstr!(
                    "expected an Iceberg table whole for the instrument registry, got a partition of one at {}",
                    holder
                        .url()
                        .map_or_else(|| "an unlocated handle".to_owned(), |url| url.to_string())
                ),
            });
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
            container,
        })
    }

    /// The options a commit writes under: the holder's own, declaring the
    /// registry's row.
    fn write_options(&self) -> RecordOptions {
        self.options.clone().with_field(IsinEntry::field())
    }
}

impl IsinRegistry {
    /// A registry bound to `holder` and loaded from it: the holder's own
    /// record stream - an Arrow IPC leaf, a Parquet one, a folder of parts,
    /// an Iceberg table, an object store - read once through
    /// [`Self::extend_from_arrow_reader`] under options resolved once
    /// ([`Self::set_holder`]); a store holding nothing yet is an empty first
    /// run, laid out by the first [`Self::commit`]. Clean after the load.
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
        let store = Store::bind(holder.into())?;
        let held = std::mem::take(&mut self.table);
        let was_dirty = self.dirty;
        let loaded = (|| -> Result<usize> {
            let reader = store.holder.read_arrow_reader(&store.options)?;
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
        #[cfg(feature = "iceberg")]
        if container && let Some(mut located) = crate::iceberg::located(&store.holder)? {
            if self.table.is_empty() {
                located.clear()?;
            } else {
                located.overwrite_whole(Self::snapshot_reader(&self.table)?)?;
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
        }
        let result = if self.table.is_empty() {
            IOResult::default()
        } else {
            let snapshot = Self::snapshot_reader(&self.table)?;
            store
                .holder
                .write_arrow_reader(snapshot, IOMode::Overwrite, &store.write_options())?
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
