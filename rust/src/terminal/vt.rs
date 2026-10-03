//! VT mirror port of `internal/hub/vt_buffer.go` (oracle 21d0bc7).
//! This is a text mirror for detection, not a replacement terminal emulator.
use super::width::cell_width;
use std::collections::VecDeque;

pub const DEFAULT_COLS: usize = 200;
pub const DEFAULT_ROWS: usize = 50;
pub const MAX_ESCAPE_BYTES: usize = 256;
pub const MAX_STRING_SKIP_BYTES: usize = 1 << 20;
pub const MAX_SCROLLBACK_LINES: usize = 500;
const CONTINUATION: char = '\0';

#[derive(Clone, Debug)]
pub struct VtBuffer {
    cols: usize,
    rows: usize,
    cells: Vec<Vec<char>>,
    row: usize,
    col: usize,
    scrollback: VecDeque<String>,
    saved_row: usize,
    saved_col: usize,
    wrap_pending: bool,
    utf8_pending: Vec<u8>,
    escape: Vec<u8>,
    string_sequence: bool,
    string_escape: bool,
    string_skip_bytes: usize,
    alt_screen: bool,
}
impl VtBuffer {
    pub fn new(cols: usize, rows: usize) -> Self {
        let cols = if cols == 0 { DEFAULT_COLS } else { cols };
        let rows = if rows == 0 { DEFAULT_ROWS } else { rows };
        Self {
            cols,
            rows,
            cells: vec![vec![' '; cols]; rows],
            row: 0,
            col: 0,
            scrollback: VecDeque::new(),
            saved_row: 0,
            saved_col: 0,
            wrap_pending: false,
            utf8_pending: Vec::new(),
            escape: Vec::new(),
            string_sequence: false,
            string_escape: false,
            string_skip_bytes: 0,
            alt_screen: false,
        }
    }
    pub fn rows(&self) -> usize {
        self.rows
    }
    pub fn cols(&self) -> usize {
        self.cols
    }
    pub fn cursor(&self) -> (usize, usize) {
        (self.row, self.col)
    }
    pub fn alt_screen(&self) -> bool {
        self.alt_screen
    }
    pub fn pending_bytes(&self) -> usize {
        self.escape.len() + self.utf8_pending.len()
    }
    pub fn skipping_string(&self) -> bool {
        self.string_sequence
    }
    pub fn reset(&mut self) {
        let alt = self.alt_screen;
        *self = Self::new(self.cols, self.rows);
        self.alt_screen = alt;
    }
    pub fn resize(&mut self, cols: usize, rows: usize) {
        let cols = if cols == 0 { DEFAULT_COLS } else { cols };
        let rows = if rows == 0 { DEFAULT_ROWS } else { rows };
        if cols != self.cols || rows != self.rows {
            self.scrollback.clear();
        }
        self.cells.resize_with(rows, || vec![' '; cols]);
        for row in &mut self.cells {
            row.resize(cols, ' ');
        }
        self.cols = cols;
        self.rows = rows;
        self.row = self.row.min(rows - 1);
        self.col = self.col.min(cols - 1);
        self.saved_row = self.saved_row.min(rows - 1);
        self.saved_col = self.saved_col.min(cols - 1);
    }
    pub fn lines(&self) -> Vec<String> {
        self.cells.iter().map(|r| render(r)).collect()
    }
    pub fn tail_lines(&self, n: usize) -> Vec<String> {
        let lines = self.lines();
        if n == 0 || n >= lines.len() {
            lines
        } else {
            lines[lines.len() - n..].to_vec()
        }
    }
    pub fn tail_lines_with_scrollback(&self, n: usize) -> Vec<String> {
        let mut all: Vec<_> = self.scrollback.iter().cloned().collect();
        all.extend(self.lines());
        if n > 0 && n < all.len() {
            all.drain(..all.len() - n);
        }
        all
    }
    pub fn write(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let mut joined = std::mem::take(&mut self.utf8_pending);
        joined.extend_from_slice(bytes);
        let mut rest = joined.as_slice();
        while !rest.is_empty() {
            let byte = rest[0];
            if self.string_sequence {
                rest = &rest[1..];
                // Carry ESC across writes so ST split at any byte is equivalent.
                // Intentional fix to Go's same-chunk-only two-byte ST check.
                if byte == 7 || (self.string_escape && byte == b'\\') {
                    self.string_sequence = false;
                    self.string_escape = false;
                    self.string_skip_bytes = 0;
                    continue;
                }
                self.string_escape = byte == 0x1b;
                self.string_skip_bytes += 1;
                if self.string_skip_bytes > MAX_STRING_SKIP_BYTES {
                    self.string_sequence = false;
                    self.string_escape = false;
                    self.string_skip_bytes = 0;
                }
                continue;
            }
            if !self.escape.is_empty() {
                self.escape.push(byte);
                rest = &rest[1..];
                if self.escape.len() >= 2
                    && matches!(self.escape[1], b']' | b'P' | b'X' | b'^' | b'_')
                {
                    self.escape.clear();
                    self.string_sequence = true;
                    self.string_skip_bytes = 0;
                    self.string_escape = false;
                } else if self.escape_complete() {
                    let seq = std::mem::take(&mut self.escape);
                    self.process_escape(&seq);
                } else if self.escape.len() > MAX_ESCAPE_BYTES {
                    self.escape.clear();
                }
                continue;
            }
            if byte == 0x1b {
                self.escape.push(byte);
                rest = &rest[1..];
                continue;
            }
            let expected = match byte {
                0..=0x7f => 1,
                0xc2..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf4 => 4,
                _ => 1,
            };
            let candidate = &rest[..expected.min(rest.len())];
            match std::str::from_utf8(candidate) {
                Ok(text) => {
                    let c = text.chars().next().unwrap();
                    self.write_char(c);
                    rest = &rest[c.len_utf8()..];
                }
                Err(error) if error.error_len().is_none() => {
                    self.utf8_pending.extend_from_slice(rest);
                    break;
                }
                Err(_) => {
                    self.write_char('\u{fffd}');
                    rest = &rest[1..];
                }
            }
        }
    }
    fn escape_complete(&self) -> bool {
        if self.escape.len() < 2 {
            return false;
        }
        if self.escape[1] != b'[' {
            return !matches!(self.escape[1], b'(' | b')' | b'*' | b'+') || self.escape.len() >= 3;
        }
        self.escape.len() >= 3 && matches!(self.escape.last(), Some(0x40..=0x7e))
    }
    fn process_escape(&mut self, seq: &[u8]) {
        if seq == b"\x1b7" {
            self.saved_row = self.row;
            self.saved_col = self.col;
            return;
        }
        if seq == b"\x1b8" {
            self.row = self.saved_row;
            self.col = self.saved_col;
            self.wrap_pending = false;
            return;
        }
        if !seq.starts_with(b"\x1b[") || seq.len() < 3 {
            return;
        }
        let final_byte = seq[seq.len() - 1];
        let body = String::from_utf8_lossy(&seq[2..seq.len() - 1]);
        let private = body.starts_with('?');
        let body = body.strip_prefix('?').unwrap_or(&body);
        let params: Vec<i64> = if body.is_empty() {
            Vec::new()
        } else {
            body.split(';')
                .map(|p| p.trim().parse().unwrap_or(0))
                .collect()
        };
        let p = |i: usize, default: i64| {
            params
                .get(i)
                .copied()
                .filter(|p| *p != 0)
                .unwrap_or(default)
        };
        if matches!(
            final_byte,
            b'A' | b'B' | b'C' | b'D' | b'G' | b'H' | b'f' | b'u'
        ) {
            self.wrap_pending = false;
        }
        let clamp = |v: i64, max: usize| v.clamp(0, (max - 1) as i64) as usize;
        match final_byte {
            b'A' => self.row = clamp((self.row as i64).saturating_sub(p(0, 1)), self.rows),
            b'B' => self.row = clamp((self.row as i64).saturating_add(p(0, 1)), self.rows),
            b'C' => self.col = clamp((self.col as i64).saturating_add(p(0, 1)), self.cols),
            b'D' => self.col = clamp((self.col as i64).saturating_sub(p(0, 1)), self.cols),
            b'G' => self.col = clamp(p(0, 1).saturating_sub(1), self.cols),
            b'H' | b'f' => {
                self.row = clamp(p(0, 1).saturating_sub(1), self.rows);
                self.col = clamp(p(1, 1).saturating_sub(1), self.cols);
            }
            b'J' => self.erase_display(p(0, 0)),
            b'K' => match p(0, 0) {
                1 => self.clear_row(self.row, 0, self.col),
                2 => self.clear_row(self.row, 0, self.cols - 1),
                _ => self.clear_row(self.row, self.col, self.cols - 1),
            },
            b'X' => self.clear_row(
                self.row,
                self.col,
                self.col.saturating_add(p(0, 1).max(1) as usize - 1),
            ),
            b's' => {
                self.saved_row = self.row;
                self.saved_col = self.col;
            }
            b'u' => {
                self.row = self.saved_row;
                self.col = self.saved_col;
            }
            b'h' | b'l' if private && params.first() == Some(&1049) => {
                self.alt_screen = final_byte == b'h';
                self.clear_all();
                self.row = 0;
                self.col = 0;
                self.wrap_pending = false;
            }
            _ => {}
        }
    }
    fn write_char(&mut self, c: char) {
        match c {
            '\r' => {
                self.col = 0;
                self.wrap_pending = false;
            }
            '\n' => {
                self.new_line();
                self.wrap_pending = false;
            }
            '\x08' => {
                self.col = self.col.saturating_sub(1);
                self.wrap_pending = false;
            }
            '\t' => {
                self.col = ((self.col / 8 + 1) * 8).min(self.cols - 1);
                self.wrap_pending = false;
            }
            c if c < ' ' => {}
            c => {
                let width = cell_width(c);
                if width == 0 {
                    return;
                }
                if self.wrap_pending {
                    self.col = 0;
                    self.new_line();
                    self.wrap_pending = false;
                }
                if self.col + width > self.cols {
                    self.col = 0;
                    self.new_line();
                }
                for i in 0..width.min(self.cols - self.col) {
                    self.invalidate_wide(self.row, self.col + i);
                }
                self.cells[self.row][self.col] = c;
                for i in 1..width.min(self.cols - self.col) {
                    self.cells[self.row][self.col + i] = CONTINUATION;
                }
                self.col += width;
                if self.col >= self.cols {
                    self.col = self.cols - 1;
                    self.wrap_pending = true;
                }
            }
        }
    }
    fn new_line(&mut self) {
        self.row += 1;
        if self.row < self.rows {
            return;
        }
        let line = render(&self.cells[0]);
        if !line.is_empty() || self.scrollback.back().is_none_or(|s| !s.is_empty()) {
            self.scrollback.push_back(line);
        }
        while self.scrollback.len() > MAX_SCROLLBACK_LINES {
            self.scrollback.pop_front();
        }
        self.cells.rotate_left(1);
        self.cells[self.rows - 1].fill(' ');
        self.row = self.rows - 1;
    }
    fn erase_display(&mut self, mode: i64) {
        match mode {
            1 => {
                for row in 0..self.row {
                    self.clear_row(row, 0, self.cols - 1);
                }
                self.clear_row(self.row, 0, self.col);
            }
            2 | 3 => {
                self.clear_all();
                self.row = 0;
                self.col = 0;
            }
            _ => {
                self.clear_row(self.row, self.col, self.cols - 1);
                for row in self.row + 1..self.rows {
                    self.clear_row(row, 0, self.cols - 1);
                }
            }
        }
    }
    fn clear_all(&mut self) {
        for row in &mut self.cells {
            row.fill(' ');
        }
    }
    fn clear_row(&mut self, row: usize, start: usize, end: usize) {
        let start = start.min(self.cols - 1);
        let end = end.min(self.cols - 1);
        self.invalidate_wide(row, start);
        self.invalidate_wide(row, end);
        if start <= end {
            self.cells[row][start..=end].fill(' ');
        }
    }
    fn invalidate_wide(&mut self, row: usize, col: usize) {
        if self.cells[row][col] == CONTINUATION {
            if col > 0 {
                self.cells[row][col - 1] = ' ';
            }
            self.cells[row][col] = ' ';
        } else if cell_width(self.cells[row][col]) == 2
            && col + 1 < self.cols
            && self.cells[row][col + 1] == CONTINUATION
        {
            self.cells[row][col] = ' ';
            self.cells[row][col + 1] = ' ';
        }
    }
}
fn render(cells: &[char]) -> String {
    cells
        .iter()
        .copied()
        .filter(|c| *c != CONTINUATION)
        .collect::<String>()
        .trim_end_matches(' ')
        .to_owned()
}
