use anyhow::Result;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use std::time::SystemTime;

pub async fn send_discord_command(token: &str, channel_id: &str, content: &str) -> Result<()> {
    let mut headers = HeaderMap::new();

    headers.insert(AUTHORIZATION, HeaderValue::from_str(token)?);
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"),
    );

    // Mimic official client headers to avoid detection
    headers.insert(
        "X-Super-Properties",
        HeaderValue::from_static(
            "eyJvcyI6IkxpbnV4IiwiYnJvd3NlciI6IkNocm9tZSIsImRldmljZSI6IiIsInN5c3RlbV9sb2NhbGUi\
            OiJlbi1VUyIsImJyb3dzZXJfdXNlcl9hZ2VudCI6Ik1vemlsbGEvNS4wIChYMTE7IExpbnV4IHg4Nl82\
            NCkgQXBwbGVXZWJLaXQvNTM3LjM2IChLSFRNTCwgbGlrZSBHZWNrbykgQ2hyb21lLzEyMC4wLjAuMCBT\
            YWZhcmkvNTM3LjM2IiwiYnJvd3Nlcl92ZXJzaW9uIjoiMTIwLjAuMC4wIiwib3NfdmVyc2lvbiI6IiIs\
            InJlZmVycmVyIjoiIiwicmVmZXJyaW5nX2RvbWFpbiI6IiIsInJlZmVycmVyX2N1cnJlbnQiOiIiLCJy\
            ZWZlcnJpbmdfZG9tYWluX2N1cnJlbnQiOiIiLCJyZWxlYXNlX2NoYW5uZWwiOiJzdGFibGUiLCJjbGll\
            bnRfYnVpbGRfbnVtYmVyIjoyNTAwMDAsImNsaWVudF9ldmVudF9zb3VyY2UiOm51bGx9",
        ),
    );

    headers.insert("X-Discord-Locale", HeaderValue::from_static("en-US"));
    headers.insert(
        "X-Debug-Options",
        HeaderValue::from_static("bugReporterEnabled"),
    );
    headers.insert(
        "Sec-Ch-Ua",
        HeaderValue::from_static(
            "\"Not_A Brand\";v=\"8\", \"Chromium\";v=\"120\", \"Google Chrome\";v=\"120\"",
        ),
    );
    headers.insert("Sec-Ch-Ua-Mobile", HeaderValue::from_static("?0"));
    headers.insert("Sec-Ch-Ua-Platform", HeaderValue::from_static("\"Linux\""));
    headers.insert("Sec-Fetch-Dest", HeaderValue::from_static("empty"));
    headers.insert("Sec-Fetch-Mode", HeaderValue::from_static("cors"));
    headers.insert("Sec-Fetch-Site", HeaderValue::from_static("same-origin"));
    headers.insert("Origin", HeaderValue::from_static("https://discord.com"));
    headers.insert(
        "Referer",
        HeaderValue::from_static("https://discord.com/channels/@me"),
    );

    // Generate Snowflake nonce mimicking official web client
    let now_ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let nonce = ((now_ms as u64).saturating_sub(1420070400000)) << 22;
    let nonce_str = nonce.to_string();

    let client = reqwest::Client::builder()
        .default_headers(headers)
        .build()?;

    let url = format!(
        "https://discord.com/api/v9/channels/{}/messages",
        channel_id
    );
    let payload = serde_json::json!({
        "content": content,
        "nonce": nonce_str,
        "tts": false
    });

    let response = client.post(&url).json(&payload).send().await?;

    if response.status().is_success() {
        Ok(())
    } else {
        let status = response.status();
        let err_text = response.text().await?;
        Err(anyhow::anyhow!(
            "Discord API error {}: {}",
            status,
            err_text
        ))
    }
}
