//! Colour themes: the bundled TextMate themes from bat's collection, any
//! `.tmTheme` file, and the workbench's own default. A theme colours code by
//! its own scope rules, and the rest of the app is derived from its colours.
use crate::highlight::Token;
use anyhow::{Context, Result};
use std::{path::Path, str::FromStr};
use syntect::{
    highlighting::{
        Color as ThemeColor, FontStyle, Highlighter as ScopeColors, Theme as TextMateTheme,
        ThemeSet,
    },
    parsing::{Scope, ScopeStack},
};
use two_face::theme::EmbeddedThemeName;

/// The workbench's own theme, used when no other is chosen.
pub const DEFAULT: &str = "Pi Workbench";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self(r, g, b, 255)
    }

    fn from_theme(color: ThemeColor) -> Self {
        Self(color.r, color.g, color.b, color.a)
    }

    /// Relative luminance, 0 for black to 1 for white.
    fn luminance(self) -> f32 {
        let channel = |c: u8| {
            let c = f32::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(self.0) + 0.7152 * channel(self.1) + 0.0722 * channel(self.2)
    }

    /// `self` moved toward `other` by `amount`, from 0 to 1.
    fn mix(self, other: Rgba, amount: f32) -> Rgba {
        let blend =
            |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * amount).round() as u8;
        Rgba(
            blend(self.0, other.0),
            blend(self.1, other.1),
            blend(self.2, other.2),
            255,
        )
    }

    fn opaque(self) -> Rgba {
        Rgba(self.0, self.1, self.2, 255)
    }

    /// A translucent colour as it looks drawn over `under`.
    fn over(self, under: Rgba) -> Rgba {
        under.mix(self.opaque(), f32::from(self.3) / 255.0)
    }

    /// Backs off toward the background it came from if `text` would no
    /// longer read on it.
    fn with_text(self, text: Rgba, background: Rgba) -> Rgba {
        // Text that is dim on the background already cannot do better here.
        let wanted = (text.contrast(background) * 0.75).min(3.5);
        (0..=10)
            .map(|step| self.mix(background, f32::from(step as u8) * 0.1))
            .find(|c| text.contrast(*c) >= wanted)
            .unwrap_or(background)
    }

    /// How far the colour is from grey, 0 to 255.
    fn saturation(self) -> u8 {
        let (max, min) = (
            self.0.max(self.1).max(self.2),
            self.0.min(self.1).min(self.2),
        );
        max - min
    }

    /// Hue in degrees, 0 to 360.
    fn hue(self) -> f32 {
        let (r, g, b) = (
            f32::from(self.0) / 255.0,
            f32::from(self.1) / 255.0,
            f32::from(self.2) / 255.0,
        );
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let delta = max - min;
        if delta == 0.0 {
            return 0.0;
        }
        let hue = if max == r {
            60.0 * ((g - b) / delta).rem_euclid(6.0)
        } else if max == g {
            60.0 * ((b - r) / delta + 2.0)
        } else {
            60.0 * ((r - g) / delta + 4.0)
        };
        hue.rem_euclid(360.0)
    }

    fn hue_distance(self, other: Rgba) -> f32 {
        let d = (self.hue() - other.hue()).abs();
        d.min(360.0 - d)
    }

    fn contrast(self, other: Rgba) -> f32 {
        let (a, b) = (self.luminance() + 0.05, other.luminance() + 0.05);
        a.max(b) / a.min(b)
    }
}

/// How one kind of code is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenStyle {
    pub color: Rgba,
    pub bold: bool,
    pub italic: bool,
}

/// Every colour the workbench draws with.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub name: String,
    pub dark: bool,
    /// The window behind everything.
    pub background: Rgba,
    /// Side panels, tab bars and dialogs.
    pub panel: Rgba,
    /// Cards raised inside panels.
    pub surface: Rgba,
    pub border: Rgba,
    pub text: Rgba,
    pub muted: Rgba,
    pub accent: Rgba,
    pub info: Rgba,
    pub warn: Rgba,
    pub added: Rgba,
    pub removed: Rgba,
    pub editor_background: Rgba,
    pub current_line: Rgba,
    pub gutter_text: Rgba,
    pub selection: Rgba,
    pub caret: Rgba,
    /// The selected row in lists.
    pub highlight: Rgba,
    /// Text drawn on the accent, as on buttons.
    pub on_accent: Rgba,
    /// The terminal's first sixteen colours.
    pub ansi: [Rgba; 16],
    tokens: Vec<(Token, TokenStyle)>,
}

