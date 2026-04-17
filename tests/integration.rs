//! Integration tests for mail-laser using testcontainers with MockServer.
//!
//! These tests verify the full SMTP → parse → webhook delivery pipeline,
//! including resilience behavior (retry, circuit breaker).
//!
//! Run with: cargo test --test integration --features test-http
//! Requires Docker.

use std::net::TcpListener as StdTcpListener;
use std::sync::Arc;
use std::time::Duration;

use acton_reactive::prelude::*;
use mail_laser::attachment::{inline::InlineBackend, AttachmentBackend};
use mail_laser::config::Config;
use mail_laser::policy::PolicyEngine;
use mail_laser::smtp::SmtpListenerState;
use mail_laser::webhook::WebhookState;
use testcontainers::core::wait::WaitFor;
use testcontainers::core::IntoContainerPort;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

fn init_crypto() {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok();
}

// --- Helpers ---

fn get_free_port() -> u16 {
    let listener = StdTcpListener::bind("127.0.0.1:0").expect("Failed to bind to port 0");
    listener.local_addr().unwrap().port()
}

async fn wait_for_smtp(addr: &str, timeout: Duration) {
    let start = std::time::Instant::now();
    loop {
        if TcpStream::connect(addr).await.is_ok() {
            return;
        }
        if start.elapsed() > timeout {
            panic!(
                "SMTP server at {} did not become ready within {:?}",
                addr, timeout
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn smtp_send_email(
    addr: &str,
    sender: &str,
    recipient: &str,
    subject: &str,
    body: &str,
) -> anyhow::Result<()> {
    let stream = TcpStream::connect(addr).await?;
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();

    // Read greeting
    reader.read_line(&mut line).await?;
    assert!(
        line.starts_with("220"),
        "Expected 220 greeting, got: {}",
        line
    );

    // EHLO
    write_half.write_all(b"EHLO test\r\n").await?;
    write_half.flush().await?;
    loop {
        line.clear();
        reader.read_line(&mut line).await?;
        if line.starts_with("250 ") {
            break; // last line of multiline 250 response
        }
        assert!(line.starts_with("250"), "EHLO failed: {}", line);
    }

    // MAIL FROM
    write_half
        .write_all(format!("MAIL FROM:<{}>\r\n", sender).as_bytes())
        .await?;
    write_half.flush().await?;
    line.clear();
    reader.read_line(&mut line).await?;
    assert!(line.starts_with("250"), "MAIL FROM failed: {}", line);

    // RCPT TO
    write_half
        .write_all(format!("RCPT TO:<{}>\r\n", recipient).as_bytes())
        .await?;
    write_half.flush().await?;
    line.clear();
    reader.read_line(&mut line).await?;
    assert!(line.starts_with("250"), "RCPT TO failed: {}", line);

    // DATA
    write_half.write_all(b"DATA\r\n").await?;
    write_half.flush().await?;
    line.clear();
    reader.read_line(&mut line).await?;
    assert!(line.starts_with("354"), "DATA failed: {}", line);

    // Email content (RFC 2822 format)
    let email_content = format!(
        "From: {}\r\nTo: {}\r\nSubject: {}\r\n\r\n{}\r\n.\r\n",
        sender, recipient, subject, body
    );
    write_half.write_all(email_content.as_bytes()).await?;
    write_half.flush().await?;
    line.clear();
    reader.read_line(&mut line).await?;
    assert!(line.starts_with("250"), "DATA end failed: {}", line);

    // QUIT
    write_half.write_all(b"QUIT\r\n").await?;
    write_half.flush().await?;

    Ok(())
}

async fn start_mockserver() -> (ContainerAsync<GenericImage>, String) {
    let container = GenericImage::new("mockserver/mockserver", "5.15.0")
        .with_exposed_port(1080.tcp())
        .with_wait_for(WaitFor::message_on_stdout("started on port: 1080"))
        .start()
        .await
        .expect("Failed to start MockServer container");

    let host_port = container
        .get_host_port_ipv4(1080.tcp())
        .await
        .expect("Failed to get MockServer host port");
    let base_url = format!("http://127.0.0.1:{}", host_port);

    (container, base_url)
}

async fn configure_mockserver(
    base_url: &str,
    path: &str,
    status: u16,
    times: Option<u32>,
    priority: Option<i32>,
) {
    let client = reqwest::Client::new();

    let mut expectation = serde_json::json!({
        "httpRequest": {
            "method": "POST",
            "path": path,
        },
        "httpResponse": {
            "statusCode": status,
        }
    });

    if let Some(t) = times {
        expectation["times"] = serde_json::json!({
            "remainingTimes": t,
            "unlimited": false
        });
    } else {
        expectation["times"] = serde_json::json!({
            "unlimited": true
        });
    }

    if let Some(p) = priority {
        expectation["priority"] = serde_json::json!(p);
    }

    let resp = client
        .put(format!("{}/mockserver/expectation", base_url))
        .json(&expectation)
        .send()
        .await
        .expect("Failed to configure MockServer expectation");

    assert!(
        resp.status().is_success(),
        "MockServer expectation config failed: {}",
        resp.status()
    );
}

async fn get_mockserver_requests(base_url: &str, path: &str) -> Vec<serde_json::Value> {
    let client = reqwest::Client::new();

    let body = serde_json::json!({
        "method": "POST",
        "path": path,
    });

    let resp = client
        .put(format!(
            "{}/mockserver/retrieve?type=REQUESTS&format=JSON",
            base_url
        ))
        .json(&body)
        .send()
        .await
        .expect("Failed to retrieve MockServer requests");

    if resp.status().is_success() {
        resp.json::<Vec<serde_json::Value>>()
            .await
            .unwrap_or_default()
    } else {
        vec![]
    }
}

/// Open Cedar policy used by integration tests: allows every sender + attachment.
const TEST_POLICY: &str = r#"
    permit(principal, action == Action::"SendMail", resource);
    permit(principal, action == Action::"Attach", resource);
"#;

fn test_policy() -> Arc<PolicyEngine> {
    Arc::new(PolicyEngine::from_strings(TEST_POLICY, None).expect("test policy parses"))
}

fn test_backend() -> Arc<dyn AttachmentBackend> {
    Arc::new(InlineBackend::new())
}

fn test_config(smtp_port: u16, webhook_url: &str) -> Config {
    Config {
        target_emails: vec!["target@example.com".to_string()],
        webhook_url: webhook_url.to_string(),
        smtp_bind_address: "127.0.0.1".to_string(),
        smtp_port,
        health_check_bind_address: "127.0.0.1".to_string(),
        health_check_port: get_free_port(),
        header_prefixes: vec![],
        webhook_timeout_secs: 10,
        webhook_max_retries: 3,
        circuit_breaker_threshold: 5,
        circuit_breaker_reset_secs: 60,
        webhook_signing_secret: None,
        cedar_policies_path: std::path::PathBuf::from("tests/fixtures/integration.cedar"),
        cedar_entities_path: None,
        max_message_size_bytes: 26_214_400,
        max_attachment_size_bytes: 10_485_760,
        attachment_delivery: mail_laser::config::AttachmentDelivery::Inline,
        dmarc_mode: mail_laser::config::DmarcMode::Off,
        dmarc_dns_timeout_secs: 5,
        dmarc_dns_servers: vec![],
        dmarc_temperror_action: mail_laser::config::DmarcTempErrorAction::Reject,
    }
}

// --- Tests ---

#[tokio::test]
async fn test_end_to_end_email_forwarding() {
    init_crypto();
    let (_container, mock_url) = start_mockserver().await;
    configure_mockserver(&mock_url, "/webhook", 200, None, None).await;

    let smtp_port = get_free_port();
    let webhook_url = format!("{}/webhook", mock_url);
    let config = test_config(smtp_port, &webhook_url);

    let mut runtime = ActonApp::launch_async().await;
    let webhook_handle = WebhookState::create(&mut runtime, &config).await.unwrap();
    let _smtp_handle = SmtpListenerState::create(&mut runtime, &config, webhook_handle, test_policy(), test_backend(), None)
        .await
        .unwrap();

    let smtp_addr = format!("127.0.0.1:{}", smtp_port);
    wait_for_smtp(&smtp_addr, Duration::from_secs(5)).await;

    smtp_send_email(
        &smtp_addr,
        "sender@test.com",
        "target@example.com",
        "Integration Test",
        "Hello from integration test!",
    )
    .await
    .unwrap();

    // Wait for webhook delivery
    tokio::time::sleep(Duration::from_secs(2)).await;

    let requests = get_mockserver_requests(&mock_url, "/webhook").await;
    assert_eq!(
        requests.len(),
        1,
        "Expected 1 webhook request, got {}",
        requests.len()
    );

    // Check the request body contains expected fields
    let req = &requests[0];
    // MockServer may nest the body as {"body": {"type": "STRING", "string": "..."}}
    // or as {"body": {"type": "JSON", "json": {...}}} or other formats
    let body_json: serde_json::Value = if let Some(json_val) = req["body"]["json"].as_object() {
        serde_json::Value::Object(json_val.clone())
    } else if let Some(s) = req["body"]["string"].as_str() {
        serde_json::from_str(s).expect("Webhook body should be valid JSON")
    } else if let Some(s) = req["body"].as_str() {
        serde_json::from_str(s).expect("Webhook body should be valid JSON")
    } else {
        panic!(
            "Could not extract body from recorded request: {}",
            serde_json::to_string_pretty(&req["body"]).unwrap_or_default()
        );
    };

    assert_eq!(body_json["sender"], "sender@test.com");
    assert_eq!(body_json["recipient"], "target@example.com");
    assert_eq!(body_json["subject"], "Integration Test");
    assert!(
        body_json["body"]
            .as_str()
            .unwrap()
            .contains("Hello from integration test!"),
        "Body should contain the email text"
    );

    runtime.shutdown_all().await.ok();
}

#[tokio::test]
async fn test_webhook_signing_headers_match_shared_secret() {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    init_crypto();
    let (_container, mock_url) = start_mockserver().await;
    configure_mockserver(&mock_url, "/webhook", 200, None, None).await;

    let smtp_port = get_free_port();
    let webhook_url = format!("{}/webhook", mock_url);
    let mut config = test_config(smtp_port, &webhook_url);
    let secret = "shared-test-secret";
    config.webhook_signing_secret = Some(secret.to_string());

    let mut runtime = ActonApp::launch_async().await;
    let webhook_handle = WebhookState::create(&mut runtime, &config).await.unwrap();
    let _smtp_handle = SmtpListenerState::create(
        &mut runtime,
        &config,
        webhook_handle,
        test_policy(),
        test_backend(),
        None,
    )
    .await
    .unwrap();

    let smtp_addr = format!("127.0.0.1:{}", smtp_port);
    wait_for_smtp(&smtp_addr, Duration::from_secs(5)).await;

    smtp_send_email(
        &smtp_addr,
        "sender@test.com",
        "target@example.com",
        "Signed webhook",
        "Verify me.",
    )
    .await
    .unwrap();

    tokio::time::sleep(Duration::from_secs(2)).await;

    let requests = get_mockserver_requests(&mock_url, "/webhook").await;
    assert_eq!(requests.len(), 1, "expected one signed webhook request");
    let req = &requests[0];

    // MockServer serializes headers as an object {name: [values]} in recent
    // versions; older versions use an array of {name, values}. Support both.
    let header = |name: &str| -> String {
        if let Some(obj) = req["headers"].as_object() {
            for (k, v) in obj {
                if k.eq_ignore_ascii_case(name) {
                    return v[0].as_str().unwrap_or("").to_string();
                }
            }
        } else if let Some(entries) = req["headers"].as_array() {
            for entry in entries {
                let entry_name = entry["name"].as_str().unwrap_or("");
                if entry_name.eq_ignore_ascii_case(name) {
                    return entry["values"][0].as_str().unwrap_or("").to_string();
                }
            }
        }
        panic!(
            "header {} not found. headers payload: {}",
            name,
            serde_json::to_string_pretty(&req["headers"]).unwrap_or_default()
        );
    };

    let ts_header = header("X-MailLaser-Timestamp");
    let sig_header = header("X-MailLaser-Signature-256");

    // MockServer may return the body as a raw string, or as a nested
    // {"string": "..."} / {"json": {..}} / {"rawBytes": "<base64>"} object
    // depending on content-type detection. The signature was computed over
    // the wire bytes, so prefer `rawBytes` when available.
    let body: String = if let Some(raw) = req["body"]["rawBytes"].as_str() {
        use base64::Engine as _;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(raw)
            .expect("rawBytes base64-decodes");
        String::from_utf8(decoded).expect("body is UTF-8")
    } else if let Some(s) = req["body"]["string"].as_str() {
        s.to_string()
    } else if let Some(s) = req["body"].as_str() {
        s.to_string()
    } else if req["body"]["json"].is_object() {
        // MockServer parsed the JSON; re-serialize with our crate's encoder to
        // match the exact wire bytes we produced.
        let parsed: mail_laser::webhook::EmailPayload =
            serde_json::from_value(req["body"]["json"].clone()).expect("payload roundtrips");
        serde_json::to_string(&parsed).expect("re-encode payload")
    } else {
        panic!(
            "cannot recover body bytes from: {}",
            serde_json::to_string_pretty(&req["body"]).unwrap_or_default()
        );
    };

    let timestamp: u64 = ts_header.parse().expect("timestamp parses as u64");
    let sig_hex = sig_header
        .strip_prefix("sha256=")
        .expect("signature header uses sha256= scheme");

    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body.as_bytes());
    let expected = hex::encode(mac.finalize().into_bytes());

    assert_eq!(
        sig_hex, expected,
        "signature must match HMAC-SHA256(secret, \"<timestamp>.<body>\")"
    );

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert!(
        now.saturating_sub(timestamp) < 300,
        "timestamp must be recent (within 5 minutes)"
    );

    runtime.shutdown_all().await.ok();
}

#[tokio::test]
async fn test_webhook_signing_headers_absent_when_secret_not_set() {
    init_crypto();
    let (_container, mock_url) = start_mockserver().await;
    configure_mockserver(&mock_url, "/webhook", 200, None, None).await;

    let smtp_port = get_free_port();
    let webhook_url = format!("{}/webhook", mock_url);
    let config = test_config(smtp_port, &webhook_url); // no signing secret

    let mut runtime = ActonApp::launch_async().await;
    let webhook_handle = WebhookState::create(&mut runtime, &config).await.unwrap();
    let _smtp_handle = SmtpListenerState::create(
        &mut runtime,
        &config,
        webhook_handle,
        test_policy(),
        test_backend(),
        None,
    )
    .await
    .unwrap();

    let smtp_addr = format!("127.0.0.1:{}", smtp_port);
    wait_for_smtp(&smtp_addr, Duration::from_secs(5)).await;

    smtp_send_email(
        &smtp_addr,
        "sender@test.com",
        "target@example.com",
        "Unsigned",
        "No signature.",
    )
    .await
    .unwrap();

    tokio::time::sleep(Duration::from_secs(2)).await;

    let requests = get_mockserver_requests(&mock_url, "/webhook").await;
    assert_eq!(requests.len(), 1);
    let headers = &requests[0]["headers"];

    let has_header = |name: &str| -> bool {
        if let Some(obj) = headers.as_object() {
            obj.keys().any(|k| k.eq_ignore_ascii_case(name))
        } else if let Some(entries) = headers.as_array() {
            entries.iter().any(|e| {
                e["name"]
                    .as_str()
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            })
        } else {
            false
        }
    };

    assert!(
        !has_header("X-MailLaser-Signature-256"),
        "signature header must be absent when no secret configured: {}",
        serde_json::to_string_pretty(headers).unwrap_or_default()
    );
    assert!(
        !has_header("X-MailLaser-Timestamp"),
        "timestamp header must be absent when no secret configured"
    );

    runtime.shutdown_all().await.ok();
}

#[tokio::test]
async fn test_webhook_retry_on_failure() {
    init_crypto();
    let (_container, mock_url) = start_mockserver().await;

    // First 2 requests → 500 (high priority), then → 200 (lower priority, unlimited)
    configure_mockserver(&mock_url, "/webhook", 500, Some(2), Some(10)).await;
    configure_mockserver(&mock_url, "/webhook", 200, None, Some(5)).await;

    let smtp_port = get_free_port();
    let webhook_url = format!("{}/webhook", mock_url);
    let mut config = test_config(smtp_port, &webhook_url);
    config.webhook_max_retries = 3;

    let mut runtime = ActonApp::launch_async().await;
    let webhook_handle = WebhookState::create(&mut runtime, &config).await.unwrap();
    let _smtp_handle = SmtpListenerState::create(&mut runtime, &config, webhook_handle, test_policy(), test_backend(), None)
        .await
        .unwrap();

    let smtp_addr = format!("127.0.0.1:{}", smtp_port);
    wait_for_smtp(&smtp_addr, Duration::from_secs(5)).await;

    smtp_send_email(
        &smtp_addr,
        "sender@test.com",
        "target@example.com",
        "Retry Test",
        "Testing retry logic",
    )
    .await
    .unwrap();

    // Wait for retries (backoff: 100ms, 200ms, 400ms + processing time)
    tokio::time::sleep(Duration::from_secs(5)).await;

    let requests = get_mockserver_requests(&mock_url, "/webhook").await;
    assert_eq!(
        requests.len(),
        3,
        "Expected 3 webhook requests (2 failures + 1 success), got {}",
        requests.len()
    );

    runtime.shutdown_all().await.ok();
}

/// Monitor-mode DMARC smoke test. Drives the full acton pipeline with a
/// DmarcValidator configured, using the RFC 2606 `.invalid` TLD so the DMARC
/// record lookup is deterministically NXDOMAIN — producing a `none` outcome
/// without needing any specific network topology. Confirms that the webhook
/// payload gains the `dmarc_result` annotation and omits `authenticated_from`.
#[tokio::test]
async fn test_dmarc_monitor_mode_accepts_and_annotates() {
    init_crypto();
    let (_container, mock_url) = start_mockserver().await;
    configure_mockserver(&mock_url, "/webhook", 200, None, None).await;

    let smtp_port = get_free_port();
    let webhook_url = format!("{}/webhook", mock_url);
    let mut config = test_config(smtp_port, &webhook_url);
    config.dmarc_mode = mail_laser::config::DmarcMode::Monitor;
    // Small timeout so the test doesn't wait 5s on whatever the system
    // resolver does with `.invalid` lookups.
    config.dmarc_dns_timeout_secs = 3;

    let mut runtime = ActonApp::launch_async().await;
    let webhook_handle = WebhookState::create(&mut runtime, &config).await.unwrap();
    let dmarc = mail_laser::dmarc::DmarcValidator::load(&config)
        .expect("DMARC validator builds in monitor mode");
    assert!(dmarc.is_some(), "validator should be Some when mode=monitor");

    let _smtp_handle = SmtpListenerState::create(
        &mut runtime,
        &config,
        webhook_handle,
        test_policy(),
        test_backend(),
        dmarc,
    )
    .await
    .unwrap();

    let smtp_addr = format!("127.0.0.1:{}", smtp_port);
    wait_for_smtp(&smtp_addr, Duration::from_secs(5)).await;

    smtp_send_email(
        &smtp_addr,
        "sender@mail-laser-test.invalid",
        "target@example.com",
        "DMARC monitor smoke",
        "Hello from an unaligned sender.",
    )
    .await
    .unwrap();

    tokio::time::sleep(Duration::from_secs(2)).await;

    let requests = get_mockserver_requests(&mock_url, "/webhook").await;
    assert_eq!(
        requests.len(),
        1,
        "monitor mode must still forward to the webhook"
    );

    let req = &requests[0];
    let body_json: serde_json::Value = if let Some(json_val) = req["body"]["json"].as_object() {
        serde_json::Value::Object(json_val.clone())
    } else if let Some(s) = req["body"]["string"].as_str() {
        serde_json::from_str(s).expect("Webhook body should be valid JSON")
    } else if let Some(s) = req["body"].as_str() {
        serde_json::from_str(s).expect("Webhook body should be valid JSON")
    } else {
        panic!(
            "Could not extract body: {}",
            serde_json::to_string_pretty(&req["body"]).unwrap_or_default()
        );
    };

    // The .invalid TLD can't publish DMARC, so the outcome must be None or
    // TempError depending on how the local resolver answers NXDOMAIN.
    let dmarc_result = body_json["dmarc_result"]
        .as_str()
        .expect("dmarc_result present in monitor mode");
    assert!(
        matches!(dmarc_result, "none" | "temperror"),
        "expected none/temperror for .invalid TLD, got {}",
        dmarc_result
    );
    assert!(
        body_json.get("authenticated_from").is_none(),
        "authenticated_from must be omitted unless dmarc_result == pass"
    );

    runtime.shutdown_all().await.ok();
}

#[tokio::test]
async fn test_oversize_message_rejected_with_552() {
    init_crypto();
    let (_container, mock_url) = start_mockserver().await;
    configure_mockserver(&mock_url, "/webhook", 200, None, None).await;

    let smtp_port = get_free_port();
    let webhook_url = format!("{}/webhook", mock_url);
    let mut config = test_config(smtp_port, &webhook_url);
    config.max_message_size_bytes = 1024;

    let mut runtime = ActonApp::launch_async().await;
    let webhook_handle = WebhookState::create(&mut runtime, &config).await.unwrap();
    let _smtp_handle = SmtpListenerState::create(
        &mut runtime,
        &config,
        webhook_handle,
        test_policy(),
        test_backend(),
        None,
    )
    .await
    .unwrap();

    let smtp_addr = format!("127.0.0.1:{}", smtp_port);
    wait_for_smtp(&smtp_addr, Duration::from_secs(5)).await;

    let stream = TcpStream::connect(&smtp_addr).await.unwrap();
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();

    reader.read_line(&mut line).await.unwrap();
    assert!(line.starts_with("220"), "greeting: {}", line);

    write_half.write_all(b"EHLO oversize-test\r\n").await.unwrap();
    write_half.flush().await.unwrap();
    let mut saw_size = false;
    loop {
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "250-SIZE 1024" {
            saw_size = true;
        }
        assert!(line.starts_with("250"), "EHLO failed: {}", line);
        if line.starts_with("250 ") {
            break;
        }
    }
    assert!(
        saw_size,
        "EHLO must advertise the configured SIZE (250-SIZE 1024)"
    );

    write_half
        .write_all(b"MAIL FROM:<sender@test.com>\r\n")
        .await
        .unwrap();
    write_half.flush().await.unwrap();
    line.clear();
    reader.read_line(&mut line).await.unwrap();
    assert!(line.starts_with("250"), "MAIL FROM: {}", line);

    write_half
        .write_all(b"RCPT TO:<target@example.com>\r\n")
        .await
        .unwrap();
    write_half.flush().await.unwrap();
    line.clear();
    reader.read_line(&mut line).await.unwrap();
    assert!(line.starts_with("250"), "RCPT TO: {}", line);

    write_half.write_all(b"DATA\r\n").await.unwrap();
    write_half.flush().await.unwrap();
    line.clear();
    reader.read_line(&mut line).await.unwrap();
    assert!(line.starts_with("354"), "DATA: {}", line);

    let big_line = "A".repeat(1000);
    let email_content = format!(
        "From: sender@test.com\r\n\
         To: target@example.com\r\n\
         Subject: Oversize\r\n\
         \r\n\
         {}\r\n\
         {}\r\n\
         .\r\n",
        big_line, big_line
    );
    write_half.write_all(email_content.as_bytes()).await.unwrap();
    write_half.flush().await.unwrap();

    line.clear();
    reader.read_line(&mut line).await.unwrap();
    assert!(
        line.starts_with("552"),
        "Expected 552 reply for oversize message, got: {}",
        line
    );

    write_half.write_all(b"QUIT\r\n").await.unwrap();
    write_half.flush().await.unwrap();

    tokio::time::sleep(Duration::from_secs(1)).await;

    let requests = get_mockserver_requests(&mock_url, "/webhook").await;
    assert_eq!(
        requests.len(),
        0,
        "Oversize-rejected message must not trigger a webhook POST (got {})",
        requests.len()
    );

    runtime.shutdown_all().await.ok();
}

#[tokio::test]
async fn test_circuit_breaker_opens() {
    init_crypto();
    let (_container, mock_url) = start_mockserver().await;

    // All requests → 500
    configure_mockserver(&mock_url, "/webhook", 500, None, None).await;

    let smtp_port = get_free_port();
    let webhook_url = format!("{}/webhook", mock_url);
    let mut config = test_config(smtp_port, &webhook_url);
    config.circuit_breaker_threshold = 3;
    config.webhook_max_retries = 0; // No retries

    let mut runtime = ActonApp::launch_async().await;
    let webhook_handle = WebhookState::create(&mut runtime, &config).await.unwrap();
    let _smtp_handle = SmtpListenerState::create(&mut runtime, &config, webhook_handle, test_policy(), test_backend(), None)
        .await
        .unwrap();

    let smtp_addr = format!("127.0.0.1:{}", smtp_port);
    wait_for_smtp(&smtp_addr, Duration::from_secs(5)).await;

    // Send 3 emails to trigger circuit breaker threshold
    for i in 0..3 {
        smtp_send_email(
            &smtp_addr,
            "sender@test.com",
            "target@example.com",
            &format!("CB Test {}", i),
            &format!("Testing circuit breaker {}", i),
        )
        .await
        .unwrap();
        // Wait between emails so WebhookResult is processed and circuit breaker state updates
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // Send 4th email — should be dropped by open circuit breaker
    smtp_send_email(
        &smtp_addr,
        "sender@test.com",
        "target@example.com",
        "CB Test 3 (should be dropped)",
        "This should be dropped by circuit breaker",
    )
    .await
    .unwrap();

    // Wait for processing
    tokio::time::sleep(Duration::from_secs(1)).await;

    let requests = get_mockserver_requests(&mock_url, "/webhook").await;
    assert!(
        requests.len() <= 3,
        "Expected at most 3 webhook requests (4th dropped by circuit breaker), got {}",
        requests.len()
    );

    runtime.shutdown_all().await.ok();
}
