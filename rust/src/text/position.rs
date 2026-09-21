//! Shared byte-position accounting for text parser diagnostics.

use std::collections::VecDeque;

/// Convert a one-based line and column pair to a bounded byte offset.
pub(crate) fn line_column_to_byte_offset(input: &[u8], line: usize, column: usize) -> usize {
    if line == 0 {
        return 0;
    }
    if line == 1 {
        return column.saturating_sub(1).min(input.len());
    }
    let mut current_line = 1_usize;
    for (index, byte) in input.iter().enumerate() {
        if *byte == b'\n' {
            current_line += 1;
            if current_line == line {
                return index.saturating_add(column).min(input.len());
            }
        }
    }
    input.len()
}

/// Tracks recent line starts while a streaming parser consumes bytes.
pub(crate) struct LineOffsets {
    bytes: usize,
    first_line: usize,
    starts: VecDeque<usize>,
    window: usize,
}

impl LineOffsets {
    pub(crate) fn new(window: usize) -> Self {
        let mut starts = VecDeque::with_capacity(window);
        starts.push_back(0);
        Self {
            bytes: 0,
            first_line: 1,
            starts,
            window: window.max(1),
        }
    }

    pub(crate) fn observe(&mut self, input: &[u8]) {
        for (index, byte) in input.iter().enumerate() {
            if *byte == b'\n' {
                if self.starts.len() == self.window {
                    self.starts.pop_front();
                    self.first_line = self.first_line.saturating_add(1);
                }
                self.starts
                    .push_back(self.bytes.saturating_add(index).saturating_add(1));
            }
        }
        self.bytes = self.bytes.saturating_add(input.len());
    }

    pub(crate) fn position(&self, line: usize, column: usize) -> usize {
        if line == 0 {
            return self.bytes;
        }
        let start = line
            .checked_sub(self.first_line)
            .and_then(|index| self.starts.get(index))
            .copied()
            .unwrap_or(self.bytes);
        start
            .saturating_add(column.saturating_sub(1))
            .min(self.bytes)
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/text/position.rs` pins and a caller cannot reach.
    //!
    //! A byte offset is what a parser's refusal names, and a caller reads the
    //! offset rather than the accounting that produced it. `LineOffsets` is a
    //! running state rather than a value, so it is opened as a forwarding
    //! wrapper: the real type keeps its `pub(crate)` visibility.

    /// Convert a one-based line and column pair to a bounded byte offset.
    #[must_use]
    pub fn line_column_to_byte_offset(input: &[u8], line: usize, column: usize) -> usize {
        super::line_column_to_byte_offset(input, line, column)
    }

    /// The recent line starts a streaming parser tracks.
    pub struct LineOffsets(super::LineOffsets);

    impl LineOffsets {
        /// Track at most `window` recent line starts.
        #[must_use]
        pub fn new(window: usize) -> Self {
            Self(super::LineOffsets::new(window))
        }

        /// Account for the next run of bytes the parser consumed.
        pub fn observe(&mut self, input: &[u8]) {
            self.0.observe(input);
        }

        /// The bounded byte offset of a one-based line and column.
        #[must_use]
        pub fn position(&self, line: usize, column: usize) -> usize {
            self.0.position(line, column)
        }
    }
}
