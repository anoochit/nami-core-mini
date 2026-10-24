use adk_rust::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

#[derive(JsonSchema, Serialize, Deserialize)]
pub struct DatetimeParams {
    /// Optional timezone (e.g., 'UTC' or 'Local')
    pub timezone: Option<String>,
}

/// Tool to get current date and time (Local & UTC)
pub fn create_datetime_tool() -> Arc<dyn Tool> {
    let tool = FunctionTool::new(
        "get_datetime",
        "Get current date, time, and timestamp",
        |_ctx, args| async move {
            let timezone = args
                .get("timezone")
                .and_then(|v| v.as_str())
                .unwrap_or("local");
            let now_utc = chrono::Utc::now().to_rfc3339();
            let now_local = chrono::Local::now().to_rfc3339();
            Ok(json!({
                "datetime_utc": now_utc,
                "datetime_local": now_local,
                "timezone_selected": timezone,
                "timestamp": chrono::Utc::now().timestamp()
            }))
        },
    )
    .with_parameters_schema::<DatetimeParams>();

    Arc::new(tool)
}