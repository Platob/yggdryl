//! Shared streams retain their native parts and replay each source once.

use crate::{
    ChunkedSerie, Field, KeySerie, Serie, SpillOptions, StreamChunkedSerie, StreamKeySerie,
    StreamSerie,
};
use smol_str::{SmolStr, format_smolstr};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

/// The sealed native-part seam; a key stream retains its items and layout.
pub trait Streamed: sealed::Sealed + Send + Sized + 'static {
    type Part: Clone + Send + Sync + 'static;
    type Context: Clone + Send + Sync + 'static;
    fn root(&self) -> &Field;
    fn context(&self) -> Self::Context;
    fn pull_part(&mut self) -> Option<crate::arrow::Result<Self::Part>>;
    fn from_parts(
        context: Self::Context,
        parts: impl Iterator<Item = crate::arrow::Result<Self::Part>> + Send + 'static,
    ) -> crate::arrow::Result<Self>;
    fn hold(part: &mut Self::Part) -> crate::arrow::Result<()>;
    fn retain(part: &mut Self::Part) -> crate::arrow::Result<()>;
    fn chunks(part: &Self::Part) -> (Vec<Serie>, Option<SmolStr>);
    fn memory_size(part: &Self::Part) -> usize;
    fn resident_size(part: &Self::Part) -> usize;
    fn spill(part: &mut Self::Part, options: &SpillOptions) -> crate::Result<()>;
}

mod sealed {
    pub trait Sealed {}
}
impl sealed::Sealed for StreamChunkedSerie {}
impl sealed::Sealed for StreamSerie {}
impl sealed::Sealed for StreamKeySerie {}

macro_rules! record_parts {
    () => {
        type Part = Serie;
        type Context = Arc<Field>;
        fn root(&self) -> &Field {
            self.field()
        }
        fn context(&self) -> Arc<Field> {
            Arc::new(self.field().clone())
        }
        fn hold(part: &mut Serie) -> crate::arrow::Result<()> {
            *part = part.clone().settled()?;
            Ok(())
        }
        fn retain(part: &mut Serie) -> crate::arrow::Result<()> {
            Self::hold(part)
        }
        fn chunks(part: &Serie) -> (Vec<Serie>, Option<SmolStr>) {
            (vec![part.clone()], None)
        }
        fn memory_size(part: &Serie) -> usize {
            part.memory_size()
        }
        fn resident_size(part: &Serie) -> usize {
            part.resident_size()
        }
        fn spill(part: &mut Serie, options: &SpillOptions) -> crate::Result<()> {
            part.spill(options)
        }
    };
}

impl Streamed for StreamChunkedSerie {
    record_parts!();
    fn pull_part(&mut self) -> Option<crate::arrow::Result<Serie>> {
        self.next_chunk()
    }
    fn from_parts(
        root: Arc<Field>,
        parts: impl Iterator<Item = crate::arrow::Result<Serie>> + Send + 'static,
    ) -> crate::arrow::Result<Self> {
        Self::from_landed_iter(root, parts)
    }
}

impl Streamed for StreamSerie {
    record_parts!();
    fn pull_part(&mut self) -> Option<crate::arrow::Result<Serie>> {
        self.next_chunk()
    }
    fn from_parts(
        root: Arc<Field>,
        parts: impl Iterator<Item = crate::arrow::Result<Serie>> + Send + 'static,
    ) -> crate::arrow::Result<Self> {
        let rows = parts.flat_map(|part| {
            let (part, failed) = match part {
                Ok(part) => (Some(part), None),
                Err(error) => (None, Some(Err(crate::Error::from(error)))),
            };
            let len = part.as_ref().map_or(0, Serie::len);
            failed.into_iter().chain(
                (0..len).map(move |row| part.as_ref().expect("a successful part").scalar(row)),
            )
        });
        Ok(Self::from_proven_rows(root.as_ref().clone(), rows))
    }
}

