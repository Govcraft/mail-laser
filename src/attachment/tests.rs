use crate::attachment::{
    inline::InlineBackend, AttachmentBackend, AttachmentPayload, SerializedAttachment,
};
use crate::smtp::email_parser::Attachment;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;

fn sample_attachment() -> Attachment {
    Attachment {
        filename: Some("brief.pdf".to_string()),
        content_type: "application/pdf".to_string(),
        size_bytes: 16,
        content_id: None,
        data: b"%PDF-1.4\n%abc\n%%".to_vec(),
    }
}

#[tokio::test]
async fn inline_backend_round_trips_bytes() {
    let backend = InlineBackend::new();
    let att = sample_attachment();
    let expected_bytes = att.data.clone();

    let serialized = backend.prepare(att).await.expect("inline prepare");
    assert_eq!(serialized.filename.as_deref(), Some("brief.pdf"));
    assert_eq!(serialized.content_type, "application/pdf");
    assert_eq!(serialized.size_bytes, 16);

    match serialized.payload {
        AttachmentPayload::Inline { data_base64 } => {
            let decoded = STANDARD.decode(&data_base64).expect("valid base64");
            assert_eq!(decoded, expected_bytes);
        }
        other => panic!("expected Inline payload, got {:?}", other),
    }
}

#[test]
fn serialized_attachment_json_carries_delivery_tag_inline() {
    let sa = SerializedAttachment {
        filename: Some("x.pdf".to_string()),
        content_type: "application/pdf".to_string(),
        size_bytes: 3,
        content_id: None,
        payload: AttachmentPayload::Inline {
            data_base64: "YWJj".to_string(),
        },
    };
    let json = serde_json::to_value(&sa).expect("serialize");
    assert_eq!(json["delivery"], "inline");
    assert_eq!(json["data_base64"], "YWJj");
    assert_eq!(json["content_type"], "application/pdf");
    assert!(json.get("content_id").is_none());
}

#[test]
fn serialized_attachment_json_carries_delivery_tag_s3_without_presigned() {
    let sa = SerializedAttachment {
        filename: None,
        content_type: "image/png".to_string(),
        size_bytes: 10,
        content_id: Some("logo".to_string()),
        payload: AttachmentPayload::S3 {
            url: "s3://bucket/key".to_string(),
            presigned_url: None,
        },
    };
    let json = serde_json::to_value(&sa).expect("serialize");
    assert_eq!(json["delivery"], "s3");
    assert_eq!(json["url"], "s3://bucket/key");
    assert!(json.get("presigned_url").is_none());
    assert!(json.get("filename").is_none());
    assert_eq!(json["content_id"], "logo");
}

#[test]
fn serialized_attachment_json_carries_delivery_tag_s3_with_presigned() {
    let sa = SerializedAttachment {
        filename: Some("r.pdf".to_string()),
        content_type: "application/pdf".to_string(),
        size_bytes: 42,
        content_id: None,
        payload: AttachmentPayload::S3 {
            url: "s3://bucket/key".to_string(),
            presigned_url: Some("https://bucket.s3.amazonaws.com/key?sig=..".to_string()),
        },
    };
    let json = serde_json::to_value(&sa).expect("serialize");
    assert_eq!(json["delivery"], "s3");
    assert_eq!(json["url"], "s3://bucket/key");
    assert!(json["presigned_url"].is_string());
}
