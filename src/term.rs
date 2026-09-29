use std::collections::VecDeque;

use unicode_width::UnicodeWidthChar;
use vte::{Params, Perform};

pub const DEFAULT_FG: Rgb = Rgb {
    r: 0xE6,
    g: 0xE8,
    b: 0xEE,
};
pub const DEFAULT_BG: Rgb = Rgb {
    r: 0x0C,
    g: 0x0E,
    b: 0x12,
};
const MAX_SCROLLBACK: usize = 8_000;

const ANSI: [Rgb; 16] = [
    Rgb { r: 0x00, g: 0x00, b: 0x00 },
    Rgb { r: 0xCD, g: 0x00, b: 0x00 },
    Rgb { r: 0x00, g: 0xCD, b: 0x00 },
    Rgb { r: 0xCD, g: 0xCD, b: 0x00 },
    Rgb { r: 0x00, g: 0x00, b: 0xEE },
    Rgb { r: 0xCD, g: 0x00, b: 0xCD },
    Rgb { r: 0x00, g: 0xCD, b: 0xCD },
    Rgb { r: 0xE5, g: 0xE5, b: 0xE5 },
    Rgb { r: 0x7F, g: 0x7F, b: 0x7F },
    Rgb { r: 0xFF, g: 0x00, b: 0x00 },
    Rgb { r: 0x00, g: 0xFF, b: 0x00 },
    Rgb { r: 0xFF, g: 0xFF, b: 0x00 },
    Rgb { r: 0x5C, g: 0x5C, b: 0xFF },
    Rgb { r: 0xFF, g: 0x00, b: 0xFF },
    Rgb { r: 0x00, g: 0xFF, b: 0xFF },
    Rgb { r: 0xFF, g: 0xFF, b: 0xFF },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub width: u8,
    pub fg: Rgb,
    pub bg: Rgb,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}

