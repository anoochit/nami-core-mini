use adk_rust::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

#[derive(JsonSchema, Serialize, Deserialize)]
pub struct WeatherParams {
    /// City or location name for weather forecast (e.g. 'Bangkok', 'Tokyo', 'London')
    pub location: String,
}

/// Tool สำหรับดึงข้อมูลสภาพอากาศแบบเรียลไทม์จาก wttr.in
pub fn create_weather_tool() -> Arc<dyn Tool> {
    let tool = FunctionTool::new(
        "get_weather_forecast",
        "Get current weather and forecast for a given location using wttr.in service",
        |_ctx, args| async move {
            let location = args
                .get("location")
                .and_then(|v| v.as_str())
                .unwrap_or("Bangkok");

            let url = format!("https://wttr.in/{}?format=4", location);
            match reqwest::get(&url).await {
                Ok(response) => {
                    if response.status().is_success() {
                        let report = response.text().await.unwrap_or_default();
                        Ok(json!({
                            "location": location,
                            "report": report.trim(),
                            "source": "wttr.in"
                        }))
                    } else {
                        Ok(json!({
                            "location": location,
                            "error": format!("Failed to fetch weather from wttr.in (status: {})", response.status())
                        }))
                    }
                }
                Err(err) => Ok(json!({
                    "location": location,
                    "error": format!("Network error connecting to wttr.in: {}", err)
                })),
            }
        },
    )
    .with_parameters_schema::<WeatherParams>();

    Arc::new(tool)
}
