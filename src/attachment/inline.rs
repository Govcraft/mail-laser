//! Inline (base64) attachment backend.

use super::{AttachmentBackend, AttachmentPayload, SerializedAttachment};
use crate::smtp::email_parser::Attachment;
use anyhow::Result;
use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;

/// Encodes attachment bytes as standard base64 and embeds them directly in the
/// JSON webhook payload.
#[derive(Debug, Default)]
pub struct InlineBackend;

impl InlineBackend {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl AttachmentBackend for InlineBackend {
    async fn prepare(&self, att: Attachment) -> Result<SerializedAttachment> {
        let data_base64 = STANDARD.encode(&att.data);
        Ok(SerializedAttachment {
            filename: att.filename,
            content_type: att.content_type,
            size_bytes: att.size_bytes,
            content_id: att.content_id,
            payload: AttachmentPayload::Inline { data_base64 },
        })
    }
}
