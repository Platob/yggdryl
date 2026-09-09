//! One S3 prefix, or a whole bucket, as a container.

use std::sync::Arc;

use super::client::{Client, DELETE_BATCH};
use super::file::File;
use crate::holder::Holder;
use crate::{Error, IOBase, IOFolder, IOKind, Listing, MediaType, Result, Url};

/// A key prefix on S3, addressed as a container.
///
/// A prefix is not a thing the store holds: it exists exactly while some key
/// starts with it. That single fact settles most of this handle's behavior -
/// creating one costs nothing, deleting one costs nothing, and listing one is
/// the only question the store can actually answer.
///
/// # What each operation costs
///
/// | operation | requests |
/// | --- | --- |
/// | building the handle | none |
/// | [`IOBase::child_by_path`] | none |
/// | [`IOBase::ls`], flat or recursive | one listing per 1000 entries |
/// | [`IOBase::glob`] | one listing per 1000 entries under the fixed prefix |
/// | [`IOFolder::folder_exists`] | one listing of one key |
/// | [`IOBase::clear`] | one listing per 1000 keys, plus one bulk delete per 1000 |
/// | [`IOBase::remove`] with `recursive` | the same |
/// | [`IOBase::remove`] without | one listing of one key |
///
/// A **recursive** listing is the case worth stating: it is one flat listing
/// of the prefix, not one request per directory. S3 returns keys in byte
/// order, which is depth-first pre-order once the container each key implies
/// is emitted before it - so a lake of ten thousand parts across a thousand
/// partitions is ten requests, not a thousand.
#[derive(Clone)]
pub struct Folder {
    client: Arc<Client>,
    /// The location, with any credentials the caller wrote into it removed.
    url: Url,
    bucket: String,
    /// The key prefix, decoded, ending in `/` unless it is the bucket root.
    prefix: String,
}

impl Folder {
    /// Describe the prefix `url` names on `client`, touching nothing.
    pub(super) fn new(client: Arc<Client>, url: Url) -> Result<Self> {
        let (bucket, key) = super::split_location(&url)?;
        // A container's prefix always ends in the delimiter, because that is
        // what makes it a prefix of the keys beneath it rather than of the
        // keys merely starting with its name.
        let prefix = if key.is_empty() || key.ends_with('/') {
            key
        } else {
            format!("{key}/")
        };
        Ok(Self {
            client,
            url: super::without_credentials(url),
            bucket,
            prefix,
        })
    }

    /// Borrow the described location.
    pub const fn url(&self) -> &Url {
        &self.url
    }

    /// The bucket this prefix is in.
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// The key prefix, as the store sees it.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// How many requests this handle's client has sent, by shape.
    pub fn stats(&self) -> super::StatsSnapshot {
        self.client.snapshot()
    }

    /// Return whether anything lives under this prefix.
    ///
    /// One listing of a single key: the cheapest question the store answers.
    pub fn exists(&self) -> bool {
        self.folder_exists()
    }

    /// Whether anything lives under this prefix, or the store's refusal.
    ///
    /// [`IOFolder::folder_exists`] answers `bool` and so has to read a refusal
    /// as a `false`. Anything that *acts* on the answer asks here instead: a
    /// listing nobody was allowed to see must not read as an empty prefix and
    /// turn a refused removal into a silent success.
    fn populated(&self) -> Result<bool> {
        if self.prefix.is_empty() {
            return self.client.head_bucket(&self.bucket);
        }
        let page = self
            .client
            .list_objects(&self.bucket, &self.prefix, None, None, 1)?;
        Ok(!page.objects.is_empty() || !page.prefixes.is_empty())
    }

