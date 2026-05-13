use crate::config::{AppConfig, ProviderConfig};
use crate::prompt::{system_prompt, user_prompt, RequestContext};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::error::Error as StdError;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GeneratedCommand {
    pub command: String,
    #[serde(default)]
    pub explanation: String,
}

#[async_trait]
pub trait CommandGenerator: Send + Sync {
    async fn generate(
        &self,
        description: &str,
        context: &RequestContext,
    ) -> Result<GeneratedCommand, LlmError>;
}

pub fn create_generator(
    provider_name: &str,
    model_override: Option<String>,
    config: &AppConfig,
    verbose: bool,
) -> Result<Box<dyn CommandGenerator>, LlmError> {
    let provider_config = config.provider(provider_name);
    let type_name = provider_config
        .map(|provider| provider.type_name.as_str())
        .unwrap_or(provider_name);

    match type_name {
        "openai_compat" => Ok(Box::new(OpenAiCompatGenerator::from_config(
            provider_name,
            model_override,
            provider_config,
            verbose,
        )?)),
        "gemini" => Ok(Box::new(GeminiGenerator::from_config(
            provider_name,
            model_override,
            provider_config,
            verbose,
        )?)),
        other => Err(LlmError::Config(format!(
            "unknown provider '{provider_name}' with type '{other}'"
        ))),
    }
}

fn api_key(
    provider: &str,
    config: Option<&ProviderConfig>,
    default_env: &str,
) -> Result<String, LlmError> {
    let env_name = config
        .and_then(|provider| provider.api_key_env.as_deref())
        .unwrap_or(default_env);

    if let Ok(value) = std::env::var(env_name) {
        let value = value.trim().to_string();
        if !value.is_empty() {
            return Ok(value);
        }
    }

    if let Some(value) = config.and_then(|provider| provider.api_key.as_deref()) {
        let value = value.trim().to_string();
        if !value.is_empty() {
            return Ok(value);
        }
    }

    Err(LlmError::Config(format!(
        "missing API key for provider '{provider}'. Set {env_name} or configure api_key."
    )))
}

fn model(
    model_override: Option<String>,
    config: Option<&ProviderConfig>,
    env_name: &str,
    fallback: &str,
) -> String {
    model_override
        .or_else(|| config.and_then(|provider| provider.model.clone()))
        .or_else(|| std::env::var(env_name).ok())
        .unwrap_or_else(|| fallback.to_string())
}

fn openai_compat_base_url(
    provider: &str,
    config: Option<&ProviderConfig>,
) -> Result<String, LlmError> {
    config
        .and_then(|provider| provider.base_url.clone())
        .or_else(|| std::env::var("OPENAI_COMPAT_BASE_URL").ok())
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
        .ok_or_else(|| {
            LlmError::Config(format!(
                "missing base_url for provider '{provider}'. Set OPENAI_COMPAT_BASE_URL or configure base_url."
            ))
        })
}

fn openai_compat_model(
    provider: &str,
    model_override: Option<String>,
    config: Option<&ProviderConfig>,
) -> Result<String, LlmError> {
    model_override
        .or_else(|| config.and_then(|provider| provider.model.clone()))
        .or_else(|| std::env::var("OPENAI_COMPAT_MODEL").ok())
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
        .ok_or_else(|| {
            LlmError::Config(format!(
                "missing model for provider '{provider}'. Pass --model, set OPENAI_COMPAT_MODEL, or configure model."
            ))
        })
}

fn client() -> Client {
    Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(http_timeout_secs()))
        .build()
        .unwrap_or_else(|_| Client::new())
}

