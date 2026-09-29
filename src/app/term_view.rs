use iced::alignment::{Horizontal, Vertical};
use iced::widget::canvas::{self, Cache, Canvas};
use iced::{Color, Element, Event, Fill, Font, Pixels, Point, Rectangle, Size};

use crate::term::{self, KeyCode, KeyInput, Rgb, Terminal, DEFAULT_BG};

use super::*;

pub(crate) const FONT_SIZE: f32 = 15.0;
pub(crate) const CELL_H: f32 = 20.0;

pub(crate) fn term_canvas(tab: &Tab, accept_keys: bool) -> Element<'_, Message> {
    Canvas::new(TermView {
        tab_id: tab.id,
        term: &tab.term,
        cache: &tab.cache,
        accept_keys,
        selecting: tab.selecting,
    })
    .width(Fill)
    .height(Fill)
    .into()
}

pub(crate) struct TermView<'a> {
    tab_id: u64,
    term: &'a Terminal,
    cache: &'a Cache,
    accept_keys: bool,
    selecting: bool,
}

impl canvas::Program<Message> for TermView<'_> {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            paint_term(frame, self.term, bounds.size());
        });
        vec![geometry]
    }

    fn update(
        &self,
        _state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: iced::mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if let Some((cols, rows)) = grid_for(bounds) {
            if cols != self.term.cols() || rows != self.term.rows() {
                return Some(canvas::Action::publish(Message::PaneResized(cols, rows)));
            }
        }
        match event {
            Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key,
                modifiers,
                text,
                ..
            }) if self.accept_keys => {
                if modifiers.control()
                    && !modifiers.alt()
                    && matches!(key, iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab))
                {
                    let delta = if modifiers.shift() { -1 } else { 1 };
                    return Some(canvas::Action::publish(Message::CycleTab(delta)));
                }
                let input = KeyInput {
                    key: map_key(key, text.as_deref())?,
                    text: text.as_ref().map(|value| value.to_string()),
                    ctrl: modifiers.control(),
                    alt: modifiers.alt(),
                    shift: modifiers.shift(),
                    logo: modifiers.logo(),
                };
                Some(canvas::Action::publish(Message::TermKey {
                    tab: self.tab_id,
                    input,
                }))
            }
            Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)) => {
                let (col, row) = hit(self.term, bounds, cursor)?;
                Some(canvas::Action::publish(Message::Pointer {
                    tab: self.tab_id,
                    action: Pointer::Down(col, row),
                }))
            }
            Event::Mouse(iced::mouse::Event::CursorMoved { .. }) if self.selecting => {
                let (col, row) = hit(self.term, bounds, cursor)?;
                Some(canvas::Action::publish(Message::Pointer {
                    tab: self.tab_id,
                    action: Pointer::Drag(col, row),
                }))
            }
            Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left))
                if self.selecting =>
            {
                Some(canvas::Action::publish(Message::Pointer {
                    tab: self.tab_id,
                    action: Pointer::Up,
                }))
            }
            Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Right)) => {
                Some(canvas::Action::publish(Message::Pointer {
                    tab: self.tab_id,
                    action: Pointer::Right,
                }))
            }
            Event::Mouse(iced::mouse::Event::WheelScrolled { delta }) => {
                let y = match delta {
                    iced::mouse::ScrollDelta::Lines { y, .. } => *y,
                    iced::mouse::ScrollDelta::Pixels { y, .. } => *y / CELL_H,
                };
                if y.abs() < f32::EPSILON {
                    None
                } else {
                    Some(canvas::Action::publish(Message::Wheel {
                        tab: self.tab_id,
                        delta: y,
                    }))
                }
            }
            _ => None,
        }
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        _bounds: Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> iced::mouse::Interaction {
        iced::mouse::Interaction::Text
    }
}

