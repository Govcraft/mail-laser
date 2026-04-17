//! Cedar-based authorization for mail-laser.
//!
//! Two decisions are expressed as Cedar authorization requests:
//!
//! * [`PolicyEngine::can_send`] — may the MAIL FROM principal send any mail at all?
//!   Invoked at the SMTP `MAIL FROM` command.
//! * [`PolicyEngine::can_attach`] — may the principal attach *this* file (by MIME
//!   type, size, filename)? Invoked once per attachment after parsing.
//!
//! Policies and optional entities are loaded once at startup from paths supplied
//! in [`crate::config::Config`]. The engine is cheap to clone via `Arc` and safe
//! to share across tasks.

use anyhow::{anyhow, Context as _, Result};
use cedar_policy::{
    Authorizer, Context, Decision, Entities, EntityUid, PolicySet, Request, RestrictedExpression,
};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::str::FromStr;

/// Static view of an attachment used for policy evaluation. Data is not included.
#[derive(Debug, Clone)]
pub struct AttachmentCheck<'a> {
    pub filename: Option<&'a str>,
    pub content_type: &'a str,
    pub size_bytes: u64,
}

/// Cedar authorization engine.
pub struct PolicyEngine {
    policies: PolicySet,
    entities: Entities,
    authorizer: Authorizer,
}

impl PolicyEngine {
    /// Loads a Cedar policy file (text format) and an optional entities JSON file.
    ///
    /// When `entities_path` is `None`, the engine evaluates with an empty entity
    /// store — principals still need to be referenced directly in `permit` rules
    /// (no group membership resolution is possible).
    pub fn load(policies_path: &Path, entities_path: Option<&Path>) -> Result<Self> {
        let policy_text = fs::read_to_string(policies_path).with_context(|| {
            format!(
                "failed to read Cedar policy file at {}",
                policies_path.display()
            )
        })?;
        let policies = PolicySet::from_str(&policy_text).map_err(|e| {
            anyhow!(
                "failed to parse Cedar policies from {}: {}",
                policies_path.display(),
                e
            )
        })?;

        let entities = match entities_path {
            Some(path) => {
                let json = fs::read_to_string(path).with_context(|| {
                    format!("failed to read Cedar entities file at {}", path.display())
                })?;
                Entities::from_json_str(&json, None).map_err(|e| {
                    anyhow!(
                        "failed to parse Cedar entities from {}: {}",
                        path.display(),
                        e
                    )
                })?
            }
            None => Entities::empty(),
        };

        Ok(Self {
            policies,
            entities,
            authorizer: Authorizer::new(),
        })
    }

    /// Builds an engine directly from in-memory strings. Useful for tests.
    pub fn from_strings(policies: &str, entities_json: Option<&str>) -> Result<Self> {
        let policies = PolicySet::from_str(policies)
            .map_err(|e| anyhow!("failed to parse Cedar policies: {}", e))?;
        let entities = match entities_json {
            Some(json) => Entities::from_json_str(json, None)
                .map_err(|e| anyhow!("failed to parse Cedar entities: {}", e))?,
            None => Entities::empty(),
        };
        Ok(Self {
            policies,
            entities,
            authorizer: Authorizer::new(),
        })
    }

    /// Returns `true` when the `SendMail` action is permitted for `sender`.
    pub fn can_send(&self, sender: &str) -> bool {
        let principal = match user_uid(sender) {
            Ok(uid) => uid,
            Err(e) => {
                tracing::warn!(sender = sender, error = %e, "rejecting sender — failed to build principal UID");
                return false;
            }
        };
        let action = match action_uid("SendMail") {
            Ok(uid) => uid,
            Err(e) => {
                tracing::error!(error = %e, "failed to build SendMail action UID — denying");
                return false;
            }
        };
        let resource = match mail_resource_uid() {
            Ok(uid) => uid,
            Err(e) => {
                tracing::error!(error = %e, "failed to build Mail resource UID — denying");
                return false;
            }
        };

        self.decide(principal, action, resource, Context::empty())
    }