    /// Create the bucket when this is a bucket root, and nothing otherwise.
    ///
    /// A prefix comes into being when a key under it is written, so there is
    /// nothing to create and no marker object is invented. A bucket is a real
    /// resource, so a root asks for one.
    ///
    /// # Errors
    ///
    /// Returns the store's refusal to create the bucket.
    pub fn create(&self) -> Result<()> {
        if self.prefix.is_empty() {
            if !self.client.options().bucket_creation() {
                return Err(refused("create", &self.bucket));
            }
            return self.client.create_bucket(&self.bucket);
        }
        Ok(())
    }

    /// Yield every key under this prefix, one page at a time.
    ///
    /// `delimiter` rolls sub-prefixes up into one entry each, which is what
    /// makes a flat listing one level rather than a subtree.
    fn pages(
        &self,
        delimiter: Option<&'static str>,
    ) -> impl Iterator<Item = Result<Entry>> + use<> {
        let client = self.client.clone();
        let bucket = self.bucket.clone();
        let prefix = self.prefix.clone();
        let page_size = client.options().list_page_size();
        let mut continuation: Option<String> = None;
        let mut finished = false;
        // Each page is fetched when the previous one runs out, so a caller
        // that takes three entries from a prefix of a hundred thousand pays
        // for one page.
        std::iter::from_fn(move || {
            if finished {
                return None;
            }
            let page = client.list_objects(
                &bucket,
                &prefix,
                delimiter,
                continuation.as_deref(),
                page_size,
            );
            let page = match page {
                Ok(page) => page,
                Err(error) => {
                    finished = true;
                    return Some(Err(error));
                }
            };
            continuation = page.next_continuation_token.clone();
            if !page.is_truncated || continuation.is_none() {
                finished = true;
            }
            let objects = page.objects.into_iter().map(|object| {
                Ok(Entry::Object {
                    key: object.key,
                    size: object.size,
                })
            });
            let prefixes = page
                .prefixes
                .into_iter()
                .map(|prefix| Ok(Entry::Prefix { key: prefix }));
            Some(Ok(objects.chain(prefixes).collect::<Vec<_>>()))
        })
        .flat_map(|page| match page {
            Ok(entries) => entries,
            Err(error) => vec![Err(error)],
        })
    }

    /// One level's entries: the keys directly under this prefix, and the
    /// sub-prefixes, each once.
    fn level(&self, include_private: bool) -> Listing {
        let client = self.client.clone();
        let url = self.url.clone();
        let prefix = self.prefix.clone();
        Listing::new(self.pages(Some("/")).filter_map(move |entry| {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => return Some(Err(error)),
            };
            let relative = entry.key().strip_prefix(prefix.as_str())?;
            // A prefix listing echoes the prefix itself when a key is spelled
            // exactly like it; that is this container, not a child.
            if relative.is_empty() {
                return None;
            }
            if !include_private && relative.trim_end_matches('/').starts_with('.') {
                return None;
            }
            Some(hold(&client, &url, relative, &entry))
        }))
    }

    /// The whole subtree, in one flat listing, containers included.
    ///
    /// S3 answers keys in byte order, and a container sorts before everything
    /// under it, so emitting each key's not-yet-seen ancestors just before the
    /// key itself yields exactly the depth-first pre-order a recursive walk
    /// promises - out of one request per page rather than one per directory.
    fn subtree(&self, include_private: bool) -> Listing {
        let client = self.client.clone();
        let url = self.url.clone();
        let prefix = self.prefix.clone();
        // The ancestors already emitted, held as the path from this prefix
        // down to the last key. Keys arrive in byte order, so a directory the
        // walk has left never comes back: what is held is bounded by the
        // tree's depth rather than by the number of entries.
        let mut held: Vec<String> = Vec::new();
        let mut pending: std::vec::IntoIter<Result<Holder>> = Vec::new().into_iter();
        let mut entries = self.pages(None);
        Listing::new(std::iter::from_fn(move || {
            loop {
                if let Some(ready) = pending.next() {
                    return Some(ready);
                }
                let entry = match entries.next()? {
                    Ok(entry) => entry,
                    Err(error) => return Some(Err(error)),
                };
                let Some(relative) = entry.key().strip_prefix(prefix.as_str()) else {
                    continue;
                };
                if relative.is_empty() {
                    continue;
                }
                if !include_private && relative.split('/').any(|part| part.starts_with('.')) {
                    continue;
                }
                let mut batch: Vec<Result<Holder>> = Vec::new();
                // Every directory this key implies, emitted before the key.
                let segments: Vec<&str> = relative.split('/').collect();
                let leaf = segments.len() - 1;
                let mut walked = String::new();
                for (depth, part) in segments[..leaf].iter().enumerate() {
                    walked.push_str(part);
                    walked.push('/');
                    if held.get(depth).is_some_and(|seen| seen == &walked) {
                        continue;
                    }
                    held.truncate(depth);
                    held.push(walked.clone());
                    batch.push(hold(
                        &client,
                        &url,
                        &walked,
                        &Entry::Prefix {
                            key: walked.clone(),
                        },
                    ));
                }
                // A key spelled with a trailing delimiter is a directory
                // marker - a console or an older tool wrote it so the prefix
                // would show up - and it names the container the keys under it
                // already imply. It is emitted once, as that container, never
                // as a leaf holding zero bytes.
                if !segments[leaf].is_empty() {
                    batch.push(hold(&client, &url, relative, &entry));
                } else if batch.is_empty() {
                    // The container it names was emitted with an earlier key.
                    continue;
                }
                pending = batch.into_iter();
            }
        }))
    }

    /// Every key under this prefix, for the operations that delete them.
    fn keys(&self) -> impl Iterator<Item = Result<String>> + use<> {
        self.pages(None).map(|entry| entry.map(Entry::into_key))
    }
}

