use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::models::{CodexStatus, Message};

#[derive(Serialize)]
struct ClaudeRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<ClaudeMessage>,
    system: String,
}

#[derive(Serialize)]
struct ClaudeMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ClaudeResponse {
    content: Vec<ClaudeContent>,
}

#[derive(Deserialize)]
struct ClaudeContent {
    text: Option<String>,
}

const BATCH_SIZE: usize = 20;
const CLASSIFIER_RETRIES: usize = 2;
const CLAUDE_MODEL: &str = "claude-haiku-4-5-20251001";
const OPENAI_MODEL: &str = "gpt-5-mini";
const CODEX_REASONING_OVERRIDE: &str = r#"model_reasoning_effort="high""#;
const CODEX_COMMON_PATHS: [&str; 2] = ["/opt/homebrew/bin/codex", "/usr/local/bin/codex"];

pub async fn classify_messages_claude(
    api_key: &str,
    system_prompt: &str,
    messages: &[Message],
    categories: &[String],
) -> Result<Vec<(String, String)>, String> {
    if messages.is_empty() {
        return Ok(vec![]);
    }

    let mut all_results = Vec::new();
    for chunk in messages.chunks(BATCH_SIZE) {
        let results = classify_batch_claude(api_key, system_prompt, chunk, categories).await?;
        all_results.extend(results);
    }

    Ok(all_results)
}

pub async fn classify_messages_openai(
    api_key: &str,
    system_prompt: &str,
    messages: &[Message],
    categories: &[String],
) -> Result<Vec<(String, String)>, String> {
    if messages.is_empty() {
        return Ok(vec![]);
    }

    let mut all_results = Vec::new();
    for chunk in messages.chunks(BATCH_SIZE) {
        let results = classify_batch_openai(api_key, system_prompt, chunk, categories).await?;
        all_results.extend(results);
    }

    Ok(all_results)
}

pub async fn classify_messages_codex(
    system_prompt: &str,
    messages: &[Message],
    categories: &[String],
) -> Result<Vec<(String, String)>, String> {
    if messages.is_empty() {
        return Ok(vec![]);
    }

    let mut all_results = Vec::new();
    for chunk in messages.chunks(BATCH_SIZE) {
        let results = classify_batch_codex(system_prompt, chunk, categories).await?;
        all_results.extend(results);
    }

    Ok(all_results)
}

pub async fn get_codex_status() -> CodexStatus {
    let codex_bin = resolve_codex_binary().await;
    let version_warning = match Command::new(&codex_bin).arg("--version").output().await {
        Ok(output) => {
            if output.status.success() {
                None
            } else {
                let text = normalize_command_output(&output.stdout, &output.stderr);
                Some(if text.is_empty() {
                    "Codex CLI returned an error when checking version".to_string()
                } else {
                    format!("Codex CLI version check failed: {}", text)
                })
            }
        }
        Err(err) => {
            return CodexStatus {
                installed: false,
                authenticated: false,
                auth_mode: None,
                has_codex_subscription: false,
                message: if err.kind() == io::ErrorKind::NotFound {
                    "Codex CLI is not installed or not visible to Dispatch".to_string()
                } else {
                    format!("Failed to run Codex CLI: {}", err)
                },
            };
        }
    };

    let append_version_warning = |message: String| -> String {
        match version_warning.as_deref() {
            Some(warn) if !warn.is_empty() => format!("{} ({})", message, warn),
            _ => message,
        }
    };

    match Command::new(&codex_bin)
        .arg("-c")
        .arg(CODEX_REASONING_OVERRIDE)
        .arg("login")
        .arg("status")
        .output()
        .await
    {
        Ok(output) => {
            let text = normalize_command_output(&output.stdout, &output.stderr);
            let (authenticated, auth_mode, has_codex_subscription, message) =
                parse_codex_login_status_output(output.status.success(), &text);
            CodexStatus {
                installed: true,
                authenticated,
                auth_mode,
                has_codex_subscription,
                message: append_version_warning(message),
            }
        }
        Err(err) => CodexStatus {
            installed: true,
            authenticated: false,
            auth_mode: None,
            has_codex_subscription: false,
            message: append_version_warning(format!("Failed to read Codex login status: {}", err)),
        },
    }
}

