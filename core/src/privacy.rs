use crate::schema::{EventEnvelope, PrivacyContext};
use hmac::{Hmac, Mac};
use regex::Regex;
use serde_json::Value;
use sha2::Sha256;
use std::collections::HashSet;

type HmacSha256 = Hmac<Sha256>;

pub struct PrivacyGuard {
    mask_keys: HashSet<String>,
    #[allow(dead_code)]
    hash_keys: HashSet<String>,
    drop_keys: HashSet<String>,
    email_regex: Regex,
    hash_salt: String,
}

impl PrivacyGuard {
    pub fn new(salt: String) -> Self {
        // Default rules (Mirroring common Python configs)
        let mut mask_keys = HashSet::new();
        mask_keys.insert("password".to_string());
        mask_keys.insert("secret".to_string());
        mask_keys.insert("token".to_string());

        // Email pattern (Compile once or panic early)
        let email_regex = Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")
            .expect("Invalid Regex Pattern in PrivacyGuard");

        Self {
            mask_keys,
            hash_keys: HashSet::new(),
            drop_keys: HashSet::new(),
            email_regex,
            hash_salt: salt,
        }
    }

    pub fn apply(&self, mut envelope: EventEnvelope) -> Option<EventEnvelope> {
        // 1. App Allow/Deny (Skipped for simple MVP, can add later)

        let mut redactions = Vec::new();

        // 2. Hash Window/Resource IDs
        if let Some(window_id) = &envelope.window_id {
            envelope.window_id = Some(self.hash_value(window_id));
            redactions.push("window_id_hashed".to_string());
        }

        if let Some(res) = &mut envelope.resource {
            if res.id != "unknown" {
                res.id = self.hash_value(&res.id);
                redactions.push("resource_id_hashed".to_string());
            }
        }

        // 3. Payload Sanitization
        if let Value::Object(ref mut map) = envelope.payload {
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                let key_lower = key.to_lowercase();

                // Drop
                if self.drop_keys.contains(&key_lower) {
                    map.remove(&key);
                    redactions.push(format!("drop:{}", key));
                    continue;
                }

                // Mask/Hash
                if let Some(val) = map.get_mut(&key) {
                    if self.mask_keys.contains(&key_lower) {
                        *val = Value::String("***MASKED***".to_string());
                        redactions.push(format!("mask:{}", key));
                    }
                    // Detect Emails in strings
                    else if let Value::String(s) = val {
                        if self.email_regex.is_match(s) {
                            *val = Value::String("[EMAIL REDACTED]".to_string());
                            redactions.push(format!("email_redacted:{}", key));
                        }
                        // URL Sanitization
                        else if s.starts_with("http") {
                            let sanitized = self.sanitize_url(s);
                            if sanitized != *s {
                                *val = Value::String(sanitized);
                                redactions.push(format!("url_sanitized:{}", key));
                            }
                        }
                    }
                }
            }
        }

        // 4. Update Metadata
        let mut privacy = envelope.privacy.clone().unwrap_or(PrivacyContext {
            pii_types: vec![],
            hash_method: "hmac_sha256".to_string(),
            is_masked: false,
        });

        if !redactions.is_empty() {
            privacy.is_masked = true;
            // Append redactions logic if we add that field to PrivacyContext later
        }
        envelope.privacy = Some(privacy);

        Some(envelope)
    }

    fn hash_value(&self, value: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(self.hash_salt.as_bytes())
            .expect("HMAC can take any key size");
        mac.update(value.as_bytes());
        let result = mac.finalize();
        hex::encode(result.into_bytes())
    }

    /// Strip query parameters and hash fragments (e.g., ?token=xyz, #access_token=...)
    fn sanitize_url(&self, url: &str) -> String {
        let url_str = url.to_string();

        // Find split point (first of '?' or '#')
        let split_idx = url_str.find(['?', '#']);

        if let Some(idx) = split_idx {
            url_str[..idx].to_string()
        } else {
            url_str
        }
    }
    /// Public helper for masking arbitrary text (e.g. window titles)
    pub fn mask_sensitive_text(&self, text: &str) -> String {
        let mut masked = text.to_string();

        // 1. Mask Emails
        if self.email_regex.is_match(&masked) {
            masked = self
                .email_regex
                .replace_all(&masked, "[EMAIL REDACTED]")
                .to_string();
        }

        // 2. Simple Credit Card (Luhn check too expensive, just regex)
        // Matches 13-16 digits often separated by space or dash
        let cc_regex = Regex::new(r"\b(?:\d[ -]*?){13,16}\b").unwrap();
        if cc_regex.is_match(&masked) {
            masked = cc_regex.replace_all(&masked, "[CC REDACTED]").to_string();
        }

        masked
    }
}
