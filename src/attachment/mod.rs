//! Attachment delivery backends for the webhook payload.
//!
//! The [`AttachmentBackend`] trait abstracts over two modes — chosen at
//! deploy time via [`crate::config::AttachmentDelivery`]:
//!
//! * [`InlineBackend`] — base64-encodes the attachment bytes and embeds them
//!   in the JSON payload.
//! * [`S3Backend`] — uploads the bytes to an S3-compatible bucket and puts a
//!   URL (optionally a presigned GET URL) into the payload.
//!
//! Both modes produce a [`SerializedAttachment`] with a `delivery` discriminant
//! so the webhook consumer can handle either shape.

use crate::config::{AttachmentDelivery, Config};
use crate::smtp::email_parser::Attachment;
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub mod inline;
pub mod s3;

#[cfg(test)]
mod tests;

/// Metadata and delivery payload for a single attachment as it appears in the
/// JSON webhook body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SerializedAttachment {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    pub content_type: String,
    pub size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_id: Option<String>,
    #[serde(flatten)]
    pub payload: AttachmentPayload,
}

/// How the attachment bytes are delivered to the webhook consumer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "delivery", rename_all = "snake_case")]
pub enum AttachmentPayload {
    /// Bytes are embedded in the JSON itself, standard base64 encoded.
    Inline { data_base64: String },
    /// Bytes were uploaded to an S3-compatible bucket.
    S3 {
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        presigned_url: Option<String>,
    },
}

/// Turns a parsed [`Attachment`] into a [`SerializedAttachment`] for transport.
#[async_trait]
pub trait AttachmentBackend: Send + Sync {
    async fn prepare(&self, att: Attachment) -> Result<SerializedAttachment>;
}

/// Selects and constructs the attachment backend based on configuration.
pub async fn build(config: &Config) -> Result<Arc<dyn AttachmentBackend>> {
    match &config.attachment_delivery {
        AttachmentDelivery::Inline => Ok(Arc::new(inline::InlineBackend::new())),
        AttachmentDelivery::S3(settings) => {
            let backend = s3::S3Backend::new(settings.clone()).await?;
            Ok(Arc::new(backend))
        }
    }
}
