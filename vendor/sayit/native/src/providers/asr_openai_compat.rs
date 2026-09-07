//! User-configured OpenAI audio/transcriptions endpoint, including local servers.
use super::types::{AsrProviderConfig, AsrResult, TestResult};
use base64::Engine;
use std::time::Instant;

fn endpoint(raw: &str) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(raw.trim()).map_err(|_| "Invalid ASR endpoint URL")?;
    if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none()
        || !url.username().is_empty() || url.password().is_some() {
        return Err("ASR endpoint must be an HTTP(S) URL without embedded credentials".into());
    }
    let path = url.path().trim_end_matches('/');
    if !path.ends_with("/audio/transcriptions") {
        let base = if path.is_empty() { "/v1" } else { path };
        url.set_path(&format!("{base}/audio/transcriptions"));
    }
    url.set_fragment(None);
    Ok(url)
}

pub async fn transcribe(audio: &str, sample_rate: u32, config: &AsrProviderConfig, hotwords: &[String]) -> Result<AsrResult, String> {
    let url = endpoint(config.extra["api_url"].as_str().unwrap_or(""))?;
    let model = config.extra["model"].as_str().unwrap_or("").trim();
    if model.is_empty() { return Err("ASR model name is required".into()); }
    if sample_rate != 16000 { return Err("Expected 16 kHz mono PCM".into()); }
    let pcm = base64::engine::general_purpose::STANDARD.decode(audio).map_err(|_| "Invalid PCM base64")?;
    if pcm.is_empty() || pcm.len() % 2 != 0 || pcm.len() > 16_000 * 2 * 300 {
        return Err("PCM audio must contain 16-bit samples and be at most five minutes".into());
    }
    let wav = super::asr_groq::pcm_to_wav(&pcm, sample_rate);
    let file = reqwest::multipart::Part::bytes(wav).file_name("recording.wav")
        .mime_str("audio/wav").map_err(|e| e.to_string())?;
    let mut form = reqwest::multipart::Form::new().part("file", file)
        .text("model", model.to_owned()).text("response_format", "json");
    if let Some(lang) = config.extra["language"].as_str().filter(|s| !s.is_empty() && *s != "auto") {
        form = form.text("language", lang.to_owned());
    }
    if !hotwords.is_empty() { form = form.text("prompt", hotwords.join(", ")); }
    let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(120)).build().map_err(|e| e.to_string())?;
    let mut request = client.post(url).multipart(form);
    if !config.api_key.trim().is_empty() { request = request.bearer_auth(config.api_key.trim()); }
    let start = Instant::now();
    let response = request.send().await.map_err(|e| format!("ASR connection failed: {}", e.without_url()))?;
    if !response.status().is_success() {
        return Err(format!("ASR HTTP {}", response.status()));
    }
    let data: serde_json::Value = response.json().await.map_err(|_| "ASR response is not valid JSON")?;
    let text = data["text"].as_str().ok_or("ASR response is missing the text field")?.trim().to_owned();
    Ok(AsrResult { text, elapsed_ms: start.elapsed().as_millis() as u64 })
}

pub async fn test_connection(config: &AsrProviderConfig) -> TestResult {
    let audio = base64::engine::general_purpose::STANDARD.encode(vec![0; 16000]);
    match transcribe(&audio, 16000, config, &[]).await {
        Ok(result) => TestResult { ok: true, message: "Connected".into(), elapsed_ms: result.elapsed_ms, detail: result.text },
        Err(message) => TestResult { ok: false, message, elapsed_ms: 0, detail: String::new() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn server(status: &str, body: &str) -> (String, std::thread::JoinHandle<String>) {
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", socket.local_addr().unwrap());
        let reply = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        let task = std::thread::spawn(move || {
            let (mut stream, _) = socket.accept().unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0; 4096];
            loop {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0, "Request ended early");
                bytes.extend_from_slice(&buf[..n]);
                if let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                    let length: usize = headers.lines().find_map(|line| line.strip_prefix("content-length:")).unwrap().trim().parse().unwrap();
                    if bytes.len() >= end + 4 + length { break; }
                }
            }
            stream.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&bytes).into_owned()
        });
        (address, task)
    }

    #[tokio::test]
    async fn custom_model_and_wav_reach_the_configured_service() {
        let (url, task) = server("200 OK", r#"{"text":"recognized text"}"#);
        let config = AsrProviderConfig { provider: "openai_compat".into(), api_key: String::new(), app_id: String::new(),
            extra: serde_json::json!({"api_url": url, "model": "my-local-asr"}) };
        let result = transcribe("AAAAAA==", 16000, &config, &["VoiceHub".into()]).await.unwrap();
        assert_eq!(result.text, "recognized text");
        let request = task.join().unwrap();
        assert!(request.starts_with("POST /v1/audio/transcriptions "));
        assert!(request.contains("name=\"model\"\r\n\r\nmy-local-asr"));
        assert!(request.contains("RIFF"));
        assert!(request.contains("VoiceHub"));
        assert!(!request.to_lowercase().contains("authorization:"));
    }

    #[tokio::test]
    async fn authentication_errors_do_not_echo_secrets() {
        let (url, task) = server("401 Unauthorized", r#"{"error":"fake-test-key"}"#);
        let config = AsrProviderConfig { provider: "openai_compat".into(), api_key: "fake-test-key".into(), app_id: String::new(),
            extra: serde_json::json!({"api_url": url, "model": "custom"}) };
        let error = transcribe("AAAAAA==", 16000, &config, &[]).await.unwrap_err();
        assert!(error.contains("401"));
        assert!(!error.contains("fake-test-key"));
        assert!(task.join().unwrap().to_lowercase().contains("authorization: bearer fake-test-key"));
    }
    #[test]
    fn urls_preserve_custom_prefix_and_query_without_duplicate_suffix() {
        assert_eq!(endpoint("http://127.0.0.1:8000").unwrap().as_str(), "http://127.0.0.1:8000/v1/audio/transcriptions");
        assert_eq!(endpoint("https://example.org/custom/v1/?version=1").unwrap().as_str(), "https://example.org/custom/v1/audio/transcriptions?version=1");
        assert_eq!(endpoint("https://example.org/v1/audio/transcriptions").unwrap().path(), "/v1/audio/transcriptions");
        assert!(endpoint("file:///tmp/audio").is_err());
        assert!(endpoint("https://user:secret@example.org").is_err());
    }
}
