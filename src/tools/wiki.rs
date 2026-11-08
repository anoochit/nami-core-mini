use adk_memory::{MemoryService, SearchRequest};
use adk_rust::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

/// Identity ที่ `NamiMemory::init` ใช้ index `wiki/*.md` — tool ต้องค้น identity เดียวกัน
/// ไม่เช่นนั้น search จะว่างเปล่าเพราะ memory store แยกข้อมูลตาม (app_name, user_id)
const WIKI_APP_NAME: &str = "nami_app";
const WIKI_USER_ID: &str = "global_wiki";

#[derive(JsonSchema, Serialize, Deserialize)]
pub struct WikiSearchParams {
    /// Search query matched against the indexed wiki documents (e.g. 'compaction trigger')
    pub query: String,
    /// Optional number of results to return (1-10, default 5)
    pub limit: Option<u8>,
}

/// Tool to search the local wiki knowledge base (`wiki/*.md` indexed into long-term
/// memory by `NamiMemory::init` at startup). Read-only: the on-disk `wiki/` directory
/// stays the single source of truth and is re-indexed on every boot.
pub fn create_wiki_search_tool(memory_service: Arc<dyn MemoryService>) -> Arc<dyn Tool> {
    let tool = FunctionTool::new(
        "search_wiki",
        "Search the local wiki knowledge base (Markdown documents indexed from the wiki/ directory at startup). Prefer this over web search for project-specific questions.",
        move |_ctx, args| {
            // Clone per call: the handler must stay `Fn` (reusable) while the returned
            // future owns the shared memory-service handle for its lifetime.
            let memory_service = Arc::clone(&memory_service);
            async move {
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if query.is_empty() {
                    return Ok(json!({ "error": "Missing required argument: query" }));
                }

                // Clamp limit to range 1-10 with a default of 5
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(5)
                    .clamp(1, 10) as usize;

                let request = SearchRequest {
                    query,
                    app_name: WIKI_APP_NAME.to_string(),
                    user_id: WIKI_USER_ID.to_string(),
                    limit: Some(limit),
                    min_score: None,
                    project_id: None,
                };

                match memory_service.search(request).await {
                    Ok(response) => {
                        let results: Vec<Value> = response
                            .memories
                            .iter()
                            .filter_map(|entry| {
                                let text = entry.content.parts.iter().find_map(|p| p.text())?;
                                Some(json!({
                                    "text": text,
                                    "author": entry.author,
                                }))
                            })
                            .collect();
                        Ok(json!({
                            "results": results,
                            "count": results.len(),
                        }))
                    }
                    Err(err) => Ok(json!({
                        "error": format!("Wiki search failed: {}", err),
                    })),
                }
            }
        },
    )
    .with_parameters_schema::<WikiSearchParams>()
    .with_read_only(true);

    Arc::new(tool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use adk_memory::InMemoryMemoryService;

    fn test_service() -> Arc<dyn MemoryService> {
        Arc::new(InMemoryMemoryService::new())
    }

    fn test_ctx() -> Arc<dyn ToolContext> {
        Arc::new(adk_rust::tool::SimpleToolContext::new("test"))
    }

    async fn index_wiki_entry(service: &Arc<dyn MemoryService>, text: &str) {
        use adk_memory::MemoryEntry;
        service
            .add_entry(
                WIKI_APP_NAME,
                WIKI_USER_ID,
                MemoryEntry {
                    content: Content::new("user").with_text(text),
                    author: "wiki_indexer".to_string(),
                    timestamp: chrono::Utc::now(),
                },
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_search_wiki_returns_matching_wiki_entry() {
        let service = test_service();
        index_wiki_entry(
            &service,
            "Wiki [compaction.md]: Compaction trims conversation history every N turns.",
        )
        .await;

        let tool = create_wiki_search_tool(service);
        let res = tool
            .execute(test_ctx(), json!({ "query": "compaction" }))
            .await
            .unwrap();

        assert_eq!(
            res["count"], 1,
            "expected one matched wiki entry: {:?}",
            res
        );
        let text = res["results"][0]["text"].as_str().unwrap();
        assert!(text.contains("compaction.md"), "unexpected text: {text}");
        assert_eq!(res["results"][0]["author"], "wiki_indexer");
    }

    #[tokio::test]
    async fn test_search_wiki_ignores_other_memory_scopes() {
        // Entry stored under a different (app_name, user_id) must not surface —
        // ต้องค้นเฉพาะ scope ที่ wiki indexer ใช้จริง
        use adk_memory::MemoryEntry;
        let service = test_service();
        service
            .add_entry(
                "other_app",
                "other_user",
                MemoryEntry {
                    content: Content::new("user").with_text("Wiki [leak.md]: secret entry"),
                    author: "wiki_indexer".to_string(),
                    timestamp: chrono::Utc::now(),
                },
            )
            .await
            .unwrap();

        let tool = create_wiki_search_tool(service);
        let res = tool
            .execute(test_ctx(), json!({ "query": "secret entry" }))
            .await
            .unwrap();

        assert_eq!(res["count"], 0, "must not return other scopes: {:?}", res);
    }

    #[tokio::test]
    async fn test_search_wiki_missing_query_fails_schema_validation() {
        // The parameters schema rejects missing `query` before the handler runs
        let tool = create_wiki_search_tool(test_service());
        let err = tool.execute(test_ctx(), json!({})).await.unwrap_err();
        assert!(
            format!("{err}").contains("query"),
            "expected schema error about query: {err}"
        );
    }

    #[tokio::test]
    async fn test_search_wiki_limit_is_clamped() {
        let service = test_service();
        for i in 0..8 {
            index_wiki_entry(
                &service,
                &format!("Wiki [doc{i}.md]: README topic_{i} howto"),
            )
            .await;
        }

        let tool = create_wiki_search_tool(service);
        // 99 clamps to 10 — result is bounded by stored entries (8), not the clamp
        let res = tool
            .execute(test_ctx(), json!({ "query": "howto", "limit": 99 }))
            .await
            .unwrap();
        assert_eq!(res["count"], 8, "unexpected count: {:?}", res);

        // negative value fails schema validation before the handler's clamp sees it
        let err = tool
            .execute(test_ctx(), json!({ "query": "howto", "limit": -1 }))
            .await
            .unwrap_err();
        assert!(
            format!("{err}").contains("limit"),
            "expected schema error about limit: {err}"
        );
    }
}
