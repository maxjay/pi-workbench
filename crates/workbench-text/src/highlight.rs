//! Syntax highlighting with Sublime grammars, reduced to a small token palette.
use std::{path::Path, sync::OnceLock};
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};

/// Larger documents are left plain so highlighting never stalls the editor.
pub const MAX_HIGHLIGHT_BYTES: usize = 1024 * 1024;
/// Very long lines (minified code) make backtracking grammars slow.
pub const MAX_LINE_BYTES: usize = 4000;

/// The colour classes the UI maps to its theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Token {
    Keyword,
    String,
    Comment,
    Function,
    Type,
    Number,
    Constant,
    Operator,
    Punctuation,
    Variable,
    Attribute,
    Tag,
    Plain,
}

/// A coloured byte range within one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub token: Token,
}

/// A grammar in the highlighter's syntax set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Language(usize);

/// Scope prefixes in priority order: specific prefixes precede their parents.
const SCOPES: &[(&str, Token)] = &[
    ("comment", Token::Comment),
    ("punctuation.definition.comment", Token::Comment),
    ("string", Token::String),
    ("punctuation.definition.string", Token::String),
    ("markup.raw", Token::String),
    ("punctuation.definition.raw", Token::String),
    ("constant.character.escape", Token::Constant),
    ("constant.numeric", Token::Number),
    ("constant", Token::Constant),
    ("keyword.operator", Token::Operator),
    ("keyword", Token::Keyword),
    ("storage", Token::Keyword),
    ("markup.heading", Token::Keyword),
    ("punctuation.definition.heading", Token::Keyword),
    ("variable.language", Token::Keyword),
    ("entity.name.function", Token::Function),
    ("support.function", Token::Function),
    ("variable.function", Token::Function),
    ("entity.name.type", Token::Type),
    ("entity.name.class", Token::Type),
    ("entity.name.struct", Token::Type),
    ("entity.name.enum", Token::Type),
    ("entity.name.trait", Token::Type),
    ("entity.name.interface", Token::Type),
    ("entity.name.union", Token::Type),
    ("support.type", Token::Type),
    ("support.class", Token::Type),
    ("entity.name.tag", Token::Tag),
    // Sections, namespaces and other named definitions.
    ("entity.name", Token::Type),
    ("entity.other.attribute-name", Token::Attribute),
    ("meta.attribute", Token::Attribute),
    ("meta.annotation", Token::Attribute),
    ("punctuation", Token::Punctuation),
    ("variable", Token::Variable),
];

/// Loads grammars on first use; build one and share it across threads.
#[derive(Default)]
pub struct Highlighter {
    syntaxes: OnceLock<SyntaxSet>,
    scopes: OnceLock<Vec<(Scope, Token)>>,
}

impl Highlighter {
    pub fn new() -> Self {
        Self::default()
    }

    fn syntaxes(&self) -> &SyntaxSet {
        self.syntaxes.get_or_init(two_face::syntax::extra_newlines)
    }

    fn scopes(&self) -> &[(Scope, Token)] {
        self.scopes.get_or_init(|| {
            SCOPES
                .iter()
                .map(|&(name, token)| (Scope::new(name).expect("valid scope"), token))
                .collect()
        })
    }

    /// Matches by extension or file name, then by the first line (shebangs, modelines).
    pub fn language_for(&self, path: &Path, first_line: &str) -> Option<Language> {
        let syntaxes = self.syntaxes();
        let by_name = |name: Option<&std::ffi::OsStr>| {
            name.and_then(|name| name.to_str())
                .and_then(|name| syntaxes.find_syntax_by_extension(name))
        };
        let syntax = by_name(path.extension())
            .or_else(|| by_name(path.file_name()))
            .or_else(|| syntaxes.find_syntax_by_first_line(first_line))?;
        let plain = syntaxes.find_syntax_plain_text();
        (syntax.name != plain.name).then(|| self.language(syntax))
    }

    fn language(&self, syntax: &SyntaxReference) -> Language {
        let index = self
            .syntaxes()
            .syntaxes()
            .iter()
            .position(|candidate| std::ptr::eq(candidate, syntax))
            .expect("syntax belongs to this set");
        Language(index)
    }

    pub fn language_name(&self, language: Language) -> &str {
        &self.syntaxes().syntaxes()[language.0].name
    }

