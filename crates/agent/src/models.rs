//! `models.json` and the defaults in `settings.json`: which models there
//! are, and which one new sessions start with. Plain functions from text
//! to text; the caller reads and writes the files.

use protocol::{Available, Defaults, Model, Models, Provider};
use serde_json::Value;

use crate::error::Error;
use crate::json;

/// The models a live session can switch to, from Pi's
/// `get_available_models`: every model it has a key or login for.
pub fn available(data: &Value) -> Vec<Available> {
    data["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|model| {
            let id = model["id"].as_str()?.to_string();
            Some(Available {
                provider: model["provider"].as_str()?.to_string(),
                name: model["name"].as_str().unwrap_or(&id).to_string(),
                id,
                reasoning: model["reasoning"].as_bool().unwrap_or(false),
                context_window: model["contextWindow"].as_u64(),
            })
        })
        .collect()
}

pub fn list(models_text: &str, settings_text: &str) -> Result<Models, Error> {
    let models = json::object(models_text, "models.json")?;
    let settings = json::object(settings_text, "settings.json")?;
    let providers = match models.get("providers") {
        None => Vec::new(),
        Some(Value::Object(providers)) => providers
            .iter()
            .map(|(id, p)| provider(id, p))
            .collect::<Result<_, _>>()?,
        Some(_) => return Err(Error::invalid("models.json", "providers isn't an object")),
    };
    let text = |key: &str| {
        settings
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let enabled = settings
        .get("enabledModels")
        .and_then(Value::as_array)
        .map(|patterns| {
            patterns
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Ok(Models {
        providers,
        defaults: Defaults {
            provider: text("defaultProvider"),
            model: text("defaultModel"),
            thinking: text("defaultThinkingLevel"),
        },
        enabled,
    })
}

fn provider(id: &str, entry: &Value) -> Result<Provider, Error> {
    let entry = entry
        .as_object()
        .ok_or_else(|| Error::invalid("models.json", format!("provider {id} isn't an object")))?;
    let text = |key: &str| entry.get(key).and_then(Value::as_str).map(str::to_string);
    let models = entry
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(model)
        .collect();
    Ok(Provider {
        id: id.to_string(),
        name: text("name"),
        base_url: text("baseUrl"),
        models,
    })
}

fn model(entry: &Value) -> Option<Model> {
    Some(Model {
        id: entry.get("id")?.as_str()?.to_string(),
        name: entry
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string),
        reasoning: entry
            .get("reasoning")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        context_window: entry
            .get("contextWindow")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok()),
    })
}

/// `settings.json` with a new default model, everything else kept.
pub fn set_default(settings_text: &str, provider: &str, model: &str) -> Result<String, Error> {
    let mut settings = json::object(settings_text, "settings.json")?;
    settings.insert("defaultProvider".into(), Value::String(provider.into()));
    settings.insert("defaultModel".into(), Value::String(model.into()));
    Ok(json::pretty(&settings))
}

/// `settings.json` with a new default thinking level, everything else kept.
pub fn set_thinking(settings_text: &str, level: &str) -> Result<String, Error> {
    let mut settings = json::object(settings_text, "settings.json")?;
    settings.insert("defaultThinkingLevel".into(), Value::String(level.into()));
    Ok(json::pretty(&settings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_available_models() {
        let data = serde_json::json!({ "models": [
            { "id": "sonnet", "name": "Claude Sonnet", "provider": "anthropic", "reasoning": true, "contextWindow": 200000 },
            { "id": "qwen", "provider": "local" },
            { "name": "no id" },
        ]});
        let models = available(&data);
        assert_eq!(models.len(), 2);
        assert_eq!(models[1].name, "qwen");
        assert_eq!(models[0].context_window, Some(200_000));
    }

    const MODELS: &str = r#"{
  "providers": {
    "local": {
      "name": "My proxy",
      "baseUrl": "http://localhost:4000/v1",
      "api": "openai-completions",
      "models": [
        { "id": "claude-opus-5-5", "name": "Claude Opus 5.5", "reasoning": true, "contextWindow": 200000 },
        { "id": "gpt-5" },
        { "name": "no id, skipped" }
      ]
    }
  }
}"#;

    const SETTINGS: &str = r#"{
  "theme": "dark",
  "defaultProvider": "local",
  "defaultModel": "gpt-5",
  "enabledModels": ["local/*"]
}"#;

    #[test]
    fn reads_providers_models_and_defaults() {
        let models = list(MODELS, SETTINGS).unwrap();
        let local = &models.providers[0];
        assert_eq!(local.id, "local");
        assert_eq!(local.base_url.as_deref(), Some("http://localhost:4000/v1"));
        assert_eq!(local.models.len(), 2);
        assert!(local.models[0].reasoning);
        assert_eq!(local.models[0].context_window, Some(200_000));
        assert_eq!(local.models[1].name, None);
        assert_eq!(models.defaults.model.as_deref(), Some("gpt-5"));
        assert_eq!(models.defaults.thinking, None);
        assert_eq!(models.enabled, ["local/*"]);
    }

    #[test]
    fn no_files_means_nothing_set() {
        let models = list("", "").unwrap();
        assert!(models.providers.is_empty());
        assert_eq!(models.defaults, Defaults::default());
    }

    #[test]
    fn setting_defaults_keeps_other_settings() {
        let text = set_default(SETTINGS, "local", "claude-opus-5-5").unwrap();
        let text = set_thinking(&text, "high").unwrap();
        let models = list(MODELS, &text).unwrap();
        assert_eq!(models.defaults.model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(models.defaults.thinking.as_deref(), Some("high"));
        assert!(text.starts_with("{\n  \"theme\": \"dark\""));
    }

    #[test]
    fn says_which_file_is_broken() {
        let err = list(MODELS, "{ not json").unwrap_err();
        assert!(err.to_string().starts_with("settings.json isn't valid"));
    }
}
