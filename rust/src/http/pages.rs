//! `Pages`: the walk over a paginated resource, one [`Response`] per page,
//! and the Arrow reader that lays every page out as one batch.
//!
//! The walk holds the session, the next request, the pagination, the URLs
//! already visited and the page limit. A page request that fails is yielded
//! as the failure and kept as the next request, so a caller that asks again
//! resumes from that page's own request and never from the first; a `Retry-After`
//! or a rate-limit pause the page asked for is honoured before the next one,
//! up to the session's `max_pause`; a next URL already visited, a `has_more`
//! of `false`, an empty page or a page under [`Pagination::None`] ends the
//! walk. [`Pages::into_arrow_reader`] parses each page as it arrives, takes
//! its rows at the records path - declared on the request, else detected on
//! the first page and kept - and lays them out through
//! [`Serie::from_scalars`] under one root, inferred from the first page
//! unless given; one page is in memory at a time.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use arrow_array::{RecordBatch, RecordBatchReader};
use arrow_schema::{ArrowError, SchemaRef};
use smol_str::format_smolstr;

use super::{NextPage, Pagination, Request, Response, Session};
use crate::arrow::BatchReader;
use crate::{
    ArrowCastOptions, Error, Field, FieldPath, FieldSegment, Result, Scalar, Serie, SerieReader,
};

/// The pages of one request, walked lazily.
///
/// ```no_run
/// use yggdryl::http::{Pagination, Request};
///
/// # fn main() -> yggdryl::Result<()> {
/// let request = Request::get("https://api.example.com/v1/orders?limit=100")?
///     .with_pagination(Pagination::Link);
/// let mut rows = 0;
/// for page in request.pages() {
///     let page = page?;
///     rows += page.scalar()?.path("data").map_or(0, |data| data.len());
/// }
/// assert!(rows > 0);
/// // Or every page as one Arrow batch under one inferred root.
/// let reader = request.pages().into_arrow_reader(None, 0)?;
/// assert_eq!(reader.schema().fields().len(), reader.schema().fields().len());
/// # Ok(())
/// # }
/// ```
pub struct Pages {
    session: Session,
    next: Option<Request>,
    pagination: Pagination,
    records: Option<FieldPath>,
    visited: HashSet<u64>,
    page_index: u64,
    yielded: usize,
    page_limit: Option<usize>,
    pause: Option<Duration>,
    /// A first page already fetched, yielded before any request goes out.
    held: Option<Response>,
}

impl std::fmt::Debug for Pages {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Pages")
            .field("next", &self.next.as_ref().map(Request::url))
            .field("pagination", &self.pagination)
            .field("records", &self.records)
            .field("page_index", &self.page_index)
            .field("page_limit", &self.page_limit)
            .finish()
    }
}

impl Pages {
    /// The walk from `first` through `session`, under the request's
    /// pagination, records path and page limit.
    pub(crate) fn new(session: Session, first: Request) -> Self {
        Self {
            pagination: first.pagination().clone(),
            records: first.records().cloned(),
            page_limit: first.page_limit(),
            session,
            next: Some(first),
            visited: HashSet::new(),
            page_index: 0,
            yielded: 0,
            pause: None,
            held: None,
        }
    }

    /// The walk from `first` through `session` with its first page already in
    /// hand: `page` is yielded first and read for where the next one is, so a
    /// caller that fetched it to look at it does not fetch it again.
    ///
    /// # Errors
    ///
    /// A `Link` or a cursor on `page` that will not parse.
    pub(crate) fn from_first(session: Session, first: Request, page: Response) -> Result<Self> {
        let mut pages = Self::new(session, first.clone());
        pages.next = None;
        pages.visited.insert(first.url().stable_hash());
        if page.is_ok() {
            pages.advance(&page, &first)?;
        }
        pages.held = Some(page);
        Ok(pages)
    }

    /// The first page: [`Iterator::next`].
    pub fn first_page(&mut self) -> Option<Result<Response>> {
        self.next()
    }

