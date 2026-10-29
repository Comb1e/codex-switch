use anyhow::{Result, bail};
use url::Url;

const RESERVED_NAMES: &[&str] = &[
    "add", "gpt", "list", "status", "current", "remove", "help", "version",
];

pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 {
        bail!("profile name must contain between 1 and 64 characters");
    }
    if RESERVED_NAMES.contains(&name) {
        bail!("profile name '{name}' is reserved");
    }

    let mut characters = name.chars();
    let first = characters.next().expect("name is not empty");
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        bail!("profile name must start with a lowercase letter or digit");
    }
    if !characters.all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '.' | '_' | '-')
    }) {
        bail!("profile name may contain only lowercase letters, digits, '.', '_' and '-'");
    }
    Ok(())
}

pub fn validate_base_url(base_url: &str) -> Result<()> {
    if base_url.trim() != base_url {
        bail!("base URL must not have leading or trailing whitespace");
    }
    let parsed = Url::parse(base_url).map_err(|_| anyhow::anyhow!("base URL is not valid"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        bail!("base URL scheme must be http or https");
    }
    if parsed.host_str().is_none() {
        bail!("base URL must include a host");
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        bail!("base URL must not contain credentials");
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        bail!("base URL must not contain a query string or fragment");
    }
    Ok(())
}

pub fn validate_api_key(api_key: &str) -> Result<()> {
    if api_key.is_empty() {
        bail!("API key must not be empty");
    }
    if api_key.chars().any(char::is_control) {
        bail!("API key must not contain control characters");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{validate_api_key, validate_base_url, validate_name};

    #[test]
    fn validates_profile_names() {
        for name in ["work", "proxy-1", "team_api", "v1.2", "status-1"] {
            validate_name(name).unwrap();
        }
        for name in ["", "Work", "bad name", "-bad", "list", "status", "gpt"] {
            assert!(
                validate_name(name).is_err(),
                "accepted invalid name: {name}"
            );
        }
    }

    #[test]
    fn validates_base_urls() {
        for url in ["https://api.example.test/v1", "http://127.0.0.1:8080/"] {
            validate_base_url(url).unwrap();
        }
        for url in [
            "example.test/v1",
            "ftp://example.test",
            "https://user:pass@example.test",
            "https://example.test?v=1",
            "https://example.test/#part",
        ] {
            assert!(
                validate_base_url(url).is_err(),
                "accepted invalid URL: {url}"
            );
        }
    }

    #[test]
    fn validates_api_keys_without_assuming_a_vendor_prefix() {
        validate_api_key("custom-key_123").unwrap();
        assert!(validate_api_key("").is_err());
        assert!(validate_api_key("line\nbreak").is_err());
    }
}
