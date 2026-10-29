use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use toml_edit::{DocumentMut, Item, Table, value};

use crate::profile::ProviderMode;
use crate::relay::{PROVIDER_ID, base_url};

const OFFICIAL_PROVIDER_ID: &str = "openai";
const WIRE_API: &str = "responses";
const PROVIDER_NAME: &str = "Codex Switch Relay";
const LEGACY_PROVIDER_ID: &str = "OpenAI";

pub fn prepare(original_config: Option<&[u8]>, mode: ProviderMode) -> Result<Vec<u8>> {
    let mut config = parse_config(original_config)?;
    if mode == ProviderMode::Gpt {
        set_toml_string(&mut config, "model_provider", OFFICIAL_PROVIDER_ID);
        config.remove("openai_base_url");
        config.remove("chatgpt_base_url");
        if let Some(providers) = config
            .get_mut("model_providers")
            .and_then(Item::as_table_like_mut)
        {
            for id in [OFFICIAL_PROVIDER_ID, LEGACY_PROVIDER_ID, PROVIDER_ID] {
                providers.remove(id);
            }
        }
        return Ok(config.to_string().into_bytes());
    }
    set_toml_string(&mut config, "model_provider", PROVIDER_ID);
    set_toml_bool(&mut config, "disable_response_storage", true);
    set_toml_string(&mut config, "network_access", "enabled");
    set_toml_bool(&mut config, "windows_wsl_setup_acknowledged", true);
    config.remove("openai_base_url");

    if let Some(providers) = config
        .get_mut("model_providers")
        .and_then(Item::as_table_like_mut)
    {
        providers.remove(LEGACY_PROVIDER_ID);
    }

    config["model_providers"][PROVIDER_ID] = Item::Table(Table::new());
    config["model_providers"][PROVIDER_ID]["name"] = value(PROVIDER_NAME);
    config["model_providers"][PROVIDER_ID]["base_url"] = value(base_url());
    config["model_providers"][PROVIDER_ID]["wire_api"] = value(WIRE_API);
    config["model_providers"][PROVIDER_ID]["requires_openai_auth"] = value(true);

    config["features"]["goals"] = value(true);
    config["features"]["remote_compaction_v2"] = value(true);
    Ok(config.to_string().into_bytes())
}

pub fn file_matches(config_path: &Path, mode: ProviderMode) -> Result<bool> {
    let config = match fs::read(config_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| format!("could not read {}", config_path.display()));
        }
    };
    Ok(matches_bytes(&config, mode))
}

pub fn matches_bytes(config: &[u8], mode: ProviderMode) -> bool {
    let Ok(document) = std::str::from_utf8(config)
        .ok()
        .and_then(|text| text.parse::<DocumentMut>().ok())
        .ok_or(())
    else {
        return false;
    };

    if mode == ProviderMode::Gpt {
        return toml_string(&document, "model_provider") == Some(OFFICIAL_PROVIDER_ID)
            && document.get("openai_base_url").is_none()
            && document.get("chatgpt_base_url").is_none()
            && document.get("model_providers").is_none_or(|item| {
                item.as_table_like().is_some_and(|providers| {
                    [OFFICIAL_PROVIDER_ID, LEGACY_PROVIDER_ID, PROVIDER_ID]
                        .iter()
                        .all(|id| !providers.contains_key(id))
                })
            });
    }

    toml_string(&document, "model_provider") == Some(PROVIDER_ID)
        && toml_bool(&document, "disable_response_storage") == Some(true)
        && toml_string(&document, "network_access") == Some("enabled")
        && toml_bool(&document, "windows_wsl_setup_acknowledged") == Some(true)
        && document["model_providers"][PROVIDER_ID]["name"].as_str() == Some(PROVIDER_NAME)
        && document["model_providers"][PROVIDER_ID]["base_url"].as_str()
            == Some(base_url().as_str())
        && document["model_providers"][PROVIDER_ID]["wire_api"].as_str() == Some(WIRE_API)
        && document["model_providers"][PROVIDER_ID]["requires_openai_auth"].as_bool() == Some(true)
        && document["features"]["goals"].as_bool() == Some(true)
        && document["features"]["remote_compaction_v2"].as_bool() == Some(true)
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

fn set_toml_bool(document: &mut DocumentMut, key: &str, enabled: bool) {
    if let Some(item) = document.get_mut(key) {
        let decor = item.as_value().map(|existing| existing.decor().clone());
        *item = value(enabled);
        if let (Some(decor), Some(updated)) = (decor, item.as_value_mut()) {
            *updated.decor_mut() = decor;
        }
    } else {
        document[key] = value(enabled);
    }
}

fn toml_string<'a>(document: &'a DocumentMut, key: &str) -> Option<&'a str> {
    document.get(key).and_then(Item::as_str)
}