    /// Every page laid out as Arrow batches under one root: `field` when
    /// given, else the non-null record the first page's rows infer.
    ///
    /// One batch per page, split into several of at most `batch_row_size`
    /// rows when the page is longer (zero is no bound). A page whose rows the
    /// root refuses is an error naming the page URL and the row; a page
    /// answering a status of 400 or more is [`Error::Remote`].
    ///
    /// # Errors
    ///
    /// The first page's failure, a first page with no rows to infer a root
    /// from, or a root that is not a record.
    pub fn into_arrow_reader(
        mut self,
        field: Option<&Field>,
        batch_row_size: usize,
    ) -> Result<BatchReader> {
        let chunk = if batch_row_size == 0 {
            usize::MAX
        } else {
            batch_row_size
        };
        let (root, pending) = match self.next() {
            None => {
                let root = field.cloned().ok_or_else(|| {
                    Error::absent("page to infer a record root from", "http pages")
                })?;
                (Arc::new(root), Vec::new())
            }
            Some(page) => {
                let page = page?;
                page.raise_for_status()?;
                let rows = self.rows_of_page(&page)?;
                let root = match field {
                    Some(field) => field.clone(),
                    None => Scalar::from_sequence(rows.iter().cloned()).inferred_struct_field()?,
                };
                let root = Arc::new(root);
                let batches = batches_of(&root, &rows, chunk, page.url().to_string())?;
                (root, batches)
            }
        };
        let schema = Serie::from_scalars(Arc::clone(&root), Vec::new())?
            .into_arrow_batch()?
            .schema();
        Ok(Box::new(PageBatches {
            pages: self,
            root,
            schema,
            pending: pending.into(),
            chunk,
        }))
    }

    /// [`Self::into_arrow_reader`] as a [`SerieReader`], one record column
    /// per page.
    ///
    /// # Errors
    ///
    /// As [`Self::into_arrow_reader`].
    pub fn into_serie_reader(self, field: Option<&Field>) -> Result<SerieReader> {
        let reader = self.into_arrow_reader(field, 0)?;
        Ok(SerieReader::from_arrow_reader(
            field,
            reader,
            ArrowCastOptions::default(),
        )?)
    }

    /// The rows of `page` at the records path, the path detected on the
    /// first page and kept.
    fn rows_of_page(&mut self, page: &Response) -> Result<Vec<Scalar>> {
        let body = page.scalar()?;
        let path =
            Pagination::records_path(&body, self.records.as_ref()).ok_or_else(|| Error::Codec {
                format: "http page",
                position: 0,
                reason: format_smolstr!("page {} holds no sequence of rows", page.url()),
            })?;
        let rows = rows_at(&body, &path).ok_or_else(|| Error::Codec {
            format: "http page",
            position: 0,
            reason: format_smolstr!("page {} holds no sequence at {path}", page.url()),
        })?;
        self.records = Some(path);
        Ok(rows)
    }

    /// Settle where the page after `page` is, and the pause before it.
    fn advance(&mut self, page: &Response, request: &Request) -> Result<()> {
        let body = page.scalar().ok();
        let rows = match &body {
            Some(body) => {
                let path = Pagination::records_path(body, self.records.as_ref());
                if let Some(path) = &path {
                    if self.records.is_none() {
                        self.records = Some(path.clone());
                    }
                }
                path.and_then(|path| rows_at(body, &path))
                    .map_or(1, |rows| rows.len())
            }
            None => 1,
        };
        let next = self.pagination.next(
            page.url(),
            page.headers(),
            body.as_ref(),
            self.page_index,
            rows,
        )?;
        self.page_index += 1;
        let next = match next {
            None => return Ok(()),
            Some(NextPage::Url(url)) => request.with_url(url),
            Some(NextPage::Parameter { name, value }) => request.with_parameter(&name, &value)?,
        };
        if self.visited.contains(&next.url().stable_hash()) {
            return Ok(());
        }
        let now = self.session.now_ns();
        let asked = match page.headers().retry_after(now)? {
            Some(pause) => Some(pause),
            None => page.headers().rate_limit_pause(now)?,
        };
        self.pause = asked
            .filter(|pause| !pause.is_zero())
            .map(|pause| pause.min(self.session.max_pause()));
        self.next = Some(next);
        Ok(())
    }
}