fn parse_codex_login_status_output(
    status_success: bool,
    output: &str,
) -> (bool, Option<String>, bool, String) {
    let trimmed = output.trim();
    if status_success {
        let lower = trimmed.to_ascii_lowercase();
        if lower.contains("chatgpt") {
            return (
                true,
                Some("chatgpt".to_string()),
                true,
                "Logged in with ChatGPT subscription".to_string(),
            );
        }
        if lower.contains("api key") {
            return (
                true,
                Some("api_key".to_string()),
                false,
                "Logged in with OpenAI API key".to_string(),
            );
        }
        return (
            true,
            None,
            false,
            if trimmed.is_empty() {
                "Codex is authenticated".to_string()
            } else {
                trimmed.to_string()
            },
        );
    }

    (
        false,
        None,
        false,
        if trimmed.is_empty() {
            "Not authenticated with Codex".to_string()
        } else {
            trimmed.to_string()
        },
    )
}

async fn classify_batch_claude(
    api_key: &str,
    system_prompt: &str,
    messages: &[Message],
    categories: &[String],
) -> Result<Vec<(String, String)>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;

    let mut headers = HeaderMap::new();
    headers.insert(
        "x-api-key",
        HeaderValue::from_str(api_key).map_err(|e| e.to_string())?,
    );
    headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    let request_body = ClaudeRequest {
        model: CLAUDE_MODEL.to_string(),
        max_tokens: max_output_tokens(messages),
        system: system_prompt.to_string(),
        messages: vec![ClaudeMessage {
            role: "user".to_string(),
            content: build_user_content(messages, categories),
        }],
    };

    let mut attempt = 0usize;
    let claude_response: ClaudeResponse = loop {
        let response = client
            .post("https://api.anthropic.com/v1/messages")
            .headers(headers.clone())
            .json(&request_body)
            .send()
            .await;

        match response {
            Ok(resp) => {
                let status = resp.status();
                if (status.as_u16() == 429 || status.is_server_error())
                    && attempt < CLASSIFIER_RETRIES
                {
                    let delay_ms = 250u64.saturating_mul(1u64 << attempt.min(4));
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    attempt += 1;
                    continue;
                }
                if !status.is_success() {
                    let body = resp.text().await.unwrap_or_default();
                    return Err(format!("Claude API error ({}): {}", status, body));
                }

                let parsed: ClaudeResponse = resp
                    .json()
                    .await
                    .map_err(|e| format!("Failed to parse Claude response: {}", e))?;
                break parsed;
            }
            Err(e) => {
                let retryable = e.is_timeout() || e.is_connect() || e.is_request();
                if retryable && attempt < CLASSIFIER_RETRIES {
                    let delay_ms = 250u64.saturating_mul(1u64 << attempt.min(4));
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    attempt += 1;
                    continue;
                }
                return Err(format!("Claude API request failed: {}", e));
            }
        }
    };

    let text = claude_response
        .content
        .first()
        .and_then(|c| c.text.as_ref())
        .ok_or_else(|| "Empty response from Claude".to_string())?;

    parse_classifications(text, categories)
}

async fn classify_batch_openai(
    api_key: &str,
    system_prompt: &str,
    messages: &[Message],
    categories: &[String],
) -> Result<Vec<(String, String)>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;

    let request_body = json!({
        "model": OPENAI_MODEL,
        "instructions": system_prompt,
        "input": [
            {
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": build_user_content(messages, categories)
                    }
                ]
            }
        ],
        "max_output_tokens": max_output_tokens(messages),
        "text": {
            "format": {
                "type": "json_schema",
                "name": "classifications",
                "strict": true,
                "schema": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "classification": {
                                "type": "string",
                                "enum": categories
                            }
                        },
                        "required": ["id", "classification"],
                        "additionalProperties": false
                    }
                }
            }
        }
    });

    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", api_key)).map_err(|e| e.to_string())?,
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    let mut attempt = 0usize;
    let openai_response: Value = loop {
        let response = client
            .post("https://api.openai.com/v1/responses")
            .headers(headers.clone())
            .json(&request_body)
            .send()
            .await;

        match response {
            Ok(resp) => {
                let status = resp.status();
                if (status.as_u16() == 429 || status.is_server_error())
                    && attempt < CLASSIFIER_RETRIES
                {
                    let delay_ms = 250u64.saturating_mul(1u64 << attempt.min(4));
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    attempt += 1;
                    continue;
                }
                if !status.is_success() {
                    let body = resp.text().await.unwrap_or_default();
                    return Err(format!("OpenAI API error ({}): {}", status, body));
                }

                let parsed = resp
                    .json::<Value>()
                    .await
                    .map_err(|e| format!("Failed to parse OpenAI response: {}", e))?;
                break parsed;
            }
            Err(e) => {
                let retryable = e.is_timeout() || e.is_connect() || e.is_request();
                if retryable && attempt < CLASSIFIER_RETRIES {
                    let delay_ms = 250u64.saturating_mul(1u64 << attempt.min(4));
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    attempt += 1;
                    continue;
                }
                return Err(format!("OpenAI API request failed: {}", e));
            }
        }
    };

    let text = extract_openai_output_text(&openai_response)
        .ok_or_else(|| "Empty response from OpenAI".to_string())?;
    parse_classifications(&text, categories)
}

