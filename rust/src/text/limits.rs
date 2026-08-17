use serde::{Deserialize, Serialize};

/// Resource limits applied while decoding caller-controlled data.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Limits {
    /// Maximum structural nesting per document. A root container has depth 1.
    max_depth: usize,
    /// Maximum bytes consumed by one decoder invocation.
    max_input_bytes: usize,
    /// Maximum scalar and container nodes per document.
    max_nodes: usize,
    /// Maximum values or documents yielded by a stream.
    max_documents: usize,
}

impl Limits {
    /// Construct explicit resource limits.
    pub const fn new(
        max_depth: usize,
        max_input_bytes: usize,
        max_nodes: usize,
        max_documents: usize,
    ) -> Self {
        Self {
            max_depth,
            max_input_bytes,
            max_nodes,
            max_documents,
        }
    }

    /// Return the maximum per-document nesting depth.
    pub const fn max_depth(self) -> usize {
        self.max_depth
    }

    /// Return the maximum number of encoded input bytes.
    pub const fn max_input_bytes(self) -> usize {
        self.max_input_bytes
    }

    /// Return the maximum number of decoded value nodes per document.
    pub const fn max_nodes(self) -> usize {
        self.max_nodes
    }

    /// Return the maximum number of stream documents.
    pub const fn max_documents(self) -> usize {
        self.max_documents
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::new(128, 64 * 1024 * 1024, 1_000_000, 1_024)
    }
}
