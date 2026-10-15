//! A shell in a pseudo-terminal, emulated by `alacritty_terminal`. The view
//! draws snapshots of the screen, taken on the UI thread when output arrives.
use alacritty_terminal::{
    event::{Event, EventListener, Notify, WindowSize},
    event_loop::{EventLoop, EventLoopSender, Msg, Notifier},
    grid::{Dimensions, Scroll},
    index::Point,
    sync::FairMutex,
    term::{Config, Term, TermMode, cell::Flags},
    tty,
    vte::ansi::{Color as Ansi, NamedColor},
};
use floem::{
    ext_event::{ExtSendTrigger, register_ext_trigger},
    peniko::Color,
    prelude::*,
    reactive::{Effect, ReadSignal, Scope},
};
use std::{
    borrow::Cow,
    collections::HashMap,
    ops::Range,
    path::Path,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// Output arriving faster than frames is drawn once.
#[derive(Clone)]
struct Listener {
    trigger: ExtSendTrigger,
    pending: Arc<AtomicBool>,
    exited: Arc<AtomicBool>,
    /// Answers the terminal owes the program, such as cursor position reports.
    replies: Arc<std::sync::Mutex<Vec<String>>>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        match event {
            Event::PtyWrite(text) => self.replies.lock().unwrap().push(text),
            Event::Exit | Event::ChildExit(_) => self.exited.store(true, Ordering::Relaxed),
            Event::Wakeup | Event::Title(_) | Event::ResetTitle | Event::Bell => {}
            _ => return,
        }
        if !self.pending.swap(true, Ordering::AcqRel) {
            register_ext_trigger(self.trigger);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Size {
    columns: usize,
    lines: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.lines
    }
    fn screen_lines(&self) -> usize {
        self.lines
    }
    fn columns(&self) -> usize {
        self.columns
    }
}

/// A run of cells drawn the same way.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    /// Bytes of the row's text.
    pub range: Range<usize>,
    /// Screen cells, which differ from characters for wide ones.
    pub columns: Range<usize>,
    pub color: Color,
    pub background: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Row {
    pub text: String,
    pub spans: Vec<Span>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Screen {
    pub rows: Vec<Row>,
    /// Line and column of the cursor, when it is shown.
    pub cursor: Option<(usize, usize)>,
    /// Lines scrolled back into history.
    pub scrolled: usize,
}

pub struct Terminal {
    term: Arc<FairMutex<Term<Listener>>>,
    sender: EventLoopSender,
    listener: Listener,
    size: std::cell::Cell<Size>,
    screen: RwSignal<Rc<Screen>>,
    exited: RwSignal<bool>,
    scope: Scope,
}

/// Where the shell comes from: the settings file, or the platform default.
fn shell() -> Option<tty::Shell> {
    let settings = crate::settings::get();
    let program = settings.terminal.command().map(str::to_owned).or_else(|| {
        (!cfg!(windows))
            .then(|| std::env::var("SHELL").ok())
            .flatten()
    })?;
    Some(tty::Shell::new(program, settings.terminal.args.clone()))
}

impl Terminal {
    pub fn start(
        scope: Scope,
        cwd: &Path,
        columns: usize,
        lines: usize,
    ) -> anyhow::Result<Rc<Self>> {
        let scope = scope.create_child();
        let size = Size {
            columns: columns.max(2),
            lines: lines.max(1),
        };
        let trigger = scope.enter(ExtSendTrigger::new);
        let listener = Listener {
            trigger,
            pending: Arc::default(),
            exited: Arc::default(),
            replies: Arc::default(),
        };
        let options = tty::Options {
            shell: shell(),
            // Shells show verbatim `\\?\` paths as they are; give them the usual form.
            working_directory: Some(std::path::PathBuf::from(pi_client::sessions::plain_cwd(
                cwd,
            ))),
            drain_on_exit: false,
            env: HashMap::from([("TERM".to_owned(), "xterm-256color".to_owned())]),
            #[cfg(windows)]
            escape_args: true,
        };
        let pty = tty::new(&options, window_size(size), 0)?;
        let config = Config {
            scrolling_history: 5000,
            ..Config::default()
        };
        let term = Arc::new(FairMutex::new(Term::new(config, &size, listener.clone())));
        let event_loop = EventLoop::new(term.clone(), listener.clone(), pty, false, false)?;
        let sender = event_loop.channel();
        event_loop.spawn();
        let terminal = Rc::new(Self {
            term,
            sender,
            listener,
            size: std::cell::Cell::new(size),
            screen: scope.enter(|| RwSignal::new(Rc::new(Screen::default()))),
            exited: scope.enter(|| RwSignal::new(false)),
            scope,
        });
        let weak = Rc::downgrade(&terminal);
        scope.enter(|| {
            Effect::new(move |_| {
                trigger.track();
                // A new theme recolours the screen.
                crate::ui::color::theme().track();
                if let Some(terminal) = weak.upgrade() {
                    terminal.update();
                }
            })
        });
        Ok(terminal)
    }

    /// Columns and lines.
    pub fn size(&self) -> (usize, usize) {
        let size = self.size.get();
        (size.columns, size.lines)
    }

    pub fn screen(&self) -> ReadSignal<Rc<Screen>> {
        self.screen.read_only()
    }

    pub fn exited(&self) -> ReadSignal<bool> {
        self.exited.read_only()
    }

    fn update(&self) {
        self.listener.pending.store(false, Ordering::Release);
        let replies: Vec<String> = std::mem::take(&mut *self.listener.replies.lock().unwrap());
        for reply in replies {
            self.write(reply.into_bytes());
        }
        if self.listener.exited.load(Ordering::Relaxed) {
            self.exited.set(true);
        }
        let screen = Rc::new(Effect::untrack(|| snapshot(&self.term.lock())));
        if self.screen.with_untracked(|s| **s != *screen) {
            self.screen.set(screen);
        }
    }

    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        // Typing returns to the bottom, as in any terminal.
        if self.screen.with_untracked(|s| s.scrolled > 0) {
            self.term.lock().scroll_display(Scroll::Bottom);
        }
        Notifier(self.sender.clone()).notify(bytes);
    }

    /// Pastes text, marked as a paste when the program asked for that.
    pub fn paste(&self, text: &str) {
        let bracketed = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        if bracketed {
            let clean = text.replace("\x1b[201~", "");
            self.write(format!("\x1b[200~{clean}\x1b[201~").into_bytes());
        } else {
            self.write(text.into_bytes());
        }
    }

    /// Whether arrow keys should send application sequences.
    pub fn app_cursor(&self) -> bool {
        self.term.lock().mode().contains(TermMode::APP_CURSOR)
    }

    pub fn scroll(&self, lines: i32) {
        self.term.lock().scroll_display(Scroll::Delta(lines));
        self.update();
    }

    pub fn resize(&self, columns: usize, lines: usize) {
        let size = Size {
            columns: columns.max(2),
            lines: lines.max(1),
        };
        if size == self.size.get() {
            return;
        }
        self.size.set(size);
        self.term.lock().resize(size);
        let _ = self.sender.send(Msg::Resize(window_size(size)));
        self.update();
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.sender.send(Msg::Shutdown);
        self.scope.dispose();
    }
}

fn window_size(size: Size) -> WindowSize {
    WindowSize {
        num_lines: size.lines as u16,
        num_cols: size.columns as u16,
        cell_width: 8,
        cell_height: 16,
    }
}

fn snapshot(term: &Term<Listener>) -> Screen {
    let content = term.renderable_content();
    let lines = term.screen_lines();
    let offset = content.display_offset as i32;
    let mut rows: Vec<Row> = vec![Row::default(); lines];
    for indexed in content.display_iter {
        let row = (indexed.point.line.0 + offset) as usize;
        let Some(row) = rows.get_mut(row) else {
            continue;
        };
        let cell = indexed.cell;
        if cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            continue;
        }
        let (mut fg, mut bg) = (
            color(cell.fg, content.colors),
            background(cell.bg, content.colors),
        );
        if cell.flags.contains(Flags::INVERSE) {
            let swapped = bg.unwrap_or_else(crate::ui::color::bg);
            bg = Some(fg);
            fg = swapped;
        }
        if cell.flags.contains(Flags::DIM) {
            fg = fg.with_alpha(0.6);
        }
        let ch = if cell.flags.contains(Flags::HIDDEN) {
            ' '
        } else {
            cell.c
        };
        let start = row.text.len();
        row.text.push(ch);
        let column = indexed.point.column.0;
        let width = if cell.flags.contains(Flags::WIDE_CHAR) {
            2
        } else {
            1
        };
        let span = Span {
            range: start..row.text.len(),
            columns: column..column + width,
            color: fg,
            background: bg,
            bold: cell.flags.contains(Flags::BOLD),
            italic: cell.flags.contains(Flags::ITALIC),
            underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
        };
        match row.spans.last_mut() {
            Some(last)
                if last.range.end == start
                    && (
                        last.color,
                        last.background,
                        last.bold,
                        last.italic,
                        last.underline,
                    ) == (
                        span.color,
                        span.background,
                        span.bold,
                        span.italic,
                        span.underline,
                    ) =>
            {
                last.range.end = span.range.end;
                last.columns.end = span.columns.end;
            }
            _ => row.spans.push(span),
        }
    }
    for row in &mut rows {
        // Trailing blanks go, unless they are coloured, as status bars are.
        let keep = row
            .spans
            .iter()
            .filter(|span| span.background.is_some())
            .map(|span| span.range.end)
            .max()
            .unwrap_or(0);
        let trimmed = row.text.trim_end().len().max(keep);
        row.text.truncate(trimmed);
        row.spans.retain_mut(|span| {
            if span.range.end > trimmed {
                let cut = span.range.end - trimmed;
                span.range.end = trimmed;
                span.columns.end = span.columns.end.saturating_sub(cut);
            }
            span.range.start < span.range.end
        });
    }
    let cursor = content.cursor.point;
    let shown = content.mode.contains(TermMode::SHOW_CURSOR) && offset == 0;
    Screen {
        rows,
        cursor: shown.then(|| (cursor_line(cursor, lines), cursor.column.0)),
        scrolled: content.display_offset,
    }
}

fn cursor_line(point: Point, lines: usize) -> usize {
    (point.line.0.max(0) as usize).min(lines.saturating_sub(1))
}

fn indexed(index: u8) -> Color {
    match index {
        0..=15 => crate::ui::color::ansi(index as usize),
        16..=231 => {
            let i = index - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Color::from_rgb8(level(i / 36), level((i / 6) % 6), level(i % 6))
        }
        _ => {
            let grey = 8 + (index - 232) * 10;
            Color::from_rgb8(grey, grey, grey)
        }
    }
}

fn named(name: NamedColor) -> Color {
    match name {
        NamedColor::Foreground | NamedColor::BrightForeground => crate::ui::color::text(),
        NamedColor::Background => crate::ui::color::bg(),
        NamedColor::DimForeground => crate::ui::color::text().with_alpha(0.6),
        NamedColor::Cursor => crate::ui::color::text(),
        other => {
            let index = other as usize;
            if index < 16 {
                crate::ui::color::ansi(index)
            } else if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize)
                .contains(&index)
            {
                crate::ui::color::ansi(index - NamedColor::DimBlack as usize).with_alpha(0.7)
            } else {
                crate::ui::color::text()
            }
        }
    }
}

