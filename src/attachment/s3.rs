//! S3-compatible attachment backend.
//!
//! Uploads each attachment to a configured bucket using `aws-sdk-s3`. The SDK's
//! optional endpoint override makes this backend usable against any
//! S3-compatible store (MinIO, R2, Wasabi) when `endpoint` is set in the
//! [`S3Settings`][crate::config::S3Settings].

use super::{AttachmentBackend, AttachmentPayload, SerializedAttachment};
use crate::config::S3Settings;
use crate::smtp::email_parser::Attachment;
use anyhow::{anyhow, Context as _, Result};
use async_trait::async_trait;
use aws_config::BehaviorVersion;
use aws_sdk_s3::config::Region;
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;
use std::time::Duration;
use uuid::Uuid;

/// Uploads attachments to an S3-compatible bucket and produces an `s3://` URL
/// (plus an optional presigned GET URL) for the webhook payload.
pub struct S3Backend {
    client: Client,
    settings: S3Settings,
}

impl S3Backend {
    pub async fn new(settings: S3Settings) -> Result<Self> {
        let mut loader = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(settings.region.clone()));

        if let Some(endpoint) = &settings.endpoint {
            loader = loader.endpoint_url(endpoint);
        }

        let sdk_config = loader.load().await;
        let mut builder = aws_sdk_s3::config::Builder::from(&sdk_config);
        // For non-AWS S3-compatible stores, path-style addressing is the safe default.
        if settings.endpoint.is_some() {
            builder = builder.force_path_style(true);
        }

        Ok(Self {
            client: Client::from_conf(builder.build()),
            settings,
        })
    }

    fn make_key(&self, filename: Option<&str>) -> String {
        let object_id = Uuid::new_v4();
        let safe_name = filename
            .map(sanitize_filename)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("attachment-{}", object_id));
        format!("{}{}-{}", self.settings.key_prefix, object_id, safe_name)
    }
}

#[async_trait]
impl AttachmentBackend for S3Backend {
    async fn prepare(&self, att: Attachment) -> Result<SerializedAttachment> {
        let key = self.make_key(att.filename.as_deref());
        let body = ByteStream::from(att.data);

        let mut put = self
            .client
            .put_object()
            .bucket(&self.settings.bucket)
            .key(&key)
            .body(body)
            .content_type(att.content_type.clone());
        if let Some(name) = att.filename.as_deref() {
            put = put.content_disposition(format!("attachment; filename=\"{}\"", name));
        }
        put.send()
            .await
            .with_context(|| format!("S3 put_object failed for key {}", key))?;

        let url = format!("s3://{}/{}", self.settings.bucket, key);

        let presigned_url = match self.settings.presign_ttl_secs {
            Some(ttl) => {
                let cfg = PresigningConfig::expires_in(Duration::from_secs(ttl))
                    .map_err(|e| anyhow!("invalid presign TTL: {}", e))?;
                let req = self
                    .client
                    .get_object()
                    .bucket(&self.settings.bucket)
                    .key(&key)
                    .presigned(cfg)
                    .await
                    .with_context(|| format!("S3 presign failed for key {}", key))?;
                Some(req.uri().to_string())
            }
            None => None,
        };

        Ok(SerializedAttachment {
            filename: att.filename,
            content_type: att.content_type,
            size_bytes: att.size_bytes,
            content_id: att.content_id,
            payload: AttachmentPayload::S3 { url, presigned_url },
        })
    }
}

/// Replaces characters that are problematic in S3 keys with `_`.
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            // Keep: alnum, `-`, `_`, `.`
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c,
            _ => '_',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_unsafe_chars() {
        assert_eq!(sanitize_filename("brief.pdf"), "brief.pdf");
        assert_eq!(
            sanitize_filename("weird name (1).pdf"),
            "weird_name__1_.pdf"
        );
        assert_eq!(sanitize_filename("/etc/passwd"), "_etc_passwd");
    }

    #[test]
    fn make_key_embeds_prefix_uuid_and_sanitized_name() {
        let settings = S3Settings {
            bucket: "b".to_string(),
            region: "us-east-1".to_string(),
            endpoint: None,
            key_prefix: "inbound/".to_string(),
            presign_ttl_secs: None,
        };
        let backend = S3Backend {
            // Using any valid Client won't actually be called here.
            // We construct minimally via aws_sdk_s3::Config directly.
            client: Client::from_conf(
                aws_sdk_s3::config::Builder::new()
                    .behavior_version(BehaviorVersion::latest())
                    .region(Region::new("us-east-1"))
                    .build(),
            ),
            settings,
        };
        let key = backend.make_key(Some("weird name (1).pdf"));
        assert!(key.starts_with("inbound/"));
        assert!(key.ends_with("-weird_name__1_.pdf"));
    }

    #[test]
    fn make_key_handles_missing_filename() {
        let settings = S3Settings {
            bucket: "b".to_string(),
            region: "us-east-1".to_string(),
            endpoint: None,
            key_prefix: String::new(),
            presign_ttl_secs: None,
        };
        let backend = S3Backend {
            client: Client::from_conf(
                aws_sdk_s3::config::Builder::new()
                    .behavior_version(BehaviorVersion::latest())
                    .region(Region::new("us-east-1"))
                    .build(),
            ),
            settings,
        };
        let key = backend.make_key(None);
        assert!(key.contains("attachment-"));
    }
}
