use adk_rust::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

/// Parameters for `web_fetch`
#[derive(JsonSchema, Serialize, Deserialize, Debug)]
pub struct WebFetchParams {
    /// The HTTP or HTTPS URL of the web page to fetch (e.g. 'https://example.com/article')
    pub url: String,
    /// Maximum character length of content to return (optional; default 10000, max 50000)
    pub max_length: Option<usize>,
    /// Return raw HTML if true; default is false (converts HTML to plain readable text)
    pub raw_html: Option<bool>,
}

/// Converts raw HTML string into clean, readable plain text by removing tags, script/style blocks,
/// decoding common HTML entities, and normalizing whitespace.
pub fn html_to_text(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut current_tag = String::new();
    let mut in_script = false;
    let mut in_style = false;

    let chars: Vec<char> = html.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let c = chars[i];
        if c == '<' {
            current_tag.clear();
            i += 1;
            // Read tag name
            while i < len && chars[i] != '>' && !chars[i].is_whitespace() {
                current_tag.push(chars[i].to_ascii_lowercase());
                i += 1;
            }
            // Skip rest of tag until '>'
            while i < len && chars[i] != '>' {
                i += 1;
            }
            if i < len && chars[i] == '>' {
                i += 1;
            }

            if current_tag == "script" {
                in_script = true;
            } else if current_tag == "/script" {
                in_script = false;
            } else if current_tag == "style" {
                in_style = true;
            } else if current_tag == "/style" {
                in_style = false;
            } else if matches!(
                current_tag.as_str(),
                "p" | "/p"
                    | "div"
                    | "/div"
                    | "br"
                    | "h1"
                    | "/h1"
                    | "h2"
                    | "/h2"
                    | "h3"
                    | "/h3"
                    | "h4"
                    | "/h4"
                    | "h5"
                    | "/h5"
                    | "h6"
                    | "/h6"
                    | "li"
                    | "/li"
                    | "tr"
                    | "/tr"
                    | "section"
                    | "/section"
                    | "article"
                    | "/article"
            ) {
                result.push('\n');
            }
            continue;
        }

        if in_script || in_style {
            i += 1;
            continue;
        }

        if c == '&' {
            let mut entity = String::new();
            let mut j = i + 1;
            while j < len && j < i + 10 && chars[j] != ';' && !chars[j].is_whitespace() {
                entity.push(chars[j]);
                j += 1;
            }
            if j < len && chars[j] == ';' {
                let decoded = match entity.as_str() {
                    "nbsp" => Some(' '),
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" | "#39" | "#x27" => Some('\''),
                    _ => None,
                };
                if let Some(ch) = decoded {
                    result.push(ch);
                    i = j + 1;
                    continue;
                }
            }
        }

        result.push(c);
        i += 1;
    }

    let mut clean_lines = Vec::new();
    for line in result.lines() {
        let trimmed = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if !trimmed.is_empty() {
            clean_lines.push(trimmed);
        }
    }

    clean_lines.join("\n\n")
}

/// Tool for fetching content from a web page URL over HTTP/HTTPS.
pub fn create_web_fetch_tool() -> Arc<dyn Tool> {
    let tool = FunctionTool::new(
        "web_fetch",
        "Fetch content from a web page URL over HTTP/HTTPS. By default converts HTML to plain text. Returns the web page content and metadata.",
        |_ctx, args| async move {
            let params: WebFetchParams = match serde_json::from_value(args) {
                Ok(p) => p,
                Err(e) => return Ok(json!({ "error": format!("Invalid arguments: {e}") })),
            };

            let url = params.url.trim();
            if url.is_empty() {
                return Ok(json!({ "error": "Missing required argument: url" }));
            }

            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Ok(json!({
                    "error": format!("Invalid URL scheme for '{url}' — only 'http://' and 'https://' URLs are supported.")
                }));
            }

            let max_length = params.max_length.unwrap_or(10_000).clamp(100, 50_000);
            let raw_html = params.raw_html.unwrap_or(false);

            let client = match reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) NamiMini/1.0")
                .build()
            {
                Ok(c) => c,
                Err(e) => return Ok(json!({ "error": format!("Failed to create HTTP client: {e}") })),
            };

            match client.get(url).send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if !resp.status().is_success() {
                        return Ok(json!({
                            "url": url,
                            "status": status,
                            "error": format!("HTTP request failed with status code {status}")
                        }));
                    }

                    match resp.text().await {
                        Ok(body) => {
                            let extracted = if raw_html {
                                body
                            } else {
                                html_to_text(&body)
                            };

                            let char_count = extracted.chars().count();
                            let truncated = char_count > max_length;
                            let content: String = if truncated {
                                extracted.chars().take(max_length).collect::<String>() + "\n...[Truncated]"
                            } else {
                                extracted
                            };

                            Ok(json!({
                                "url": url,
                                "status": status,
                                "content": content,
                                "content_length": content.len(),
                                "truncated": truncated
                            }))
                        }
                        Err(e) => Ok(json!({
                            "url": url,
                            "status": status,
                            "error": format!("Failed to read response body: {e}")
                        })),
                    }
                }
                Err(e) => Ok(json!({
                    "url": url,
                    "error": format!("Network error fetching URL: {e}")
                })),
            }
        },
    )
    .with_parameters_schema::<WebFetchParams>();

    Arc::new(tool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_html_to_text_basic() {
        let html = "<html><head><style>body { color: red; }</style></head><body><h1>Title</h1><p>Hello &amp; <b>World</b></p><script>console.log('test');</script></body></html>";
        let text = html_to_text(html);
        assert_eq!(text, "Title\n\nHello & World");
    }

    #[test]
    fn test_html_to_text_strips_script_and_style_with_attributes() {
        let html = r#"<script type="text/javascript">var a = 1;</script><p>Text</p><style media="screen">p { margin: 0; }</style>"#;
        let text = html_to_text(html);
        assert_eq!(text, "Text");
    }

    #[test]
    fn test_web_fetch_tool_rejects_non_http() {
        let tool = create_web_fetch_tool();
        assert_eq!(tool.name(), "web_fetch");
    }
}