async fn classify_batch_codex(
    system_prompt: &str,
    messages: &[Message],
    categories: &[String],
) -> Result<Vec<(String, String)>, String> {
    let prompt = format!(
        "{system_prompt}\n\n{}\n\n{}",
        "Return ONLY a JSON array: [{\"id\":\"...\",\"classification\":\"...\"}, ...].",
        build_user_content(messages, categories)
    );

    let text = run_codex_exec(&prompt).await?;
    parse_classifications(&text, categories)
}

async fn run_codex_exec(prompt: &str) -> Result<String, String> {
    let codex_bin = resolve_codex_binary().await;
    let mut child = Command::new(&codex_bin)
        .arg("-c")
        .arg(CODEX_REASONING_OVERRIDE)
        .arg("exec")
        .arg("--json")
        .arg("--skip-git-repo-check")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                "Codex CLI is not installed or not visible to Dispatch".to_string()
            } else {
                format!("Failed to start Codex CLI: {}", e)
            }
        })?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(prompt.as_bytes())
            .await
            .map_err(|e| format!("Failed to write Codex prompt: {}", e))?;
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| format!("Failed to wait for Codex response: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    extract_codex_agent_message_from_jsonl(&stdout).map_err(|e| {
        let stderr_trimmed = stderr.trim();
        if stderr_trimmed.is_empty() {
            format!("Codex classification failed: {}", e)
        } else {
            format!("Codex classification failed: {} ({})", e, stderr_trimmed)
        }
    })
}

fn extract_codex_agent_message_from_jsonl(stdout: &str) -> Result<String, String> {
    let mut agent_message: Option<String> = None;
    let mut last_error: Option<String> = None;

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(msg) = event.get("msg") else {
            continue;
        };
        let Some(msg_type) = msg.get("type").and_then(Value::as_str) else {
            continue;
        };

        if msg_type == "agent_message" {
            if let Some(text) = msg.get("message").and_then(Value::as_str) {
                agent_message = Some(text.to_string());
            }
        } else if msg_type == "error" {
            if let Some(text) = msg.get("message").and_then(Value::as_str) {
                last_error = Some(text.to_string());
            }
        }
    }

    if let Some(text) = agent_message {
        return Ok(text);
    }

    if let Some(err) = last_error {
        return Err(err);
    }

    Err("Codex did not return an agent message".to_string())
}

fn normalize_command_output(stdout: &[u8], stderr: &[u8]) -> String {
    let stdout_text = String::from_utf8_lossy(stdout);
    let stderr_text = String::from_utf8_lossy(stderr);
    let joined = if stdout_text.trim().is_empty() {
        stderr_text.to_string()
    } else if stderr_text.trim().is_empty() {
        stdout_text.to_string()
    } else {
        format!("{}\n{}", stdout_text, stderr_text)
    };
    joined.trim().to_string()
}

fn first_non_empty_line(value: &str) -> Option<String> {
    value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.to_string())
}

async fn resolve_codex_binary() -> String {
    if let Ok(from_env) = std::env::var("CODEX_PATH") {
        let candidate = from_env.trim();
        if !candidate.is_empty() {
            return candidate.to_string();
        }
    }

    for candidate in CODEX_COMMON_PATHS {
        if Path::new(candidate).is_file() {
            return candidate.to_string();
        }
    }

    if let Ok(output) = Command::new("which").arg("codex").output().await {
        if output.status.success() {
            if let Some(path) = first_non_empty_line(&normalize_command_output(&output.stdout, &[]))
            {
                return path;
            }
        }
    }

    for shell in ["zsh", "bash"] {
        if let Ok(output) = Command::new(shell)
            .arg("-lc")
            .arg("command -v codex")
            .output()
            .await
        {
            if output.status.success() {
                if let Some(path) =
                    first_non_empty_line(&normalize_command_output(&output.stdout, &[]))
                {
                    return path;
                }
            }
        }
    }

    "codex".to_string()
}

