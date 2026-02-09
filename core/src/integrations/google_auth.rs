use anyhow::Result;
use std::path::PathBuf;
use yup_oauth2::{InstalledFlowAuthenticator, InstalledFlowReturnMethod};

/// All scopes required for Gmail and Calendar access
pub const ALL_SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/gmail.readonly",
    "https://www.googleapis.com/auth/gmail.send",
    "https://mail.google.com/", // Full Gmail access for sending
    "https://www.googleapis.com/auth/calendar.readonly",
    "https://www.googleapis.com/auth/calendar.events",
];

/// Get the path to the credentials file
fn credentials_path() -> PathBuf {
    let mut path = std::env::current_dir().unwrap_or_default();
    path.push("credentials.json");
    path
}

/// Get the path to store the token cache
fn token_cache_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let mut path = PathBuf::from(home);
    path.push(".local-os-agent");
    std::fs::create_dir_all(&path).ok();
    path.push("google_token.json");
    path
}

/// Type alias for the authenticator used throughout the Google integration
pub type GoogleAuthenticator = yup_oauth2::authenticator::Authenticator<
    hyper_rustls::HttpsConnector<hyper::client::HttpConnector>,
>;

/// Create an authenticator for Google APIs with all scopes pre-authorized
pub async fn get_authenticator() -> Result<GoogleAuthenticator> {
    let creds_path = credentials_path();

    if !creds_path.exists() {
        return Err(anyhow::anyhow!(
            "credentials.json not found!\n\
            Please download it from Google Cloud Console:\n\
            1. Go to https://console.cloud.google.com/\n\
            2. Create/Select a project\n\
            3. Enable Gmail API and Calendar API\n\
            4. Create OAuth 2.0 Client ID (Desktop App)\n\
            5. Download and save as 'core/credentials.json'"
        ));
    }

    let secret = yup_oauth2::read_application_secret(&creds_path).await?;

    let auth = InstalledFlowAuthenticator::builder(secret, InstalledFlowReturnMethod::HTTPRedirect)
        .persist_tokens_to_disk(token_cache_path())
        .build()
        .await?;

    // Pre-authorize all scopes at once to avoid multiple auth prompts
    println!("🔐 Requesting Google authorization for all scopes...");
    let _token = auth.token(ALL_SCOPES).await?;
    println!("✅ Google authorization complete!");

    Ok(auth)
}

/// Get a valid access token, triggering OAuth flow if needed
#[allow(dead_code)]
pub async fn get_access_token(auth: &GoogleAuthenticator) -> Result<String> {
    let token = auth.token(ALL_SCOPES).await?;

    match token.token() {
        Some(t) => Ok(t.to_string()),
        None => Err(anyhow::anyhow!("Failed to get access token")),
    }
}
