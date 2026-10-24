//! Pi's JSON files as objects: read leniently (a missing file is empty),
//! written back pretty, in the order keys were written.

use serde_json::{Map, Value};

use crate::error::Error;

pub fn object(text: &str, file: &'static str) -> Result<Map<String, Value>, Error> {
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str(text).map_err(|err| Error::invalid(file, err))? {
        Value::Object(doc) => Ok(doc),
        _ => Err(Error::invalid(file, "it isn't a JSON object")),
    }
}

pub fn pretty(doc: &Map<String, Value>) -> String {
    let mut text = serde_json::to_string_pretty(doc).expect("JSON values serialize");
    text.push('\n');
    text
}
