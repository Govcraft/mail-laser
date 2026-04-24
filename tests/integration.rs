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
use hickory_server::authority::{AuthorityObject, Catalog, ZoneType};
use hickory_server::proto::rr::rdata::{SOA, TXT};
use hickory_server::proto::rr::{LowerName, Name, RData, Record};
use hickory_server::store::in_memory::InMemoryAuthority;
use hickory_server::ServerFuture;
use mail_laser::attachment::{inline::InlineBackend, AttachmentBackend};
use mail_laser::config::{Config, DmarcMode};
use mail_laser::policy::PolicyEngine;
use mail_laser::smtp::SmtpListenerState;
use mail_laser::webhook::WebhookState;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use tokio_rustls::TlsConnector;
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

/// Policy that only permits SendMail when DMARC passes. With DMARC off the
/// `dmarc_result` context field is `"off"`, so this denies by default.
const DMARC_PASS_REQUIRED_POLICY: &str = r#"
    permit(principal, action == Action::"SendMail", resource)
      when { context.dmarc_result == "pass" };
    permit(principal, action == Action::"Attach", resource);
"#;

fn test_policy() -> Arc<PolicyEngine> {
    Arc::new(PolicyEngine::from_strings(TEST_POLICY, None).expect("test policy parses"))
}

fn dmarc_pass_required_policy() -> Arc<PolicyEngine> {
    Arc::new(
        PolicyEngine::from_strings(DMARC_PASS_REQUIRED_POLICY, None)
            .expect("dmarc-pass-required policy parses"),
    )
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
        max_concurrent_per_ip: 0,
        max_unknown_rcpts_per_session: 0,
    }
}