impl Theme {
    pub fn token(&self, token: Token) -> Option<TokenStyle> {
        self.tokens
            .iter()
            .find(|(t, _)| *t == token)
            .map(|(_, style)| *style)
    }
}

const DARK_ANSI: [Rgba; 16] = [
    Rgba::rgb(40, 44, 52),
    Rgba::rgb(224, 108, 117),
    Rgba::rgb(152, 195, 121),
    Rgba::rgb(229, 192, 123),
    Rgba::rgb(97, 175, 239),
    Rgba::rgb(198, 120, 221),
    Rgba::rgb(86, 182, 194),
    Rgba::rgb(171, 178, 191),
    Rgba::rgb(92, 99, 112),
    Rgba::rgb(240, 135, 143),
    Rgba::rgb(173, 219, 141),
    Rgba::rgb(240, 210, 150),
    Rgba::rgb(130, 196, 250),
    Rgba::rgb(214, 150, 236),
    Rgba::rgb(110, 205, 216),
    Rgba::rgb(220, 224, 232),
];

/// Darker colours that read on a light background.
const LIGHT_ANSI: [Rgba; 16] = [
    Rgba::rgb(56, 58, 66),
    Rgba::rgb(202, 18, 67),
    Rgba::rgb(80, 161, 79),
    Rgba::rgb(193, 132, 1),
    Rgba::rgb(64, 120, 242),
    Rgba::rgb(166, 38, 164),
    Rgba::rgb(1, 132, 188),
    Rgba::rgb(160, 161, 167),
    Rgba::rgb(105, 108, 119),
    Rgba::rgb(228, 86, 73),
    Rgba::rgb(80, 161, 79),
    Rgba::rgb(193, 132, 1),
    Rgba::rgb(64, 120, 242),
    Rgba::rgb(166, 38, 164),
    Rgba::rgb(1, 132, 188),
    Rgba::rgb(250, 250, 250),
];

/// The workbench's original look.
pub fn default_theme() -> Theme {
    let style = |r, g, b, italic| TokenStyle {
        color: Rgba::rgb(r, g, b),
        bold: false,
        italic,
    };
    Theme {
        name: DEFAULT.into(),
        dark: true,
        background: Rgba::rgb(21, 24, 30),
        panel: Rgba::rgb(28, 32, 40),
        surface: Rgba::rgb(34, 39, 49),
        border: Rgba::rgb(47, 54, 66),
        text: Rgba::rgb(218, 224, 233),
        muted: Rgba::rgb(143, 156, 176),
        accent: Rgba::rgb(128, 190, 175),
        info: Rgba::rgb(122, 162, 247),
        warn: Rgba::rgb(229, 184, 100),
        added: Rgba::rgb(134, 207, 150),
        removed: Rgba::rgb(236, 135, 135),
        editor_background: Rgba::rgb(40, 44, 52),
        current_line: Rgba::rgb(46, 51, 61),
        gutter_text: Rgba::rgb(84, 94, 110),
        selection: Rgba::rgb(57, 76, 81),
        caret: Rgba::rgb(128, 190, 175),
        highlight: highlight(
            Rgba::rgb(28, 32, 40),
            Rgba::rgb(128, 190, 175),
            Rgba::rgb(218, 224, 233),
        ),
        on_accent: Rgba::rgb(21, 24, 30),
        ansi: DARK_ANSI,
        tokens: vec![
            (Token::Keyword, style(198, 146, 233, false)),
            (Token::String, style(163, 205, 133, false)),
            (Token::Comment, style(110, 120, 138, true)),
            (Token::Function, style(120, 178, 245, false)),
            (Token::Type, style(232, 197, 128, false)),
            (Token::Number, style(222, 160, 110, false)),
            (Token::Constant, style(222, 160, 110, false)),
            (Token::Operator, style(120, 200, 210, false)),
            (Token::Attribute, style(230, 120, 130, false)),
            (Token::Tag, style(230, 120, 130, false)),
        ],
    }
}

