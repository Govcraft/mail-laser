use crate::config::Config;
use acton_reactive::prelude::*;
use anyhow::Result;
use bytes::Bytes;
use http_body_util::Full;
use hyper::Request;
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::{
    client::legacy::{connect::HttpConnector, Client},
    rt::TokioExecutor,
};
use log::{error, info};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

type HttpsConn = hyper_rustls::HttpsConnector<HttpConnector>;
type WebhookHttpClient = Client<HttpsConn, Full<Bytes>>;

// --- Message types ---

#[acton_message]
pub struct ForwardEmail {
    pub payload: EmailPayload,
}

#[acton_message]
struct WebhookResult {
    success: bool,
    #[allow(dead_code)] // read via ctx.message() in actor handler
    sender_info: String,
}

// --- Public data structures ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailPayload {
    pub sender: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_name: Option<String>,
    pub recipient: String,
    pub subject: String,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html_body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
}

// --- WebhookClient (unchanged transport layer) ---

pub struct WebhookClient {
    config: Config,
    client: WebhookHttpClient,
    user_agent: String,
}

impl WebhookClient {
    pub fn new(config: Config) -> Self {
        let https = {
            let connector = HttpsConnectorBuilder::new()
                .with_native_roots()
                .expect("Failed to load native root certificates for hyper-rustls");
            #[cfg(debug_assertions)]
            let connector = connector.https_or_http();
            #[cfg(not(debug_assertions))]
            let connector = connector.https_only();
            connector.enable_http1().build()
        };

        let client: WebhookHttpClient = Client::builder(TokioExecutor::new()).build(https);

        let user_agent = format!("{}/{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));

        Self {
            config,
            client,
            user_agent,
        }
    }

    pub async fn forward_email(&self, email: EmailPayload) -> Result<()> {
        info!(
            "Forwarding email from sender '{}' (Name: {}) with subject: '{}'",
            email.sender,
            email.sender_name.as_deref().unwrap_or("N/A"),
            email.subject
        );

        let json_body = serde_json::to_string(&email)?;

        let request = Request::builder()
            .method(hyper::Method::POST)
            .uri(&self.config.webhook_url)
            .header("content-type", "application/json")
            .header("user-agent", &self.user_agent)
            .body(Full::new(Bytes::from(json_body)))?;

        let response = self.client.request(request).await?;

        let status = response.status();
        if !status.is_success() {
            let msg = format!(
                "Webhook request to {} failed with status: {}",
                self.config.webhook_url, status
            );
            error!("{}", msg);
            return Err(anyhow::anyhow!(msg));
        }

        info!(
            "Email successfully forwarded to webhook {}, status: {}",
            self.config.webhook_url, status
        );

        Ok(())
    }
}

// --- WebhookActor ---

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[acton_actor]
pub struct WebhookState {
    consecutive_failures: u32,
    circuit_open: bool,
    circuit_opened_at_ms: u64,
    total_forwarded: u64,
    total_failed: u64,
    webhook_timeout_secs: u64,
    max_retries: u32,
    circuit_threshold: u32,
    circuit_reset_secs: u64,
}

impl WebhookState {
    pub async fn create(
        runtime: &mut ActorRuntime,
        config: &Config,
    ) -> anyhow::Result<ActorHandle> {
        let actor_config = ActorConfig::new(Ern::with_root("webhook-dispatcher")?, None, None)?
            .with_restart_policy(RestartPolicy::Permanent);

        let mut builder = runtime.new_actor_with_config::<Self>(actor_config);

        builder.model.webhook_timeout_secs = config.webhook_timeout_secs;
        builder.model.max_retries = config.webhook_max_retries;
        builder.model.circuit_threshold = config.circuit_breaker_threshold;
        builder.model.circuit_reset_secs = config.circuit_breaker_reset_secs;

        let client = Arc::new(WebhookClient::new(config.clone()));

        // ForwardEmail handler: circuit breaker check + async delivery with timeout + retry
        builder.mutate_on::<ForwardEmail>(move |actor, ctx| {
            let client = client.clone();
            let payload = ctx.message().payload.clone();
            let timeout_secs = actor.model.webhook_timeout_secs;
            let max_retries = actor.model.max_retries;
            let sender_info = payload.sender.clone();

            // Circuit breaker check (synchronous — can mutate state)
            if actor.model.circuit_open {
                let elapsed = current_time_ms() - actor.model.circuit_opened_at_ms;
                if elapsed > actor.model.circuit_reset_secs * 1000 {
                    actor.model.circuit_open = false;
                    actor.model.consecutive_failures = 0;
                    tracing::info!("Circuit breaker half-open, allowing request");
                } else {
                    tracing::warn!("Circuit breaker OPEN, dropping email from {}", sender_info);
                    actor.model.total_failed += 1;
                    return Reply::ready();
                }
            }

            let self_handle = actor.handle().clone();

            Reply::pending(async move {
                let mut success = false;
                for attempt in 0..=max_retries {
                    if attempt > 0 {
                        let backoff_ms = 100 * 2u64.pow(attempt - 1);
                        tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                        tracing::info!("Retry attempt {} for email from {}", attempt, sender_info);
                    }

                    let result = tokio::time::timeout(
                        Duration::from_secs(timeout_secs),
                        client.forward_email(payload.clone()),
                    )
                    .await;

                    match result {
                        Ok(Ok(())) => {
                            success = true;
                            break;
                        }
                        Ok(Err(e)) => {
                            tracing::warn!("Webhook attempt {} failed: {:#}", attempt + 1, e);
                        }
                        Err(_) => {
                            tracing::warn!(
                                "Webhook attempt {} timed out ({}s)",
                                attempt + 1,
                                timeout_secs
                            );
                        }
                    }
                }

                if !success {
                    tracing::error!(
                        "Webhook delivery failed after {} retries for {}",
                        max_retries,
                        sender_info
                    );
                }

                self_handle
                    .send(WebhookResult {
                        success,
                        sender_info,
                    })
                    .await;
            })
        });

        // WebhookResult handler: update circuit breaker state
        builder.mutate_on::<WebhookResult>(|actor, ctx| {
            let result = ctx.message();
            if result.success {
                actor.model.consecutive_failures = 0;
                actor.model.total_forwarded += 1;
            } else {
                actor.model.consecutive_failures += 1;
                actor.model.total_failed += 1;
                if actor.model.consecutive_failures >= actor.model.circuit_threshold {
                    actor.model.circuit_open = true;
                    actor.model.circuit_opened_at_ms = current_time_ms();
                    tracing::error!(
                        "Circuit breaker OPENED after {} consecutive failures",
                        actor.model.consecutive_failures
                    );
                }
            }
            Reply::ready()
        });

        builder.after_stop(|actor| {
            tracing::info!(
                "WebhookActor stopped. Forwarded: {}, Failed: {}",
                actor.model.total_forwarded,
                actor.model.total_failed
            );
            Reply::ready()
        });

        Ok(builder.start().await)
    }
}

#[cfg(test)]
mod tests;