/// One thing a listing page reports.
enum Entry {
    /// A key, with the size the listing already gave.
    Object { key: String, size: u64 },
    /// A rolled-up sub-prefix, which reads as a container.
    Prefix { key: String },
}

impl Entry {
    fn key(&self) -> &str {
        match self {
            Self::Object { key, .. } | Self::Prefix { key } => key,
        }
    }

    fn into_key(self) -> String {
        match self {
            Self::Object { key, .. } | Self::Prefix { key } => key,
        }
    }
}

/// Build the handle one listing entry names.
///
/// The listing already reported the size, so a caller reading it pays nothing:
/// this is why a partition scan can weigh a lake without a `HEAD` per file.
fn hold(client: &Arc<Client>, root: &Url, relative: &str, entry: &Entry) -> Result<Holder> {
    let child = root.joinpath(&super::encode_key_path(relative))?;
    match entry {
        Entry::Prefix { .. } => Folder::new(client.clone(), child).map(Holder::ObjectFolder),
        Entry::Object { size, .. } => File::new(client.clone(), child)
            .map(|file| Holder::ObjectFile(file.with_known_size(*size))),
    }
}

/// An S3 prefix is the container role over the store.
impl IOFolder for Folder {
    fn folder_url(&self) -> &Url {
        &self.url
    }

    /// Return whether anything is under this prefix.
    ///
    /// One listing bounded to a single key: a prefix exists exactly while a
    /// key starts with it, so one entry settles it and the page stops there.
    fn folder_exists(&self) -> bool {
        self.populated().unwrap_or(false)
    }

    fn create_folder(&self) -> Result<()> {
        self.create()
    }

    fn list_folder(&self, recursive: bool, include_private: bool) -> Listing {
        if recursive {
            self.subtree(include_private)
        } else {
            self.level(include_private)
        }
    }

    /// Delete the bucket when this is a root, and nothing otherwise.
    ///
    /// A prefix is not stored, so there is nothing to delete and its absence
    /// is the success the contract asks for.
    fn delete_folder(&mut self) -> Result<()> {
        if self.prefix.is_empty() {
            if !self.client.options().bucket_deletion() {
                return Err(refused("delete", &self.bucket));
            }
            return self.client.delete_bucket(&self.bucket);
        }
        Ok(())
    }

