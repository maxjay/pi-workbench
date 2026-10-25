//! wb-agent's own question tool, `ask_user`. Pi's question tools draw in
//! its terminal, which RPC mode doesn't have, so this one is a Pi extension
//! (`ask_user.ts`) that asks wb-agent over a Unix socket instead. Each
//! session has its socket, named in `WB_ASK`; the extension writes the
//! questions as one line and waits for the answers on one line back.

use std::path::{Path, PathBuf};

use protocol::{AskReply, Choice, Question};
use serde_json::{Value, json};

const EXTENSION: &str = include_str!("ask_user.ts");

/// Writes the extension where Pi can load it, and says where.
pub fn install(dir: &Path) -> std::io::Result<PathBuf> {
    let path = dir.join("ask_user.ts");
    std::fs::create_dir_all(dir)?;
    if std::fs::read_to_string(&path).ok().as_deref() != Some(EXTENSION) {
        std::fs::write(&path, EXTENSION)?;
    }
    Ok(path)
}

/// The questions in the extension's line. None for a line that isn't one.
pub fn questions(line: &str) -> Option<Vec<Question>> {
    let value: Value = serde_json::from_str(line).ok()?;
    let questions: Vec<Question> = value["questions"]
        .as_array()?
        .iter()
        .enumerate()
        .map(|(i, q)| Question {
            label: q["label"]
                .as_str()
                .filter(|label| !label.trim().is_empty())
                .map_or_else(|| format!("Q{}", i + 1), str::to_string),
            question: q["question"].as_str().unwrap_or_default().to_string(),
            choices: q["choices"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| {
                    Some(Choice {
                        label: c["label"].as_str()?.to_string(),
                        description: c["description"].as_str().map(str::to_string),
                    })
                })
                .collect(),
            other: q["other"].as_bool().unwrap_or(true),
        })
        .collect();
    (!questions.is_empty()).then_some(questions)
}

/// The line that answers the extension: the answers, or none when dismissed.
pub fn reply(reply: &AskReply) -> String {
    let answers = match reply {
        AskReply::Answers { answers } => Some(answers.clone()),
        AskReply::Value { value } => Some(vec![value.clone()]),
        AskReply::Confirmed { confirmed } => {
            Some(vec![if *confirmed { "Yes" } else { "No" }.to_string()])
        }
        AskReply::Cancelled => None,
    };
    let mut line = json!({ "answers": answers }).to_string();
    line.push('\n');
    line
}

/// Where a session's socket goes: a folder of wb-agent's own, as socket
/// paths can't be long.
pub fn socket_path(id: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!("wb-agent-{}", std::process::id()))
        .join(format!("{id}.sock"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_extensions_questions() {
        let line = r#"{"questions":[{"label":"","question":"How much downtime?","choices":[{"label":"None","description":"Dual-write"},{"label":"30 min"}],"other":false}]}"#;
        let asked = questions(line).unwrap();
        assert_eq!(asked[0].label, "Q1");
        assert_eq!(asked[0].choices.len(), 2);
        assert_eq!(asked[0].choices[1].description, None);
        assert!(!asked[0].other);
        assert_eq!(questions(r#"{"questions":[]}"#), None);
        assert_eq!(questions("nonsense"), None);
    }

    #[test]
    fn answers_in_one_line() {
        let answers = AskReply::Answers {
            answers: vec!["None".into(), "A week".into()],
        };
        assert_eq!(reply(&answers), "{\"answers\":[\"None\",\"A week\"]}\n");
        assert_eq!(reply(&AskReply::Cancelled), "{\"answers\":null}\n");
    }

    #[test]
    fn installs_the_extension_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = install(dir.path()).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("ask_user"));
        assert_eq!(install(dir.path()).unwrap(), path);
    }
}
