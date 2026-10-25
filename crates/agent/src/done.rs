//! Which scratchpads are marked done. Pi has no such idea, so wb-agent
//! keeps it: a list of session files, in a JSON file of its own.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::json;

use crate::error::Error;

pub struct Done {
    path: PathBuf,
}

impl Done {
    pub fn new(path: PathBuf) -> Self {
        Done { path }
    }

    /// The session files marked done. A missing or unreadable file is none.
    pub fn all(&self) -> BTreeSet<String> {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|value| {
                value["done"].as_array().map(|files| {
                    files
                        .iter()
                        .filter_map(|file| file.as_str().map(str::to_string))
                        .collect()
                })
            })
            .unwrap_or_default()
    }

    pub fn contains(&self, file: &str) -> bool {
        self.all().contains(file)
    }

    pub fn set(&self, file: &str, done: bool) -> Result<(), Error> {
        let mut all = self.all();
        let changed = if done {
            all.insert(file.to_string())
        } else {
            all.remove(file)
        };
        if changed {
            crate::config::write_file(&self.path, &json!({ "done": all }).to_string())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_and_unmarks() {
        let dir = tempfile::tempdir().unwrap();
        let done = Done::new(dir.path().join("state").join("done.json"));
        assert!(!done.contains("/a.jsonl"));
        done.set("/a.jsonl", true).unwrap();
        done.set("/b.jsonl", true).unwrap();
        assert!(done.contains("/a.jsonl"));
        done.set("/a.jsonl", false).unwrap();
        assert_eq!(done.all().into_iter().collect::<Vec<_>>(), ["/b.jsonl"]);
    }
}