fn http_timeout_secs() -> u64 {
    std::env::var("AICLI_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(180)
}

fn network_error(error: reqwest::Error) -> LlmError {
    LlmError::Network(format_reqwest_error(error))
}

fn format_reqwest_error(error: reqwest::Error) -> String {
    let error = error.without_url();
    let mut parts = vec![redact_sensitive(&error.to_string())];
    let mut source = StdError::source(&error);

    while let Some(error) = source {
        let text = redact_sensitive(&error.to_string());
        if !parts.iter().any(|part| part == &text) {
            parts.push(format!("caused by: {text}"));
        }
        source = error.source();
    }

    parts.join("; ")
}

fn log_verbose(verbose: bool, message: impl AsRef<str>) {
    if verbose {
        eprintln!("[aicli] {}", message.as_ref());
    }
}

fn redact_sensitive(input: &str) -> String {
    let mut value = redact_query_secret(input);
    value = redact_url_userinfo(&value);
    value
}

fn redact_query_secret(input: &str) -> String {
    let mut value = input.to_string();
    for marker in ["key=", "api_key=", "access_token=", "token="] {
        let mut search_start = 0;
        while let Some(relative_start) = value[search_start..].find(marker) {
            let secret_start = search_start + relative_start + marker.len();
            let secret_end = value[secret_start..]
                .find(|ch: char| ch == '&' || ch.is_whitespace() || ch == '\'' || ch == '"')
                .map(|offset| secret_start + offset)
                .unwrap_or_else(|| value.len());
            value.replace_range(secret_start..secret_end, "***");
            search_start = secret_start + 3;
        }
    }
    value
}

fn redact_url_userinfo(input: &str) -> String {
    let mut output = input.to_string();
    let mut search_start = 0;

    while let Some(relative_scheme) = output[search_start..].find("://") {
        let authority_start = search_start + relative_scheme + 3;
        let authority_end = output[authority_start..]
            .find(|ch: char| ch == '/' || ch == '?' || ch == '#' || ch.is_whitespace())
            .map(|offset| authority_start + offset)
            .unwrap_or_else(|| output.len());

        if let Some(relative_at) = output[authority_start..authority_end].find('@') {
            let at = authority_start + relative_at;
            output.replace_range(authority_start..at, "***");
            search_start = authority_start + 4;
        } else {
            search_start = authority_end;
        }
    }

    output
}

struct OpenAiCompatGenerator {
    provider_name: String,
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
    verbose: bool,
}

impl OpenAiCompatGenerator {
    fn from_config(
        provider_name: &str,
        model_override: Option<String>,
        config: Option<&ProviderConfig>,
        verbose: bool,
    ) -> Result<Self, LlmError> {
        Ok(Self {
            provider_name: provider_name.to_string(),
            client: client(),
            api_key: api_key(provider_name, config, "OPENAI_COMPAT_API_KEY")?,
            base_url: openai_compat_base_url(provider_name, config)?,
            model: openai_compat_model(provider_name, model_override, config)?,
            verbose,
        })
    }
}

#[async_trait]
impl CommandGenerator for OpenAiCompatGenerator {
    async fn generate(
        &self,
        description: &str,
        context: &RequestContext,
    ) -> Result<GeneratedCommand, LlmError> {
        let request = OpenAiChatRequest {
            model: &self.model,
            temperature: 0.0,
            messages: vec![
                OpenAiMessage {
                    role: "system",
                    content: system_prompt().to_string(),
                },
                OpenAiMessage {
                    role: "user",
                    content: user_prompt(description, context),
                },
            ],
        };

        log_verbose(
            self.verbose,
            format!(
                "provider={} type=openai_compat model={} endpoint={}",
                self.provider_name,
                self.model,
                redact_sensitive(&self.base_url)
            ),
        );
        log_verbose(self.verbose, "sending model request");
        log_verbose(
            self.verbose,
            format!("timeout_secs={}", http_timeout_secs()),
        );
        let response = self
            .client
            .post(&self.base_url)
            .bearer_auth(&self.api_key)
            .json(&request)
            .send()
            .await
            .map_err(network_error)?;

        let status = response.status();
        log_verbose(self.verbose, format!("model response status={status}"));
        let body = response.text().await.map_err(network_error)?;
        log_verbose(
            self.verbose,
            format!("model response body_bytes={}", body.len()),
        );
        if !status.is_success() {
            return Err(LlmError::Api(format!(
                "OpenAI-compatible API returned {status}: {body}"
            )));
        }

        let parsed: OpenAiChatResponse = serde_json::from_str(&body)?;
        let content = parsed
            .choices
            .into_iter()
            .next()
            .map(|choice| choice.message.content)
            .ok_or_else(|| {
                LlmError::Api("OpenAI-compatible response had no choices".to_string())
            })?;

        parse_generated_command(&content)
    }
}

#[derive(Serialize)]
struct OpenAiChatRequest<'a> {
    model: &'a str,
    temperature: f32,
    messages: Vec<OpenAiMessage>,
}

#[derive(Serialize)]
struct OpenAiMessage {
    role: &'static str,
    content: String,
}

#[derive(Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiChoiceMessage,
}

#[derive(Deserialize)]
struct OpenAiChoiceMessage {
    content: String,
}

struct GeminiGenerator {
    provider_name: String,
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
    verbose: bool,
}

impl GeminiGenerator {
    fn from_config(
        provider_name: &str,
        model_override: Option<String>,
        config: Option<&ProviderConfig>,
        verbose: bool,
    ) -> Result<Self, LlmError> {
        Ok(Self {
            provider_name: provider_name.to_string(),
            client: client(),
            api_key: api_key(provider_name, config, "GEMINI_API_KEY")?,
            base_url: config
                .and_then(|provider| provider.base_url.clone())
                .unwrap_or_else(|| "https://generativelanguage.googleapis.com".to_string()),
            model: model(model_override, config, "GEMINI_MODEL", "gemini-2.5-flash"),
            verbose,
        })
    }

    fn endpoint(&self) -> String {
        format!(
            "{}/v1beta/models/{}:generateContent?key={}",
            self.base_url.trim_end_matches('/'),
            self.model,
            self.api_key
        )
    }
}

