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

#[cfg(test)]
mod tests {
    use super::{LineOffsets, line_column_to_byte_offset};

    #[test]
    fn positions_are_bounded_byte_offsets() {
        let input = b"one\ntwo\nthree";
        assert_eq!(line_column_to_byte_offset(input, 2, 2), 5);
        assert_eq!(line_column_to_byte_offset(input, 2, 1), 4);
        assert_eq!(line_column_to_byte_offset(input, 9, 1), input.len());
        assert_eq!(line_column_to_byte_offset(input, 9, 9), input.len());

        let mut offsets = LineOffsets::new(2);
        offsets.observe(b"one\ntwo\n");
        offsets.observe(b"three");
        assert_eq!(offsets.position(2, 2), 5);
        assert_eq!(offsets.position(1, 1), input.len());
    }
}
