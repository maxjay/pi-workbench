//! The colours in use, from the current theme. Each is read from a signal,
//! so a style closure that uses one restyles when the theme changes.
use floem::{
    peniko::Color,
    prelude::*,
    reactive::{ReadSignal, Scope},
};
use std::{cell::RefCell, collections::HashMap, rc::Rc};
use workbench_text::{
    highlight::Token,
    theme::{self, Rgba, Theme},
};

thread_local! {
    /// In its own scope, so no window closing can dispose of it.
    static CURRENT: RwSignal<Rc<Theme>> =
        Scope::new().enter(|| RwSignal::new(Rc::new(theme::default_theme())));
    /// The theme the settings name; a cancelled preview returns to it.
    static CHOSEN: RefCell<Rc<Theme>> = RefCell::new(Rc::new(theme::default_theme()));
    static LOADED: RefCell<HashMap<String, Rc<Theme>>> = RefCell::new(HashMap::new());
}

fn convert(c: Rgba) -> Color {
    Color::from_rgba8(c.0, c.1, c.2, c.3)
}

fn read(pick: impl Fn(&Theme) -> Rgba) -> Color {
    CURRENT.with(|current| current.with(|theme| convert(pick(theme))))
}

/// Tracked: the whole theme.
pub fn theme() -> ReadSignal<Rc<Theme>> {
    CURRENT.with(|current| current.read_only())
}

/// A theme by name or `.tmTheme` path. Bundled themes are loaded once;
/// files are read each time, so edits to them show.
pub fn load(name: &str) -> anyhow::Result<Rc<Theme>> {
    let file = name.to_ascii_lowercase().ends_with(".tmtheme");
    if !file && let Some(theme) = LOADED.with(|loaded| loaded.borrow().get(name).cloned()) {
        return Ok(theme);
    }
    let theme = Rc::new(theme::load(name)?);
    if !file {
        LOADED.with(|loaded| loaded.borrow_mut().insert(name.to_owned(), theme.clone()));
    }
    Ok(theme)
}

/// Shows a theme without choosing it.
pub fn preview(theme: Rc<Theme>) {
    CURRENT.with(|current| {
        if current.with_untracked(|now| **now != *theme) {
            current.set(theme);
        }
    });
}

/// Makes a theme the one to return to.
pub fn choose(theme: Rc<Theme>) {
    CHOSEN.with(|chosen| *chosen.borrow_mut() = theme.clone());
    preview(theme);
}

/// Goes back to the chosen theme after previewing others.
pub fn revert() {
    preview(CHOSEN.with(|chosen| chosen.borrow().clone()));
}

pub fn chosen_name() -> String {
    CHOSEN.with(|chosen| chosen.borrow().name.clone())
}

/// Applies the theme the settings name, if it changed. Returns an error to
/// report when it cannot be loaded; the current theme then stays.
pub fn follow_settings() -> Option<String> {
    let wanted = crate::settings::get()
        .theme
        .clone()
        .unwrap_or_else(|| theme::DEFAULT.to_owned());
    let current = CHOSEN.with(|chosen| chosen.borrow().name.clone());
    let path_named = std::path::Path::new(&wanted)
        .file_stem()
        .is_some_and(|stem| stem.to_string_lossy() == current);
    if wanted.eq_ignore_ascii_case(&current) || path_named {
        return None;
    }
    match load(&wanted) {
        Ok(theme) => {
            choose(theme);
            None
        }
        Err(err) => Some(format!("Theme: {err:#}")),
    }
}

/// Whether the theme is dark: its background is nearer black than white.
pub fn is_dark() -> bool {
    let c = bg().to_rgba8();
    0.2126 * f64::from(c.r) + 0.7152 * f64::from(c.g) + 0.0722 * f64::from(c.b) < 128.0
}

pub fn bg() -> Color {
    read(|t| t.background)
}
pub fn panel() -> Color {
    read(|t| t.panel)
}
/// Raised cards inside panels.
pub fn surface() -> Color {
    read(|t| t.surface)
}
pub fn border() -> Color {
    read(|t| t.border)
}
pub fn text() -> Color {
    read(|t| t.text)
}
pub fn muted() -> Color {
    read(|t| t.muted)
}
pub fn accent() -> Color {
    read(|t| t.accent)
}
pub fn info() -> Color {
    read(|t| t.info)
}
pub fn warn() -> Color {
    read(|t| t.warn)
}
/// Line backgrounds for added and removed lines.
pub fn added() -> Color {
    read(|t| Rgba(t.added.0, t.added.1, t.added.2, 46))
}
pub fn removed() -> Color {
    read(|t| Rgba(t.removed.0, t.removed.1, t.removed.2, 46))
}
pub fn added_text() -> Color {
    read(|t| t.added)
}
pub fn removed_text() -> Color {
    read(|t| t.removed)
}
pub fn editor_bg() -> Color {
    read(|t| t.editor_background)
}
pub fn current_line() -> Color {
    read(|t| t.current_line)
}
pub fn gutter_text() -> Color {
    read(|t| t.gutter_text)
}
pub fn selection() -> Color {
    read(|t| t.selection)
}
pub fn caret() -> Color {
    read(|t| t.caret)
}
/// The selected row in lists.
pub fn highlight() -> Color {
    read(|t| t.highlight)
}
/// Text on the accent colour, as on buttons.
pub fn on_accent() -> Color {
    read(|t| t.on_accent)
}
/// The terminal's first sixteen colours.
pub fn ansi(index: usize) -> Color {
    read(|t| t.ansi[index.min(15)])
}

/// How code of a kind is drawn: colour and whether italic. Untracked: it is
/// read while laying out text, and the editor restyles on `theme()` instead.
pub fn token(token: Token) -> Option<(Color, bool)> {
    CURRENT.with(|current| {
        current.with_untracked(|theme| {
            theme
                .token(token)
                .map(|style| (convert(style.color), style.italic))
        })
    })
}