/// Every theme by name: the default first, then the bundled ones.
pub fn names() -> Vec<String> {
    std::iter::once(DEFAULT.to_owned())
        .chain(
            two_face::theme::EmbeddedLazyThemeSet::theme_names()
                .iter()
                // Terminal-palette themes colour by ANSI index, not by colour.
                .filter(|name| {
                    !matches!(
                        name,
                        EmbeddedThemeName::Ansi
                            | EmbeddedThemeName::Base16
                            | EmbeddedThemeName::Base16_256
                    )
                })
                .map(|name| name.as_name().to_owned()),
        )
        .collect()
}

/// A theme by name, or from a `.tmTheme` file when given a path.
pub fn load(name: &str) -> Result<Theme> {
    if name.eq_ignore_ascii_case(DEFAULT) {
        return Ok(default_theme());
    }
    let path = Path::new(name);
    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("tmtheme"))
    {
        let theme = ThemeSet::get_theme(path)
            .with_context(|| format!("Cannot read the theme {}", path.display()))?;
        let name = path
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| name.to_owned());
        return Ok(derive(name, &theme));
    }
    let embedded = two_face::theme::EmbeddedLazyThemeSet::theme_names()
        .iter()
        .find(|embedded| embedded.as_name().eq_ignore_ascii_case(name))
        .with_context(|| format!("No theme named {name:?}"))?;
    // The bundled theme's own spelling, whatever the settings say.
    Ok(derive(
        embedded.as_name().to_owned(),
        two_face::theme::extra().get(*embedded),
    ))
}

/// The scope each token kind is coloured as.
const TOKEN_SCOPES: &[(Token, &str)] = &[
    (Token::Keyword, "keyword.control"),
    (Token::String, "string.quoted.double"),
    (Token::Comment, "comment.line"),
    (Token::Function, "entity.name.function"),
    (Token::Type, "entity.name.type"),
    (Token::Number, "constant.numeric"),
    (Token::Constant, "constant.language"),
    (Token::Operator, "keyword.operator"),
    (Token::Variable, "variable.parameter"),
    (Token::Attribute, "entity.other.attribute-name"),
    (Token::Tag, "entity.name.tag"),
];

