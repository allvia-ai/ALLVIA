#![allow(dead_code)]
use crate::integrations::google_auth;
use anyhow::Result;
use reqwest::Client;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct MessageList {
    messages: Option<Vec<MessageRef>>,
}

#[derive(Debug, Deserialize)]
struct MessageRef {
    id: String,
}

#[derive(Debug, Deserialize)]
struct MessageDetails {
    id: String,
    payload: Option<Payload>,
    snippet: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Payload {
    headers: Option<Vec<Header>>,
}

#[derive(Debug, Deserialize)]
struct Header {
    name: String,
    value: String,
}

pub struct GmailClient {
    client: Client,
    access_token: String,
}

impl GmailClient {
    pub async fn new() -> Result<Self> {
        let auth = google_auth::get_authenticator().await?;
        let token = google_auth::get_access_token(&auth).await?;

        Ok(Self {
            client: Client::new(),
            access_token: token,
        })
    }

    /// List recent messages from inbox
    pub async fn list_messages(&self, max_results: u32) -> Result<Vec<(String, String, String)>> {
        let url = format!(
            "https://gmail.googleapis.com/gmail/v1/users/me/messages?maxResults={}",
            max_results
        );

        let resp: MessageList = self
            .client
            .get(&url)
            .bearer_auth(&self.access_token)
            .send()
            .await?
            .json()
            .await?;

        let mut messages = Vec::new();

        if let Some(msg_list) = resp.messages {
            for msg_ref in msg_list.iter().take(max_results as usize) {
                if let Ok(details) = self.get_message_details(&msg_ref.id).await {
                    messages.push(details);
                }
            }
        }

        Ok(messages)
    }

    /// Get message details (id, subject, from)
    async fn get_message_details(&self, id: &str) -> Result<(String, String, String)> {
        let url = format!(
            "https://gmail.googleapis.com/gmail/v1/users/me/messages/{}?format=metadata&metadataHeaders=Subject&metadataHeaders=From",
            id
        );

        let resp: MessageDetails = self
            .client
            .get(&url)
            .bearer_auth(&self.access_token)
            .send()
            .await?
            .json()
            .await?;

        let mut subject = String::from("(No Subject)");
        let mut from = String::from("(Unknown)");

        if let Some(payload) = resp.payload {
            if let Some(headers) = payload.headers {
                for header in headers {
                    match header.name.as_str() {
                        "Subject" => subject = header.value,
                        "From" => from = header.value,
                        _ => {}
                    }
                }
            }
        }

        Ok((id.to_string(), subject, from))
    }

    /// Get full message content
    pub async fn get_message(&self, id: &str) -> Result<String> {
        let url = format!(
            "https://gmail.googleapis.com/gmail/v1/users/me/messages/{}?format=full",
            id
        );

        let resp: MessageDetails = self
            .client
            .get(&url)
            .bearer_auth(&self.access_token)
            .send()
            .await?
            .json()
            .await?;

        let mut content = String::new();

        if let Some(payload) = &resp.payload {
            if let Some(headers) = &payload.headers {
                for header in headers {
                    if header.name == "Subject" || header.name == "From" || header.name == "Date" {
                        content.push_str(&format!("{}: {}\n", header.name, header.value));
                    }
                }
            }
        }

        content.push_str("\n---\n\n");
        if let Some(snippet) = resp.snippet {
            content.push_str(&snippet);
        }

        Ok(content)
    }

    /// Send an email
    pub async fn send_message(&self, to: &str, subject: &str, body: &str) -> Result<String> {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

        let email = format!(
            "To: {}\r\nSubject: {}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{}",
            to, subject, body
        );

        let encoded = URL_SAFE_NO_PAD.encode(email.as_bytes());

        let url = "https://gmail.googleapis.com/gmail/v1/users/me/messages/send";

        let resp: serde_json::Value = self
            .client
            .post(url)
            .bearer_auth(&self.access_token)
            .json(&serde_json::json!({ "raw": encoded }))
            .send()
            .await?
            .json()
            .await?;

        Ok(resp["id"].as_str().unwrap_or("sent").to_string())
    }
}