    /// Returns `true` when the `Attach` action is permitted for `sender` with the
    /// given attachment characteristics in policy context.
    pub fn can_attach(&self, sender: &str, att: &AttachmentCheck<'_>) -> bool {
        let principal = match user_uid(sender) {
            Ok(uid) => uid,
            Err(e) => {
                tracing::warn!(sender = sender, error = %e, "rejecting attachment — failed to build principal UID");
                return false;
            }
        };
        let action = match action_uid("Attach") {
            Ok(uid) => uid,
            Err(e) => {
                tracing::error!(error = %e, "failed to build Attach action UID — denying");
                return false;
            }
        };
        let resource = match attachment_resource_uid() {
            Ok(uid) => uid,
            Err(e) => {
                tracing::error!(error = %e, "failed to build Attachment resource UID — denying");
                return false;
            }
        };

        let mut pairs: HashMap<String, RestrictedExpression> = HashMap::new();
        pairs.insert(
            "content_type".to_string(),
            RestrictedExpression::new_string(att.content_type.to_string()),
        );
        // Cedar longs are i64; guard against silent truncation.
        let size_expr = if att.size_bytes > i64::MAX as u64 {
            tracing::warn!(
                size = att.size_bytes,
                "attachment size exceeds i64::MAX — treating as max value"
            );
            RestrictedExpression::new_long(i64::MAX)
        } else {
            RestrictedExpression::new_long(att.size_bytes as i64)
        };
        pairs.insert("size_bytes".to_string(), size_expr);
        pairs.insert(
            "filename".to_string(),
            RestrictedExpression::new_string(att.filename.unwrap_or("").to_string()),
        );

        let context = match Context::from_pairs(pairs) {
            Ok(ctx) => ctx,
            Err(e) => {
                tracing::error!(error = %e, "failed to build Cedar context — denying attachment");
                return false;
            }
        };

        self.decide(principal, action, resource, context)
    }

    fn decide(
        &self,
        principal: EntityUid,
        action: EntityUid,
        resource: EntityUid,
        context: Context,
    ) -> bool {
        let request = match Request::new(principal, action, resource, context, None) {
            Ok(req) => req,
            Err(e) => {
                tracing::error!(error = %e, "failed to build Cedar request — denying");
                return false;
            }
        };
        let response = self
            .authorizer
            .is_authorized(&request, &self.policies, &self.entities);
        match response.decision() {
            Decision::Allow => true,
            Decision::Deny => false,
        }
    }
}

fn user_uid(email: &str) -> Result<EntityUid> {
    let lit = format!(r#"User::"{}""#, escape_entity_id(&email.to_lowercase()));
    EntityUid::from_str(&lit).map_err(|e| anyhow!("invalid User UID from '{}': {}", email, e))
}

fn action_uid(name: &str) -> Result<EntityUid> {
    let lit = format!(r#"Action::"{}""#, name);
    EntityUid::from_str(&lit).map_err(|e| anyhow!("invalid Action UID for '{}': {}", name, e))
}

fn mail_resource_uid() -> Result<EntityUid> {
    EntityUid::from_str(r#"Mail::"inbound""#).map_err(|e| anyhow!("invalid Mail UID: {}", e))
}

fn attachment_resource_uid() -> Result<EntityUid> {
    EntityUid::from_str(r#"Attachment::"inbound""#)
        .map_err(|e| anyhow!("invalid Attachment UID: {}", e))
}

/// Cedar entity IDs allow arbitrary strings when quoted, but backslashes and
/// double quotes must be escaped before splicing into the `Type::"id"` form.
fn escape_entity_id(raw: &str) -> String {
    raw.replace('\\', r"\\").replace('"', r#"\""#)
}

#[cfg(test)]
mod tests;