fn extract_openai_output_text(response: &Value) -> Option<String> {
    if let Some(text) = response.get("output_text").and_then(Value::as_str) {
        if !text.trim().is_empty() {
            return Some(text.to_string());
        }
    }

    let output = response.get("output")?.as_array()?;
    for item in output {
        let Some(content) = item.get("content").and_then(Value::as_array) else {
            continue;
        };
        for block in content {
            if let Some(text) = block.get("text").and_then(Value::as_str) {
                if !text.trim().is_empty() {
                    return Some(text.to_string());
                }
            }
            if let Some(text) = block.get("output_text").and_then(Value::as_str) {
                if !text.trim().is_empty() {
                    return Some(text.to_string());
                }
            }
        }
    }

    None
}

fn build_user_content(messages: &[Message], categories: &[String]) -> String {
    let category_list = categories.join(", ");
    let category_json_example: Vec<String> = categories
        .iter()
        .take(2)
        .map(|c| format!("\"{}\"", c))
        .collect();
    let example_values = category_json_example.join(" or ");

    let mut user_content = format!(
        "Using the classification criteria from your instructions, classify each message below.\n\n\
         Valid categories: {}\n\n\
         Return ONLY a JSON array: [{{\"id\": \"...\", \"classification\": {}}}, ...]\n\n\
         When in doubt, lean toward 'important' if it exists as a category.\n\nMessages:\n",
        category_list, example_values
    );

    for msg in messages {
        user_content.push_str(&format!(
            "\n---\nID: {}\nSource: {}\nSender: {}\nChannel: {}\nBody:\n{}\n",
            msg.id,
            msg.source,
            msg.sender,
            msg.subject.as_deref().unwrap_or("unknown"),
            {
                let end = msg.body.len().min(500);
                let safe_end = msg.body.floor_char_boundary(end);
                &msg.body[..safe_end]
            },
        ));
    }

    user_content
}

fn max_output_tokens(messages: &[Message]) -> u32 {
    (messages.len() as u32 * 80).max(1024)
}

fn parse_classifications(
    text: &str,
    categories: &[String],
) -> Result<Vec<(String, String)>, String> {
    let json_start = text.find('[').unwrap_or(0);
    let json_end = text.rfind(']').map(|i| i + 1).unwrap_or(text.len());
    let json_str = &text[json_start..json_end];

    #[derive(Deserialize)]
    struct Classification {
        id: String,
        classification: String,
    }

    match serde_json::from_str::<Vec<Classification>>(json_str) {
        Ok(classifications) => Ok(classifications
            .into_iter()
            .map(|c| {
                let lower = c.classification.to_lowercase();
                let class = if categories.iter().any(|cat| cat.to_lowercase() == lower) {
                    lower
                } else {
                    "other".to_string()
                };
                (c.id, class)
            })
            .collect()),
        Err(e) => Err(format!(
            "Failed to parse classifier response ({}). Raw: {}",
            e,
            &text[..text.len().min(200)]
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_codex_login_status_chatgpt_mode() {
        let (authenticated, mode, has_subscription, message) =
            parse_codex_login_status_output(true, "Logged in using ChatGPT");
        assert!(authenticated);
        assert_eq!(mode.as_deref(), Some("chatgpt"));
        assert!(has_subscription);
        assert!(message.contains("ChatGPT"));
    }

    #[test]
    fn parse_codex_login_status_api_key_mode() {
        let (authenticated, mode, has_subscription, message) =
            parse_codex_login_status_output(true, "Logged in using OpenAI API key");
        assert!(authenticated);
        assert_eq!(mode.as_deref(), Some("api_key"));
        assert!(!has_subscription);
        assert!(message.contains("API key"));
    }

    #[test]
    fn parse_codex_login_status_error() {
        let (authenticated, mode, has_subscription, message) =
            parse_codex_login_status_output(false, "Error checking login status: No such file");
        assert!(!authenticated);
        assert_eq!(mode, None);
        assert!(!has_subscription);
        assert!(message.contains("Error checking login status"));
    }

    #[test]
    fn parse_codex_jsonl_agent_message() {
        let jsonl = r#"
{"id":"0","msg":{"type":"task_started"}}
{"id":"0","msg":{"type":"agent_message","message":"[{\"id\":\"m1\",\"classification\":\"important\"}]"}}
"#;
        let parsed = extract_codex_agent_message_from_jsonl(jsonl).expect("agent message");
        assert!(parsed.contains("m1"));
    }

    #[test]
    fn parse_codex_jsonl_error() {
        let jsonl = r#"
{"id":"0","msg":{"type":"stream_error","message":"retrying"}}
{"id":"0","msg":{"type":"error","message":"exceeded retry limit, last status: 401 Unauthorized"}}
"#;
        let err = extract_codex_agent_message_from_jsonl(jsonl).expect_err("should fail");
        assert!(err.contains("401 Unauthorized"));
    }
}
