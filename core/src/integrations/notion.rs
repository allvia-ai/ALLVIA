use anyhow::Result;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};

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

    async fn database_title_property_name(&self, database_id: &str) -> Result<String> {
        let url = format!("https://api.notion.com/v1/databases/{}", database_id);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Notion-Version", "2022-06-28")
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            return Err(anyhow::anyhow!("Notion Database Error: {}", err));
        }

        let db_json: Value = resp.json().await?;
        if let Some(properties) = db_json.get("properties").and_then(|v| v.as_object()) {
            for (name, prop) in properties {
                if prop.get("type").and_then(|v| v.as_str()) == Some("title") {
                    return Ok(name.clone());
                }
            }
        }
        Err(anyhow::anyhow!(
            "Notion Database Error: title property not found in database {}",
            database_id
        ))
    }

    fn extract_title_from_property(prop: &Value) -> Option<String> {
        let title_arr = prop.get("title").and_then(|v| v.as_array())?;
        let mut parts: Vec<String> = Vec::new();
        for seg in title_arr {
            if let Some(text) = seg.get("plain_text").and_then(|v| v.as_str()).or_else(|| {
                seg.get("text")
                    .and_then(|v| v.get("content"))
                    .and_then(|v| v.as_str())
            }) {
                let t = text.trim();
                if !t.is_empty() {
                    parts.push(t.to_string());
                }
            }
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(""))
        }
    }

    fn extract_title_from_properties(properties: &Value) -> String {
        if let Some(props) = properties.as_object() {
            for prop in props.values() {
                if prop.get("type").and_then(|v| v.as_str()) == Some("title") {
                    if let Some(title) = Self::extract_title_from_property(prop) {
                        return title;
                    }
                }
            }
        }
        "Untitled".to_string()
    }

    /// Create a new page in a database using paragraph content.
    pub async fn create_page(
        &self,
        database_id: &str,
        title: &str,
        content: &str,
    ) -> Result<String> {
        // Notion rich_text content is limited to 2000 chars per text object.
        // Split by lines first, then by char window to stay below the limit.
        let mut chunks: Vec<String> = Vec::new();
        let mut current = String::new();
        let max_len = 1800usize;
        for line in content.lines() {
            let candidate = if current.is_empty() {
                line.to_string()
            } else {
                format!("{}\n{}", current, line)
            };
            if candidate.chars().count() <= max_len {
                current = candidate;
            } else {
                if !current.trim().is_empty() {
                    chunks.push(current.clone());
                }
                let mut remaining = line.to_string();
                while remaining.chars().count() > max_len {
                    let part: String = remaining.chars().take(max_len).collect();
                    chunks.push(part);
                    remaining = remaining.chars().skip(max_len).collect();
                }
                current = remaining;
            }
        }
        if !current.trim().is_empty() {
            chunks.push(current);
        }
        if chunks.is_empty() {
            chunks.push(content.to_string());
        }

        let children: Vec<Value> = chunks
            .into_iter()
            .map(|chunk| {
                json!({
                    "object": "block",
                    "type": "paragraph",
                    "paragraph": {
                        "rich_text": [{ "type": "text", "text": { "content": chunk } }]
                    }
                })
            })
            .collect();
        self.create_database_page_with_children(database_id, title, &children)
            .await
    }

    /// Create a new page in a database using explicit block children.
    pub async fn create_database_page_with_children(
        &self,
        database_id: &str,
        title: &str,
        children: &[Value],
    ) -> Result<String> {
        let url = "https://api.notion.com/v1/pages";
        let title_prop = self
            .database_title_property_name(database_id)
            .await
            .unwrap_or_else(|_| "Name".to_string());
        let mut properties = serde_json::Map::new();
        properties.insert(
            title_prop,
            json!({
                "title": [{ "text": { "content": title } }]
            }),
        );

        let body = json!({
            "parent": { "database_id": database_id },
            "properties": Value::Object(properties),
            "children": children
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

    /// Create a child page under a parent page using explicit block children.
    pub async fn create_child_page_with_children(
        &self,
        parent_page_id: &str,
        title: &str,
        children: &[Value],
    ) -> Result<String> {
        let url = "https://api.notion.com/v1/pages";
        let body = json!({
            "parent": { "page_id": parent_page_id },
            "properties": {
                "title": {
                    "title": [{ "text": { "content": title } }]
                }
            },
            "children": children
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
            return Err(anyhow::anyhow!("Notion Child Page Error: {}", err));
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
        let title = Self::extract_title_from_properties(&page_json["properties"]);

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
        let title_prop = self
            .database_title_property_name(database_id)
            .await
            .unwrap_or_else(|_| "Name".to_string());

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
                let title = page
                    .get("properties")
                    .and_then(|props| props.get(&title_prop))
                    .and_then(Self::extract_title_from_property)
                    .unwrap_or_else(|| {
                        page.get("properties")
                            .map(Self::extract_title_from_properties)
                            .unwrap_or_else(|| "Untitled".to_string())
                    });

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

        self.append_blocks(page_id, &children).await
    }

    /// Append arbitrary blocks to an existing page.
    pub async fn append_blocks(&self, page_id: &str, blocks: &[Value]) -> Result<()> {
        if blocks.is_empty() {
            return Ok(());
        }

        let url = format!("https://api.notion.com/v1/blocks/{}/children", page_id);
        let body = json!({ "children": blocks });
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
