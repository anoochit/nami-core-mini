use adk_rust::Event;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct NamiApiClient {
    http: reqwest::Client,
    base_url: String,
    app_name: String,
    user_id: String,
    session_id: String,
}

impl NamiApiClient {
    pub async fn connect(base_url: &str, app_name: &str, user_id: &str) -> anyhow::Result<Self> {
        let http = reqwest::Client::new();
        let base_url = base_url.trim_end_matches('/').to_string();
        let health_url = format!("{}/api/health", base_url);

        let resp = http.get(&health_url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!(
                "Health check failed at {}: status {}",
                health_url,
                resp.status()
            );
        }

        let mut client = Self {
            http,
            base_url,
            app_name: app_name.to_string(),
            user_id: user_id.to_string(),
            session_id: String::new(),
        };

        client.new_session().await?;
        Ok(client)
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn app_name(&self) -> &str {
        &self.app_name
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub async fn new_session(&mut self) -> anyhow::Result<()> {
        let url = format!("{}/api/sessions", self.base_url);
        let body = serde_json::json!({
            "appName": self.app_name,
            "userId": self.user_id,
        });

        let resp = self.http.post(&url).json(&body).send().await?;
        if resp.status().is_success()
            && let Ok(json) = resp.json::<serde_json::Value>().await
            && let Some(id) = json.get("id").and_then(|v| v.as_str())
        {
            self.session_id = id.to_string();
            return Ok(());
        }

        self.session_id = Uuid::new_v4().to_string();
        Ok(())
    }

    pub async fn run_stream(&self, text: &str) -> anyhow::Result<reqwest::Response> {
        let url = format!("{}/api/run_sse", self.base_url);
        let body = serde_json::json!({
            "appName": self.app_name,
            "userId": self.user_id,
            "sessionId": self.session_id,
            "newMessage": {
                "role": "user",
                "parts": [
                    { "text": text }
                ]
            },
            "streaming": true
        });

        let resp = self.http.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let err_body = resp.text().await.unwrap_or_default();
            anyhow::bail!("API error ({}): {}", status, err_body);
        }

        Ok(resp)
    }
}

pub struct SseParser {
    buffer: String,
}

impl SseParser {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
        }
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) -> Vec<Event> {
        let text = match std::str::from_utf8(bytes) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        self.buffer.push_str(text);

        let mut events = Vec::new();
        while let Some(pos) = self.buffer.find("\n\n") {
            let frame = self.buffer[..pos].to_string();
            self.buffer.drain(..pos + 2);

            if let Some(event) = parse_frame(&frame) {
                events.push(event);
            }
        }
        events
    }
}

pub fn parse_frame(frame: &str) -> Option<Event> {
    let mut data_lines = Vec::new();
    for line in frame.lines() {
        let trimmed = line.trim_end_matches('\r');
        if trimmed.starts_with(':') {
            // Comment or keep-alive frame
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("data:") {
            let data = rest.strip_prefix(' ').unwrap_or(rest);
            data_lines.push(data);
        }
    }

    if data_lines.is_empty() {
        return None;
    }

    let joined = data_lines.join("\n");
    serde_json::from_str::<Event>(&joined).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frame_comment() {
        let frame = ": keep-alive";
        assert!(parse_frame(frame).is_none());
    }

    #[test]
    fn test_sse_parser_chunks() {
        let mut parser = SseParser::new();
        let mut sample_event = Event::new("inv_123");
        sample_event.author = "test_author".to_string();
        let json = serde_json::to_string(&sample_event).unwrap();

        let full_payload = format!("data: {}\n\n: keep-alive\n\n", json);
        let (chunk1, chunk2) = full_payload.as_bytes().split_at(full_payload.len() / 2);

        let events1 = parser.push_bytes(chunk1);
        assert!(events1.is_empty());

        let events2 = parser.push_bytes(chunk2);
        assert_eq!(events2.len(), 1);
        assert_eq!(events2[0].author, "test_author");
        assert_eq!(events2[0].invocation_id, "inv_123");
    }

    #[test]
    fn test_sse_parser_multiline_data() {
        let mut parser = SseParser::new();
        let mut sample_event = Event::new("inv_456");
        sample_event.author = "test_multi".to_string();
        let json = serde_json::to_string_pretty(&sample_event).unwrap();

        // Format as multi-line data SSE frame
        let mut payload = String::new();
        for line in json.lines() {
            payload.push_str("data: ");
            payload.push_str(line);
            payload.push('\n');
        }
        payload.push('\n');

        let events = parser.push_bytes(payload.as_bytes());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].author, "test_multi");
        assert_eq!(events[0].invocation_id, "inv_456");
    }
}
