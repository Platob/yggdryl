//! Listings: one request per page, asked for when the page before it is
//! drained.
//!
//! The three listings of the service - table buckets, namespaces, tables -
//! page the same way, so the walk is written once in [`Listing`] and each
//! public iterator is that walk over one kind of entry.

use std::iter::FusedIterator;

use super::bucket::TableBucket;
use super::client::{Call, Reader, S3Tables};
use super::namespace::NamespaceSummary;
use super::table::TableSummary;
use crate::{Arn, Result};

/// How many entries one page asks for.
///
/// The session's HTTP client holds an answer whole and bounds it at 256 KiB,
/// and an entry whose names run to the 255 bytes the model allows is close
/// to a kilobyte of JSON, so this is the largest page that always fits -
/// under the service's own cap of 1000.
const PAGE_SIZE: u32 = 250;

/// The request of one page of a listing, without its continuation token.
pub(crate) struct Page {
    /// The operation as the model names it.
    pub(crate) operation: &'static str,
    /// The wire path, labels percent-encoded once.
    pub(crate) path: String,
    /// The query pairs every page carries, raw.
    pub(crate) query: Vec<(String, String)>,
    /// The table bucket listed under, whose ARN names the region.
    pub(crate) bucket: Option<Arn>,
    /// What a refusal reports.
    pub(crate) addressed: String,
    /// The member of the answer that holds the page's entries.
    pub(crate) entries: &'static str,
    /// The query name the page size is stated under.
    pub(crate) limit: &'static str,
}

/// Where a walk stands between two pages.
enum Next {
    /// Nothing asked yet: the first page has no token.
    First,
    /// The token the service answered the last page with.
    Token(String),
    /// The last page was the last, or a page failed.
    Done,
}

/// The walk every listing shares: the entries of the page in hand, then the
/// next page, one request at a time.
///
/// Building one sends nothing. A failure - of the page's request, of an
/// entry that does not read, of a continuation token the service answers
/// twice, or of the address the listing was built with - is handed over
/// once, in its place: the entries read before it are handed over first,
/// and the walk ends there.
struct Listing<T> {
    client: S3Tables,
    /// The page to ask for, or why no page can be asked for at all.
    page: Option<Result<Page>>,
    /// What reads one entry of a page.
    read: fn(Reader<'_>) -> Result<T>,
    /// The entries of the page in hand, not yet handed over, ending at the
    /// first that failed.
    held: std::vec::IntoIter<Result<T>>,
    next: Next,
}

impl<T> Listing<T> {
    fn new(client: S3Tables, page: Result<Page>, read: fn(Reader<'_>) -> Result<T>) -> Self {
        Self {
            client,
            page: Some(page),
            read,
            held: Vec::new().into_iter(),
            next: Next::First,
        }
    }

    /// Ask for the page `token` continues, with one request, and answer its
    /// entries - up to and including the first that does not read - and the
    /// token of the page after it, none after a failure.
    fn fetch(&self, page: &Page, token: Option<&str>) -> Result<(Vec<Result<T>>, Option<String>)> {
        let mut call = Call::get(page.operation, page.path.clone(), page.addressed.clone())
            .with_query(page.limit, &PAGE_SIZE.to_string());
        for (name, value) in &page.query {
            call = call.with_query(name, value);
        }
        if let Some(token) = token {
            call = call.with_query("continuationToken", token);
        }
        if let Some(bucket) = &page.bucket {
            call = call.in_bucket(bucket);
        }
        let reply = self.client.send(&call)?;
        let reader = reply.reader();
        let mut entries = Vec::new();
        for entry in reader.entries(page.entries)? {
            let read = (self.read)(entry);
            let failed = read.is_err();
            entries.push(read);
            if failed {
                return Ok((entries, None));
            }
        }
        let next = match reader.optional_text("continuationToken") {
            // A token that names the page just read would walk it forever,
            // and ending there would call a listing whole that is not.
            Some(next) if Some(next) == token => {
                entries.push(Err(reader.malformed(
                    "expected a continuation token other than the one the page was asked \
                     with, got the same one",
                )));
                None
            }
            next => next.map(str::to_owned),
        };
        Ok((entries, next))
    }
}

impl<T> Iterator for Listing<T> {
    type Item = Result<T>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(entry) = self.held.next() {
                return Some(entry);
            }
            // Taken before the request, so whatever it answers the walk is
            // over unless a page hands the next token back.
            let token = match std::mem::replace(&mut self.next, Next::Done) {
                Next::First => None,
                Next::Token(token) => Some(token),
                Next::Done => return None,
            };
            let page = match self.page.take()? {
                Ok(page) => page,
                Err(error) => return Some(Err(error)),
            };
            match self.fetch(&page, token.as_deref()) {
                Ok((entries, next)) => {
                    self.held = entries.into_iter();
                    if let Some(next) = next {
                        self.next = Next::Token(next);
                        self.page = Some(Ok(page));
                    }
                }
                Err(error) => return Some(Err(error)),
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let held = self.held.len();
        (held, matches!(self.next, Next::Done).then_some(held))
    }
}

/// One public iterator over one kind of entry.
macro_rules! listing {
    ($(#[$doc:meta])* $name:ident, $entry:ty, $read:path) => {
        $(#[$doc])*
        pub struct $name(Listing<$entry>);

        impl $name {
            /// The walk over `page`, or the one refusal that keeps a page
            /// from being asked for; nothing is sent.
            pub(crate) fn new(client: S3Tables, page: Result<Page>) -> Self {
                Self(Listing::new(client, page, $read))
            }
        }

        impl Iterator for $name {
            type Item = Result<$entry>;

            fn next(&mut self) -> Option<Self::Item> {
                self.0.next()
            }

            fn size_hint(&self) -> (usize, Option<usize>) {
                self.0.size_hint()
            }
        }

        impl FusedIterator for $name {}

        impl std::fmt::Debug for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter
                    .debug_struct(stringify!($name))
                    .field("held", &self.0.held.len())
                    .field("drained", &matches!(self.0.next, Next::Done))
                    .finish_non_exhaustive()
            }
        }
    };
}

listing!(
    /// The table buckets of an account in one region, read lazily: one
    /// request per page, the first when the first entry is asked for, and
    /// nothing after the first error.
    TableBuckets,
    TableBucket,
    TableBucket::read
);

listing!(
    /// The namespaces of a table bucket, read lazily: one request per page,
    /// the first when the first entry is asked for, and nothing after the
    /// first error.
    NamespaceSummaries,
    NamespaceSummary,
    NamespaceSummary::read
);

listing!(
    /// The tables of a table bucket, or of one namespace of it, read lazily:
    /// one request per page, the first when the first entry is asked for,
    /// and nothing after the first error.
    TableSummaries,
    TableSummary,
    TableSummary::read
);
