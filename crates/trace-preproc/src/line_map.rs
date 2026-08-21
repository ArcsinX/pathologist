use std::path::PathBuf;

/// Maps output line/column positions back to original source locations.
#[derive(Debug, Clone, Default)]
pub struct LineMap {
    /// For each byte offset in output, store (file, line, col).
    pub entries: Vec<LineMapEntry>,
}

#[derive(Debug, Clone)]
pub struct LineMapEntry {
    pub output_offset: usize,
    pub file: PathBuf,
    pub line: u32,
    pub col: u32,
}

impl LineMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, output_offset: usize, file: PathBuf, line: u32, col: u32) {
        self.entries.push(LineMapEntry {
            output_offset,
            file,
            line,
            col,
        });
    }

    pub fn lookup(&self, output_offset: usize) -> Option<&LineMapEntry> {
        self.entries
            .iter()
            .rev()
            .find(|e| e.output_offset <= output_offset)
    }

    pub fn lookup_line(&self, _output_line: u32) -> Option<&LineMapEntry> {
        // Approximate: find last entry before this line
        self.entries.last()
    }
}
