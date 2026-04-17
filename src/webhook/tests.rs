use super::*;
use crate::config::{AttachmentDelivery, Config, DmarcMode, DmarcTempErrorAction};
use std::collections::HashMap;
use std::path::PathBuf;

fn test_config() -> Config {
    Config {
        webhook_url: "http://example.com/webhook".to_string(),
        target_emails: vec!["test@example.com".to_string()],
        smtp_bind_address: "127.0.0.1".to_string(),
        smtp_port: 2525,
        health_check_bind_address: "127.0.0.1".to_string(),
        health_check_port: 8080,
        header_prefixes: vec![],
        webhook_timeout_secs: 30,
        webhook_max_retries: 3,
        circuit_breaker_threshold: 5,
        circuit_breaker_reset_secs: 60,
        webhook_signing_secret: None,
        cedar_policies_path: PathBuf::from("/tmp/policies.cedar"),
        cedar_entities_path: None,
        max_message_size_bytes: 26_214_400,
        max_attachment_size_bytes: 10_485_760,
        attachment_delivery: AttachmentDelivery::Inline,
        dmarc_mode: DmarcMode::Off,
        dmarc_dns_timeout_secs: 5,
        dmarc_dns_servers: vec![],
        dmarc_temperror_action: DmarcTempErrorAction::Reject,
    }
}