fn derive(name: String, theme: &TextMateTheme) -> Theme {
    let settings = &theme.settings;
    let colors = ScopeColors::new(theme);
    let text = settings
        .foreground
        .map(Rgba::from_theme)
        .unwrap_or(Rgba::rgb(220, 220, 220))
        .opaque();
    let editor_background = settings
        .background
        .map(Rgba::from_theme)
        .unwrap_or(Rgba::rgb(30, 30, 30))
        .opaque();
    let dark = editor_background.luminance() < 0.4;
    let style_of = |scope: &str| {
        let scope = Scope::new(scope).ok()?;
        let style = colors.style_for_stack(ScopeStack::from_vec(vec![scope]).as_slice());
        let color = Rgba::from_theme(style.foreground).opaque();
        // A scope the theme leaves alone comes back in the plain text colour.
        (color != text).then_some(TokenStyle {
            color,
            bold: style.font_style.contains(FontStyle::BOLD),
            italic: style.font_style.contains(FontStyle::ITALIC),
        })
    };
    let tokens: Vec<(Token, TokenStyle)> = TOKEN_SCOPES
        .iter()
        .filter_map(|(token, scope)| Some((*token, style_of(scope)?)))
        .collect();
    let color_of = |scope: &str| style_of(scope).map(|s| s.color);
    // Panels sit a little behind the editor: darker on dark themes, and a
    // touch darker on light ones.
    let shade = if dark {
        Rgba::rgb(0, 0, 0)
    } else {
        Rgba::rgb(60, 60, 70)
    };
    let panel = editor_background.mix(shade, if dark { 0.28 } else { 0.05 });
    let background = editor_background.mix(shade, if dark { 0.45 } else { 0.09 });
    let readable = |color: Rgba| color.contrast(panel) >= 2.5;
    // The accent is the theme's own if it has one, else its most colourful
    // readable syntax colour.
    let accent = settings
        .accent
        .map(Rgba::from_theme)
        .map(Rgba::opaque)
        .filter(|c| readable(*c) && c.saturation() > 40)
        .or_else(|| {
            settings
                .caret
                .map(Rgba::from_theme)
                .map(Rgba::opaque)
                .filter(|c| readable(*c) && c.saturation() > 60)
        })
        .or_else(|| {
            [
                "entity.name.function",
                "keyword.control",
                "string.quoted.double",
            ]
            .iter()
            .filter_map(|scope| color_of(scope))
            .find(|c| readable(*c) && c.saturation() > 40)
        })
        .unwrap_or(if dark {
            Rgba::rgb(128, 190, 175)
        } else {
            Rgba::rgb(20, 120, 110)
        });
    let pick = |scopes: &[&str], fallback: Rgba| {
        scopes
            .iter()
            .filter_map(|scope| color_of(scope))
            .find(|c| readable(*c) && c.saturation() > 30)
            .unwrap_or(fallback)
    };
    let (blue, amber, green, red) = if dark {
        (
            Rgba::rgb(122, 162, 247),
            Rgba::rgb(229, 184, 100),
            Rgba::rgb(134, 207, 150),
            Rgba::rgb(236, 135, 135),
        )
    } else {
        (
            Rgba::rgb(40, 90, 200),
            Rgba::rgb(170, 110, 0),
            Rgba::rgb(30, 130, 60),
            Rgba::rgb(200, 40, 50),
        )
    };
    let magenta = if dark {
        Rgba::rgb(198, 120, 221)
    } else {
        Rgba::rgb(150, 40, 170)
    };
    // Whoever wrote a line must stay tellable apart at a glance: Pi's accent,
    // your colour and others' colour need clearly different hues.
    let apart = |a: Rgba, b: Rgba| a.hue_distance(b) >= 45.0;
    let themed_info = pick(&["entity.name.function", "support.function"], blue);
    let info = [themed_info, blue, magenta, green]
        .into_iter()
        .find(|c| apart(*c, accent))
        .unwrap_or(blue);
    let themed_warn = pick(
        &["entity.name.type", "storage.type", "constant.numeric"],
        amber,
    );
    let warn = [themed_warn, amber, red, magenta, green]
        .into_iter()
        .find(|c| apart(*c, accent) && apart(*c, info))
        .unwrap_or(amber);
    let surface = panel.mix(text, 0.04);
    // Fixed mixes wash out on light themes; aim for contrast instead.
    let border = reach(editor_background, text, 0.15, |c| c.contrast(panel) >= 1.4);
    let muted = (0..=20)
        .map(|step| text.mix(editor_background, 0.4 - f32::from(step as u8) * 0.02))
        .find(|c| c.contrast(panel) >= 4.0)
        .unwrap_or(text);
    // A theme's highlight colours are often translucent: draw them over the
    // editor before judging them.
    let current_line = settings
        .line_highlight
        .map(Rgba::from_theme)
        .map(|c| c.over(editor_background))
        .filter(|c| {
            // As readable as the theme's own text, near enough.
            text.contrast(*c) >= (text.contrast(editor_background) * 0.85).min(4.5)
                && *c != editor_background
        })
        .unwrap_or_else(|| editor_background.mix(text, 0.06));
    let selection = settings
        .selection
        .map(Rgba::from_theme)
        .map(|c| c.over(editor_background))
        .unwrap_or_else(|| accent.mix(editor_background, 0.7));
    let selection = reach(selection, text, 0.0, |c| {
        c.contrast(editor_background) >= 1.3
    })
    .with_text(text, editor_background);
    Theme {
        name,
        dark,
        background,
        panel,
        surface,
        border,
        text,
        muted,
        accent,
        info,
        warn,
        added: pick(&["markup.inserted", "string.quoted.double"], green),
        removed: pick(&["markup.deleted", "invalid"], red),
        editor_background,
        current_line,
        gutter_text: settings
            .gutter_foreground
            .map(Rgba::from_theme)
            .map(|c| c.over(editor_background))
            .filter(|c| c.contrast(editor_background) >= 1.8)
            .unwrap_or_else(|| text.mix(editor_background, 0.55)),
        selection,
        caret: settings
            .caret
            .map(Rgba::from_theme)
            .map(|c| c.over(editor_background))
            .filter(|c| c.contrast(editor_background) >= 2.0)
            .unwrap_or(accent),
        highlight: highlight(panel, accent, text),
        on_accent: on(
            accent,
            [
                background,
                text,
                Rgba::rgb(0, 0, 0),
                Rgba::rgb(255, 255, 255),
            ],
        ),
        ansi: if dark { DARK_ANSI } else { LIGHT_ANSI },
        tokens,
    }
}