pub(crate) fn paint_term(frame: &mut canvas::Frame, term: &Terminal, size: Size) {
    frame.fill_rectangle(Point::new(0.0, 0.0), size, rgb(DEFAULT_BG));
    let cursor = term.cursor_view_pos();
    let mut runs = Vec::new();
    for row in 0..term.rows() {
        let mut col = 0;
        while col < term.cols() {
            let cell = term.cell_at_view(row, col);
            if cell.width == 0 {
                col += 1;
                continue;
            }
            let advance = usize::from(cell.width.max(1));
            let style = cell_style(&cell, term.is_selected(row, col));
            if cell.ch.is_ascii() {
                let start = col;
                let mut content = String::new();
                content.push(cell.ch);
                col += advance;
                while col < term.cols() {
                    let next = term.cell_at_view(row, col);
                    if next.width == 0 || !next.ch.is_ascii() {
                        break;
                    }
                    let next_advance = usize::from(next.width.max(1));
                    if cell_style(&next, term.is_selected(row, col)) != style {
                        break;
                    }
                    content.push(next.ch);
                    col += next_advance;
                }
                runs.push((content, row, start, col, style, false));
            } else {
                let start = col;
                col += advance;
                runs.push((cell.ch.to_string(), row, start, col, style, true));
            }
        }
    }
    for (_, row, start, end, style, _) in &runs {
        paint_run_bg(frame, *row, *start, *end, *style);
    }
    for (content, row, start, end, style, fallback) in &runs {
        paint_run_text(frame, content, *row, *start, *end, *style, *fallback);
    }
    if let Some((col, row)) = cursor {
        let x = col as f32 * cell_w();
        let y = row as f32 * CELL_H;
        frame.fill_rectangle(
            Point::new(x, y + 3.0),
            Size::new(2.0, CELL_H - 6.0),
            Color::from_rgb8(236, 239, 244),
        );
    }
}

pub(crate) fn paint_run_bg(
    frame: &mut canvas::Frame,
    row: usize,
    start: usize,
    end: usize,
    style: (Rgb, Rgb, bool, bool, bool),
) {
    let (_, bg, _, _, _) = style;
    let x = start as f32 * cell_w();
    let y = row as f32 * CELL_H;
    let width = (end - start) as f32 * cell_w();
    frame.fill_rectangle(Point::new(x, y), Size::new(width, CELL_H), rgb(bg));
}

pub(crate) fn paint_run_text(
    frame: &mut canvas::Frame,
    content: &str,
    row: usize,
    start: usize,
    end: usize,
    style: (Rgb, Rgb, bool, bool, bool),
    fallback: bool,
) {
    let (fg, _, bold, italic, underline) = style;
    let x = start as f32 * cell_w();
    let y = row as f32 * CELL_H;
    let width = (end - start) as f32 * cell_w();
    if content.chars().any(|ch| ch != ' ') {
        let size = if fallback { wide_size() } else { FONT_SIZE };
        let mut drawn = canvas::Text::from(content);
        drawn.position = Point::new(x, y + ((CELL_H - size) / 2.0).max(0.0));
        drawn.color = rgb(fg);
        drawn.size = Pixels(size);
        drawn.font = mono(bold, italic);
        drawn.shaping = if fallback {
            iced::widget::text::Shaping::Advanced
        } else {
            iced::widget::text::Shaping::Basic
        };
        drawn.align_x = Horizontal::Left.into();
        drawn.align_y = Vertical::Top;
        frame.fill_text(drawn);
    }
    if underline {
        frame.fill_rectangle(
            Point::new(x, y + CELL_H - 2.0),
            Size::new(width, 1.0),
            rgb(fg),
        );
    }
}

pub(crate) fn wide_size() -> f32 {
    static SIZE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *SIZE.get_or_init(|| {
        let sample = "中";
        let width = measure_text_advanced(sample);
        let target = cell_w() * 2.0;
        if width.is_finite() && width > 1.0 {
            (FONT_SIZE * target / width).clamp(8.0, CELL_H - 2.0)
        } else {
            FONT_SIZE
        }
    })
}

pub(crate) fn cell_style(cell: &term::Cell, selected: bool) -> (Rgb, Rgb, bool, bool, bool) {
    let (fg, mut bg) = if cell.inverse {
        (cell.bg, cell.fg)
    } else {
        (cell.fg, cell.bg)
    };
    if selected {
        bg = Rgb::new(38, 79, 120);
    }
    (fg, bg, cell.bold, cell.italic, cell.underline)
}

pub(crate) fn cell_w() -> f32 {
    static WIDTH: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *WIDTH.get_or_init(|| {
        let sample = "00000000000000000000000000000000";
        let width = measure_text(sample) / sample.len() as f32;
        if width.is_finite() && (4.0..=24.0).contains(&width) {
            width
        } else {
            9.0
        }
    })
}

pub(crate) fn measure_text(content: &str) -> f32 {
    measure_text_with(content, iced::widget::text::Shaping::Basic)
}

pub(crate) fn measure_text_advanced(content: &str) -> f32 {
    measure_text_with(content, iced::widget::text::Shaping::Advanced)
}

