//! Turns Pi's markdown replies into blocks the timeline lays out.
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Style {
    Strong,
    Emphasis,
    Code,
    Link,
}

/// Text with byte ranges that carry inline styles.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Inline {
    pub text: String,
    pub styles: Vec<(Range<usize>, Style)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Block {
    Heading(Inline),
    Paragraph(Inline),
    /// A list item; `marker` is "•" or "3.", `depth` counts nesting from 0.
    Item {
        depth: usize,
        marker: String,
        inline: Inline,
    },
    Quote(Inline),
    Code {
        language: String,
        text: String,
    },
    Rule,
}

struct Builder {
    blocks: Vec<Block>,
    inline: Inline,
    open: Vec<(usize, Style)>,
    /// Next number per nesting level; `None` for bullet lists.
    lists: Vec<Option<u64>>,
    item: Option<String>,
    quote: usize,
    code: Option<(String, String)>,
}

impl Builder {
    fn flush(&mut self, make: impl FnOnce(Inline) -> Block) {
        let mut inline = std::mem::take(&mut self.inline);
        let trimmed = inline.text.trim_end().len();
        inline.text.truncate(trimmed);
        for (range, _) in &mut inline.styles {
            range.end = range.end.min(trimmed);
        }
        inline.styles.retain(|(range, _)| range.start < range.end);
        if !inline.text.trim().is_empty() {
            self.blocks.push(make(inline));
        }
    }

    /// Ends whatever text block is in progress: a list item, quote, or paragraph.
    fn end_text(&mut self) {
        if let Some(marker) = self.item.take() {
            let depth = self.lists.len().saturating_sub(1);
            self.flush(|inline| Block::Item {
                depth,
                marker,
                inline,
            });
        } else if self.quote > 0 {
            self.flush(Block::Quote);
        } else {
            self.flush(Block::Paragraph);
        }
    }
}

pub fn parse(markdown: &str) -> Vec<Block> {
    let mut b = Builder {
        blocks: Vec::new(),
        inline: Inline::default(),
        open: Vec::new(),
        lists: Vec::new(),
        item: None,
        quote: 0,
        code: None,
    };
    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                b.end_text();
                let language = match kind {
                    CodeBlockKind::Fenced(lang) => {
                        lang.split_whitespace().next().unwrap_or("").into()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                b.code = Some((language, String::new()));
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((language, mut text)) = b.code.take() {
                    if text.ends_with('\n') {
                        text.pop();
                    }
                    b.blocks.push(Block::Code { language, text });
                }
            }
            Event::Text(text) if b.code.is_some() => {
                if let Some((_, code)) = &mut b.code {
                    code.push_str(&text);
                }
            }
            Event::Start(Tag::List(start)) => {
                b.end_text();
                b.lists.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                b.end_text();
                b.lists.pop();
            }
            Event::Start(Tag::Item) => {
                b.end_text();
                let marker = match b.lists.last_mut() {
                    Some(Some(number)) => {
                        *number += 1;
                        format!("{}.", *number - 1)
                    }
                    _ => "•".into(),
                };
                b.item = Some(marker);
            }
            Event::End(TagEnd::Item) => b.end_text(),
            Event::Start(Tag::BlockQuote(_)) => {
                b.end_text();
                b.quote += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                b.end_text();
                b.quote = b.quote.saturating_sub(1);
            }
            Event::End(TagEnd::Heading(_)) => b.flush(Block::Heading),
            Event::End(TagEnd::Paragraph) => {
                // Paragraphs inside items and quotes continue the same block.
                if b.item.is_some() || b.quote > 0 {
                    b.inline.text.push(' ');
                } else {
                    b.flush(Block::Paragraph);
                }
            }
            Event::Rule => {
                b.end_text();
                b.blocks.push(Block::Rule);
            }
            Event::Start(tag) => {
                let style = match tag {
                    Tag::Strong => Some(Style::Strong),
                    Tag::Emphasis => Some(Style::Emphasis),
                    Tag::Link { .. } => Some(Style::Link),
                    _ => None,
                };
                if let Some(style) = style {
                    b.open.push((b.inline.text.len(), style));
                }
            }
            Event::End(TagEnd::Strong | TagEnd::Emphasis | TagEnd::Link) => {
                if let Some((start, style)) = b.open.pop() {
                    b.inline.styles.push((start..b.inline.text.len(), style));
                }
            }
            Event::Code(code) => {
                let start = b.inline.text.len();
                b.inline.text.push_str(&code);
                b.inline
                    .styles
                    .push((start..b.inline.text.len(), Style::Code));
            }
            Event::Text(text) => b.inline.text.push_str(&text),
            Event::SoftBreak => b.inline.text.push(' '),
            Event::HardBreak => b.inline.text.push('\n'),
            Event::TaskListMarker(done) => {
                b.inline.text.push_str(if done { "☑ " } else { "☐ " })
            }
            _ => {}
        }
    }
    // Streaming text may end mid-block.
    if let Some((language, text)) = b.code.take() {
        b.blocks.push(Block::Code { language, text });
    }
    b.end_text();
    b.blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_reply_structure() {
        let blocks = parse(
            "## Plan\n\nRename `greet` to **welcome**.\n\n1. Edit\n2. Test\n\n- a\n  - b\n\n```rust\nfn main() {}\n```\n\n> note\n\n---",
        );
        assert_eq!(blocks.len(), 9);
        assert!(matches!(&blocks[0], Block::Heading(i) if i.text == "Plan"));
        let Block::Paragraph(inline) = &blocks[1] else {
            panic!("{:?}", blocks[1]);
        };
        assert_eq!(inline.text, "Rename greet to welcome.");
        assert_eq!(
            inline.styles,
            [(7..12, Style::Code), (16..23, Style::Strong)]
        );
        assert!(matches!(&blocks[2], Block::Item { marker, depth: 0, .. } if marker == "1."));
        assert!(matches!(&blocks[3], Block::Item { marker, .. } if marker == "2."));
        assert!(
            matches!(&blocks[5], Block::Item { marker, depth: 1, inline } if marker == "•" && inline.text == "b")
        );
        assert_eq!(
            blocks[6],
            Block::Code {
                language: "rust".into(),
                text: "fn main() {}".into()
            }
        );
        assert!(matches!(&blocks[7], Block::Quote(i) if i.text == "note"));
        assert_eq!(blocks[8], Block::Rule);
    }

    #[test]
    fn unfinished_streaming_text_still_renders() {
        let blocks = parse("Here:\n\n```js\nlet a =");
        assert_eq!(
            blocks[1],
            Block::Code {
                language: "js".into(),
                text: "let a =".into()
            }
        );
        assert_eq!(
            parse("**bold"),
            [Block::Paragraph(Inline {
                text: "**bold".into(),
                styles: vec![]
            })]
        );
    }
}