impl Cell {
    fn blank() -> Self {
        Self {
            ch: ' ',
            width: 1,
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
            bold: false,
            italic: false,
            underline: false,
            inverse: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Pen {
    fg: Rgb,
    bg: Rgb,
    fg_index: Option<u8>,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    inverse: bool,
}

impl Default for Pen {
    fn default() -> Self {
        Self {
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
            fg_index: None,
            bold: false,
            dim: false,
            italic: false,
            underline: false,
            inverse: false,
        }
    }
}

impl Pen {
    fn effective_fg(&self) -> Rgb {
        let mut color = if self.bold {
            self.fg_index
                .filter(|index| *index < 8)
                .map(|index| ANSI[index as usize + 8])
                .unwrap_or(self.fg)
        } else {
            self.fg
        };
        if self.dim {
            color.r /= 2;
            color.g /= 2;
            color.b /= 2;
        }
        color
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pos {
    pub row: usize,
    pub col: usize,
}

#[derive(Clone, Copy, Debug)]
struct Selection {
    anchor: Pos,
    focus: Pos,
}

#[derive(Clone, Debug)]
pub struct KeyInput {
    pub key: KeyCode,
    pub text: Option<String>,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
}

#[derive(Clone, Debug)]
pub enum KeyCode {
    Char(char),
    Enter,
    Backspace,
    Tab,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    F(u8),
}

pub enum InputEffect {
    Bytes(Vec<u8>),
    Scroll(i32),
    Copy,
    Paste,
    Ignore,
}

pub struct Terminal {
    cols: usize,
    rows: usize,
    cells: Vec<Cell>,
    other: Vec<Cell>,
    scrollback: VecDeque<Vec<Cell>>,
    cursor_x: usize,
    cursor_y: usize,
    primary_cursor: (usize, usize),
    saved_cursor: (usize, usize),
    pen: Pen,
    scroll_top: usize,
    scroll_bot: usize,
    alt: bool,
    autowrap: bool,
    cursor_visible: bool,
    app_cursor: bool,
    bracketed_paste: bool,
    viewport: usize,
    selection: Option<Selection>,
    title: Option<String>,
    title_dirty: Option<String>,
    last_char: Option<char>,
    parser: vte::Parser,
}

impl Terminal {
    pub fn new(cols: usize, rows: usize) -> Self {
        let cols = cols.max(2);
        let rows = rows.max(2);
        let cells = vec![Cell::blank(); cols * rows];
        Self {
            cols,
            rows,
            other: cells.clone(),
            cells,
            scrollback: VecDeque::new(),
            cursor_x: 0,
            cursor_y: 0,
            primary_cursor: (0, 0),
            saved_cursor: (0, 0),
            pen: Pen::default(),
            scroll_top: 0,
            scroll_bot: rows,
            alt: false,
            autowrap: true,
            cursor_visible: true,
            app_cursor: false,
            bracketed_paste: false,
            viewport: 0,
            selection: None,
            title: None,
            title_dirty: None,
            last_char: None,
            parser: vte::Parser::new(),
        }
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn is_alt(&self) -> bool {
        self.alt
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Option<String> {
        let mut parser = std::mem::replace(&mut self.parser, vte::Parser::new());
        parser.advance(self, bytes);
        self.parser = parser;
        self.title_dirty.take()
    }

    pub fn resize(&mut self, cols: usize, rows: usize) -> bool {
        let cols = cols.max(2);
        let rows = rows.max(2);
        if cols == self.cols && rows == self.rows {
            return false;
        }
        self.cells = resize_buffer(&self.cells, self.cols, self.rows, cols, rows);
        self.other = vec![Cell::blank(); cols * rows];
        for line in &mut self.scrollback {
            line.resize(cols, Cell::blank());
        }
        self.cols = cols;
        self.rows = rows;
        self.scroll_top = 0;
        self.scroll_bot = rows;
        self.cursor_x = self.cursor_x.min(cols.saturating_sub(1));
        self.cursor_y = self.cursor_y.min(rows.saturating_sub(1));
        self.selection = None;
        if self.viewport > self.scrollback.len() {
            self.viewport = self.scrollback.len();
        }
        true
    }

    pub fn scroll_by(&mut self, delta: i32) {
        if self.alt {
            return;
        }
        let max = self.scrollback.len() as i32;
        let next = (self.viewport as i32 + delta).clamp(0, max);
        self.viewport = next as usize;
    }

    pub fn scroll_to_bottom(&mut self) {
        self.viewport = 0;
    }

    pub fn cell_at_view(&self, view_row: usize, col: usize) -> Cell {
        if view_row >= self.rows || col >= self.cols {
            return Cell::blank();
        }
        self.cell_abs(self.visible_start() + view_row, col)
    }

    pub fn cursor_view_pos(&self) -> Option<(usize, usize)> {
        if !self.cursor_visible {
            return None;
        }
        let abs = if self.alt {
            self.cursor_y
        } else {
            self.scrollback.len() + self.cursor_y
        };
        let start = self.visible_start();
        if abs < start || abs >= start + self.rows {
            return None;
        }
        let x = self.cursor_x.min(self.cols.saturating_sub(1));
        Some((x, abs - start))
    }

    pub fn is_selected(&self, view_row: usize, col: usize) -> bool {
        let Some(selection) = self.selection else {
            return false;
        };
        let pos = Pos {
            row: self.visible_start() + view_row,
            col,
        };
        let (start, end) = ordered(selection.anchor, selection.focus);
        if pos.row < start.row || pos.row > end.row {
            return false;
        }
        if start.row == end.row {
            return pos.col >= start.col && pos.col <= end.col;
        }
        if pos.row == start.row {
            return pos.col >= start.col;
        }
        if pos.row == end.row {
            return pos.col <= end.col;
        }
        true
    }

    pub fn start_select(&mut self, view_row: usize, col: usize) {
        let pos = self.clamp_view(view_row, col);
        self.selection = Some(Selection {
            anchor: pos,
            focus: pos,
        });
    }

    pub fn extend_select(&mut self, view_row: usize, col: usize) {
        let pos = self.clamp_view(view_row, col);
        if let Some(selection) = &mut self.selection {
            selection.focus = pos;
        } else {
            self.selection = Some(Selection {
                anchor: pos,
                focus: pos,
            });
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection_text()
    }

    pub fn finish_select(&mut self, dragged: bool) -> Option<String> {
        if !dragged {
            self.selection = None;
            return None;
        }
        let text = self.selection_text()?;
        if text.is_empty() {
            self.selection = None;
            None
        } else {
            Some(text)
        }
    }

    pub fn paste_bytes(&self, text: &str) -> Vec<u8> {
        let body = text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\n', "\r");
        if self.bracketed_paste {
            let mut out = b"\x1b[200~".to_vec();
            out.extend(body.as_bytes());
            out.extend_from_slice(b"\x1b[201~");
            out
        } else {
            body.into_bytes()
        }
    }

    pub fn encode_key(&self, input: &KeyInput) -> InputEffect {
        if input.logo && !input.ctrl && !input.alt {
            return InputEffect::Ignore;
        }
        match input.key {
            KeyCode::Enter => InputEffect::Bytes(vec![b'\r']),
            KeyCode::Backspace => {
                if input.ctrl {
                    InputEffect::Bytes(vec![0x17])
                } else {
                    InputEffect::Bytes(vec![0x7f])
                }
            }
            KeyCode::Tab => {
                if input.shift {
                    InputEffect::Bytes(b"\x1b[Z".to_vec())
                } else {
                    InputEffect::Bytes(vec![b'\t'])
                }
            }
            KeyCode::Escape => InputEffect::Bytes(vec![0x1b]),
            KeyCode::Up => InputEffect::Bytes(self.arrow(b'A', input)),
            KeyCode::Down => InputEffect::Bytes(self.arrow(b'B', input)),
            KeyCode::Right => InputEffect::Bytes(self.arrow(b'C', input)),
            KeyCode::Left => InputEffect::Bytes(self.arrow(b'D', input)),
            KeyCode::Home => InputEffect::Bytes(b"\x1b[H".to_vec()),
            KeyCode::End => InputEffect::Bytes(b"\x1b[F".to_vec()),
            KeyCode::PageUp if input.shift && !input.ctrl => {
                InputEffect::Scroll(self.rows.saturating_sub(1) as i32)
            }
            KeyCode::PageDown if input.shift && !input.ctrl => {
                InputEffect::Scroll(-(self.rows.saturating_sub(1) as i32))
            }
            KeyCode::PageUp => InputEffect::Bytes(b"\x1b[5~".to_vec()),
            KeyCode::PageDown => InputEffect::Bytes(b"\x1b[6~".to_vec()),
            KeyCode::Insert => InputEffect::Bytes(b"\x1b[2~".to_vec()),
            KeyCode::Delete => InputEffect::Bytes(b"\x1b[3~".to_vec()),
            KeyCode::F(n) => InputEffect::Bytes(function_key(n)),
            KeyCode::Char(ch) => self.encode_char(ch, input),
        }
    }

    fn encode_char(&self, ch: char, input: &KeyInput) -> InputEffect {
        if input.ctrl && input.shift && matches!(ch, 'c' | 'C') {
            return InputEffect::Copy;
        }
        if input.ctrl && input.shift && matches!(ch, 'v' | 'V') {
            return InputEffect::Paste;
        }
        if input.ctrl && !input.alt {
            let lower = ch.to_ascii_lowercase();
            if lower.is_ascii_lowercase() {
                return InputEffect::Bytes(vec![lower as u8 - b'a' + 1]);
            }
            if ch == ' ' {
                return InputEffect::Bytes(vec![0]);
            }
        }
        if input.alt && !input.ctrl {
            let mut out = vec![0x1b];
            let mut buf = [0; 4];
            out.extend(ch.encode_utf8(&mut buf).as_bytes());
            return InputEffect::Bytes(out);
        }
        if let Some(text) = input.text.as_deref() {
            if !input.ctrl && !input.alt && !text.is_empty() {
                return InputEffect::Bytes(text.as_bytes().to_vec());
            }
        }
        let mut buf = [0; 4];
        InputEffect::Bytes(ch.encode_utf8(&mut buf).as_bytes().to_vec())
    }

    fn arrow(&self, letter: u8, input: &KeyInput) -> Vec<u8> {
        let mut modifier = 1;
        if input.shift {
            modifier += 1;
        }
        if input.alt {
            modifier += 2;
        }
        if input.ctrl {
            modifier += 4;
        }
        if modifier == 1 {
            if self.app_cursor {
                vec![0x1b, b'O', letter]
            } else {
                vec![0x1b, b'[', letter]
            }
        } else {
            format!("\x1b[1;{modifier}{}", letter as char).into_bytes()
        }
    }

    fn selection_text(&self) -> Option<String> {
        let selection = self.selection?;
        let (start, end) = ordered(selection.anchor, selection.focus);
        if start == end {
            return None;
        }
        let mut lines = Vec::new();
        for row in start.row..=end.row {
            let from = if row == start.row { start.col } else { 0 };
            let to = if row == end.row { end.col } else { self.cols - 1 };
            let mut text = String::new();
            let mut col = from;
            while col <= to && col < self.cols {
                let cell = self.cell_abs(row, col);
                if cell.width == 0 {
                    col += 1;
                    continue;
                }
                text.push(cell.ch);
                col += cell.width.max(1) as usize;
            }
            while text.ends_with(' ') {
                text.pop();
            }
            lines.push(text);
        }
        Some(lines.join("\n"))
    }

    fn clamp_view(&self, view_row: usize, col: usize) -> Pos {
        Pos {
            row: self.visible_start() + view_row.min(self.rows.saturating_sub(1)),
            col: col.min(self.cols.saturating_sub(1)),
        }
    }

    fn visible_start(&self) -> usize {
        if self.alt {
            0
        } else {
            let total = self.scrollback.len() + self.rows;
            total.saturating_sub(self.rows + self.viewport)
        }
    }

    fn cell_abs(&self, abs_row: usize, col: usize) -> Cell {
        if self.alt {
            return self
                .cells
                .get(abs_row * self.cols + col)
                .copied()
                .unwrap_or_else(Cell::blank);
        }
        if abs_row < self.scrollback.len() {
            self.scrollback[abs_row]
                .get(col)
                .copied()
                .unwrap_or_else(Cell::blank)
        } else {
            let y = abs_row - self.scrollback.len();
            self.cells
                .get(y * self.cols + col)
                .copied()
                .unwrap_or_else(Cell::blank)
        }
    }

    fn print_char(&mut self, ch: char) {
        let width = UnicodeWidthChar::width(ch).unwrap_or(1).clamp(1, 2);
        if self.cursor_x >= self.cols && self.autowrap {
            self.cursor_x = 0;
            self.linefeed();
        }
        if self.cursor_x + width > self.cols && self.autowrap {
            self.cursor_x = 0;
            self.linefeed();
        }
        if self.cursor_x >= self.cols {
            return;
        }
        self.put(self.cursor_x, self.cursor_y, ch, width as u8);
        self.cursor_x += width;
        self.last_char = Some(ch);
    }

    fn put(&mut self, x: usize, y: usize, ch: char, width: u8) {
        if x >= self.cols || y >= self.rows {
            return;
        }
        let cell = Cell {
            ch,
            width,
            fg: self.pen.effective_fg(),
            bg: self.pen.bg,
            bold: self.pen.bold,
            italic: self.pen.italic,
            underline: self.pen.underline,
            inverse: self.pen.inverse,
        };
        self.cells[y * self.cols + x] = cell;
        if width == 2 && x + 1 < self.cols {
            let mut spacer = self.blank_cell();
            spacer.width = 0;
            spacer.ch = ' ';
            self.cells[y * self.cols + x + 1] = spacer;
        }
    }

    fn blank_cell(&self) -> Cell {
        let mut cell = Cell::blank();
        cell.bg = self.pen.bg;
        cell
    }

    fn linefeed(&mut self) {
        if self.cursor_y + 1 == self.scroll_bot {
            self.scroll_up(1);
        } else if self.cursor_y + 1 < self.rows {
            self.cursor_y += 1;
        }
    }

    fn reverse_index(&mut self) {
        if self.cursor_y == self.scroll_top {
            self.scroll_down(1);
        } else if self.cursor_y > 0 {
            self.cursor_y -= 1;
        }
    }

    fn scroll_up(&mut self, n: usize) {
        let top = self.scroll_top;
        let bot = self.scroll_bot;
        if bot <= top {
            return;
        }
        for _ in 0..n {
            if top == 0 && bot == self.rows && !self.alt {
                let line = self.cells[..self.cols].to_vec();
                self.push_history(line);
            }
            for y in top..bot.saturating_sub(1) {
                self.copy_row(y + 1, y);
            }
            self.clear_row(bot - 1);
        }
    }

    fn scroll_down(&mut self, n: usize) {
        let top = self.scroll_top;
        let bot = self.scroll_bot;
        if bot <= top {
            return;
        }
        for _ in 0..n {
            for y in (top + 1..bot).rev() {
                self.copy_row(y - 1, y);
            }
            self.clear_row(top);
        }
    }

    fn push_history(&mut self, line: Vec<Cell>) {
        if self.viewport > 0 {
            self.viewport += 1;
        }
        self.scrollback.push_back(line);
        while self.scrollback.len() > MAX_SCROLLBACK {
            self.scrollback.pop_front();
        }
        if self.viewport > self.scrollback.len() {
            self.viewport = self.scrollback.len();
        }
    }

    fn copy_row(&mut self, from: usize, to: usize) {
        if from == to || from >= self.rows || to >= self.rows {
            return;
        }
        for x in 0..self.cols {
            let cell = self.cells[from * self.cols + x];
            self.cells[to * self.cols + x] = cell;
        }
    }

    fn clear_row(&mut self, y: usize) {
        if y >= self.rows {
            return;
        }
        for x in 0..self.cols {
            self.cells[y * self.cols + x] = self.blank_cell();
        }
    }

    fn clear_all(&mut self) {
        let blank = self.blank_cell();
        self.cells.fill(blank);
    }

    fn execute_byte(&mut self, byte: u8) {
        match byte {
            0x08 => {
                self.cursor_x = self.cursor_x.saturating_sub(1);
            }
            0x09 => {
                let next = (self.cursor_x / 8 + 1) * 8;
                self.cursor_x = next.min(self.cols.saturating_sub(1));
            }
            0x0A | 0x0B | 0x0C => self.linefeed(),
            0x0D => self.cursor_x = 0,
            _ => {}
        }
    }

    fn set_mode(&mut self, params: &Params, intermediates: &[u8], enable: bool) {
        let private = intermediates.contains(&b'?');
        if !private {
            return;
        }
        for param in params.iter() {
            let code = param.first().copied().unwrap_or(0);
            match code {
                1 => self.app_cursor = enable,
                7 => self.autowrap = enable,
                25 => self.cursor_visible = enable,
                47 | 1047 | 1049 => self.set_alt(enable),
                2004 => self.bracketed_paste = enable,
                _ => {}
            }
        }
    }

    fn set_alt(&mut self, enable: bool) {
        if enable == self.alt {
            return;
        }
        if enable {
            self.primary_cursor = (self.cursor_x, self.cursor_y);
            std::mem::swap(&mut self.cells, &mut self.other);
            self.alt = true;
            self.clear_all();
            self.cursor_x = 0;
            self.cursor_y = 0;
            self.viewport = 0;
            self.selection = None;
        } else {
            std::mem::swap(&mut self.cells, &mut self.other);
            self.alt = false;
            self.cursor_x = self.primary_cursor.0.min(self.cols.saturating_sub(1));
            self.cursor_y = self.primary_cursor.1.min(self.rows.saturating_sub(1));
            self.selection = None;
        }
    }

    fn apply_sgr(&mut self, params: &Params) {
        let mut codes: Vec<u16> = params.iter().flat_map(|param| param.iter().copied()).collect();
        if codes.is_empty() {
            codes.push(0);
        }
        let mut index = 0;
        while index < codes.len() {
            match codes[index] {
                0 => self.pen = Pen::default(),
                1 => self.pen.bold = true,
                2 => self.pen.dim = true,
                3 => self.pen.italic = true,
                4 => self.pen.underline = true,
                7 => self.pen.inverse = true,
                22 => {
                    self.pen.bold = false;
                    self.pen.dim = false;
                }
                23 => self.pen.italic = false,
                24 => self.pen.underline = false,
                27 => self.pen.inverse = false,
                39 => {
                    self.pen.fg = DEFAULT_FG;
                    self.pen.fg_index = None;
                }
                49 => self.pen.bg = DEFAULT_BG,
                30..=37 => self.set_fg_index((codes[index] - 30) as u8),
                40..=47 => self.set_bg_index((codes[index] - 40) as u8),
                90..=97 => self.set_fg_index((codes[index] - 90) as u8 + 8),
                100..=107 => self.set_bg_index((codes[index] - 100) as u8 + 8),
                38 | 48 => {
                    let foreground = codes[index] == 38;
                    if codes.get(index + 1) == Some(&5) {
                        if let Some(color) = codes.get(index + 2) {
                            self.set_extended(foreground, color_256(*color as u8));
                            index += 2;
                        }
                    } else if codes.get(index + 1) == Some(&2) {
                        if let (Some(r), Some(g), Some(b)) = (
                            codes.get(index + 2),
                            codes.get(index + 3),
                            codes.get(index + 4),
                        ) {
                            self.set_extended(foreground, Rgb::new(*r as u8, *g as u8, *b as u8));
                            index += 4;
                        }
                    }
                }
                _ => {}
            }
            index += 1;
        }
    }

    fn set_fg_index(&mut self, index: u8) {
        self.pen.fg = ANSI[index as usize];
        self.pen.fg_index = Some(index);
    }

    fn set_bg_index(&mut self, index: u8) {
        self.pen.bg = ANSI[index as usize];
    }

    fn set_extended(&mut self, foreground: bool, color: Rgb) {
        if foreground {
            self.pen.fg = color;
            self.pen.fg_index = None;
        } else {
            self.pen.bg = color;
        }
    }

    fn param(params: &Params, index: usize, default: usize) -> usize {
        match params.iter().nth(index).and_then(|param| param.first().copied()) {
            Some(0) | None => default,
            Some(value) => value as usize,
        }
    }

    fn erase_display(&mut self, mode: usize) {
        match mode {
            0 => {
                self.erase_line(0);
                for y in self.cursor_y + 1..self.rows {
                    self.clear_row(y);
                }
            }
            1 => {
                for y in 0..self.cursor_y {
                    self.clear_row(y);
                }
                self.erase_line(1);
            }
            3 => {
                self.scrollback.clear();
                self.viewport = 0;
                self.clear_all();
            }
            _ => self.clear_all(),
        }
    }

    fn erase_line(&mut self, mode: usize) {
        let y = self.cursor_y;
        let blank = self.blank_cell();
        match mode {
            1 => {
                for x in 0..=self.cursor_x.min(self.cols - 1) {
                    self.cells[y * self.cols + x] = blank;
                }
            }
            2 => self.clear_row(y),
            _ => {
                for x in self.cursor_x.min(self.cols - 1)..self.cols {
                    self.cells[y * self.cols + x] = blank;
                }
            }
        }
    }

    fn insert_lines(&mut self, n: usize) {
        if self.cursor_y < self.scroll_top || self.cursor_y >= self.scroll_bot {
            return;
        }
        let n = n.max(1);
        let start = self.cursor_y;
        let bot = self.scroll_bot;
        if start + n >= bot {
            for y in start..bot {
                self.clear_row(y);
            }
            return;
        }
        for y in (start..bot - n).rev() {
            self.copy_row(y, y + n);
        }
        for y in start..start + n {
            self.clear_row(y);
        }
    }

    fn delete_lines(&mut self, n: usize) {
        if self.cursor_y < self.scroll_top || self.cursor_y >= self.scroll_bot {
            return;
        }
        let n = n.max(1).min(self.scroll_bot - self.cursor_y);
        let start = self.cursor_y;
        let bot = self.scroll_bot;
        for y in start..bot - n {
            self.copy_row(y + n, y);
        }
        for y in bot - n..bot {
            self.clear_row(y);
        }
    }

    fn insert_cells(&mut self, n: usize) {
        let y = self.cursor_y;
        let x = self.cursor_x.min(self.cols - 1);
        let n = n.max(1);
        let blank = self.blank_cell();
        for col in (x..self.cols).rev() {
            let src = col as isize - n as isize;
            self.cells[y * self.cols + col] = if src >= x as isize {
                self.cells[y * self.cols + src as usize]
            } else {
                blank
            };
        }
    }

    fn delete_cells(&mut self, n: usize) {
        let y = self.cursor_y;
        let x = self.cursor_x.min(self.cols - 1);
        let n = n.max(1);
        let blank = self.blank_cell();
        for col in x..self.cols {
            let src = col + n;
            self.cells[y * self.cols + col] = if src < self.cols {
                self.cells[y * self.cols + src]
            } else {
                blank
            };
        }
    }

    fn erase_chars(&mut self, n: usize) {
        let y = self.cursor_y;
        let blank = self.blank_cell();
        let end = (self.cursor_x + n.max(1)).min(self.cols);
        for x in self.cursor_x..end {
            self.cells[y * self.cols + x] = blank;
        }
    }

    fn reset(&mut self) {
        let cols = self.cols;
        let rows = self.rows;
        *self = Self::new(cols, rows);
    }
}

impl Perform for Terminal {
    fn print(&mut self, ch: char) {
        self.print_char(ch);
    }

    fn execute(&mut self, byte: u8) {
        self.execute_byte(byte);
    }

    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _action: char) {}

    fn put(&mut self, _byte: u8) {}

    fn unhook(&mut self) {}

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if params.len() >= 2 && (params[0] == b"0" || params[0] == b"2") {
            if let Ok(title) = std::str::from_utf8(params[1]) {
                let title = title.trim().to_string();
                if !title.is_empty() {
                    self.title = Some(title.clone());
                    self.title_dirty = Some(title);
                }
            }
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        if ignore {
            return;
        }
        match action {
            'A' => {
                let n = Self::param(params, 0, 1);
                self.cursor_y = self.cursor_y.saturating_sub(n);
            }
            'B' => {
                let n = Self::param(params, 0, 1);
                self.cursor_y = (self.cursor_y + n).min(self.rows - 1);
            }
            'C' => {
                let n = Self::param(params, 0, 1);
                self.cursor_x = (self.cursor_x + n).min(self.cols - 1);
            }
            'D' => {
                let n = Self::param(params, 0, 1);
                self.cursor_x = self.cursor_x.saturating_sub(n);
            }
            'E' => {
                let n = Self::param(params, 0, 1);
                self.cursor_y = (self.cursor_y + n).min(self.rows - 1);
                self.cursor_x = 0;
            }
            'F' => {
                let n = Self::param(params, 0, 1);
                self.cursor_y = self.cursor_y.saturating_sub(n);
                self.cursor_x = 0;
            }
            'G' => {
                let col = Self::param(params, 0, 1);
                self.cursor_x = col.saturating_sub(1).min(self.cols - 1);
            }
            'H' | 'f' => {
                let row = Self::param(params, 0, 1);
                let col = Self::param(params, 1, 1);
                self.cursor_y = row.saturating_sub(1).min(self.rows - 1);
                self.cursor_x = col.saturating_sub(1).min(self.cols - 1);
            }
            'J' => self.erase_display(Self::param(params, 0, 0)),
            'K' => self.erase_line(Self::param(params, 0, 0)),
            'L' => self.insert_lines(Self::param(params, 0, 1)),
            'M' => self.delete_lines(Self::param(params, 0, 1)),
            'P' => self.delete_cells(Self::param(params, 0, 1)),
            'S' => self.scroll_up(Self::param(params, 0, 1)),
            'T' => {
                if params.iter().count() <= 1 {
                    self.scroll_down(Self::param(params, 0, 1));
                }
            }
            'X' => self.erase_chars(Self::param(params, 0, 1)),
            '@' => self.insert_cells(Self::param(params, 0, 1)),
            'd' => {
                let row = Self::param(params, 0, 1);
                self.cursor_y = row.saturating_sub(1).min(self.rows - 1);
            }
            'm' => self.apply_sgr(params),
            'r' => {
                let top = Self::param(params, 0, 1);
                let bot = Self::param(params, 1, self.rows);
                let top = top.saturating_sub(1).min(self.rows.saturating_sub(1));
                let bot = bot.min(self.rows).max(top + 1);
                self.scroll_top = top;
                self.scroll_bot = bot;
                self.cursor_x = 0;
                self.cursor_y = 0;
            }
            'h' => self.set_mode(params, intermediates, true),
            'l' => self.set_mode(params, intermediates, false),
            's' => self.saved_cursor = (self.cursor_x, self.cursor_y),
            'u' => {
                self.cursor_x = self.saved_cursor.0.min(self.cols - 1);
                self.cursor_y = self.saved_cursor.1.min(self.rows - 1);
            }
            'b' => {
                if let Some(ch) = self.last_char {
                    let n = Self::param(params, 0, 1);
                    for _ in 0..n {
                        self.print_char(ch);
                    }
                }
            }
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], ignore: bool, byte: u8) {
        if ignore {
            return;
        }
        match byte {
            b'7' => self.saved_cursor = (self.cursor_x, self.cursor_y),
            b'8' => {
                self.cursor_x = self.saved_cursor.0.min(self.cols - 1);
                self.cursor_y = self.saved_cursor.1.min(self.rows - 1);
            }
            b'D' => self.linefeed(),
            b'M' => self.reverse_index(),
            b'c' => self.reset(),
            _ => {}
        }
    }
}

fn ordered(a: Pos, b: Pos) -> (Pos, Pos) {
    if (a.row, a.col) <= (b.row, b.col) {
        (a, b)
    } else {
        (b, a)
    }
}

fn resize_buffer(src: &[Cell], old_cols: usize, old_rows: usize, cols: usize, rows: usize) -> Vec<Cell> {
    let mut next = vec![Cell::blank(); cols * rows];
    let copy_rows = old_rows.min(rows);
    let copy_cols = old_cols.min(cols);
    for y in 0..copy_rows {
        for x in 0..copy_cols {
            next[y * cols + x] = src[y * old_cols + x];
        }
    }
    next
}

fn color_256(index: u8) -> Rgb {
    match index {
        0..=15 => ANSI[index as usize],
        16..=231 => {
            let offset = index - 16;
            let r = offset / 36;
            let g = (offset % 36) / 6;
            let b = offset % 6;
            let level = |component| {
                if component == 0 {
                    0
                } else {
                    55 + component * 40
                }
            };
            Rgb::new(level(r), level(g), level(b))
        }
        232..=255 => {
            let value = 8 + (index - 232) * 10;
            Rgb::new(value, value, value)
        }
    }
}

fn function_key(n: u8) -> Vec<u8> {
    match n {
        1 => b"\x1bOP".to_vec(),
        2 => b"\x1bOQ".to_vec(),
        3 => b"\x1bOR".to_vec(),
        4 => b"\x1bOS".to_vec(),
        5 => b"\x1b[15~".to_vec(),
        6 => b"\x1b[17~".to_vec(),
        7 => b"\x1b[18~".to_vec(),
        8 => b"\x1b[19~".to_vec(),
        9 => b"\x1b[20~".to_vec(),
        10 => b"\x1b[21~".to_vec(),
        11 => b"\x1b[23~".to_vec(),
        12 => b"\x1b[24~".to_vec(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: KeyCode) -> KeyInput {
        KeyInput {
            key,
            text: None,
            ctrl: false,
            alt: false,
            shift: false,
            logo: false,
        }
    }

    #[test]
    fn prints_and_colors_text() {
        let mut term = Terminal::new(20, 6);
        term.feed(b"\x1b[31mZ");
        let cell = term.cell_at_view(0, 0);
        assert_eq!(cell.ch, 'Z');
        assert_eq!(cell.fg, ANSI[1]);
    }

    #[test]
    fn cups_and_scrolls() {
        let mut term = Terminal::new(10, 3);
        term.feed(b"\x1b[2;4H*");
        assert_eq!(term.cell_at_view(1, 3).ch, '*');
        term.feed(b"\x1b[Ha\r\nb\r\nc\r\nd");
        assert!(term.scrollback.len() >= 1);
        assert_eq!(term.cell_at_view(2, 0).ch, 'd');
        term.scroll_by(10);
        assert!(term.viewport > 0);
        term.scroll_to_bottom();
        assert_eq!(term.viewport, 0);
        assert_eq!(term.cell_at_view(2, 0).ch, 'd');
    }

    #[test]
    fn encodes_control_keys() {
        let term = Terminal::new(80, 24);
        match term.encode_key(&key(KeyCode::Enter)) {
            InputEffect::Bytes(bytes) => assert_eq!(bytes, b"\r"),
            _ => panic!("enter"),
        }
        let mut ctrl_c = key(KeyCode::Char('c'));
        ctrl_c.ctrl = true;
        match term.encode_key(&ctrl_c) {
            InputEffect::Bytes(bytes) => assert_eq!(bytes, &[0x03]),
            _ => panic!("ctrl-c"),
        }
        match term.encode_key(&key(KeyCode::Up)) {
            InputEffect::Bytes(bytes) => assert_eq!(bytes, b"\x1b[A"),
            _ => panic!("up"),
        }
    }
}
