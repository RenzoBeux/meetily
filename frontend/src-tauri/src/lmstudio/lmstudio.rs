use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::command;
use tokio::time::timeout;

const DEFAULT_ENDPOINT: &str = "http://localhost:1234";

#[derive(Debug, Serialize, Deserialize)]
pub struct LMStudioModel {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
struct LMStudioApiModel {
    id: String,
}

#[derive(Debug, Deserialize)]
struct LMStudioApiResponse {
    data: Vec<LMStudioApiModel>,
}

fn validate_endpoint_url(url: &str) -> Result<(), String> {
    if url.is_empty() {
        return Ok(());
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("URL must start with http:// or https://".to_string());
    }
    Ok(())
}

/// Normalize the endpoint to its base form (without trailing slash or trailing /v1).
fn normalize_base(endpoint: Option<&str>) -> String {
    let raw = endpoint.unwrap_or(DEFAULT_ENDPOINT);
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        trimmed[..trimmed.len() - 3].trim_end_matches('/').to_string()
    } else {
        trimmed.to_string()
    }
}

/// Fetch models currently loaded in LM Studio via its OpenAI-compatible /v1/models endpoint.
#[command]
pub async fn get_lmstudio_models(endpoint: Option<String>) -> Result<Vec<LMStudioModel>, String> {
    if let Some(ref ep) = endpoint {
        validate_endpoint_url(ep)?;
    }

    let base = normalize_base(endpoint.as_deref());
    let url = format!("{}/v1/models", base);

    let client = Client::new();
    let request = client.get(&url).timeout(Duration::from_secs(3)).send();

    let response = match timeout(Duration::from_secs(5), request).await {
        Ok(Ok(resp)) => resp,
        Ok(Err(e)) => {
            if e.is_connect() {
                return Err(format!(
                    "Cannot connect to LM Studio at {}. Make sure the local server is running.",
                    base
                ));
            }
            return Err(format!("Failed to reach LM Studio: {}", e));
        }
        Err(_) => {
            return Err(format!(
                "LM Studio request timed out. Make sure the local server is running at {}.",
                base
            ));
        }
    };

    if !response.status().is_success() {
        return Err(format!(
            "LM Studio returned HTTP {}: {}",
            response.status(),
            response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string())
        ));
    }

    let parsed: LMStudioApiResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse LM Studio response: {}", e))?;

    let models: Vec<LMStudioModel> = parsed
        .data
        .into_iter()
        .map(|m| LMStudioModel {
            name: m.id.clone(),
            id: m.id,
        })
        .collect();

    if models.is_empty() {
        return Err(
            "No models loaded in LM Studio. Load a model in the LM Studio UI first.".to_string(),
        );
    }

    Ok(models)
}