    /// Remove every key under this prefix, keeping the prefix itself.
    ///
    /// One listing per 1000 keys and one bulk delete per 1000, rather than one
    /// delete per key. An empty prefix costs one listing and no delete.
    fn folder_clear(&mut self) -> Result<()> {
        let mut batch: Vec<String> = Vec::with_capacity(DELETE_BATCH);
        for key in self.keys() {
            batch.push(key?);
            if batch.len() == DELETE_BATCH {
                self.client.delete_objects(&self.bucket, &batch)?;
                batch.clear();
            }
        }
        if !batch.is_empty() {
            self.client.delete_objects(&self.bucket, &batch)?;
        }
        Ok(())
    }

    /// Delete the subtree, or refuse a populated prefix without `recursive`.
    fn folder_remove(&mut self, recursive: bool) -> Result<()> {
        if recursive {
            self.folder_clear()?;
        } else if !self.prefix.is_empty() && self.populated()? {
            return Err(crate::iobase::not_empty(&self.url));
        }
        self.delete_folder()
    }
}

impl crate::IOMedia for Folder {
    crate::impl_default_iomedia!();
}

impl IOBase for Folder {
    fn pread(&self, _offset: u64, _buffer: &mut [u8]) -> Result<usize> {
        self.folder_pread()
    }

    fn pwrite(&mut self, _offset: u64, bytes: &[u8]) -> Result<usize> {
        self.folder_pwrite(bytes.len())
    }

    fn size(&self) -> u64 {
        0
    }

    fn capacity(&self) -> u64 {
        0
    }

    fn reserve(&mut self, _capacity: u64) -> Result<()> {
        // Reserving space in a container is meaningless but harmless.
        Ok(())
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        self.folder_truncate(size)
    }

    fn url(&self) -> Option<&Url> {
        Some(&self.url)
    }

    fn media_type(&self) -> &MediaType {
        self.folder_media_type()
    }

    fn set_media_type(&mut self, _media_type: MediaType) {
        // A prefix is a prefix; it has no content type to declare.
    }

    fn parent(&self) -> Option<Holder> {
        let parent = self.url.parent()?;
        Self::new(self.client.clone(), parent)
            .ok()
            .map(Holder::ObjectFolder)
    }

    /// Resolve a descendant without asking the store anything.
    ///
    /// A name ending in `/` is a container by its spelling; anything else is
    /// undecided until an operation needs to know, which is what
    /// [`super::Path`] is for.
    fn child_by_path(&self, name: &str) -> Result<Holder> {
        let url = self.url.joinpath(name)?;
        if url.has_trailing_slash() {
            return Self::new(self.client.clone(), url).map(Holder::ObjectFolder);
        }
        super::Path::new(self.client.clone(), url).map(Holder::ObjectPath)
    }

    fn ls(&self, recursive: bool, include_private: bool) -> Listing {
        self.folder_ls(recursive, include_private)
    }

    fn kind(&self) -> IOKind {
        self.folder_kind()
    }

    fn clear(&mut self) -> Result<()> {
        self.folder_clear()
    }

    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.folder_remove(recursive)
    }

    fn is_atomic(&self) -> bool {
        self.folder_is_atomic()
    }

    fn is_tabular(&self) -> bool {
        self.folder_is_tabular()
    }
}

impl std::fmt::Debug for Folder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Folder")
            .field("url", &self.url)
            .finish()
    }
}

/// Refuse a bucket lifecycle operation this client was told not to perform.
///
/// No request goes out: this is the client's own rule, not the store's, and
/// finding out from the store would be a round trip and an audit-log entry.
fn refused(operation: &str, bucket: &str) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        format!("this client is not allowed to {operation} the bucket {bucket}"),
    ))
}

/// Report a listing that cannot form a child location.
#[allow(dead_code)]
fn unusable(error: Error) -> Listing {
    Listing::failing(error)
}