#[test]
fn test_webhook_client_user_agent() {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok();
    let config = test_config();
    let client = WebhookClient::new(config);

    let expected_user_agent = format!("{}/{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));

    assert_eq!(client.user_agent, expected_user_agent);
}

// --- HMAC signing tests ---

#[test]
fn test_compute_signature_is_hex_sha256_of_timestamp_dot_body() {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    let secret = b"test-secret";
    let timestamp: u64 = 1_700_000_000;
    let body = br#"{"hello":"world"}"#;

    let got = super::compute_signature(secret, timestamp, body);

    let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    let expected = hex::encode(mac.finalize().into_bytes());

    assert_eq!(got, expected);
    assert_eq!(got.len(), 64, "SHA-256 hex digest is 64 chars");
}

#[test]
fn test_compute_signature_changes_when_any_input_changes() {
    let base = super::compute_signature(b"secret", 100, b"body");
    assert_ne!(base, super::compute_signature(b"secret2", 100, b"body"));
    assert_ne!(base, super::compute_signature(b"secret", 101, b"body"));
    assert_ne!(base, super::compute_signature(b"secret", 100, b"body2"));
}

#[test]
fn test_webhook_client_stores_signing_secret_bytes_when_configured() {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok();
    let mut config = test_config();
    config.webhook_signing_secret = Some("shh".to_string());
    let client = WebhookClient::new(config);
    assert_eq!(client.signing_secret.as_deref(), Some(&b"shh"[..]));
}

#[test]
fn test_webhook_client_has_no_signing_secret_when_unset() {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok();
    let client = WebhookClient::new(test_config());
    assert!(client.signing_secret.is_none());
}

#[test]
fn test_signature_prefix_envelope_uses_sha256_scheme() {
    // Documents the header-value convention consumers rely on:
    // `X-MailLaser-Signature-256: sha256=<hex>`.
    let hex_sig = super::compute_signature(b"k", 1, b"b");
    let header_value = format!("sha256={hex_sig}");
    assert!(header_value.starts_with("sha256="));
    assert_eq!(header_value.len(), "sha256=".len() + 64);
}

// --- EmailPayload serialization tests ---

#[test]
fn test_email_payload_serialization_all_fields() {
    let mut headers = HashMap::new();
    headers.insert("X-Custom-Id".to_string(), "abc123".to_string());
    headers.insert("X-Priority".to_string(), "high".to_string());

    let payload = EmailPayload {
        sender: "sender@example.com".to_string(),
        sender_name: Some("John Doe".to_string()),
        recipient: "recipient@example.com".to_string(),
        subject: "Test Subject".to_string(),
        body: "Plain text body".to_string(),
        html_body: Some("<p>HTML body</p>".to_string()),
        headers: Some(headers),
        attachments: None,
        dmarc_result: None,
        authenticated_from: None,
    };

    let json = serde_json::to_value(&payload).expect("Serialization failed");

    assert_eq!(json["sender"], "sender@example.com");
    assert_eq!(json["sender_name"], "John Doe");
    assert_eq!(json["recipient"], "recipient@example.com");
    assert_eq!(json["subject"], "Test Subject");
    assert_eq!(json["body"], "Plain text body");
    assert_eq!(json["html_body"], "<p>HTML body</p>");
    assert_eq!(json["headers"]["X-Custom-Id"], "abc123");
    assert_eq!(json["headers"]["X-Priority"], "high");
}

#[test]
fn test_email_payload_serialization_required_only() {
    let payload = EmailPayload {
        sender: "sender@example.com".to_string(),
        sender_name: None,
        recipient: "recipient@example.com".to_string(),
        subject: "Test Subject".to_string(),
        body: "Plain text body".to_string(),
        html_body: None,
        headers: None,
        attachments: None,
        dmarc_result: None,
        authenticated_from: None,
    };

    let json = serde_json::to_value(&payload).expect("Serialization failed");

    assert_eq!(json["sender"], "sender@example.com");
    assert_eq!(json["recipient"], "recipient@example.com");
    assert_eq!(json["subject"], "Test Subject");
    assert_eq!(json["body"], "Plain text body");

    // Optional fields should be absent (not null) due to skip_serializing_if
    assert!(json.get("sender_name").is_none());
    assert!(json.get("html_body").is_none());
    assert!(json.get("headers").is_none());
}

#[test]
fn test_email_payload_deserialization_roundtrip() {
    let mut headers = HashMap::new();
    headers.insert("X-Tracking".to_string(), "track-001".to_string());

    let original = EmailPayload {
        sender: "roundtrip@example.com".to_string(),
        sender_name: Some("Roundtrip User".to_string()),
        recipient: "dest@example.com".to_string(),
        subject: "Roundtrip Test".to_string(),
        body: "This is the body text.".to_string(),
        html_body: Some("<b>Bold body</b>".to_string()),
        headers: Some(headers),
        attachments: None,
        dmarc_result: None,
        authenticated_from: None,
    };

    let json_string = serde_json::to_string(&original).expect("Serialization failed");
    let deserialized: EmailPayload =
        serde_json::from_str(&json_string).expect("Deserialization failed");

    assert_eq!(deserialized.sender, original.sender);
    assert_eq!(deserialized.sender_name, original.sender_name);
    assert_eq!(deserialized.recipient, original.recipient);
    assert_eq!(deserialized.subject, original.subject);
    assert_eq!(deserialized.body, original.body);
    assert_eq!(deserialized.html_body, original.html_body);
    assert_eq!(deserialized.headers, original.headers);
}

#[test]
fn test_email_payload_deserialization_roundtrip_required_only() {
    let original = EmailPayload {
        sender: "minimal@example.com".to_string(),
        sender_name: None,
        recipient: "dest@example.com".to_string(),
        subject: "Minimal".to_string(),
        body: "Body only.".to_string(),
        html_body: None,
        headers: None,
        attachments: None,
        dmarc_result: None,
        authenticated_from: None,
    };

    let json_string = serde_json::to_string(&original).expect("Serialization failed");
    let deserialized: EmailPayload =
        serde_json::from_str(&json_string).expect("Deserialization failed");

    assert_eq!(deserialized.sender, original.sender);
    assert_eq!(deserialized.sender_name, None);
    assert_eq!(deserialized.recipient, original.recipient);
    assert_eq!(deserialized.subject, original.subject);
    assert_eq!(deserialized.body, original.body);
    assert_eq!(deserialized.html_body, None);
    assert_eq!(deserialized.headers, None);
}

#[test]
fn test_email_payload_skip_serializing_none_fields() {
    let payload = EmailPayload {
        sender: "test@example.com".to_string(),
        sender_name: None,
        recipient: "dest@example.com".to_string(),
        subject: "Skip Test".to_string(),
        body: "Body.".to_string(),
        html_body: None,
        headers: None,
        attachments: None,
        dmarc_result: None,
        authenticated_from: None,
    };

    let json_string = serde_json::to_string(&payload).expect("Serialization failed");

    assert!(!json_string.contains("sender_name"));
    assert!(!json_string.contains("html_body"));
    assert!(!json_string.contains("headers"));
    assert!(json_string.contains("sender"));
    assert!(json_string.contains("recipient"));
    assert!(json_string.contains("subject"));
    assert!(json_string.contains("body"));
}

#[test]
fn test_email_payload_with_inline_attachment() {
    use crate::attachment::{AttachmentPayload, SerializedAttachment};

    let payload = EmailPayload {
        sender: "a@x.com".to_string(),
        sender_name: None,
        recipient: "b@x.com".to_string(),
        subject: "with attachment".to_string(),
        body: "see attached".to_string(),
        html_body: None,
        headers: None,
        attachments: Some(vec![SerializedAttachment {
            filename: Some("x.pdf".to_string()),
            content_type: "application/pdf".to_string(),
            size_bytes: 3,
            content_id: None,
            payload: AttachmentPayload::Inline {
                data_base64: "YWJj".to_string(),
            },
        }]),
        dmarc_result: None,
        authenticated_from: None,
    };

    let json = serde_json::to_value(&payload).expect("serialize");
    let atts = json["attachments"].as_array().expect("attachments array");
    assert_eq!(atts.len(), 1);
    assert_eq!(atts[0]["delivery"], "inline");
    assert_eq!(atts[0]["data_base64"], "YWJj");
    assert_eq!(atts[0]["content_type"], "application/pdf");
}

#[test]
fn test_email_payload_with_s3_attachment_with_presigned() {
    use crate::attachment::{AttachmentPayload, SerializedAttachment};

    let payload = EmailPayload {
        sender: "a@x.com".to_string(),
        sender_name: None,
        recipient: "b@x.com".to_string(),
        subject: "s3".to_string(),
        body: "b".to_string(),
        html_body: None,
        headers: None,
        attachments: Some(vec![SerializedAttachment {
            filename: Some("r.pdf".to_string()),
            content_type: "application/pdf".to_string(),
            size_bytes: 7,
            content_id: None,
            payload: AttachmentPayload::S3 {
                url: "s3://bucket/key".to_string(),
                presigned_url: Some("https://presigned".to_string()),
            },
        }]),
        dmarc_result: None,
        authenticated_from: None,
    };

    let json = serde_json::to_value(&payload).expect("serialize");
    let att = &json["attachments"][0];
    assert_eq!(att["delivery"], "s3");
    assert_eq!(att["url"], "s3://bucket/key");
    assert_eq!(att["presigned_url"], "https://presigned");
}

#[test]
fn test_email_payload_attachments_omitted_when_none() {
    let payload = EmailPayload {
        sender: "a@x.com".to_string(),
        sender_name: None,
        recipient: "b@x.com".to_string(),
        subject: "s".to_string(),
        body: "b".to_string(),
        html_body: None,
        headers: None,
        attachments: None,
        dmarc_result: None,
        authenticated_from: None,
    };
    let s = serde_json::to_string(&payload).expect("serialize");
    assert!(!s.contains("attachments"));
}

#[test]
fn test_email_payload_json_structure_matches_expected() {
    let payload = EmailPayload {
        sender: "s@x.com".to_string(),
        sender_name: Some("S".to_string()),
        recipient: "r@x.com".to_string(),
        subject: "Sub".to_string(),
        body: "B".to_string(),
        html_body: Some("<p>H</p>".to_string()),
        headers: None,
        attachments: None,
        dmarc_result: None,
        authenticated_from: None,
    };

    let json: serde_json::Value = serde_json::to_value(&payload).expect("Serialization failed");
    let obj = json.as_object().expect("Expected JSON object");
    // When headers is None, it should be omitted, so we expect 6 keys
    assert_eq!(obj.len(), 6);
    assert!(obj.contains_key("sender"));
    assert!(obj.contains_key("sender_name"));
    assert!(obj.contains_key("recipient"));
    assert!(obj.contains_key("subject"));
    assert!(obj.contains_key("body"));
    assert!(obj.contains_key("html_body"));
}
