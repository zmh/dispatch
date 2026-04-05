use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde::{Deserialize, Serialize};

use crate::models::Message;

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

pub async fn classify_messages(
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
        let results = classify_batch(api_key, system_prompt, chunk, categories).await?;
        all_results.extend(results);
    }

    Ok(all_results)
}

async fn classify_batch(
    api_key: &str,
    system_prompt: &str,
    messages: &[Message],
    categories: &[String],
) -> Result<Vec<(String, String)>, String> {
    let client = reqwest::Client::new();

    let mut headers = HeaderMap::new();
    headers.insert(
        "x-api-key",
        HeaderValue::from_str(api_key).map_err(|e| e.to_string())?,
    );
    headers.insert(
        "anthropic-version",
        HeaderValue::from_static("2023-06-01"),
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

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
                // Find a valid char boundary at or before `end`
                let safe_end = msg.body.floor_char_boundary(end);
                &msg.body[..safe_end]
            },
        ));
    }

    // Scale max_tokens with message count to avoid truncation
    let max_tokens = (messages.len() as u32 * 80).max(1024);

    let request_body = ClaudeRequest {
        model: "claude-haiku-4-5-20251001".to_string(),
        max_tokens,
        system: system_prompt.to_string(),
        messages: vec![ClaudeMessage {
            role: "user".to_string(),
            content: user_content,
        }],
    };

    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .headers(headers)
        .json(&request_body)
        .send()
        .await
        .map_err(|e| format!("Claude API request failed: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Claude API error ({}): {}", status, body));
    }

    let claude_response: ClaudeResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Claude response: {}", e))?;

    let text = claude_response
        .content
        .first()
        .and_then(|c| c.text.as_ref())
        .ok_or_else(|| "Empty response from Claude".to_string())?;

    parse_classifications(text, categories)
}

fn parse_classifications(
    text: &str,
    categories: &[String],
) -> Result<Vec<(String, String)>, String> {
    // Try to find a JSON array in the response
    let json_start = text.find('[').unwrap_or(0);
    let json_end = text.rfind(']').map(|i| i + 1).unwrap_or(text.len());
    let json_str = &text[json_start..json_end];

    #[derive(Deserialize)]
    struct Classification {
        id: String,
        classification: String,
    }

    match serde_json::from_str::<Vec<Classification>>(json_str) {
        Ok(classifications) => {
            Ok(classifications
                .into_iter()
                .map(|c| {
                    let lower = c.classification.to_lowercase();
                    // Check if the returned category is valid
                    let class = if categories.iter().any(|cat| cat.to_lowercase() == lower) {
                        lower
                    } else {
                        "other".to_string()
                    };
                    (c.id, class)
                })
                .collect())
        }
        Err(e) => {
            Err(format!(
                "Failed to parse classifier response ({}). Raw: {}",
                e,
                &text[..text.len().min(200)]
            ))
        }
    }
}
