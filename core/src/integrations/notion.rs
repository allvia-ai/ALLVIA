use anyhow::Result;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Clone, Deserialize)]
pub struct NotionPage {
    pub id: String,
    pub title: String,
    pub content: String,
}

pub struct NotionClient {
    token: String,
    client: Client,
}

impl NotionClient {
    pub fn new(token: &str) -> Self {
        Self {
            token: token.to_string(),
            client: Client::new(),
        }
    }

    pub fn from_env() -> Result<Self> {
        crate::load_env_with_fallback();
        let token = std::env::var("NOTION_API_KEY")
            .map_err(|_| anyhow::anyhow!("NOTION_API_KEY not set"))?;
        Ok(Self::new(&token))
    }

    /// Create a new page in a database
    pub async fn create_page(
        &self,
        database_id: &str,
        title: &str,
        content: &str,
    ) -> Result<String> {
        let url = "https://api.notion.com/v1/pages";

        let body = json!({
            "parent": { "database_id": database_id },
            "properties": {
                "이름": {
                    "title": [{ "text": { "content": title } }]
                }
            },
            "children": [
                {
                    "object": "block",
                    "type": "paragraph",
                    "paragraph": {
                        "rich_text": [{ "type": "text", "text": { "content": content } }]
                    }
                }
            ]
        });

        let resp = self
            .client
            .post(url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Notion-Version", "2022-06-28")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            return Err(anyhow::anyhow!("Notion API Error: {}", err));
        }

        let resp_json: serde_json::Value = resp.json().await?;
        let page_id = resp_json["id"].as_str().unwrap_or("unknown").to_string();
        Ok(page_id)
    }

    /// [Phase 28] Read a page's content by ID
    pub async fn read_page(&self, page_id: &str) -> Result<NotionPage> {
        // 1. Get page metadata
        let page_url = format!("https://api.notion.com/v1/pages/{}", page_id);
        let page_resp = self
            .client
            .get(&page_url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Notion-Version", "2022-06-28")
            .send()
            .await?;

        if !page_resp.status().is_success() {
            let err = page_resp.text().await?;
            return Err(anyhow::anyhow!("Notion Page Error: {}", err));
        }

        let page_json: serde_json::Value = page_resp.json().await?;

        // Extract title from properties
        let title = page_json["properties"]["이름"]["title"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|t| t["text"]["content"].as_str())
            .unwrap_or("Untitled")
            .to_string();

        // 2. Get page blocks (content)
        let blocks_url = format!("https://api.notion.com/v1/blocks/{}/children", page_id);
        let blocks_resp = self
            .client
            .get(&blocks_url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Notion-Version", "2022-06-28")
            .send()
            .await?;

        let mut content = String::new();
        if blocks_resp.status().is_success() {
            let blocks_json: serde_json::Value = blocks_resp.json().await?;
            if let Some(results) = blocks_json["results"].as_array() {
                for block in results {
                    if let Some(para) = block["paragraph"]["rich_text"].as_array() {
                        for text in para {
                            if let Some(t) = text["text"]["content"].as_str() {
                                content.push_str(t);
                                content.push('\n');
                            }
                        }
                    }
                }
            }
        }

        Ok(NotionPage {
            id: page_id.to_string(),
            title,
            content,
        })
    }

    /// [Phase 28] Query a database for pages
    pub async fn query_database(&self, database_id: &str, limit: u32) -> Result<Vec<NotionPage>> {
        let url = format!("https://api.notion.com/v1/databases/{}/query", database_id);

        let body = json!({
            "page_size": limit
        });

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Notion-Version", "2022-06-28")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            return Err(anyhow::anyhow!("Notion Query Error: {}", err));
        }

        let resp_json: serde_json::Value = resp.json().await?;
        let mut pages = Vec::new();

        if let Some(results) = resp_json["results"].as_array() {
            for page in results {
                let id = page["id"].as_str().unwrap_or("").to_string();
                let title = page["properties"]["이름"]["title"]
                    .as_array()
                    .and_then(|arr| arr.first())
                    .and_then(|t| t["text"]["content"].as_str())
                    .unwrap_or("Untitled")
                    .to_string();

                pages.push(NotionPage {
                    id,
                    title,
                    content: String::new(), // Content requires separate fetch
                });
            }
        }

        Ok(pages)
    }

    /// Append paragraph blocks to an existing page
    pub async fn append_paragraphs(&self, page_id: &str, paragraphs: &[String]) -> Result<()> {
        if paragraphs.is_empty() {
            return Ok(());
        }

        let url = format!("https://api.notion.com/v1/blocks/{}/children", page_id);
        let children: Vec<serde_json::Value> = paragraphs
            .iter()
            .filter_map(|p| {
                let text = p.trim();
                if text.is_empty() {
                    None
                } else {
                    let clipped: String = text.chars().take(1800).collect();
                    Some(json!({
                        "object": "block",
                        "type": "paragraph",
                        "paragraph": {
                            "rich_text": [{
                                "type": "text",
                                "text": { "content": clipped }
                            }]
                        }
                    }))
                }
            })
            .collect();

        if children.is_empty() {
            return Ok(());
        }

        let body = json!({ "children": children });
        let resp = self
            .client
            .patch(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Notion-Version", "2022-06-28")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            return Err(anyhow::anyhow!("Notion Append Error: {}", err));
        }

        Ok(())
    }
}
