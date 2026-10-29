use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::atomic::{FileSensitivity, FileWriter};

const STORE_VERSION: u32 = 1;

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub base_url: String,
    pub api_key: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderMode {
    #[default]
    Relay,
    Gpt,
}

#[derive(Serialize, Deserialize)]
pub struct ProfileStore {
    pub version: u32,
    #[serde(default)]
    pub mode: ProviderMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    pub profiles: BTreeMap<String, Profile>,
}

impl Default for ProfileStore {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            mode: ProviderMode::Relay,
            active: None,
            profiles: BTreeMap::new(),
        }
    }
}

impl ProfileStore {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("could not read profile store {}", path.display()));
            }
        };
        if bytes.is_empty() {
            return Ok(Self::default());
        }

        let store: Self = serde_json::from_slice(&bytes)
            .with_context(|| format!("profile store is invalid JSON: {}", path.display()))?;
        if store.version != STORE_VERSION {
            bail!(
                "unsupported profile store version {} in {}",
                store.version,
                path.display()
            );
        }
        Ok(store)
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut bytes = serde_json::to_vec_pretty(self).context("could not serialize profiles")?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    pub fn save(&self, path: &Path, writer: &impl FileWriter) -> Result<()> {
        writer.write(path, &self.encode()?, FileSensitivity::Secret)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{Profile, ProfileStore, ProviderMode};

    #[test]
    fn versioned_store_round_trips_special_key_characters() {
        let store = ProfileStore {
            version: 1,
            mode: ProviderMode::Relay,
            active: Some("work".into()),
            profiles: BTreeMap::from([(
                "work".into(),
                Profile {
                    base_url: "https://example.test/v1".into(),
                    api_key: "quote\"slash\\key".into(),
                },
            )]),
        };
        let decoded: ProfileStore = serde_json::from_slice(&store.encode().unwrap()).unwrap();
        assert_eq!(decoded.active.as_deref(), Some("work"));
        assert_eq!(decoded.profiles["work"].api_key, "quote\"slash\\key");
    }
}
