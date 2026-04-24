//! DMARC (plus the SPF + DKIM it needs) validation for inbound SMTP.
//!
//! The validator is built once at startup from [`crate::config::Config`] and
//! handed to the SMTP listener via [`crate::smtp::SmtpListenerState::create`].
//! Within a session, [`DmarcValidator::validate`] is called once per message at
//! the top of `finalize_message`, before the webhook payload is assembled.
//!
//! # Outcomes
//!
//! Validation produces a [`DmarcOutcome`] which the caller then feeds into the
//! pure decision helper [`decide`] along with the configured [`crate::config::DmarcMode`]
//! and [`crate::config::DmarcTempErrorAction`]. That helper decides whether the
//! SMTP transaction should `Accept` or `Reject`, and what to stamp on the
//! outbound webhook payload.
//!
//! This split keeps the I/O-bearing validator thin and the authorization logic
//! pure and easy to unit-test.

use crate::config::{Config, DmarcMode, DmarcTempErrorAction};
use anyhow::{anyhow, Context as _, Result};
use mail_auth::{
    dmarc::verify::DmarcParameters, spf::verify::SpfParameters, AuthenticatedMessage, DmarcResult,
    MessageAuthenticator, SpfResult,
};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

/// Result of a single DMARC evaluation.
///
/// This is the validator's output *before* any policy interpretation. Mapping
/// to a concrete SMTP action lives in [`decide`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DmarcOutcome {
    /// DMARC `pass` — SPF or DKIM passed AND alignment matched the From-header
    /// domain. `authenticated_from` is the full From-header address as it
    /// appeared on the wire.
    Pass { authenticated_from: String },
    /// DMARC `fail` — the From domain publishes a record, but neither SPF nor
    /// DKIM aligned.
    Fail,
    /// The DMARC evaluation itself could not complete (DNS SERVFAIL, timeout,
    /// resolver unreachable). RFC 7489 maps this to SMTP 451.
    TempError,
    /// The From-header domain publishes no DMARC record — treated as accept
    /// in all modes, per RFC 7489 §6.6.3.
    NoPolicy,
}

impl DmarcOutcome {
    /// The string value that lands in the webhook payload's `dmarc_result` field.
    pub fn as_payload_str(&self) -> &'static str {
        match self {
            DmarcOutcome::Pass { .. } => "pass",
            DmarcOutcome::Fail => "fail",
            DmarcOutcome::TempError => "temperror",
            DmarcOutcome::NoPolicy => "none",
        }
    }
}

/// The SMTP-level decision derived from an outcome + mode + temperror action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DmarcDecision {
    /// Forward the message. `dmarc_result` and `authenticated_from` are stamped
    /// on the webhook payload; `authenticated_from` is `Some` only when the
    /// outcome was `Pass`.
    Accept {
        dmarc_result: &'static str,
        authenticated_from: Option<String>,
    },
    /// Reject the SMTP transaction with the given reply code + enhanced status
    /// text. The caller writes `"{code} {status}"` back to the client.
    Reject { code: u16, status: &'static str },
}

/// Pure mapping from a [`DmarcOutcome`] to a [`DmarcDecision`] given the
/// configured mode and temperror action. No I/O; safe to unit-test.
pub fn decide(
    outcome: &DmarcOutcome,
    mode: DmarcMode,
    temperror_action: DmarcTempErrorAction,
) -> DmarcDecision {
    match (mode, outcome) {
        (DmarcMode::Off, _) => DmarcDecision::Accept {
            dmarc_result: "off",
            authenticated_from: None,
        },
        (DmarcMode::Monitor, DmarcOutcome::Pass { authenticated_from }) => DmarcDecision::Accept {
            dmarc_result: "pass",
            authenticated_from: Some(authenticated_from.clone()),
        },
        (DmarcMode::Monitor, other) => DmarcDecision::Accept {
            dmarc_result: other.as_payload_str(),
            authenticated_from: None,
        },
        (DmarcMode::Enforce, DmarcOutcome::Pass { authenticated_from }) => DmarcDecision::Accept {
            dmarc_result: "pass",
            authenticated_from: Some(authenticated_from.clone()),
        },
        (DmarcMode::Enforce, DmarcOutcome::NoPolicy) => DmarcDecision::Accept {
            dmarc_result: "none",
            authenticated_from: None,
        },
        (DmarcMode::Enforce, DmarcOutcome::Fail) => DmarcDecision::Reject {
            code: 550,
            status: "5.7.1 DMARC policy violation",
        },
        (DmarcMode::Enforce, DmarcOutcome::TempError) => match temperror_action {
            DmarcTempErrorAction::Reject => DmarcDecision::Reject {
                code: 451,
                status: "4.7.0 DMARC temporary error",
            },
            DmarcTempErrorAction::Accept => DmarcDecision::Accept {
                dmarc_result: "temperror",
                authenticated_from: None,
            },
        },
    }
}

