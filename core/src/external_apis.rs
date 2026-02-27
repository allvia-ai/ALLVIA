// API Integration Points - Notion & Gmail
// Phase 12: Intrinsic Data Skills - API Research

// =====================================================
// NOTION API INTEGRATION
// =====================================================

/// Notion API Base URL: https://api.notion.com/v1
///
/// Required headers:
/// - Authorization: Bearer {NOTION_API_KEY}
/// - Notion-Version: 2022-06-28
/// - Content-Type: application/json
///
/// Key endpoints for Steer:
/// - Search: POST /search - Find pages/databases by text
/// - Get Page: GET /pages/{page_id} - Get page properties
/// - Get Block Children: GET /blocks/{block_id}/children - Get page content
/// - Query Database: POST /databases/{database_id}/query - Query database rows
/// - Create Page: POST /pages - Create new page in database
/// - Update Page: PATCH /pages/{page_id} - Update page properties
pub struct NotionApi {
    api_key: String,
    base_url: String,
}

impl NotionApi {
    pub fn new(api_key: &str) -> Self {
        Self {
            api_key: api_key.to_string(),
            base_url: "https://api.notion.com/v1".to_string(),
        }
    }

    /// Search for pages matching a query
    pub async fn search(&self, query: &str) -> anyhow::Result<serde_json::Value> {
        let client = reqwest::Client::new();
        let res = client
            .post(format!("{}/search", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Notion-Version", "2022-06-28")
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "query": query,
                "page_size": 10
            }))
            .send()
            .await?
            .json()
            .await?;
        Ok(res)
    }

    /// Get page content (blocks)
    pub async fn get_page_content(&self, block_id: &str) -> anyhow::Result<String> {
        let client = reqwest::Client::new();
        let res: serde_json::Value = client
            .get(format!("{}/blocks/{}/children", self.base_url, block_id))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Notion-Version", "2022-06-28")
            .send()
            .await?
            .json()
            .await?;

        // Extract text from blocks
        let mut text = String::new();
        if let Some(results) = res.get("results").and_then(|r| r.as_array()) {
            for block in results {
                if let Some(paragraph) = block.get("paragraph") {
                    if let Some(rich_text) = paragraph.get("rich_text").and_then(|r| r.as_array()) {
                        for rt in rich_text {
                            if let Some(plain_text) = rt.get("plain_text").and_then(|t| t.as_str())
                            {
                                text.push_str(plain_text);
                                text.push('\n');
                            }
                        }
                    }
                }
            }
        }
        Ok(text)
    }
}

// =====================================================
// GMAIL API INTEGRATION
// =====================================================

/// Gmail API Base URL: https://gmail.googleapis.com/gmail/v1
///
/// Auth: OAuth 2.0 (requires user consent flow)
/// For Steer agent, recommended options:
/// - Service Account (for automated access)
/// - Desktop App OAuth flow via browser
///
/// Key endpoints for Steer:
/// - List Messages: GET /users/me/messages - List message IDs
/// - Get Message: GET /users/me/messages/{id} - Get full message
/// - Send Message: POST /users/me/messages/send - Send email
/// - Search: GET /users/me/messages?q={query} - Search emails
pub struct GmailApi {
    access_token: String,
    base_url: String,
}

impl GmailApi {
    pub fn new(access_token: &str) -> Self {
        Self {
            access_token: access_token.to_string(),
            base_url: "https://gmail.googleapis.com/gmail/v1".to_string(),
        }
    }

    /// Search emails by query
    pub async fn search(&self, query: &str, max_results: u32) -> anyhow::Result<serde_json::Value> {
        let client = reqwest::Client::new();
        let res = client
            .get(format!("{}/users/me/messages", self.base_url))
            .header("Authorization", format!("Bearer {}", self.access_token))
            .query(&[("q", query), ("maxResults", &max_results.to_string())])
            .send()
            .await?
            .json()
            .await?;
        Ok(res)
    }

    /// Get email content by ID
    pub async fn get_message(&self, message_id: &str) -> anyhow::Result<String> {
        let client = reqwest::Client::new();
        let res: serde_json::Value = client
            .get(format!(
                "{}/users/me/messages/{}",
                self.base_url, message_id
            ))
            .header("Authorization", format!("Bearer {}", self.access_token))
            .query(&[("format", "full")])
            .send()
            .await?
            .json()
            .await?;

        // Extract snippet/body
        if let Some(snippet) = res.get("snippet").and_then(|s| s.as_str()) {
            return Ok(snippet.to_string());
        }
        Ok(format!("{:?}", res))
    }
}

// =====================================================
// ENV VARS NEEDED
// =====================================================
// NOTION_API_KEY - Notion integration secret
// GMAIL_ACCESS_TOKEN - OAuth access token (or refresh flow)
// GMAIL_CLIENT_ID - For OAuth flow
// GMAIL_CLIENT_SECRET - For OAuth flow
