//! End-to-end integration test for the S3 attachment delivery backend.
//!
//! Spins up MinIO + MockServer via `testcontainers`, drives mail-laser over
//! raw SMTP with a multipart/mixed message carrying an attachment, and
//! verifies the webhook payload shape, the `s3://` URL, the optional
//! presigned URL, and that the uploaded bytes round-trip.
//!
//! Run with: cargo test --test s3_attachment
//! Requires Docker.

use std::net::TcpListener as StdTcpListener;
use std::sync::Arc;
use std::time::Duration;

use acton_reactive::prelude::*;
use aws_config::BehaviorVersion;
use aws_sdk_s3::config::Region;
use aws_sdk_s3::Client as S3Client;
use base64::Engine;
use mail_laser::config::{AttachmentDelivery, Config, S3Settings};
use mail_laser::policy::PolicyEngine;
use mail_laser::smtp::SmtpListenerState;
use mail_laser::webhook::WebhookState;
use once_cell::sync::Lazy;
use testcontainers::core::wait::WaitFor;
use testcontainers::core::IntoContainerPort;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use uuid::Uuid;

// --- One-time environment setup -------------------------------------------

static AWS_ENV: Lazy<()> = Lazy::new(|| {
    // Safety: runs exactly once per test binary before any async work is
    // scheduled, so no other thread can be reading these vars concurrently.
    unsafe {
        std::env::set_var("AWS_ACCESS_KEY_ID", "minioadmin");
        std::env::set_var("AWS_SECRET_ACCESS_KEY", "minioadmin");
        std::env::set_var("AWS_DEFAULT_REGION", "us-east-1");
        std::env::set_var("AWS_REGION", "us-east-1");
        std::env::set_var("AWS_SHARED_CREDENTIALS_FILE", "/dev/null");
        std::env::set_var("AWS_CONFIG_FILE", "/dev/null");
        std::env::set_var("AWS_EC2_METADATA_DISABLED", "true");
    }
});

fn init_env() {
    Lazy::force(&AWS_ENV);
}

fn init_crypto() {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok();
}

// --- General helpers ------------------------------------------------------

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

// --- MockServer (webhook receiver) ----------------------------------------

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