pub(crate) fn measure_text_with(content: &str, shaping: iced::widget::text::Shaping) -> f32 {
    use iced_graphics::core::text::Paragraph as _;

    let paragraph =
        iced_graphics::text::Paragraph::with_text(iced_graphics::core::text::Text {
            content,
            bounds: Size::new(f32::INFINITY, f32::INFINITY),
            size: Pixels(FONT_SIZE),
            line_height: iced::widget::text::LineHeight::default(),
            font: mono(false, false),
            align_x: Horizontal::Left.into(),
            align_y: Vertical::Top,
            shaping,
            wrapping: iced::widget::text::Wrapping::None,
        });
    paragraph.min_width()
}

pub(crate) fn mono(bold: bool, italic: bool) -> Font {
    Font {
        weight: if bold {
            iced::font::Weight::Bold
        } else {
            iced::font::Weight::Normal
        },
        style: if italic {
            iced::font::Style::Italic
        } else {
            iced::font::Style::Normal
        },
        ..Font::MONOSPACE
    }
}

pub(crate) fn grid_for(bounds: Rectangle) -> Option<(usize, usize)> {
    if bounds.width < cell_w() * 2.0 || bounds.height < CELL_H * 2.0 {
        return None;
    }
    Some((
        ((bounds.width / cell_w()).floor() as usize).max(2),
        ((bounds.height / CELL_H).floor() as usize).max(2),
    ))
}

pub(crate) fn hit(term: &Terminal, bounds: Rectangle, cursor: iced::mouse::Cursor) -> Option<(usize, usize)> {
    let point = cursor.position_in(bounds)?;
    let col = (point.x / cell_w()) as usize;
    let row = (point.y / CELL_H) as usize;
    if col < term.cols() && row < term.rows() {
        Some((col, row))
    } else {
        None
    }
}

pub(crate) fn map_key(key: &iced::keyboard::Key, text: Option<&str>) -> Option<KeyCode> {
    match key.as_ref() {
        iced::keyboard::Key::Named(named) => match named {
            iced::keyboard::key::Named::Enter => Some(KeyCode::Enter),
            iced::keyboard::key::Named::Backspace => Some(KeyCode::Backspace),
            iced::keyboard::key::Named::Tab => Some(KeyCode::Tab),
            iced::keyboard::key::Named::Escape => Some(KeyCode::Escape),
            iced::keyboard::key::Named::ArrowUp => Some(KeyCode::Up),
            iced::keyboard::key::Named::ArrowDown => Some(KeyCode::Down),
            iced::keyboard::key::Named::ArrowLeft => Some(KeyCode::Left),
            iced::keyboard::key::Named::ArrowRight => Some(KeyCode::Right),
            iced::keyboard::key::Named::Home => Some(KeyCode::Home),
            iced::keyboard::key::Named::End => Some(KeyCode::End),
            iced::keyboard::key::Named::PageUp => Some(KeyCode::PageUp),
            iced::keyboard::key::Named::PageDown => Some(KeyCode::PageDown),
            iced::keyboard::key::Named::Insert => Some(KeyCode::Insert),
            iced::keyboard::key::Named::Delete => Some(KeyCode::Delete),
            iced::keyboard::key::Named::Space => Some(KeyCode::Char(' ')),
            iced::keyboard::key::Named::F1 => Some(KeyCode::F(1)),
            iced::keyboard::key::Named::F2 => Some(KeyCode::F(2)),
            iced::keyboard::key::Named::F3 => Some(KeyCode::F(3)),
            iced::keyboard::key::Named::F4 => Some(KeyCode::F(4)),
            iced::keyboard::key::Named::F5 => Some(KeyCode::F(5)),
            iced::keyboard::key::Named::F6 => Some(KeyCode::F(6)),
            iced::keyboard::key::Named::F7 => Some(KeyCode::F(7)),
            iced::keyboard::key::Named::F8 => Some(KeyCode::F(8)),
            iced::keyboard::key::Named::F9 => Some(KeyCode::F(9)),
            iced::keyboard::key::Named::F10 => Some(KeyCode::F(10)),
            iced::keyboard::key::Named::F11 => Some(KeyCode::F(11)),
            iced::keyboard::key::Named::F12 => Some(KeyCode::F(12)),
            _ => None,
        },
        iced::keyboard::Key::Character(value) => value.chars().next().map(KeyCode::Char),
        iced::keyboard::Key::Unidentified => text.and_then(|value| value.chars().next()).map(KeyCode::Char),
    }
}

pub(crate) fn plain_key(key: KeyCode) -> KeyInput {
    KeyInput {
        key,
        text: None,
        ctrl: false,
        alt: false,
        shift: false,
        logo: false,
    }
}
