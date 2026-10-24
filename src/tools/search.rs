use adk_rust::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(JsonSchema, Serialize, Deserialize)]
pub struct SearchParams {
    /// The search query to send to Google (e.g. 'weather in Bangkok today')
    pub query: String,
    /// Optional number of results to return (1-10, default 5)
    pub num: Option<u8>,
}

/// Tool for Google search via Serper.dev API (requires `SERPER_API_KEY` set in environment).
///
/// Returns only agent-relevant results (`organic`, `answerBox`, `knowledgeGraph`)
/// instead of raw response payloads to minimize token overhead.
pub fn create_search_tool() -> Arc<dyn Tool> {
    let tool = FunctionTool::new(
        "google_search",
        "Search Google for information using the Serper.dev API. Returns web results, answer box, and knowledge graph when available.",
        |_ctx, args| async move {
            let api_key = match std::env::var("SERPER_API_KEY") {
                Ok(key) if !key.is_empty() => key,
                _ => {
                    return Ok(json!({
                        "error": "SERPER_API_KEY not found in environment. Set SERPER_API_KEY in .env to enable Google search."
                    }));
                }
            };

            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if query.is_empty() {
                return Ok(json!({ "error": "Missing required argument: query" }));
            }

            // Clamp num to range 1-10 with a default of 5
            let num = args
                .get("num")
                .and_then(|v| v.as_u64())
                .unwrap_or(5)
                .clamp(1, 10);

            let client = reqwest::Client::new();
            let response = client
                .post("https://google.serper.dev/search")
                .header("X-API-KEY", api_key)
                .header("Content-Type", "application/json")
                .json(&json!({ "q": query, "num": num }))
                .send()
                .await;

            match response {
                Ok(resp) if resp.status().is_success() => match resp.json::<Value>().await {
                    Ok(data) => Ok(json!({
                        "query": query,
                        "answer_box": data.get("answerBox").cloned(),
                        "knowledge_graph": data.get("knowledgeGraph").cloned(),
                        "organic": data.get("organic").cloned().unwrap_or(Value::Array(vec![])),
                    })),
                    Err(err) => Ok(json!({
                        "query": query,
                        "error": format!("Failed to parse search results: {}", err),
                    })),
                },
                Ok(resp) => Ok(json!({
                    "query": query,
                    "error": format!("Serper API returned status: {}", resp.status()),
                })),
                Err(err) => Ok(json!({
                    "query": query,
                    "error": format!("Network error connecting to Serper API: {}", err),
                })),
            }
        },
    )
    .with_parameters_schema::<SearchParams>();

    Arc::new(tool)
}