async fn configure_mockserver(base_url: &str, path: &str, status: u16) {
    let client = reqwest::Client::new();

    let expectation = serde_json::json!({
        "httpRequest": { "method": "POST", "path": path },
        "httpResponse": { "statusCode": status },
        "times": { "unlimited": true },
    });

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

    let body = serde_json::json!({ "method": "POST", "path": path });

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

fn extract_webhook_body_json(req: &serde_json::Value) -> serde_json::Value {
    if let Some(json_val) = req["body"]["json"].as_object() {
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
    }
}

// --- MinIO container ------------------------------------------------------

async fn start_minio() -> (ContainerAsync<GenericImage>, String) {
    let container = GenericImage::new("minio/minio", "RELEASE.2024-12-18T13-15-44Z")
        .with_exposed_port(9000.tcp())
        .with_wait_for(WaitFor::seconds(1))
        .with_env_var("MINIO_ROOT_USER", "minioadmin")
        .with_env_var("MINIO_ROOT_PASSWORD", "minioadmin")
        .with_cmd(["server", "/data", "--address", ":9000"])
        .start()
        .await
        .expect("Failed to start MinIO container");

    let host_port = container
        .get_host_port_ipv4(9000.tcp())
        .await
        .expect("Failed to get MinIO host port");
    let endpoint = format!("http://127.0.0.1:{}", host_port);

    let client = reqwest::Client::new();
    let probe = format!("{}/minio/health/live", endpoint);
    let start = std::time::Instant::now();
    loop {
        if let Ok(resp) = client.get(&probe).send().await {
            if resp.status().is_success() {
                break;
            }
        }
        if start.elapsed() > Duration::from_secs(30) {
            panic!("MinIO at {} did not become ready within 30s", endpoint);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    (container, endpoint)
}

async fn create_bucket(endpoint: &str, bucket: &str) -> anyhow::Result<S3Client> {
    let cfg = aws_config::defaults(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .endpoint_url(endpoint)
        .load()
        .await;
    let s3_cfg = aws_sdk_s3::config::Builder::from(&cfg)
        .force_path_style(true)
        .build();
    let client = S3Client::from_conf(s3_cfg);

    match client.create_bucket().bucket(bucket).send().await {
        Ok(_) => Ok(client),
        Err(err) => {
            let rendered = format!("{:?}", err);
            if rendered.contains("BucketAlreadyOwnedByYou")
                || rendered.contains("BucketAlreadyExists")
            {
                Ok(client)
            } else {
                Err(err.into())
            }
        }
    }
}

// --- SMTP with a MIME attachment ------------------------------------------

async fn smtp_send_email_with_attachment(
    addr: &str,
    sender: &str,
    recipient: &str,
    subject: &str,
    filename: &str,
    content_type: &str,
    body_bytes: &[u8],
) -> anyhow::Result<()> {
    let stream = TcpStream::connect(addr).await?;
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();

    reader.read_line(&mut line).await?;
    assert!(
        line.starts_with("220"),
        "Expected 220 greeting, got: {}",
        line
    );

    write_half.write_all(b"EHLO test\r\n").await?;
    write_half.flush().await?;
    loop {
        line.clear();
        reader.read_line(&mut line).await?;
        if line.starts_with("250 ") {
            break;
        }
        assert!(line.starts_with("250"), "EHLO failed: {}", line);
    }

    write_half
        .write_all(format!("MAIL FROM:<{}>\r\n", sender).as_bytes())
        .await?;
    write_half.flush().await?;
    line.clear();
    reader.read_line(&mut line).await?;
    assert!(line.starts_with("250"), "MAIL FROM failed: {}", line);

    write_half
        .write_all(format!("RCPT TO:<{}>\r\n", recipient).as_bytes())
        .await?;
    write_half.flush().await?;
    line.clear();
    reader.read_line(&mut line).await?;
    assert!(line.starts_with("250"), "RCPT TO failed: {}", line);

    write_half.write_all(b"DATA\r\n").await?;
    write_half.flush().await?;
    line.clear();
    reader.read_line(&mut line).await?;
    assert!(line.starts_with("354"), "DATA failed: {}", line);

    let body_b64 = base64::engine::general_purpose::STANDARD.encode(body_bytes);
    let email_content = format!(
        "From: {sender}\r\n\
         To: {recipient}\r\n\
         Subject: {subject}\r\n\
         MIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"b1\"\r\n\
         \r\n\
         --b1\r\n\
         Content-Type: text/plain; charset=UTF-8\r\n\
         \r\n\
         hello\r\n\
         \r\n\
         --b1\r\n\
         Content-Type: {content_type}; name=\"{filename}\"\r\n\
         Content-Disposition: attachment; filename=\"{filename}\"\r\n\
         Content-Transfer-Encoding: base64\r\n\
         \r\n\
         {body_b64}\r\n\
         --b1--\r\n\
         .\r\n"
    );
    write_half.write_all(email_content.as_bytes()).await?;
    write_half.flush().await?;
    line.clear();
    reader.read_line(&mut line).await?;
    assert!(line.starts_with("250"), "DATA end failed: {}", line);

    write_half.write_all(b"QUIT\r\n").await?;
    write_half.flush().await?;

    Ok(())
}

// --- Policy + Config helpers ---------------------------------------------

const TEST_POLICY: &str = r#"
    permit(principal, action == Action::"SendMail", resource);
    permit(principal, action == Action::"Attach", resource);
"#;

fn test_policy() -> Arc<PolicyEngine> {
    Arc::new(PolicyEngine::from_strings(TEST_POLICY, None).expect("test policy parses"))
}

fn s3_config(
    smtp_port: u16,
    webhook_url: &str,
    bucket: &str,
    endpoint: &str,
    presign_ttl_secs: Option<u64>,
) -> Config {
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
        cedar_policies_path: std::path::PathBuf::from("tests/fixtures/integration.cedar"),
        cedar_entities_path: None,
        max_message_size_bytes: 26_214_400,
        max_attachment_size_bytes: 10_485_760,
        attachment_delivery: AttachmentDelivery::S3(S3Settings {
            bucket: bucket.to_string(),
            region: "us-east-1".to_string(),
            endpoint: Some(endpoint.to_string()),
            key_prefix: "inbound/".to_string(),
            presign_ttl_secs,
        }),
        dmarc_mode: mail_laser::config::DmarcMode::Off,
        dmarc_dns_timeout_secs: 5,
        dmarc_dns_servers: vec![],
        dmarc_temperror_action: mail_laser::config::DmarcTempErrorAction::Reject,
    }
}

// --- Tests ----------------------------------------------------------------

#[tokio::test]
async fn s3_attachment_delivery_without_presigned_url() {
    init_env();
    init_crypto();

    let (_minio, minio_url) = start_minio().await;
    let bucket = format!("ml-test-{}", Uuid::new_v4().simple());
    let s3 = create_bucket(&minio_url, &bucket)
        .await
        .expect("create bucket");

    let (_mock, mock_url) = start_mockserver().await;
    configure_mockserver(&mock_url, "/webhook", 200).await;

    let smtp_port = get_free_port();
    let webhook_url = format!("{}/webhook", mock_url);
    let config = s3_config(smtp_port, &webhook_url, &bucket, &minio_url, None);

    let mut runtime = ActonApp::launch_async().await;
    let backend = mail_laser::attachment::build(&config)
        .await
        .expect("build backend");
    let webhook_handle = WebhookState::create(&mut runtime, &config).await.unwrap();
    let _smtp = SmtpListenerState::create(
        &mut runtime,
        &config,
        webhook_handle,
        test_policy(),
        backend,
        None,
    )
    .await
    .unwrap();

    let smtp_addr = format!("127.0.0.1:{}", smtp_port);
    wait_for_smtp(&smtp_addr, Duration::from_secs(5)).await;

    let payload: &[u8] = b"%PDF-1.4\n%MAIL-LASER-TEST\n\xde\xad\xbe\xef\n%%EOF";
    smtp_send_email_with_attachment(
        &smtp_addr,
        "sender@test.com",
        "target@example.com",
        "S3 Attachment Test",
        "test.pdf",
        "application/pdf",
        payload,
    )
    .await
    .unwrap();

    tokio::time::sleep(Duration::from_secs(2)).await;

    let requests = get_mockserver_requests(&mock_url, "/webhook").await;
    assert_eq!(requests.len(), 1, "expected exactly 1 webhook POST");

    let body = extract_webhook_body_json(&requests[0]);
    assert_eq!(body["sender"], "sender@test.com");
    assert_eq!(body["recipient"], "target@example.com");
    assert_eq!(body["subject"], "S3 Attachment Test");

    let attachments = body["attachments"].as_array().expect("attachments array");
    assert_eq!(attachments.len(), 1, "expected exactly one attachment");

    let att = &attachments[0];
    assert_eq!(att["filename"], "test.pdf");
    assert_eq!(att["content_type"], "application/pdf");
    assert_eq!(att["size_bytes"].as_u64().unwrap(), payload.len() as u64);
    assert_eq!(att["delivery"], "s3");
    let url = att["url"].as_str().expect("url present");
    let prefix = format!("s3://{}/inbound/", bucket);
    assert!(url.starts_with(&prefix), "unexpected url: {url}");
    assert!(url.ends_with("-test.pdf"), "unexpected url: {url}");
    assert!(
        att.get("presigned_url").is_none() || att["presigned_url"].is_null(),
        "should not have presigned_url when ttl is None, got: {att}"
    );

    let key = url.strip_prefix(&format!("s3://{}/", bucket)).unwrap();
    let got = s3
        .get_object()
        .bucket(&bucket)
        .key(key)
        .send()
        .await
        .expect("get_object");
    let bytes = got.body.collect().await.expect("collect body").into_bytes();
    assert_eq!(
        bytes.as_ref(),
        payload,
        "round-tripped bytes differ from original"
    );

    runtime.shutdown_all().await.ok();
}

#[tokio::test]
async fn s3_attachment_delivery_with_presigned_url() {
    init_env();
    init_crypto();

    let (_minio, minio_url) = start_minio().await;
    let bucket = format!("ml-test-{}", Uuid::new_v4().simple());
    let _s3 = create_bucket(&minio_url, &bucket)
        .await
        .expect("create bucket");

    let (_mock, mock_url) = start_mockserver().await;
    configure_mockserver(&mock_url, "/webhook", 200).await;

    let smtp_port = get_free_port();
    let webhook_url = format!("{}/webhook", mock_url);
    let config = s3_config(smtp_port, &webhook_url, &bucket, &minio_url, Some(600));

    let mut runtime = ActonApp::launch_async().await;
    let backend = mail_laser::attachment::build(&config)
        .await
        .expect("build backend");
    let webhook_handle = WebhookState::create(&mut runtime, &config).await.unwrap();
    let _smtp = SmtpListenerState::create(
        &mut runtime,
        &config,
        webhook_handle,
        test_policy(),
        backend,
        None,
    )
    .await
    .unwrap();

    let smtp_addr = format!("127.0.0.1:{}", smtp_port);
    wait_for_smtp(&smtp_addr, Duration::from_secs(5)).await;

    let payload: &[u8] = b"%PDF-1.4\n%MAIL-LASER-PRESIGN\n\xca\xfe\xba\xbe\n%%EOF";
    smtp_send_email_with_attachment(
        &smtp_addr,
        "sender@test.com",
        "target@example.com",
        "S3 Presigned Test",
        "test.pdf",
        "application/pdf",
        payload,
    )
    .await
    .unwrap();

    tokio::time::sleep(Duration::from_secs(2)).await;

    let requests = get_mockserver_requests(&mock_url, "/webhook").await;
    assert_eq!(requests.len(), 1, "expected exactly 1 webhook POST");

    let body = extract_webhook_body_json(&requests[0]);
    let attachments = body["attachments"].as_array().expect("attachments array");
    assert_eq!(attachments.len(), 1);

    let att = &attachments[0];
    assert_eq!(att["delivery"], "s3");
    assert_eq!(att["filename"], "test.pdf");
    assert_eq!(att["size_bytes"].as_u64().unwrap(), payload.len() as u64);
    let url = att["url"].as_str().expect("url present");
    assert!(url.starts_with(&format!("s3://{}/inbound/", bucket)));

    let presigned = att["presigned_url"]
        .as_str()
        .expect("presigned_url present when ttl is Some(_)");
    assert!(
        presigned.starts_with("http://"),
        "presigned URL should be an http endpoint: {presigned}"
    );
    assert!(
        presigned.contains("X-Amz-"),
        "presigned URL should include SigV4 query parameters: {presigned}"
    );

    let resp = tokio::time::timeout(
        Duration::from_secs(5),
        reqwest::Client::new().get(presigned).send(),
    )
    .await
    .expect("presigned fetch timeout")
    .expect("presigned fetch failed");
    assert!(
        resp.status().is_success(),
        "presigned GET returned {}",
        resp.status()
    );
    let bytes = resp.bytes().await.expect("read presigned body");
    assert_eq!(
        bytes.as_ref(),
        payload,
        "presigned-URL bytes differ from original"
    );

    runtime.shutdown_all().await.ok();
}
