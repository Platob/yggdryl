//! The exchange trace: every request the server reads and every answer it
//! writes, as `message/http` documents under one folder - `NNNN-request.http`
//! and `NNNN-response.http` - numbered from `0000` in the order the requests'
//! heads arrived, across every connection.
//!
//! An HTTP/1.1 exchange is written byte for byte as it went over the wire:
//! the request as the connection consumed it, its framing included, and the
//! answer as written, an interim `100 Continue` and a chunked framing
//! included. An HTTP/2 or HTTP/3 exchange is written as the HTTP/1.1 message
//! its frames carried - the head they stated, the body as sent - since the
//! frames themselves are no message a reader of the folder can read. A
//! request file is written whole once the request is read; an answer is
//! appended as it goes out, [`SPILL`] bytes at a time, so a streamed answer
//! is never held whole for its trace.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::IOBase;
use crate::holder::Holder;

/// The most of an answer's copy held before it is appended to its file.
const SPILL: usize = 64 * 1024;

/// A server's trace: the folder, and the number the next exchange takes.
pub(super) struct Trace {
    folder: Arc<Holder>,
    next: AtomicU64,
}

impl Trace {
    pub(super) fn new(folder: Arc<Holder>) -> Self {
        Self {
            folder,
            next: AtomicU64::new(0),
        }
    }

    /// Open the next exchange: its number taken and its answer's file
    /// created empty, so an answer the peer never waited for still pairs
    /// with its request.
    pub(super) fn begin(&self) -> Exchange {
        let number = self.next.fetch_add(1, Ordering::Relaxed);
        let mut exchange = Exchange {
            folder: Arc::clone(&self.folder),
            number,
            response: None,
            pending: Vec::new(),
            failed: false,
        };
        match exchange.leaf("response").and_then(|mut leaf| {
            leaf.write_all_bytes(&[])?;
            Ok(leaf)
        }) {
            Ok(leaf) => exchange.response = Some(leaf),
            Err(_) => exchange.failed = true,
        }
        exchange
    }
}

/// One exchange's two files. A write that fails marks the exchange, and the
/// connection closes after its answer rather than go on tracing past a gap.
pub(super) struct Exchange {
    folder: Arc<Holder>,
    number: u64,
    response: Option<Holder>,
    pending: Vec<u8>,
    failed: bool,
}

impl Exchange {
    fn leaf(&self, side: &str) -> crate::Result<Holder> {
        self.folder
            .child_by_path(&format!("{:04}-{side}.http", self.number))
    }

    /// Write the request as it was read, whole.
    pub(super) fn request(&mut self, bytes: &[u8]) {
        if self
            .leaf("request")
            .and_then(|mut leaf| leaf.write_all_bytes(bytes))
            .is_err()
        {
            self.failed = true;
        }
    }

    /// Copy `bytes` of the answer, appended to its file once [`SPILL`] of
    /// them are held.
    pub(super) fn response(&mut self, bytes: &[u8]) {
        if self.failed {
            return;
        }
        self.pending.extend_from_slice(bytes);
        if self.pending.len() >= SPILL {
            self.spill();
        }
    }

    /// Append what is held now: a head written before its body, so the
    /// order on disk is the order on the wire.
    #[cfg(feature = "http2")]
    pub(super) fn flush(&mut self) {
        if !self.pending.is_empty() && !self.failed {
            self.spill();
        }
    }

    fn spill(&mut self) {
        let appended = match self.response.as_mut() {
            Some(leaf) => leaf.append_bytes(&self.pending).is_ok(),
            None => false,
        };
        self.failed |= !appended;
        self.pending.clear();
    }

    /// Append what is still held; whether every write of the exchange landed.
    pub(super) fn finish(mut self) -> bool {
        if !self.pending.is_empty() && !self.failed {
            self.spill();
        }
        !self.failed
    }
}

impl Drop for Exchange {
    fn drop(&mut self) {
        // An exchange cut short - a peer gone mid-answer, a panicking
        // handler - still leaves what was written of it.
        if !self.pending.is_empty() && !self.failed {
            self.spill();
        }
    }
}
