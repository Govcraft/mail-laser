//! Cedar-based authorization for mail-laser.
//!
//! Two decisions are expressed as Cedar authorization requests:
//!
//! * [`PolicyEngine::can_send`] — may this principal deliver a message to this
//!   recipient? Invoked at end-of-DATA, *after* DMARC has run, so the DMARC
//!   outcome and the aligned From address are available as context.
//! * [`PolicyEngine::can_attach`] — may the principal attach *this* file (by MIME
//!   type, size, filename)? Invoked once per attachment after parsing, with the
//!   same DMARC context mirrored in.
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
use std::net::IpAddr;
use std::path::Path;
use std::str::FromStr;

/// Static view of an attachment used for policy evaluation. Data is not included.
#[derive(Debug, Clone)]
pub struct AttachmentCheck<'a> {
    pub filename: Option<&'a str>,
    pub content_type: &'a str,
    pub size_bytes: u64,
}

/// Per-request DMARC facts surfaced to Cedar as context attributes.
///
/// Constructed once in `finalize_message()` after DMARC runs and reused for
/// both the `SendMail` and `Attach` evaluations so policies see a consistent
/// view of the message's authentication state.
#[derive(Debug, Clone)]
pub struct DmarcContext {
    /// `"pass" | "fail" | "none" | "temperror" | "off"` — matches the
    /// webhook payload's `dmarc_result` wire format.
    pub result: &'static str,
    /// `true` iff DMARC produced a `Pass` outcome (SPF or DKIM aligned to
    /// the From domain).
    pub aligned: bool,
    /// The DMARC-aligned From address, present only when `aligned`. Cedar has
    /// no `Option`, so the context field is emitted as an empty string when
    /// absent — policies can guard with `context.authenticated_from != ""`.
    pub authenticated_from: Option<String>,
    /// Envelope MAIL FROM, regardless of which identity ended up as
    /// principal. Lets policies compare claimed vs. authenticated identity.
    pub envelope_from: String,
    /// HELO/EHLO domain the client announced.
    pub helo: String,
    /// Peer IP address of the sending MTA.
    pub peer_ip: IpAddr,
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

    /// Returns `true` when the `SendMail` action is permitted for `principal`
    /// delivering to `recipient`, with DMARC authentication facts in context.
    ///
    /// The principal is the identity selected by the caller: the DMARC-aligned
    /// From address when DMARC passed in `enforce` mode, otherwise the envelope
    /// `MAIL FROM` sender. The envelope sender is always available in context
    /// so policies can cross-check claimed vs. authenticated identity.
    pub fn can_send(&self, principal: &str, recipient: &str, dmarc: &DmarcContext) -> bool {
        let principal_uid = match user_uid(principal) {
            Ok(uid) => uid,
            Err(e) => {
                tracing::warn!(principal = principal, error = %e, "rejecting sender — failed to build principal UID");
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
        let resource = match recipient_uid(recipient) {
            Ok(uid) => uid,
            Err(e) => {
                tracing::error!(error = %e, recipient = recipient, "failed to build Recipient UID — denying");
                return false;
            }
        };

        let context = match build_dmarc_context(dmarc) {
            Ok(ctx) => ctx,
            Err(e) => {
                tracing::error!(error = %e, "failed to build Cedar context — denying SendMail");
                return false;
            }
        };

        self.decide(principal_uid, action, resource, context)
    }

    /// Returns `true` when the `Attach` action is permitted for `principal` with
    /// the given attachment characteristics plus DMARC authentication facts.
    pub fn can_attach(
        &self,
        principal: &str,
        att: &AttachmentCheck<'_>,
        dmarc: &DmarcContext,
    ) -> bool {
        let principal_uid = match user_uid(principal) {
            Ok(uid) => uid,
            Err(e) => {
                tracing::warn!(principal = principal, error = %e, "rejecting attachment — failed to build principal UID");
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

        let mut pairs = dmarc_context_pairs(dmarc);
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

        self.decide(principal_uid, action, resource, context)
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

fn recipient_uid(email: &str) -> Result<EntityUid> {
    let lit = format!(
        r#"Recipient::"{}""#,
        escape_entity_id(&email.to_lowercase())
    );
    EntityUid::from_str(&lit)
        .map_err(|e| anyhow!("invalid Recipient UID from '{}': {}", email, e))
}

fn attachment_resource_uid() -> Result<EntityUid> {
    EntityUid::from_str(r#"Attachment::"inbound""#)
        .map_err(|e| anyhow!("invalid Attachment UID: {}", e))
}

/// Builds the DMARC-only HashMap shared between `can_send` and `can_attach`.
/// Attachment-specific keys (`content_type`, `size_bytes`, `filename`) are
/// merged in on top at the `Attach` call site.
fn dmarc_context_pairs(dmarc: &DmarcContext) -> HashMap<String, RestrictedExpression> {
    let mut pairs: HashMap<String, RestrictedExpression> = HashMap::new();
    pairs.insert(
        "dmarc_result".to_string(),
        RestrictedExpression::new_string(dmarc.result.to_string()),
    );
    pairs.insert(
        "dmarc_aligned".to_string(),
        RestrictedExpression::new_bool(dmarc.aligned),
    );
    pairs.insert(
        "authenticated_from".to_string(),
        RestrictedExpression::new_string(
            dmarc.authenticated_from.clone().unwrap_or_default(),
        ),
    );
    pairs.insert(
        "envelope_from".to_string(),
        RestrictedExpression::new_string(dmarc.envelope_from.clone()),
    );
    pairs.insert(
        "helo".to_string(),
        RestrictedExpression::new_string(dmarc.helo.clone()),
    );
    pairs.insert(
        "peer_ip".to_string(),
        RestrictedExpression::new_string(dmarc.peer_ip.to_string()),
    );
    pairs
}

fn build_dmarc_context(dmarc: &DmarcContext) -> Result<Context> {
    Context::from_pairs(dmarc_context_pairs(dmarc))
        .map_err(|e| anyhow!("failed to build Cedar context: {}", e))
}

/// Cedar entity IDs allow arbitrary strings when quoted, but backslashes and
/// double quotes must be escaped before splicing into the `Type::"id"` form.
fn escape_entity_id(raw: &str) -> String {
    raw.replace('\\', r"\\").replace('"', r#"\""#)
}

#[cfg(test)]
mod tests;
