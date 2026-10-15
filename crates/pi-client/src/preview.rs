//! Reads file edits out of tool arguments while they are still streaming.
//!
//! Partial JSON is not parseable, so this scans string tokens instead and
//! keeps the last value even if its closing quote has not arrived yet.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FilePreview {
    pub path: Option<String>,
    /// `(oldText, newText)` pairs from an `edit` call, the last possibly incomplete.
    pub edits: Vec<(String, String)>,
    /// Full content from a `write` call, possibly incomplete.
    pub content: Option<String>,
}

/// Keys and string values in order; the final value may be unterminated.
fn string_fields(json: &str) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    let mut chars = json.chars().peekable();
    let mut last_string: Option<String> = None;
    let mut key: Option<String> = None;
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let mut value = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some('n') => value.push('\n'),
                            Some('t') => value.push('\t'),
                            Some('r') => value.push('\r'),
                            Some('b') => value.push('\u{8}'),
                            Some('f') => value.push('\u{c}'),
                            Some('u') => {
                                let hex: String = chars.by_ref().take(4).collect();
                                if let Some(c) =
                                    u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32)
                                {
                                    value.push(c);
                                }
                            }
                            Some(other) => value.push(other),
                            None => {}
                        },
                        c => value.push(c),
                    }
                }
                match key.take() {
                    Some(name) => fields.push((name, value)),
                    None => last_string = Some(value),
                }
            }
            ':' => key = last_string.take(),
            ',' | '{' | '[' | '}' | ']' => {
                key = None;
                last_string = None;
            }
            _ => {}
        }
    }
    fields
}

pub fn file_preview(partial_args: &str) -> FilePreview {
    let mut preview = FilePreview::default();
    for (key, value) in string_fields(partial_args) {
        match key.as_str() {
            "path" => preview.path = Some(value),
            "content" => preview.content = Some(value),
            "oldText" => preview.edits.push((value, String::new())),
            "newText" => match preview.edits.last_mut() {
                Some((_, new)) if new.is_empty() => *new = value,
                _ => preview.edits.push((String::new(), value)),
            },
            _ => {}
        }
    }
    preview
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_complete_edit_arguments() {
        let preview = file_preview(
            r#"{"path":"src/a.rs","edits":[{"oldText":"a\n\"b\"","newText":"c"},{"oldText":"d","newText":"e"}]}"#,
        );
        assert_eq!(preview.path.as_deref(), Some("src/a.rs"));
        assert_eq!(
            preview.edits,
            [("a\n\"b\"".into(), "c".into()), ("d".into(), "e".into())]
        );
    }

    #[test]
    fn keeps_the_unterminated_value_while_streaming() {
        let preview = file_preview(r#"{"path":"a.rs","edits":[{"oldText":"hel"#);
        assert_eq!(preview.edits, [("hel".into(), String::new())]);
        let preview = file_preview(r#"{"path":"a.rs","content":"fn main() {\n    pri"#);
        assert_eq!(preview.content.as_deref(), Some("fn main() {\n    pri"));
        // A key without its value yet adds nothing.
        assert_eq!(file_preview(r#"{"pa"#), FilePreview::default());
        assert_eq!(file_preview(r#"{"path":"#).path, None);
    }
}
