use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value as JsonValue};
use toml_edit::{DocumentMut, Item, value};

use crate::profile::Profile;

const MODEL_PROVIDER: &str = "openai";
const AUTH_MODE: &str = "apikey";
const CREDENTIAL_STORE: &str = "file";

pub struct PreparedFiles {
    pub config: Vec<u8>,
    pub auth: Vec<u8>,
}

pub fn prepare(
    original_config: Option<&[u8]>,
    original_auth: Option<&[u8]>,
    profile: &Profile,
) -> Result<PreparedFiles> {
    let mut config = parse_config(original_config)?;
    set_toml_string(&mut config, "model_provider", MODEL_PROVIDER);
    set_toml_string(&mut config, "openai_base_url", &profile.base_url);
    set_toml_string(&mut config, "cli_auth_credentials_store", CREDENTIAL_STORE);

    let mut auth = parse_auth(original_auth)?;
    auth.insert("auth_mode".into(), JsonValue::String(AUTH_MODE.into()));
    auth.insert(
        "OPENAI_API_KEY".into(),
        JsonValue::String(profile.api_key.clone()),
    );

    let mut auth_bytes =
        serde_json::to_vec_pretty(&auth).context("could not serialize Codex auth.json")?;
    auth_bytes.push(b'\n');

    Ok(PreparedFiles {
        config: config.to_string().into_bytes(),
        auth: auth_bytes,
    })
}

pub fn files_match(config_path: &Path, auth_path: &Path, profile: &Profile) -> Result<bool> {
    let config = match fs::read(config_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| format!("could not read {}", config_path.display()));
        }
    };
    let auth = match fs::read(auth_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| format!("could not read {}", auth_path.display()));
        }
    };

    Ok(matches_bytes(&config, &auth, profile))
}

pub fn matches_bytes(config: &[u8], auth: &[u8], profile: &Profile) -> bool {
    let Ok(document) = std::str::from_utf8(config)
        .ok()
        .and_then(|text| text.parse::<DocumentMut>().ok())
        .ok_or(())
    else {
        return false;
    };
    let Ok(auth_object) = serde_json::from_slice::<Map<String, JsonValue>>(auth) else {
        return false;
    };

    toml_string(&document, "model_provider") == Some(MODEL_PROVIDER)
        && toml_string(&document, "openai_base_url") == Some(profile.base_url.as_str())
        && toml_string(&document, "cli_auth_credentials_store") == Some(CREDENTIAL_STORE)
        && auth_object.get("auth_mode").and_then(JsonValue::as_str) == Some(AUTH_MODE)
        && auth_object
            .get("OPENAI_API_KEY")
            .and_then(JsonValue::as_str)
            == Some(profile.api_key.as_str())
}

fn parse_config(bytes: Option<&[u8]>) -> Result<DocumentMut> {
    let Some(bytes) = bytes else {
        return Ok(DocumentMut::new());
    };
    if bytes.is_empty() {
        return Ok(DocumentMut::new());
    }
    let text = std::str::from_utf8(bytes).context("Codex config.toml is not valid UTF-8")?;
    text.parse::<DocumentMut>()
        .context("Codex config.toml is not valid TOML")
}

fn parse_auth(bytes: Option<&[u8]>) -> Result<Map<String, JsonValue>> {
    let Some(bytes) = bytes else {
        return Ok(Map::new());
    };
    if bytes.is_empty() {
        return Ok(Map::new());
    }
    let value: JsonValue =
        serde_json::from_slice(bytes).context("Codex auth.json is not valid JSON")?;
    match value {
        JsonValue::Object(object) => Ok(object),
        _ => bail!("Codex auth.json must contain a JSON object"),
    }
}

fn set_toml_string(document: &mut DocumentMut, key: &str, text: &str) {
    if let Some(item) = document.get_mut(key) {
        let decor = item.as_value().map(|existing| existing.decor().clone());
        *item = value(text);
        if let (Some(decor), Some(updated)) = (decor, item.as_value_mut()) {
            *updated.decor_mut() = decor;
        }
    } else {
        document[key] = value(text);
    }
}

fn toml_string<'a>(document: &'a DocumentMut, key: &str) -> Option<&'a str> {
    document.get(key).and_then(Item::as_str)
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::{matches_bytes, prepare};
    use crate::profile::Profile;

    fn profile() -> Profile {
        Profile {
            base_url: "https://proxy.example.test/v1".into(),
            api_key: "key-\"quoted\\value".into(),
        }
    }

    #[test]
    fn preserves_unrelated_toml_and_inline_comments() {
        let config = b"model = \"gpt-test\" # keep\nmodel_provider = \"old\" # provider\n[features]\nweb_search = true\n";
        let prepared = prepare(Some(config), None, &profile()).unwrap();
        let text = String::from_utf8(prepared.config).unwrap();
        assert!(text.contains("model = \"gpt-test\" # keep"));
        assert!(text.contains("model_provider = \"openai\" # provider"));
        assert!(text.contains("[features]\nweb_search = true"));
        assert!(text.contains("openai_base_url = \"https://proxy.example.test/v1\""));
    }

    #[test]
    fn preserves_unknown_auth_fields_and_escapes_keys() {
        let auth = br#"{"tokens":{"access_token":"keep"},"other":42}"#;
        let prepared = prepare(None, Some(auth), &profile()).unwrap();
        let json: Value = serde_json::from_slice(&prepared.auth).unwrap();
        assert_eq!(json["tokens"]["access_token"], "keep");
        assert_eq!(json["other"], 42);
        assert_eq!(json["OPENAI_API_KEY"], "key-\"quoted\\value");
        assert!(matches_bytes(&prepared.config, &prepared.auth, &profile()));
    }

    #[test]
    fn accepts_missing_and_empty_files() {
        prepare(None, None, &profile()).unwrap();
        prepare(Some(b""), Some(b""), &profile()).unwrap();
    }

    #[test]
    fn rejects_malformed_files() {
        assert!(prepare(Some(b"not = [toml"), None, &profile()).is_err());
        assert!(prepare(None, Some(b"{"), &profile()).is_err());
        assert!(prepare(None, Some(b"[]"), &profile()).is_err());
    }
}