impl Streamed for StreamKeySerie {
    type Part = KeySerie;
    type Context = Arc<crate::key_serie::KeyLayout>;
    fn root(&self) -> &Field {
        self.field()
    }
    fn context(&self) -> Self::Context {
        Arc::clone(&self.layout)
    }
    fn pull_part(&mut self) -> Option<crate::arrow::Result<KeySerie>> {
        self.next()
    }
    fn from_parts(
        layout: Self::Context,
        parts: impl Iterator<Item = crate::arrow::Result<KeySerie>> + Send + 'static,
    ) -> crate::arrow::Result<Self> {
        Ok(Self::new(layout, parts))
    }
    fn hold(part: &mut KeySerie) -> crate::arrow::Result<()> {
        part.hold_rows()
    }
    fn retain(_part: &mut KeySerie) -> crate::arrow::Result<()> {
        Ok(())
    }
    fn chunks(part: &KeySerie) -> (Vec<Serie>, Option<SmolStr>) {
        let (rows, failure) = part.global_chunks();
        (rows.chunks().to_vec(), failure.clone())
    }
    fn memory_size(part: &KeySerie) -> usize {
        part.memory_size()
    }
    fn resident_size(part: &KeySerie) -> usize {
        part.rows().resident_size()
    }
    fn spill(part: &mut KeySerie, options: &SpillOptions) -> crate::Result<()> {
        part.spill_rows(options)
    }
}

/// An opaque stream whose clones retain native parts under the spill bound.
pub struct SharedStream<S: Streamed> {
    root: Arc<Field>,
    context: S::Context,
    state: Mutex<Pulling<S>>,
    drained: OnceLock<(ChunkedSerie, Option<SmolStr>)>,
    joined: OnceLock<(Serie, Option<SmolStr>)>,
}

struct Pulling<S: Streamed> {
    stream: Option<S>,
    held: Vec<S::Part>,
    failure: Option<SmolStr>,
}

impl<S: Streamed> SharedStream<S> {
    /// The global record field, available before pulling.
    pub fn field(&self) -> &Field {
        &self.root
    }
    pub(crate) const fn root(&self) -> &Arc<Field> {
        &self.root
    }
    /// Whether every row has been held.
    pub fn is_held(&self) -> bool {
        self.drained.get().is_some()
    }
    pub(crate) fn new(stream: S) -> Self {
        Self {
            root: Arc::new(stream.root().clone()),
            context: stream.context(),
            state: Mutex::new(Pulling {
                stream: Some(stream),
                held: Vec::new(),
                failure: None,
            }),
            drained: OnceLock::new(),
            joined: OnceLock::new(),
        }
    }
    fn lock(&self) -> MutexGuard<'_, Pulling<S>> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    fn part(&self, at: usize) -> Option<crate::arrow::Result<S::Part>> {
        let mut state = self.lock();
        loop {
            if let Some(part) = state.held.get(at) {
                return Some(Ok(part.clone()));
            }
            state.stream.as_ref()?;
            // A lazy window must be held before advancing its walk.
            if let Some(previous) = state.held.last_mut()
                && let Err(error) = S::hold(previous)
            {
                return Some(Err(fail(&mut state, error)));
            }
            if let Err(error) = settle::<S>(&mut state.held) {
                return Some(Err(fail(&mut state, error.into())));
            }
            match state.stream.as_mut()?.pull_part() {
                Some(Ok(mut part)) => {
                    if let Err(error) = S::retain(&mut part) {
                        return Some(Err(fail(&mut state, error)));
                    }
                    state.held.push(part);
                    if let Err(error) = settle::<S>(&mut state.held) {
                        return Some(Err(fail(&mut state, error.into())));
                    }
                }
                Some(Err(error)) => return Some(Err(fail(&mut state, error))),
                None => {
                    state.stream = None;
                    return None;
                }
            }
        }
    }
    pub(crate) fn memory_size(&self) -> usize {
        self.lock().held.iter().map(S::memory_size).sum()
    }
    pub(crate) fn resident_size(&self) -> usize {
        self.lock().held.iter().map(S::resident_size).sum()
    }
    pub(crate) fn is_spilled(&self) -> bool {
        self.resident_size() == 0 && self.memory_size() > 0
    }
    fn failure(&self) -> Option<SmolStr> {
        self.lock().failure.clone()
    }
    pub(crate) fn drained(&self) -> &(ChunkedSerie, Option<SmolStr>) {
        self.drained.get_or_init(|| {
            let mut at = 0;
            while let Some(Ok(_)) = self.part(at) {
                at += 1;
            }
            let state = self.lock();
            let mut held = ChunkedSerie::from_landed(Arc::clone(&self.root), Vec::new());
            let mut failure = state.failure.clone();
            'parts: for part in &state.held {
                let (pieces, failed) = S::chunks(part);
                for piece in pieces {
                    let piece = if piece.field() == Some(self.root.as_ref()) {
                        piece
                    } else {
                        let record = piece.as_struct().expect("stream parts are records");
                        crate::SerieValue::into_serie(crate::StructSerie::new(
                            Arc::clone(&self.root),
                            record.children().to_vec(),
                            record.nulls().cloned(),
                            piece.len(),
                        ))
                    };
                    if let Err(error) = held.push_landed(piece) {
                        failure.get_or_insert_with(|| format_smolstr!("{error}"));
                        break 'parts;
                    }
                }
                if let Some(failed) = failed {
                    failure.get_or_insert(failed);
                    break;
                }
            }
            (held, failure)
        })
    }
    pub(crate) fn joined(&self) -> &(Serie, Option<SmolStr>) {
        self.joined.get_or_init(|| {
            let (chunks, failure) = self.drained();
            held_join(chunks, failure.clone())
        })
    }
    pub(crate) fn held_failure(&self) -> Option<&SmolStr> {
        self.drained()
            .1
            .as_ref()
            .or_else(|| self.joined.get().and_then(|(_, failure)| failure.as_ref()))
    }
    pub(crate) fn into_stream(this: Arc<Self>) -> crate::arrow::Result<S> {
        match Arc::try_unwrap(this) {
            Ok(alone) => {
                let state = alone
                    .state
                    .into_inner()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if state.held.is_empty()
                    && state.failure.is_none()
                    && let Some(stream) = state.stream
                {
                    return Ok(stream);
                }
                let failure = state
                    .failure
                    .map(|reason| Err(failed(&alone.root, &reason)));
                let mut rest = state.stream;
                S::from_parts(
                    alone.context,
                    state
                        .held
                        .into_iter()
                        .map(Ok)
                        .chain(failure)
                        .chain(std::iter::from_fn(move || rest.as_mut()?.pull_part())),
                )
            }
            Err(shared) => {
                let context = shared.context.clone();
                let mut at = 0;
                let mut ended = false;
                S::from_parts(
                    context,
                    std::iter::from_fn(move || {
                        if ended {
                            return None;
                        }
                        let part = shared.part(at);
                        at += 1;
                        if matches!(part, Some(Ok(_))) {
                            return part;
                        }
                        ended = true;
                        part.or_else(|| {
                            shared
                                .failure()
                                .map(|reason| Err(failed(&shared.root, &reason)))
                        })
                    }),
                )
            }
        }
    }
}

