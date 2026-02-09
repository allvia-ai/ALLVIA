use crate::integrations::google_auth;
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct EventList {
    items: Option<Vec<Event>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Event {
    id: Option<String>,
    summary: Option<String>,
    start: Option<EventDateTime>,
    end: Option<EventDateTime>,
}

#[derive(Debug, Deserialize, Serialize)]
struct EventDateTime {
    #[serde(rename = "dateTime")]
    date_time: Option<String>,
    date: Option<String>,
    #[serde(rename = "timeZone")]
    time_zone: Option<String>,
}

pub struct CalendarClient {
    client: Client,
    access_token: String,
}

impl CalendarClient {
    pub async fn new() -> Result<Self> {
        let auth = google_auth::get_authenticator().await?;
        let token = google_auth::get_access_token(&auth).await?;

        Ok(Self {
            client: Client::new(),
            access_token: token,
        })
    }

    /// List today's events
    pub async fn list_today(&self) -> Result<Vec<(String, String, String)>> {
        let now = Utc::now();
        let start_of_day = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc))
            .unwrap_or(now);
        let end_of_day = now
            .date_naive()
            .and_hms_opt(23, 59, 59)
            .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc))
            .unwrap_or(now);

        self.list_events_range(start_of_day, end_of_day).await
    }

    /// List this week's events
    pub async fn list_week(&self) -> Result<Vec<(String, String, String)>> {
        let now = Utc::now();
        let start = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc))
            .unwrap_or(now);
        let end = (now + Duration::days(7))
            .date_naive()
            .and_hms_opt(23, 59, 59)
            .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc))
            .unwrap_or(now);

        self.list_events_range(start, end).await
    }

    /// List events in a time range
    async fn list_events_range(
        &self,
        time_min: DateTime<Utc>,
        time_max: DateTime<Utc>,
    ) -> Result<Vec<(String, String, String)>> {
        let url = format!(
            "https://www.googleapis.com/calendar/v3/calendars/primary/events?timeMin={}&timeMax={}&singleEvents=true&orderBy=startTime",
            time_min.to_rfc3339(),
            time_max.to_rfc3339()
        );

        let resp: EventList = self
            .client
            .get(&url)
            .bearer_auth(&self.access_token)
            .send()
            .await?
            .json()
            .await?;

        let mut events = Vec::new();

        if let Some(items) = resp.items {
            for event in items {
                let id = event.id.unwrap_or_default();
                let summary = event.summary.unwrap_or_else(|| "(No Title)".to_string());
                let start_time = self.format_event_time(&event.start);
                events.push((id, summary, start_time));
            }
        }

        Ok(events)
    }

    fn format_event_time(&self, dt: &Option<EventDateTime>) -> String {
        match dt {
            Some(edt) => {
                if let Some(date_time) = &edt.date_time {
                    date_time.clone()
                } else if let Some(date) = &edt.date {
                    format!("{} (All day)", date)
                } else {
                    "(Unknown time)".to_string()
                }
            }
            None => "(Unknown time)".to_string(),
        }
    }

    /// Create a new event
    pub async fn create_event(&self, title: &str, start: &str, end: &str) -> Result<String> {
        let url = "https://www.googleapis.com/calendar/v3/calendars/primary/events";

        let event = serde_json::json!({
            "summary": title,
            "start": {
                "dateTime": start,
                "timeZone": "Asia/Seoul"
            },
            "end": {
                "dateTime": end,
                "timeZone": "Asia/Seoul"
            }
        });

        let resp: serde_json::Value = self
            .client
            .post(url)
            .bearer_auth(&self.access_token)
            .json(&event)
            .send()
            .await?
            .json()
            .await?;

        Ok(resp["id"].as_str().unwrap_or("created").to_string())
    }

    /// Delete an event
    #[allow(dead_code)]
    pub async fn delete_event(&self, event_id: &str) -> Result<()> {
        let url = format!(
            "https://www.googleapis.com/calendar/v3/calendars/primary/events/{}",
            event_id
        );

        self.client
            .delete(&url)
            .bearer_auth(&self.access_token)
            .send()
            .await?;

        Ok(())
    }
}