/// Spins up an in-process DNS authority on 127.0.0.1 with SOA + SPF + DMARC
/// TXT records for `domain`. Returns the bound `ip:port` string suitable for
/// `Config.dmarc_dns_servers`. The caller picks the SPF record content: use
/// `"v=spf1 -all"` to force SPF Fail (drives DMARC Fail), or
/// `"v=spf1 ip4:127.0.0.1 -all"` to authorize the loopback peer IP (drives
/// DMARC Pass because SPF is aligned to the From domain).
async fn start_dns_mock(domain: &str, spf_txt: &str, dmarc_txt: &str) -> String {
    let origin = Name::from_ascii(format!("{}.", domain)).expect("domain parses as DNS name");

    let mut authority = InMemoryAuthority::empty(origin.clone(), ZoneType::Primary, false);

    let soa_rdata = SOA::new(
        Name::from_ascii(format!("ns.{}.", domain)).unwrap(),
        Name::from_ascii(format!("admin.{}.", domain)).unwrap(),
        1,
        3600,
        600,
        604_800,
        60,
    );
    authority.upsert_mut(
        Record::from_rdata(origin.clone(), 60, RData::SOA(soa_rdata)),
        0,
    );
    authority.upsert_mut(
        Record::from_rdata(
            origin.clone(),
            60,
            RData::TXT(TXT::new(vec![spf_txt.to_string()])),
        ),
        1,
    );
    let dmarc_name =
        Name::from_ascii(format!("_dmarc.{}.", domain)).expect("dmarc name parses");
    authority.upsert_mut(
        Record::from_rdata(
            dmarc_name,
            60,
            RData::TXT(TXT::new(vec![dmarc_txt.to_string()])),
        ),
        2,
    );

    let mut catalog = Catalog::new();
    catalog.upsert(
        LowerName::new(&origin),
        vec![Arc::new(authority) as Arc<dyn AuthorityObject>],
    );

    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("DNS mock UDP bind");
    let addr = socket.local_addr().expect("DNS mock local_addr");

    let mut server = ServerFuture::new(catalog);
    server.register_socket(socket);
    tokio::spawn(async move {
        let _ = server.block_until_done().await;
    });

    addr.to_string()
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
    assert!(
        dmarc.is_some(),
        "validator should be Some when mode=monitor"
    );

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

    write_half
        .write_all(b"EHLO oversize-test\r\n")
        .await
        .unwrap();
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
    write_half
        .write_all(email_content.as_bytes())
        .await
        .unwrap();
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

/// When the Cedar policy requires `context.dmarc_result == "pass"` and DMARC
/// is disabled (result = "off"), the message must be rejected at end-of-DATA
/// with `550 5.7.1 Sender not authorized`. Confirms the SendMail evaluation
/// is now post-DMARC and actually consumes the context.
#[tokio::test]
async fn test_cedar_denies_at_end_of_data_when_dmarc_context_missing() {
    init_crypto();
    let (_container, mock_url) = start_mockserver().await;
    configure_mockserver(&mock_url, "/webhook", 200, None, None).await;

    let smtp_port = get_free_port();
    let webhook_url = format!("{}/webhook", mock_url);
    let config = test_config(smtp_port, &webhook_url);

    let mut runtime = ActonApp::launch_async().await;
    let webhook_handle = WebhookState::create(&mut runtime, &config).await.unwrap();
    let _smtp_handle = SmtpListenerState::create(
        &mut runtime,
        &config,
        webhook_handle,
        dmarc_pass_required_policy(),
        test_backend(),
        None, // DMARC off
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
    assert!(line.starts_with("220"));

    write_half.write_all(b"EHLO dmarc-test\r\n").await.unwrap();
    write_half.flush().await.unwrap();
    loop {
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        if line.starts_with("250 ") {
            break;
        }
    }

    // MAIL FROM must now succeed — Cedar eval is deferred until end-of-DATA.
    write_half
        .write_all(b"MAIL FROM:<spoofer@evil.example>\r\n")
        .await
        .unwrap();
    write_half.flush().await.unwrap();
    line.clear();
    reader.read_line(&mut line).await.unwrap();
    assert!(
        line.starts_with("250"),
        "MAIL FROM must be accepted provisionally (Cedar runs post-DMARC): {}",
        line
    );

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

    let email_content = "From: spoofer@evil.example\r\n\
                         To: target@example.com\r\n\
                         Subject: Denied\r\n\
                         \r\n\
                         Should be rejected by Cedar.\r\n\
                         .\r\n";
    write_half
        .write_all(email_content.as_bytes())
        .await
        .unwrap();
    write_half.flush().await.unwrap();

    line.clear();
    reader.read_line(&mut line).await.unwrap();
    assert!(
        line.starts_with("550 5.7.1"),
        "Cedar must deny with 550 5.7.1 at end-of-DATA, got: {}",
        line
    );

    write_half.write_all(b"QUIT\r\n").await.unwrap();
    write_half.flush().await.unwrap();

    tokio::time::sleep(Duration::from_secs(1)).await;

    let requests = get_mockserver_requests(&mock_url, "/webhook").await;
    assert_eq!(
        requests.len(),
        0,
        "Denied message must not trigger a webhook POST (got {})",
        requests.len()
    );

    runtime.shutdown_all().await.ok();
}

/// With `max_concurrent_per_ip = 2`, the third simultaneous connection from
/// the same peer IP must be dropped without an SMTP greeting.
///
/// Cap is 2 (not 1) to sidestep a race with `wait_for_smtp`, which briefly
/// opens a probe connection that occupies a slot until the server handler
/// observes EOF. A two-slot test run with three real connections is
/// race-free: the two held connections saturate the cap regardless of the
/// probe's state.
#[tokio::test]
async fn test_per_ip_connection_cap_drops_over_cap_connection() {
    init_crypto();
    let (_container, mock_url) = start_mockserver().await;
    configure_mockserver(&mock_url, "/webhook", 200, None, None).await;

    let smtp_port = get_free_port();
    let webhook_url = format!("{}/webhook", mock_url);
    let mut config = test_config(smtp_port, &webhook_url);
    config.max_concurrent_per_ip = 2;

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

    async fn open_and_read_greeting(addr: &str) -> (tokio::io::ReadHalf<TcpStream>, String) {
        let stream = TcpStream::connect(addr).await.expect("connect");
        let (read_half, _write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let mut line = String::new();
        tokio::time::timeout(Duration::from_secs(3), reader.read_line(&mut line))
            .await
            .expect("greeting not received in time")
            .expect("read ok");
        (reader.into_inner(), line)
    }

    // Fill both slots with long-lived connections, retry the greeting read in
    // case the probe connection from wait_for_smtp is still in its slot.
    async fn open_greeted(addr: &str) -> tokio::io::ReadHalf<TcpStream> {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let (reader, line) = open_and_read_greeting(addr).await;
            if line.starts_with("220") {
                return reader;
            }
            if std::time::Instant::now() > deadline {
                panic!("never received 220 greeting within deadline");
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    let _held_one = open_greeted(&smtp_addr).await;
    let _held_two = open_greeted(&smtp_addr).await;

    // Third connection — should be dropped without a greeting.
    let overflow = TcpStream::connect(&smtp_addr).await.unwrap();
    let (read_half, _write_half) = tokio::io::split(overflow);
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    let bytes = tokio::time::timeout(Duration::from_secs(3), reader.read_line(&mut line))
        .await
        .expect("over-cap connection must not hang — server should close it")
        .expect("read returns cleanly");
    assert_eq!(
        bytes, 0,
        "over-cap connection must be dropped without any greeting, got: {:?}",
        line
    );

    runtime.shutdown_all().await.ok();
}

/// On hitting the per-session unknown-RCPT cap, the server replies `421` and
/// closes the connection. Bounds recipient enumeration within a session.
#[tokio::test]
async fn test_unknown_rcpt_cap_closes_session() {
    init_crypto();
    let (_container, mock_url) = start_mockserver().await;
    configure_mockserver(&mock_url, "/webhook", 200, None, None).await;

    let smtp_port = get_free_port();
    let webhook_url = format!("{}/webhook", mock_url);
    let mut config = test_config(smtp_port, &webhook_url);
    // Cap of 2 → 1st unknown is tolerated (550), 2nd trips the drop (421).
    config.max_unknown_rcpts_per_session = 2;

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

    let stream = TcpStream::connect(&smtp_addr).await.expect("connect");
    let (read_half, write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);
    let mut writer = write_half;

    async fn read_reply(reader: &mut BufReader<tokio::io::ReadHalf<TcpStream>>) -> String {
        let mut line = String::new();
        tokio::time::timeout(Duration::from_secs(3), reader.read_line(&mut line))
            .await
            .expect("server response timed out")
            .expect("read ok");
        line
    }

    let greeting = read_reply(&mut reader).await;
    assert!(greeting.starts_with("220"), "greeting: {:?}", greeting);

    writer.write_all(b"HELO tester\r\n").await.unwrap();
    assert!(read_reply(&mut reader).await.starts_with("250"));

    writer
        .write_all(b"MAIL FROM:<sender@probe.example>\r\n")
        .await
        .unwrap();
    assert!(read_reply(&mut reader).await.starts_with("250"));

    // First unknown recipient — tolerated with 550.
    writer
        .write_all(b"RCPT TO:<nobody1@example.com>\r\n")
        .await
        .unwrap();
    let first = read_reply(&mut reader).await;
    assert!(first.starts_with("550"), "first unknown: {:?}", first);

    // Second unknown — hits cap, expect 421 and EOF.
    writer
        .write_all(b"RCPT TO:<nobody2@example.com>\r\n")
        .await
        .unwrap();
    let second = read_reply(&mut reader).await;
    assert!(
        second.starts_with("421"),
        "cap-triggering unknown should return 421, got: {:?}",
        second
    );

    // Server must close the socket after the 421.
    let mut trailing = String::new();
    let bytes = tokio::time::timeout(Duration::from_secs(3), reader.read_line(&mut trailing))
        .await
        .expect("post-421 read must not hang — server should close the socket")
        .expect("read returns cleanly");
    assert_eq!(
        bytes, 0,
        "server must close connection after 421, extra bytes: {:?}",
        trailing
    );

    runtime.shutdown_all().await.ok();
}

/// Below the cap, unknown recipients get a standard `550` and the session
/// continues normally (SMTP conformance: the 550 response is preserved).
#[tokio::test]
async fn test_unknown_rcpt_under_cap_returns_550_without_drop() {
    init_crypto();
    let (_container, mock_url) = start_mockserver().await;
    configure_mockserver(&mock_url, "/webhook", 200, None, None).await;

    let smtp_port = get_free_port();
    let webhook_url = format!("{}/webhook", mock_url);
    let mut config = test_config(smtp_port, &webhook_url);
    config.max_unknown_rcpts_per_session = 5;

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

    let stream = TcpStream::connect(&smtp_addr).await.expect("connect");
    let (read_half, write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);
    let mut writer = write_half;

    async fn read_reply(reader: &mut BufReader<tokio::io::ReadHalf<TcpStream>>) -> String {
        let mut line = String::new();
        tokio::time::timeout(Duration::from_secs(3), reader.read_line(&mut line))
            .await
            .expect("server response timed out")
            .expect("read ok");
        line
    }

    assert!(read_reply(&mut reader).await.starts_with("220"));
    writer.write_all(b"HELO tester\r\n").await.unwrap();
    assert!(read_reply(&mut reader).await.starts_with("250"));
    writer
        .write_all(b"MAIL FROM:<sender@probe.example>\r\n")
        .await
        .unwrap();
    assert!(read_reply(&mut reader).await.starts_with("250"));

    // Two unknowns — both should get 550 and the session should stay open.
    for addr in ["<nobody1@example.com>", "<nobody2@example.com>"] {
        writer
            .write_all(format!("RCPT TO:{}\r\n", addr).as_bytes())
            .await
            .unwrap();
        let reply = read_reply(&mut reader).await;
        assert!(reply.starts_with("550"), "expected 550, got: {:?}", reply);
    }

    // A known recipient still works (session not closed).
    writer
        .write_all(b"RCPT TO:<target@example.com>\r\n")
        .await
        .unwrap();
    let known = read_reply(&mut reader).await;
    assert!(
        known.starts_with("250"),
        "known recipient must be accepted after sub-cap 550s, got: {:?}",
        known
    );

    writer.write_all(b"QUIT\r\n").await.unwrap();
    let quit = read_reply(&mut reader).await;
    assert!(quit.starts_with("221"), "expected 221, got: {:?}", quit);

    runtime.shutdown_all().await.ok();
}

/// Under `DmarcMode::Enforce`, a message whose From-domain publishes a DMARC
/// record but has neither SPF nor DKIM aligned must be rejected at end-of-DATA
/// with `550 5.7.1 DMARC policy violation`. Uses an in-process DNS authority
/// to make the outcome deterministic (system DNS would give `none`/`temperror`
/// for `.invalid` TLD, which this test cannot exercise).
#[tokio::test]
async fn test_dmarc_enforce_rejects_fail_with_550() {
    init_crypto();
    let (_container, mock_url) = start_mockserver().await;
    configure_mockserver(&mock_url, "/webhook", 200, None, None).await;

    // DNS mock: SPF `-all` forces SPF Fail; no DKIM signature → DKIM None;
    // DMARC record present → outcome Fail (not NoPolicy).
    let dns_addr = start_dns_mock(
        "dmarcfail.example",
        "v=spf1 -all",
        "v=DMARC1; p=reject;",
    )
    .await;

    let smtp_port = get_free_port();
    let webhook_url = format!("{}/webhook", mock_url);
    let mut config = test_config(smtp_port, &webhook_url);
    config.dmarc_mode = DmarcMode::Enforce;
    config.dmarc_dns_servers = vec![dns_addr];
    config.dmarc_dns_timeout_secs = 5;

    let dmarc = mail_laser::dmarc::DmarcValidator::load(&config)
        .expect("DMARC validator builds in enforce mode")
        .expect("validator must be Some when mode != Off");

    let mut runtime = ActonApp::launch_async().await;
    let webhook_handle = WebhookState::create(&mut runtime, &config).await.unwrap();
    let _smtp_handle = SmtpListenerState::create(
        &mut runtime,
        &config,
        webhook_handle,
        test_policy(),
        test_backend(),
        Some(dmarc),
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
    assert!(line.starts_with("220"));

    write_half
        .write_all(b"EHLO sender.dmarcfail.example\r\n")
        .await
        .unwrap();
    write_half.flush().await.unwrap();
    loop {
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        if line.starts_with("250 ") {
            break;
        }
    }

    write_half
        .write_all(b"MAIL FROM:<sender@dmarcfail.example>\r\n")
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

    let email_content = "From: sender@dmarcfail.example\r\n\
                         To: target@example.com\r\n\
                         Subject: DMARC enforce smoke\r\n\
                         \r\n\
                         Body that should be rejected by DMARC enforce.\r\n\
                         .\r\n";
    write_half
        .write_all(email_content.as_bytes())
        .await
        .unwrap();
    write_half.flush().await.unwrap();

    line.clear();
    reader.read_line(&mut line).await.unwrap();
    assert!(
        line.starts_with("550 5.7.1"),
        "enforce-mode DMARC fail must 550 5.7.1 at end-of-DATA, got: {}",
        line
    );
    assert!(
        line.to_ascii_lowercase().contains("dmarc"),
        "reply should mention DMARC, got: {}",
        line
    );

    write_half.write_all(b"QUIT\r\n").await.unwrap();
    write_half.flush().await.unwrap();

    tokio::time::sleep(Duration::from_secs(1)).await;

    let requests = get_mockserver_requests(&mock_url, "/webhook").await;
    assert_eq!(
        requests.len(),
        0,
        "rejected message must not hit the webhook (got {})",
        requests.len()
    );

    runtime.shutdown_all().await.ok();
}

/// Positive counterpart to `test_cedar_denies_at_end_of_data_when_dmarc_context_missing`:
/// with DMARC fully passing and the `dmarc_pass_required_policy` in force,
/// Cedar must allow SendMail, the message must be forwarded, and the webhook
/// payload must carry `dmarc_result=pass` plus a populated `authenticated_from`.
#[tokio::test]
async fn test_cedar_allows_when_dmarc_passes() {
    init_crypto();
    let (_container, mock_url) = start_mockserver().await;
    configure_mockserver(&mock_url, "/webhook", 200, None, None).await;

    // SPF explicitly authorizes the loopback IP → SPF Pass → DMARC Pass
    // (DKIM can remain absent because SPF alone is enough for alignment when
    // the envelope-from domain matches the From-header domain).
    let dns_addr = start_dns_mock(
        "dmarcpass.example",
        "v=spf1 ip4:127.0.0.1 -all",
        "v=DMARC1; p=reject;",
    )
    .await;

    let smtp_port = get_free_port();
    let webhook_url = format!("{}/webhook", mock_url);
    let mut config = test_config(smtp_port, &webhook_url);
    config.dmarc_mode = DmarcMode::Enforce;
    config.dmarc_dns_servers = vec![dns_addr];
    config.dmarc_dns_timeout_secs = 5;

    let dmarc = mail_laser::dmarc::DmarcValidator::load(&config)
        .expect("DMARC validator builds")
        .expect("validator must be Some when mode != Off");

    let mut runtime = ActonApp::launch_async().await;
    let webhook_handle = WebhookState::create(&mut runtime, &config).await.unwrap();
    let _smtp_handle = SmtpListenerState::create(
        &mut runtime,
        &config,
        webhook_handle,
        dmarc_pass_required_policy(),
        test_backend(),
        Some(dmarc),
    )
    .await
    .unwrap();

    let smtp_addr = format!("127.0.0.1:{}", smtp_port);
    wait_for_smtp(&smtp_addr, Duration::from_secs(5)).await;

    smtp_send_email(
        &smtp_addr,
        "sender@dmarcpass.example",
        "target@example.com",
        "DMARC pass + Cedar allow",
        "Hello from an aligned, SPF-passing sender.",
    )
    .await
    .expect("DMARC-passing message must be accepted end-to-end");

    // Poll the mockserver — webhook delivery is async relative to DATA ack.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let requests = loop {
        let reqs = get_mockserver_requests(&mock_url, "/webhook").await;
        if !reqs.is_empty() || std::time::Instant::now() > deadline {
            break reqs;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };

    assert_eq!(
        requests.len(),
        1,
        "Cedar-allowed message must be forwarded exactly once, got {}",
        requests.len()
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
    assert_eq!(
        body_json["dmarc_result"].as_str(),
        Some("pass"),
        "dmarc_result must be pass in payload: {}",
        body_json
    );
    assert_eq!(
        body_json["authenticated_from"].as_str(),
        Some("sender@dmarcpass.example"),
        "authenticated_from must be the aligned From address: {}",
        body_json
    );

    runtime.shutdown_all().await.ok();
}

/// rustls `ServerCertVerifier` that accepts any certificate. Test-only — the
/// server generates a self-signed cert on every STARTTLS upgrade, so there is
/// nothing stable to pin to and no CA anchor is meaningful for this harness.
#[derive(Debug)]
struct AcceptAnyServerCert;

impl ServerCertVerifier for AcceptAnyServerCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::ED25519,
        ]
    }
}

/// A real TLS client completes the STARTTLS handshake and delivers a message
/// over the encrypted channel. Exercises the server's on-the-fly self-signed
/// cert generation (`generate_self_signed_cert` in src/smtp/mod.rs) and the
/// full plaintext → secure session transition (`handle_starttls` →
/// `handle_secure_session`).
#[tokio::test]
async fn test_starttls_end_to_end() {
    init_crypto();
    let (_container, mock_url) = start_mockserver().await;
    configure_mockserver(&mock_url, "/webhook", 200, None, None).await;

    let smtp_port = get_free_port();
    let webhook_url = format!("{}/webhook", mock_url);
    let config = test_config(smtp_port, &webhook_url);

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

    let plaintext = TcpStream::connect(&smtp_addr).await.expect("connect");
    let (read_half, mut write_half) = tokio::io::split(plaintext);
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();

    reader.read_line(&mut line).await.unwrap();
    assert!(line.starts_with("220"), "greeting: {}", line);

    // EHLO must advertise STARTTLS.
    write_half.write_all(b"EHLO tls-test\r\n").await.unwrap();
    write_half.flush().await.unwrap();
    let mut saw_starttls = false;
    loop {
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        if line.to_ascii_uppercase().contains("STARTTLS") {
            saw_starttls = true;
        }
        if line.starts_with("250 ") {
            break;
        }
        assert!(line.starts_with("250"), "EHLO line: {}", line);
    }
    assert!(saw_starttls, "EHLO must advertise STARTTLS");

    write_half.write_all(b"STARTTLS\r\n").await.unwrap();
    write_half.flush().await.unwrap();
    line.clear();
    reader.read_line(&mut line).await.unwrap();
    assert!(line.starts_with("220"), "STARTTLS reply: {}", line);

    // Re-assemble the underlying TcpStream for the TLS handshake.
    let read_half = reader.into_inner();
    let plaintext = read_half.unsplit(write_half);

    let client_config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyServerCert))
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(client_config));
    let server_name = ServerName::try_from("localhost").unwrap();
    let tls_stream = connector
        .connect(server_name, plaintext)
        .await
        .expect("TLS handshake must succeed");

    let (tls_read, mut tls_write) = tokio::io::split(tls_stream);
    let mut reader = BufReader::new(tls_read);

    // Post-handshake: server does NOT send a fresh greeting (see
    // handle_secure_session in src/smtp/mod.rs:240). Client must drive.
    tls_write.write_all(b"EHLO tls-test\r\n").await.unwrap();
    tls_write.flush().await.unwrap();
    loop {
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        if line.starts_with("250 ") {
            break;
        }
        assert!(line.starts_with("250"), "post-TLS EHLO: {}", line);
    }

    tls_write
        .write_all(b"MAIL FROM:<sender@tls.example>\r\n")
        .await
        .unwrap();
    tls_write.flush().await.unwrap();
    line.clear();
    reader.read_line(&mut line).await.unwrap();
    assert!(line.starts_with("250"), "MAIL FROM: {}", line);

    tls_write
        .write_all(b"RCPT TO:<target@example.com>\r\n")
        .await
        .unwrap();
    tls_write.flush().await.unwrap();
    line.clear();
    reader.read_line(&mut line).await.unwrap();
    assert!(line.starts_with("250"), "RCPT TO: {}", line);

    tls_write.write_all(b"DATA\r\n").await.unwrap();
    tls_write.flush().await.unwrap();
    line.clear();
    reader.read_line(&mut line).await.unwrap();
    assert!(line.starts_with("354"), "DATA: {}", line);

    let body = "From: sender@tls.example\r\n\
                To: target@example.com\r\n\
                Subject: STARTTLS smoke\r\n\
                \r\n\
                Delivered over TLS.\r\n\
                .\r\n";
    tls_write.write_all(body.as_bytes()).await.unwrap();
    tls_write.flush().await.unwrap();
    line.clear();
    reader.read_line(&mut line).await.unwrap();
    assert!(
        line.starts_with("250"),
        "end-of-DATA over TLS must 250, got: {}",
        line
    );

    tls_write.write_all(b"QUIT\r\n").await.unwrap();
    tls_write.flush().await.unwrap();

    // Webhook delivery is async — poll briefly.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let requests = loop {
        let reqs = get_mockserver_requests(&mock_url, "/webhook").await;
        if !reqs.is_empty() || std::time::Instant::now() > deadline {
            break reqs;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(
        requests.len(),
        1,
        "message delivered over TLS must hit the webhook exactly once, got {}",
        requests.len()
    );

    runtime.shutdown_all().await.ok();
}