impl Iterator for Pages {
    type Item = Result<Response>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.page_limit.is_some_and(|limit| self.yielded >= limit) {
            self.next = None;
            self.held = None;
            return None;
        }
        if let Some(page) = self.held.take() {
            self.yielded += 1;
            return Some(Ok(page));
        }
        let request = self.next.take()?;
        if let Some(pause) = self.pause.take() {
            std::thread::sleep(pause);
        }
        let page = match self.session.send(&request) {
            Ok(page) => page,
            Err(error) => {
                // Kept, so the next ask resumes from this page's own request.
                self.next = Some(request);
                return Some(Err(error));
            }
        };
        self.yielded += 1;
        self.visited.insert(request.url().stable_hash());
        if page.is_ok() {
            if let Err(error) = self.advance(&page, &request) {
                return Some(Err(error));
            }
        }
        Some(Ok(page))
    }
}

impl std::iter::FusedIterator for Pages {}

/// One batch per page, the pages pulled as the batches are.
struct PageBatches {
    pages: Pages,
    root: Arc<Field>,
    schema: SchemaRef,
    pending: VecDeque<RecordBatch>,
    chunk: usize,
}

impl Iterator for PageBatches {
    type Item = std::result::Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(batch) = self.pending.pop_front() {
                return Some(Ok(batch));
            }
            let page = match self.pages.next()? {
                Ok(page) => page,
                Err(error) => return Some(Err(external(error))),
            };
            let batches = page
                .raise_for_status()
                .and_then(|page| self.pages.rows_of_page(page))
                .and_then(|rows| batches_of(&self.root, &rows, self.chunk, page.url().to_string()));
            match batches {
                Ok(batches) => self.pending.extend(batches),
                Err(error) => return Some(Err(external(error))),
            }
        }
    }
}

impl RecordBatchReader for PageBatches {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

/// `rows` under `root`, at most `chunk` per batch; a refused row named by
/// the page and its index.
fn batches_of(
    root: &Arc<Field>,
    rows: &[Scalar],
    chunk: usize,
    url: String,
) -> Result<Vec<RecordBatch>> {
    let mut batches = Vec::new();
    if rows.is_empty() {
        return Ok(batches);
    }
    for (index, part) in rows.chunks(chunk).enumerate() {
        let serie = match Serie::from_scalars(Arc::clone(root), part.iter().cloned()) {
            Ok(serie) => serie,
            Err(error) => {
                let offset = index.saturating_mul(chunk);
                let row = part
                    .iter()
                    .position(|row| root.scalar(row.clone()).is_err())
                    .map_or(offset, |position| offset + position);
                return Err(Error::Codec {
                    format: "http page",
                    position: row,
                    reason: format_smolstr!("page {url} row {row}: {error}"),
                });
            }
        };
        batches.push(serie.into_arrow_batch()?);
    }
    Ok(batches)
}

/// The rows of `body` at the records path `declared`, or detected.
pub(crate) fn rows_of(body: &Scalar, declared: Option<&FieldPath>) -> Option<Vec<Scalar>> {
    let path = Pagination::records_path(body, declared)?;
    rows_at(body, &path)
}

/// The sequence at `path` in `body`, as owned rows.
fn rows_at(body: &Scalar, path: &FieldPath) -> Option<Vec<Scalar>> {
    let mut current = std::borrow::Cow::Borrowed(body);
    for segment in path.segments() {
        let stepped = match segment {
            FieldSegment::Field(name) => current.get_key_str(name).cloned(),
            FieldSegment::Index(index) => {
                let len = i64::try_from(current.len()).unwrap_or(i64::MAX);
                let position = if *index < 0 { len + *index } else { *index };
                usize::try_from(position)
                    .ok()
                    .and_then(|position| current.get(position))
                    .map(std::borrow::Cow::into_owned)
            }
            _ => None,
        }?;
        current = std::borrow::Cow::Owned(stepped);
    }
    current.sequence_rows().map(|rows| rows.into_owned())
}

/// A crate failure as the Arrow reader reports it.
fn external(error: Error) -> ArrowError {
    ArrowError::ExternalError(Box::new(error))
}