fn toml_bool(document: &DocumentMut, key: &str) -> Option<bool> {
    document.get(key).and_then(Item::as_bool)
}

#[cfg(test)]
mod tests {
    use super::{matches_bytes, prepare};
    use crate::profile::ProviderMode;

    #[test]
    fn configures_local_relay_and_preserves_unrelated_settings() {
        let config = b"model = \"gpt-test\" # keep\nopenai_base_url = \"https://old.test/v1\"\n[features]\nweb_search = true\n";
        let prepared = prepare(Some(config), ProviderMode::Relay).unwrap();
        let text = String::from_utf8(prepared.clone()).unwrap();
        assert!(text.contains("model = \"gpt-test\" # keep"));
        assert!(text.contains("web_search = true"));
        assert!(!text.contains("openai_base_url"));
        assert!(text.contains("model_provider = \"codex_switch_relay\""));
        assert!(text.contains("name = \"Codex Switch Relay\""));
        assert!(text.contains("base_url = \"http://127.0.0.1:14556/\""));
        assert!(matches_bytes(&prepared, ProviderMode::Relay));
    }

    #[test]
    fn removes_reserved_legacy_provider_and_replaces_conflicting_auth_fields() {
        let config = br#"[model_providers.OpenAI]
base_url = "https://legacy.example.test/v1"

[model_providers.codex_switch_relay]
env_key = "SHOULD_BE_REMOVED"
requires_openai_auth = false
"#;
        let prepared = prepare(Some(config), ProviderMode::Relay).unwrap();
        let text = String::from_utf8(prepared).unwrap();
        assert!(!text.contains("[model_providers.OpenAI]"));
        assert!(!text.contains("legacy.example.test"));
        assert!(!text.contains("SHOULD_BE_REMOVED"));
        assert!(text.contains("requires_openai_auth = true"));
    }

    #[test]
    fn configures_official_provider_and_detects_redirects() {
        use toml_edit::{DocumentMut, value};

        let config = br#"model = "gpt-test" # keep
model_provider = "codex_switch_relay"
openai_base_url = "http://127.0.0.1:14556/"
chatgpt_base_url = "https://proxy.example.test/"
[model_providers.openai]
base_url = "https://old.example.test/v1"
[model_providers.OpenAI]
base_url = "https://legacy.example.test/v1"
[model_providers.codex_switch_relay]
base_url = "http://127.0.0.1:14556/"
[model_providers.other]
base_url = "https://other.example.test/v1"
[features]
web_search = true
"#;
        let prepared = prepare(Some(config), ProviderMode::Gpt).unwrap();
        let text = std::str::from_utf8(&prepared).unwrap();
        assert!(text.contains("model = \"gpt-test\" # keep"));
        assert!(text.contains("web_search = true"));
        assert!(text.contains("https://other.example.test/v1"));
        assert!(!text.contains("127.0.0.1"));
        assert!(!text.contains("proxy.example.test"));
        assert!(matches_bytes(&prepared, ProviderMode::Gpt));
        assert!(!matches_bytes(&prepared, ProviderMode::Relay));
        assert_eq!(
            prepare(Some(&prepared), ProviderMode::Gpt).unwrap(),
            prepared
        );

        for key in ["model_provider", "openai_base_url", "chatgpt_base_url"] {
            let mut changed: DocumentMut = text.parse().unwrap();
            changed[key] = value("other");
            assert!(!matches_bytes(
                changed.to_string().as_bytes(),
                ProviderMode::Gpt
            ));
        }
        for id in ["openai", "OpenAI", "codex_switch_relay"] {
            let mut changed: DocumentMut = text.parse().unwrap();
            changed["model_providers"][id]["base_url"] = value("https://proxy.test/v1");
            assert!(!matches_bytes(
                changed.to_string().as_bytes(),
                ProviderMode::Gpt
            ));
        }
    }

    #[test]
    fn accepts_missing_and_empty_config_and_rejects_malformed_config() {
        for mode in [ProviderMode::Relay, ProviderMode::Gpt] {
            for config in [None, Some(b"".as_slice())] {
                assert!(matches_bytes(&prepare(config, mode).unwrap(), mode));
            }
            for config in [b"not = [toml".as_slice(), b"\xff"] {
                assert!(prepare(Some(config), mode).is_err());
                assert!(!matches_bytes(config, mode));
            }
        }
    }
}