/// `from` moved toward `toward` by at least `least`, and further until
/// `good` holds or the colours meet.
fn reach(from: Rgba, toward: Rgba, least: f32, good: impl Fn(Rgba) -> bool) -> Rgba {
    (0..=50)
        .map(|step| from.mix(toward, (least + f32::from(step as u8) * 0.02).min(1.0)))
        .find(|c| good(*c))
        .unwrap_or(toward)
}

/// The selected row in lists: the panel tinted with the accent, clearly
/// apart from the panel and still easy to read on.
fn highlight(panel: Rgba, accent: Rgba, text: Rgba) -> Rgba {
    reach(panel, accent, 0.16, |c| c.contrast(panel) >= 1.25).with_text(text, panel)
}

/// Of `choices`, the one that reads best on `background`.
fn on(background: Rgba, choices: [Rgba; 4]) -> Rgba {
    choices
        .into_iter()
        .max_by(|a, b| {
            a.contrast(background)
                .partial_cmp(&b.contrast(background))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(background)
}

impl FromStr for Theme {
    type Err = anyhow::Error;
    fn from_str(name: &str) -> Result<Self> {
        load(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_theme_loads_with_readable_text() {
        for name in names() {
            let theme = load(&name).unwrap();
            assert!(
                theme.text.contrast(theme.editor_background) >= 3.0,
                "{name}: text is unreadable"
            );
            assert!(
                theme.accent.contrast(theme.panel) >= 2.5,
                "{name}: accent is unreadable"
            );
            let readable = [
                (
                    "text on the current line",
                    theme.text.contrast(theme.current_line),
                    (theme.text.contrast(theme.editor_background) * 0.85).min(4.5),
                ),
                (
                    "muted text on panels",
                    theme.muted.contrast(theme.panel),
                    3.5,
                ),
                ("borders on panels", theme.border.contrast(theme.panel), 1.3),
                (
                    "selection",
                    theme.selection.contrast(theme.editor_background),
                    1.2,
                ),
                (
                    "text on the selection",
                    theme.text.contrast(theme.selection),
                    (theme.text.contrast(theme.editor_background) * 0.75).min(3.5) - 0.01,
                ),
                (
                    "the selected row",
                    theme.highlight.contrast(theme.panel),
                    1.2,
                ),
                (
                    "text on the accent",
                    theme.on_accent.contrast(theme.accent),
                    3.0,
                ),
            ];
            for (what, contrast, least) in readable {
                assert!(
                    contrast >= least,
                    "{name}: {what} at {contrast:.2}, below {least}"
                );
            }
            let origins = [theme.accent, theme.info, theme.warn];
            for (i, a) in origins.iter().enumerate() {
                for b in &origins[i + 1..] {
                    assert!(a.hue_distance(*b) >= 45.0, "{name}: origins look alike");
                }
            }
        }
    }

    #[test]
    fn light_themes_are_light_and_colour_code_their_own_way() {
        let latte = load("Catppuccin Latte").unwrap();
        assert!(!latte.dark);
        let mocha = load("catppuccin mocha").unwrap();
        assert!(mocha.dark);
        assert_eq!(mocha.name, "Catppuccin Mocha", "the bundled spelling");
        assert_ne!(latte.token(Token::Keyword), mocha.token(Token::Keyword));
        assert!(load("No Such Theme").is_err());
        assert_eq!(load(DEFAULT).unwrap(), default_theme());
    }
}