/// Runtime DMARC validator. Holds the DNS-backed mail-auth authenticator and
/// the timeout budget.
pub struct DmarcValidator {
    authenticator: MessageAuthenticator,
    timeout: Duration,
}

impl DmarcValidator {
    /// Build a validator from config. Returns `Ok(None)` when
    /// [`DmarcMode::Off`] — the caller should skip validation entirely.
    pub fn load(config: &Config) -> Result<Option<Arc<Self>>> {
        if config.dmarc_mode == DmarcMode::Off {
            return Ok(None);
        }

        let authenticator = build_authenticator(&config.dmarc_dns_servers)
            .context("failed to build DMARC DNS resolver")?;

        Ok(Some(Arc::new(Self {
            authenticator,
            timeout: Duration::from_secs(config.dmarc_dns_timeout_secs),
        })))
    }

    /// Validate one message. Wraps the SPF + DKIM + DMARC dance in a single
    /// timeout; timeout → [`DmarcOutcome::TempError`].
    pub async fn validate(
        &self,
        raw_message: &[u8],
        peer_ip: IpAddr,
        helo_domain: &str,
        envelope_from: &str,
    ) -> DmarcOutcome {
        match tokio::time::timeout(
            self.timeout,
            self.verify_inner(raw_message, peer_ip, helo_domain, envelope_from),
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(_elapsed) => {
                tracing::warn!(
                    timeout_secs = self.timeout.as_secs(),
                    helo = helo_domain,
                    envelope_from = envelope_from,
                    "DMARC validation timed out"
                );
                DmarcOutcome::TempError
            }
        }
    }

    async fn verify_inner(
        &self,
        raw_message: &[u8],
        peer_ip: IpAddr,
        helo_domain: &str,
        envelope_from: &str,
    ) -> DmarcOutcome {
        let message = match AuthenticatedMessage::parse(raw_message) {
            Some(m) => m,
            None => {
                tracing::warn!("DMARC: unable to parse message headers; treating as TempError");
                return DmarcOutcome::TempError;
            }
        };

        let envelope_from_domain = envelope_from.rsplit_once('@').map(|(_, d)| d).unwrap_or("");

        let spf_output = self
            .authenticator
            .verify_spf(SpfParameters::verify_mail_from(
                peer_ip,
                helo_domain,
                "mail-laser",
                envelope_from,
            ))
            .await;

        let dkim_output = self.authenticator.verify_dkim(&message).await;

        let dmarc_output = self
            .authenticator
            .verify_dmarc(
                DmarcParameters::new(&message, &dkim_output, envelope_from_domain, &spf_output)
                    .with_domain_suffix_fn(organizational_domain),
            )
            .await;

        // Transient DNS errors anywhere in the chain bubble up as TempError.
        if matches!(spf_output.result(), SpfResult::TempError)
            || matches!(dmarc_output.spf_result(), DmarcResult::TempError(_))
            || matches!(dmarc_output.dkim_result(), DmarcResult::TempError(_))
        {
            return DmarcOutcome::TempError;
        }

        // When the From domain publishes no DMARC record, mail-auth leaves
        // `dmarc_record` = None.
        if dmarc_output.dmarc_record().is_none() {
            return DmarcOutcome::NoPolicy;
        }

        let aligned = matches!(dmarc_output.spf_result(), DmarcResult::Pass)
            || matches!(dmarc_output.dkim_result(), DmarcResult::Pass);

        if aligned {
            let authenticated_from = message
                .from
                .first()
                .cloned()
                .unwrap_or_else(|| envelope_from.to_string());
            DmarcOutcome::Pass { authenticated_from }
        } else {
            DmarcOutcome::Fail
        }
    }
}

/// Reduces a DNS name to its organizational (registrable) domain using the
/// public-suffix list. Falls back to the raw domain if `psl` returns `None`
/// (unknown TLD).
fn organizational_domain(domain: &str) -> &str {
    psl::domain_str(domain).unwrap_or(domain)
}

fn build_authenticator(servers: &[String]) -> Result<MessageAuthenticator> {
    if servers.is_empty() {
        return MessageAuthenticator::new_system_conf()
            .map_err(|e| anyhow!("failed to read system DNS config: {}", e));
    }

    use mail_auth::hickory_resolver::config::{
        NameServerConfig, NameServerConfigGroup, ProtocolConfig, ResolverConfig, ResolverOpts,
    };

    let mut group = NameServerConfigGroup::with_capacity(servers.len() * 2);
    for entry in servers {
        let sa: SocketAddr = entry
            .parse()
            .map_err(|e| anyhow!("invalid DMARC DNS server '{}': {}", entry, e))?;
        group.push(NameServerConfig::new(sa, ProtocolConfig::Udp));
        group.push(NameServerConfig::new(sa, ProtocolConfig::Tcp));
    }
    let cfg = ResolverConfig::from_parts(None, vec![], group);

    MessageAuthenticator::new(cfg, ResolverOpts::default())
        .map_err(|e| anyhow!("failed to build DNS resolver for DMARC: {}", e))
}

#[cfg(test)]
mod tests;
