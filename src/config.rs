use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub default_provider: Option<String>,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderConfig {
    #[serde(rename = "type")]
    pub type_name: String,
    pub api_key_env: Option<String>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
}

impl AppConfig {
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        Self::load_with_source(path).map(|(config, _)| config)
    }

    pub fn load_with_source(path: Option<&Path>) -> Result<(Self, Option<PathBuf>), ConfigError> {
        if let Some(path) = path {
            return Self::load_from_path(path).map(|config| (config, Some(path.to_path_buf())));
        }

        if let Some(path) = std::env::var_os("AICLI_CONFIG").map(PathBuf::from) {
            return Self::load_from_path(&path).map(|config| (config, Some(path)));
        }

        for path in default_config_paths() {
            if path.exists() {
                return Self::load_from_path(&path).map(|config| (config, Some(path)));
            }
        }

        Ok((Self::default(), None))
    }

    pub fn load_from_path(path: &Path) -> Result<Self, ConfigError> {
        let content = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        toml::from_str(&content).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })
    }

    pub fn provider(&self, name: &str) -> Option<&ProviderConfig> {
        self.providers.get(name)
    }
}

pub fn default_config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    paths.push(PathBuf::from("config.toml"));

    if let Some(config_dir) = dirs::config_dir() {
        paths.push(config_dir.join("aicli/config.toml"));
    }

    if let Some(home_dir) = dirs::home_dir() {
        paths.push(home_dir.join(".aicli/config.toml"));
    }

    paths
}

pub fn config_template() -> &'static str {
    r#"# aicli configuration
# Save as one of:
# - ./config.toml
# - ~/.config/aicli/config.toml
# - ~/.aicli/config.toml
# Or point AICLI_CONFIG to any config file path.

default_provider = "gemini"

[providers.gemini]
type = "gemini"
api_key_env = "GEMINI_API_KEY"
model = "gemini-2.5-flash"

# Any OpenAI-compatible Chat Completions endpoint.
[providers.openai_compat]
type = "openai_compat"
api_key_env = "OPENAI_COMPAT_API_KEY"
base_url = "https://api.example.com/v1/chat/completions"
model = "example-chat-model"
"#
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config at {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse config at {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_provider_config() {
        let config: AppConfig = toml::from_str(
            r#"
default_provider = "deepseek"

[providers.deepseek]
type = "openai_compat"
api_key_env = "DEEPSEEK_API_KEY"
base_url = "https://api.deepseek.com/v1/chat/completions"
model = "deepseek-chat"

[providers.gemini]
type = "gemini"
api_key_env = "GEMINI_API_KEY"
model = "gemini-2.5-flash"
"#,
        )
        .unwrap();

        assert_eq!(config.default_provider.as_deref(), Some("deepseek"));
        assert_eq!(
            config.provider("gemini").unwrap().api_key_env.as_deref(),
            Some("GEMINI_API_KEY")
        );
    }

    #[test]
    fn template_is_valid_toml() {
        let config: AppConfig = toml::from_str(config_template()).unwrap();

        assert_eq!(config.default_provider.as_deref(), Some("gemini"));
        assert!(config.provider("openai_compat").is_some());
    }
}