    /// One span list per editor line, so a trailing newline yields a final empty line.
    /// Lines that are too long, or that the grammar fails on, stay plain.
    pub fn highlight(&self, language: Language, text: &str) -> Vec<Vec<Span>> {
        let lines: Vec<&str> = text
            .split('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line))
            .collect();
        let mut spans = vec![Vec::new(); lines.len()];
        if text.len() > MAX_HIGHLIGHT_BYTES {
            return spans;
        }
        let syntaxes = self.syntaxes();
        let mut state = ParseState::new(&syntaxes.syntaxes()[language.0]);
        let mut stack = ScopeStack::new();
        let mut buffer = String::new();
        for (line, out) in lines.iter().zip(&mut spans) {
            // Skipping keeps the parser's state, so a skipped line only loses its own colour.
            if line.len() > MAX_LINE_BYTES {
                continue;
            }
            buffer.clear();
            buffer.push_str(line);
            buffer.push('\n');
            let Ok(ops) = state.parse_line(&buffer, syntaxes) else {
                break;
            };
            let mut start = 0;
            for (position, op) in ops {
                self.push(out, start..position.min(line.len()), &stack);
                start = position;
                if stack.apply(&op).is_err() {
                    break;
                }
            }
            self.push(out, start..line.len(), &stack);
        }
        spans
    }

    /// Adds a coloured range, merging it into the previous span when contiguous.
    fn push(&self, spans: &mut Vec<Span>, range: std::ops::Range<usize>, stack: &ScopeStack) {
        if range.is_empty() {
            return;
        }
        let token = self.token(stack);
        if token == Token::Plain {
            return;
        }
        match spans.last_mut() {
            Some(last) if last.end == range.start && last.token == token => last.end = range.end,
            _ => spans.push(Span {
                start: range.start,
                end: range.end,
                token,
            }),
        }
    }

    /// The innermost scope with a known prefix decides the token.
    fn token(&self, stack: &ScopeStack) -> Token {
        let scopes = self.scopes();
        stack
            .as_slice()
            .iter()
            .rev()
            .find_map(|scope| {
                scopes
                    .iter()
                    .find(|(prefix, _)| prefix.is_prefix_of(*scope))
                    .map(|&(_, token)| token)
            })
            .unwrap_or(Token::Plain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn highlighter() -> &'static Highlighter {
        static HIGHLIGHTER: OnceLock<Highlighter> = OnceLock::new();
        HIGHLIGHTER.get_or_init(Highlighter::new)
    }

    fn rust() -> Language {
        highlighter()
            .language_for(Path::new("main.rs"), "")
            .unwrap()
    }

    fn token_of(spans: &[Span], line: &str, needle: &str) -> Token {
        let start = line.find(needle).unwrap();
        spans
            .iter()
            .find(|span| span.start <= start && start + needle.len() <= span.end)
            .map_or(Token::Plain, |span| span.token)
    }

    #[test]
    fn highlighter_can_be_shared_with_background_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Highlighter>();
    }

    #[test]
    fn common_languages_are_detected_by_extension() {
        let cases = [
            ("a.rs", "Rust"),
            ("A.java", "Java"),
            ("a.ts", "TypeScript"),
            ("a.py", "Python"),
            ("Cargo.toml", "TOML"),
            ("a.json", "JSON"),
            ("README.md", "Markdown"),
        ];
        for (path, name) in cases {
            let language = highlighter().language_for(Path::new(path), "").unwrap();
            assert_eq!(highlighter().language_name(language), name, "{path}");
        }
        assert!(
            highlighter()
                .language_for(Path::new("notes.unknown"), "")
                .is_none()
        );
    }

    #[test]
    fn first_line_identifies_scripts_without_extensions() {
        let language = highlighter()
            .language_for(Path::new("run"), "#!/usr/bin/env python3")
            .unwrap();
        assert_eq!(highlighter().language_name(language), "Python");
    }

    #[test]
    fn rust_tokens_are_classified() {
        let text = "fn main() {\n    let s = \"hi\"; // x\n}\n";
        let spans = highlighter().highlight(rust(), text);
        assert_eq!(spans.len(), 4);
        let second = text.lines().nth(1).unwrap();
        assert_eq!(token_of(&spans[0], "fn main() {", "fn"), Token::Keyword);
        assert_eq!(token_of(&spans[1], second, "let"), Token::Keyword);
        assert_eq!(token_of(&spans[1], second, "\"hi\""), Token::String);
        assert_eq!(token_of(&spans[1], second, "// x"), Token::Comment);
        assert!(spans[3].is_empty());
    }

    #[test]
    fn crlf_and_lf_highlight_identically() {
        let lf = "fn main() {\n    let s = \"hi\"; // x\n}\n";
        let crlf = lf.replace('\n', "\r\n");
        assert_eq!(
            highlighter().highlight(rust(), lf),
            highlighter().highlight(rust(), &crlf)
        );
    }

    #[test]
    fn spans_are_ordered_and_on_char_boundaries() {
        let text = "let λ = \"héllo 🦀\"; // ünïcode\nfn π() -> u8 { 0x1F }";
        let lines: Vec<&str> = text.split('\n').collect();
        for (line, spans) in lines.iter().zip(highlighter().highlight(rust(), text)) {
            let mut previous = 0;
            for span in spans {
                assert!(previous <= span.start && span.start < span.end);
                assert!(span.end <= line.len());
                assert!(line.is_char_boundary(span.start) && line.is_char_boundary(span.end));
                assert_ne!(span.token, Token::Plain);
                previous = span.end;
            }
        }
    }

    #[test]
    fn oversized_input_is_left_plain() {
        let long_line = format!("let s = \"{}\";\nlet t = 1;", "x".repeat(MAX_LINE_BYTES));
        let spans = highlighter().highlight(rust(), &long_line);
        assert!(spans[0].is_empty());
        assert!(!spans[1].is_empty());
        let huge = "let x = 1;\n".repeat(MAX_HIGHLIGHT_BYTES / 10);
        let spans = highlighter().highlight(rust(), &huge);
        assert!(spans.iter().all(Vec::is_empty));
    }
}