fn settle<S: Streamed>(parts: &mut [S::Part]) -> crate::Result<()> {
    let options = SpillOptions::from_env()?;
    let bound = options.byte_size();
    let resident = |parts: &[S::Part]| {
        parts
            .iter()
            .map(S::resident_size)
            .fold(0_u64, |sum, bytes| sum.saturating_add(bytes as u64))
    };
    if options.is_never() || resident(parts) <= bound {
        return Ok(());
    }
    let mut order: Vec<_> = parts
        .iter()
        .enumerate()
        .map(|(index, part)| (index, S::resident_size(part)))
        .collect();
    order.sort_by_key(|(_, bytes)| std::cmp::Reverse(*bytes));
    let whole = options.clone().with_byte_size(0);
    for (index, _) in order {
        if resident(parts) <= bound {
            break;
        }
        S::spill(&mut parts[index], &whole)?;
    }
    Ok(())
}

fn fail<S: Streamed>(state: &mut Pulling<S>, error: crate::arrow::Error) -> crate::arrow::Error {
    state.stream = None;
    state.failure = Some(format_smolstr!("{error}"));
    error
}

/// The refusal a held failure is raised as on every later ask.
pub(crate) fn failed(root: &Field, reason: &str) -> crate::arrow::Error {
    crate::Error::InvalidRecord {
        path: SmolStr::new(root.name()),
        reason: smol_str::format_smolstr!(
            "the stream {} failed while it was held: {reason}",
            root.name()
        ),
    }
    .into()
}

/// `chunks` as one column, the failure carried beside it: a join that cannot
/// be laid out is a failure too, answered as the empty column of the field.
pub(crate) fn held_join(
    chunks: &ChunkedSerie,
    failure: Option<SmolStr>,
) -> (Serie, Option<SmolStr>) {
    match chunks.into_serie() {
        Ok(joined) => (joined, failure),
        Err(error) => (
            Serie::empty(Arc::new(chunks.field().clone())).unwrap_or_default(),
            Some(failure.unwrap_or_else(|| smol_str::format_smolstr!("{error}"))),
        ),
    }
}

impl<S: Streamed> std::fmt::Debug for SharedStream<S> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SharedStream")
            .field("field", &self.root)
            .field("held", &self.drained.get().is_some())
            .finish_non_exhaustive()
    }
}