#[async_trait]
impl CommandGenerator for GeminiGenerator {
    async fn generate(
        &self,
        description: &str,
        context: &RequestContext,
    ) -> Result<GeneratedCommand, LlmError> {
        let request = GeminiRequest {
            system_instruction: GeminiSystemInstruction {
                parts: vec![GeminiPart {
                    text: system_prompt().to_string(),
                }],
            },
            contents: vec![GeminiContent {
                role: "user",
                parts: vec![GeminiPart {
                    text: user_prompt(description, context),
                }],
            }],
            generation_config: GeminiGenerationConfig {
                temperature: 0.0,
                response_mime_type: "application/json",
            },
        };

        let endpoint = self.endpoint();
        log_verbose(
            self.verbose,
            format!(
                "provider={} type=gemini model={} endpoint={}",
                self.provider_name,
                self.model,
                redact_sensitive(&endpoint)
            ),
        );
        log_verbose(self.verbose, "sending model request");
        log_verbose(
            self.verbose,
            format!("timeout_secs={}", http_timeout_secs()),
        );
        let response = self
            .client
            .post(endpoint)
            .json(&request)
            .send()
            .await
            .map_err(network_error)?;
        let status = response.status();
        log_verbose(self.verbose, format!("model response status={status}"));
        let body = response.text().await.map_err(network_error)?;
        log_verbose(
            self.verbose,
            format!("model response body_bytes={}", body.len()),
        );
        if !status.is_success() {
            return Err(LlmError::Api(format!(
                "Gemini API returned {status}: {body}"
            )));
        }

        let parsed: GeminiResponse = serde_json::from_str(&body)?;
        let content = parsed
            .candidates
            .into_iter()
            .next()
            .and_then(|candidate| candidate.content.parts.into_iter().next())
            .map(|part| part.text)
            .ok_or_else(|| LlmError::Api("Gemini response had no text candidate".to_string()))?;

        parse_generated_command(&content)
    }
}

#[derive(Serialize)]
struct GeminiRequest {
    #[serde(rename = "systemInstruction")]
    system_instruction: GeminiSystemInstruction,
    contents: Vec<GeminiContent>,
    #[serde(rename = "generationConfig")]
    generation_config: GeminiGenerationConfig,
}

#[derive(Serialize)]
struct GeminiSystemInstruction {
    parts: Vec<GeminiPart>,
}

#[derive(Serialize)]
struct GeminiContent {
    role: &'static str,
    parts: Vec<GeminiPart>,
}

#[derive(Serialize)]
struct GeminiPart {
    text: String,
}

#[derive(Serialize)]
struct GeminiGenerationConfig {
    temperature: f32,
    #[serde(rename = "responseMimeType")]
    response_mime_type: &'static str,
}

#[derive(Deserialize)]
struct GeminiResponse {
    candidates: Vec<GeminiCandidate>,
}

#[derive(Deserialize)]
struct GeminiCandidate {
    content: GeminiResponseContent,
}

#[derive(Deserialize)]
struct GeminiResponseContent {
    parts: Vec<GeminiResponsePart>,
}

#[derive(Deserialize)]
struct GeminiResponsePart {
    text: String,
}

pub fn parse_generated_command(content: &str) -> Result<GeneratedCommand, LlmError> {
    let cleaned =
        extract_json_object(strip_code_fence(content.trim())).unwrap_or_else(|| content.trim());
    let command: GeneratedCommand = serde_json::from_str(cleaned)?;

    if command.command.trim().is_empty() {
        return Err(LlmError::Api("model returned an empty command".to_string()));
    }

    Ok(GeneratedCommand {
        command: command.command.trim().to_string(),
        explanation: command.explanation.trim().to_string(),
    })
}

fn strip_code_fence(content: &str) -> &str {
    let Some(rest) = content.strip_prefix("```") else {
        return content;
    };

    let rest = rest.trim_start();
    let rest = rest.strip_prefix("json").unwrap_or(rest).trim_start();
    rest.strip_suffix("```").unwrap_or(rest).trim()
}

fn extract_json_object(content: &str) -> Option<&str> {
    let start = content.find('{')?;
    let mut in_string = false;
    let mut escaped = false;
    let mut depth = 0usize;

    for (offset, ch) in content[start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let end = start + offset + ch.len_utf8();
                    return Some(&content[start..end]);
                }
            }
            _ => {}
        }
    }

    None
}

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("api error: {0}")]
    Api(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_json() {
        let generated = parse_generated_command(
            r#"{"command":"find . -type f -size +10M -print","explanation":"Find large files."}"#,
        )
        .unwrap();

        assert_eq!(generated.command, "find . -type f -size +10M -print");
        assert_eq!(generated.explanation, "Find large files.");
    }

    #[test]
    fn parses_fenced_json() {
        let generated = parse_generated_command(
            r#"```json
{"command":"git status --short","explanation":"Show changed files."}
```"#,
        )
        .unwrap();

        assert_eq!(generated.command, "git status --short");
    }

    #[test]
    fn extracts_json_from_compat_text() {
        let generated = parse_generated_command(
            r#"Here is the command:
{"command":"pwd","explanation":"Print the current directory."}
Done."#,
        )
        .unwrap();

        assert_eq!(generated.command, "pwd");
    }
}