fn color(color: Ansi, colors: &alacritty_terminal::term::color::Colors) -> Color {
    match color {
        Ansi::Spec(rgb) => Color::from_rgb8(rgb.r, rgb.g, rgb.b),
        Ansi::Indexed(index) => match colors[index as usize] {
            Some(rgb) => Color::from_rgb8(rgb.r, rgb.g, rgb.b),
            None => indexed(index),
        },
        Ansi::Named(name) => match colors[name] {
            Some(rgb) => Color::from_rgb8(rgb.r, rgb.g, rgb.b),
            None => named(name),
        },
    }
}

/// `None` for the default background, so the panel's own shows through.
fn background(bg: Ansi, colors: &alacritty_terminal::term::color::Colors) -> Option<Color> {
    (bg != Ansi::Named(NamedColor::Background)).then(|| color(bg, colors))
}

/// The bytes a key sends, if any.
pub fn key_bytes(key: &Key, modifiers: Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
    let ctrl = modifiers.contains(Modifiers::CONTROL);
    let alt = modifiers.contains(Modifiers::ALT);
    let shift = modifiers.contains(Modifiers::SHIFT);
    let arrow = |code: char| {
        if app_cursor && !ctrl && !alt && !shift {
            format!("\x1bO{code}")
        } else {
            let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
            if modifier > 1 {
                format!("\x1b[1;{modifier}{code}")
            } else {
                format!("\x1b[{code}")
            }
        }
    };
    let text: String = match key {
        Key::Named(named) => match named {
            NamedKey::Enter => "\r".into(),
            NamedKey::Backspace if ctrl => "\x08".into(),
            NamedKey::Backspace => "\x7f".into(),
            NamedKey::Tab if shift => "\x1b[Z".into(),
            NamedKey::Tab => "\t".into(),
            NamedKey::Escape => "\x1b".into(),
            NamedKey::ArrowUp => arrow('A'),
            NamedKey::ArrowDown => arrow('B'),
            NamedKey::ArrowRight => arrow('C'),
            NamedKey::ArrowLeft => arrow('D'),
            NamedKey::Home => arrow('H'),
            NamedKey::End => arrow('F'),
            NamedKey::Insert => "\x1b[2~".into(),
            NamedKey::Delete => "\x1b[3~".into(),
            NamedKey::PageUp => "\x1b[5~".into(),
            NamedKey::PageDown => "\x1b[6~".into(),
            NamedKey::F1 => "\x1bOP".into(),
            NamedKey::F2 => "\x1bOQ".into(),
            NamedKey::F3 => "\x1bOR".into(),
            NamedKey::F4 => "\x1bOS".into(),
            NamedKey::F5 => "\x1b[15~".into(),
            NamedKey::F6 => "\x1b[17~".into(),
            NamedKey::F7 => "\x1b[18~".into(),
            NamedKey::F8 => "\x1b[19~".into(),
            NamedKey::F9 => "\x1b[20~".into(),
            NamedKey::F10 => "\x1b[21~".into(),
            NamedKey::F11 => "\x1b[23~".into(),
            NamedKey::F12 => "\x1b[24~".into(),
            _ => return None,
        },
        Key::Character(text) => {
            let mut chars = text.chars();
            let (Some(c), None) = (chars.next(), chars.clone().next()) else {
                return Some(text.as_bytes().to_vec());
            };
            if ctrl {
                // Ctrl+letter and friends become control characters.
                let control = match c.to_ascii_lowercase() {
                    c @ 'a'..='z' => (c as u8 - b'a' + 1) as char,
                    '@' | ' ' | '2' => '\0',
                    '[' | '3' => '\x1b',
                    '\\' | '4' => '\x1c',
                    ']' | '5' => '\x1d',
                    '^' | '6' => '\x1e',
                    '_' | '-' | '/' | '7' => '\x1f',
                    '?' | '8' => '\x7f',
                    _ => return None,
                };
                control.to_string()
            } else {
                c.to_string()
            }
        }
    };
    let mut bytes = Vec::new();
    // Alt sends Escape first, as xterm's metaSendsEscape does; arrows and
    // the like carry it in their own modifier parameter instead.
    if alt && !text.starts_with('\x1b') {
        bytes.push(0x1b);
    }
    bytes.extend(text.into_bytes());
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_become_terminal_input() {
        let none = Modifiers::empty();
        let ctrl = Modifiers::CONTROL;
        let bytes = |key: Key, modifiers, app| key_bytes(&key, modifiers, app).unwrap();
        assert_eq!(bytes(Key::Character("a".into()), none, false), b"a");
        assert_eq!(bytes(Key::Character("c".into()), ctrl, false), [3]);
        assert_eq!(
            bytes(Key::Character("x".into()), Modifiers::ALT, false),
            b"\x1bx"
        );
        assert_eq!(bytes(Key::Named(NamedKey::Enter), none, false), b"\r");
        assert_eq!(bytes(Key::Named(NamedKey::ArrowUp), none, false), b"\x1b[A");
        assert_eq!(bytes(Key::Named(NamedKey::ArrowUp), none, true), b"\x1bOA");
        assert_eq!(
            bytes(Key::Named(NamedKey::ArrowLeft), ctrl, false),
            b"\x1b[1;5D"
        );
        assert_eq!(bytes(Key::Named(NamedKey::Backspace), none, false), b"\x7f");
        assert_eq!(
            bytes(Key::Named(NamedKey::Backspace), Modifiers::ALT, false),
            b"\x1b\x7f"
        );
        assert_eq!(
            bytes(Key::Named(NamedKey::ArrowUp), Modifiers::ALT, false),
            b"\x1b[1;3A"
        );
        assert_eq!(bytes(Key::Character("/".into()), ctrl, false), [0x1f]);
        assert_eq!(key_bytes(&Key::Named(NamedKey::Shift), none, false), None);
    }

    #[test]
    fn the_256_colour_cube_and_greys() {
        assert_eq!(indexed(16), Color::from_rgb8(0, 0, 0));
        assert_eq!(indexed(231), Color::from_rgb8(255, 255, 255));
        assert_eq!(indexed(232), Color::from_rgb8(8, 8, 8));
        assert_eq!(indexed(1), crate::ui::color::ansi(1));
    }
}
