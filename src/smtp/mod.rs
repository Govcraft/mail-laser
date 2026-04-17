pub mod email_parser;
mod smtp_protocol;

use crate::attachment::AttachmentBackend;
use crate::config::{Config, DmarcMode, DmarcTempErrorAction};
use crate::dmarc::{decide, DmarcDecision, DmarcValidator};
use crate::policy::{AttachmentCheck, PolicyEngine};
use crate::webhook::{EmailPayload, ForwardEmail};
use acton_reactive::prelude::*;
use anyhow::{Context, Result};
use email_parser::EmailParser;
use log::{error, info, trace, warn};
use smtp_protocol::{SmtpCommandResult, SmtpProtocol, SmtpState};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

use rcgen::generate_simple_self_signed;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::ServerConfig as RustlsServerConfig;
use std::net::IpAddr;
use std::sync::Arc;
use tokio_rustls::TlsAcceptor;

// --- SmtpListenerActor ---

#[acton_actor]
pub struct SmtpListenerState;

/// Bundle of per-connection dependencies shared between the plaintext and
/// TLS session loops.
#[derive(Clone)]
struct SessionContext {
    webhook_handle: ActorHandle,
    target_emails: Vec<String>,
    header_prefixes: Vec<String>,
    policy: Arc<PolicyEngine>,
    backend: Arc<dyn AttachmentBackend>,
    max_message_size_bytes: u64,
    max_attachment_size_bytes: u64,
    peer_addr: IpAddr,
    dmarc: Option<Arc<DmarcValidator>>,
    dmarc_mode: DmarcMode,
    dmarc_temperror_action: DmarcTempErrorAction,
}

impl SmtpListenerState {
    pub async fn create(
        runtime: &mut ActorRuntime,
        config: &Config,
        webhook_handle: ActorHandle,
        policy: Arc<PolicyEngine>,
        backend: Arc<dyn AttachmentBackend>,
        dmarc: Option<Arc<DmarcValidator>>,
    ) -> anyhow::Result<ActorHandle> {
        let actor_config = ActorConfig::new(Ern::with_root("smtp-listener")?, None, None)?
            .with_restart_policy(RestartPolicy::Permanent);

        let mut builder = runtime.new_actor_with_config::<Self>(actor_config);

        let cancel = CancellationToken::new();
        let cancel_for_loop = cancel.clone();
        let cancel_for_stop = cancel.clone();

        let smtp_config = config.clone();
        let wh = webhook_handle.clone();

        builder.after_start(move |_actor| {
            let config = smtp_config.clone();
            let webhook_handle = wh.clone();
            let cancel = cancel_for_loop.clone();
            let policy = policy.clone();
            let backend = backend.clone();
            let dmarc = dmarc.clone();

            tokio::spawn(async move {
                let addr = format!("{}:{}", config.smtp_bind_address, config.smtp_port);
                let listener = match TcpListener::bind(&addr).await {
                    Ok(l) => {
                        tracing::info!("SMTP server listening on {}", addr);
                        l
                    }
                    Err(e) => {
                        tracing::error!("Failed to bind SMTP: {}", e);
                        return;
                    }
                };

                loop {
                    tokio::select! {
                        result = listener.accept() => {
                            match result {
                                Ok((stream, remote_addr)) => {
                                    tracing::info!("New connection from: {}", remote_addr);
                                    let ctx = SessionContext {
                                        webhook_handle: webhook_handle.clone(),
                                        target_emails: config.target_emails.clone(),
                                        header_prefixes: config.header_prefixes.clone(),
                                        policy: policy.clone(),
                                        backend: backend.clone(),
                                        max_message_size_bytes: config.max_message_size_bytes,
                                        max_attachment_size_bytes: config.max_attachment_size_bytes,
                                        peer_addr: remote_addr.ip(),
                                        dmarc: dmarc.clone(),
                                        dmarc_mode: config.dmarc_mode,
                                        dmarc_temperror_action: config.dmarc_temperror_action,
                                    };
                                    tokio::spawn(async move {
                                        if let Err(e) = handle_connection(stream, ctx).await {
                                            tracing::error!("Error handling SMTP connection from {}: {:#?}", remote_addr, e);
                                        }
                                    });
                                }
                                Err(e) => tracing::error!("Error accepting connection: {:?}", e),
                            }
                        }
                        _ = cancel.cancelled() => {
                            tracing::info!("SMTP listener shutting down gracefully");
                            break;
                        }
                    }
                }
            });

            Reply::ready()
        });

        builder.before_stop(move |_| {
            cancel_for_stop.cancel();
            Reply::ready()
        });

        Ok(builder.start().await)
    }
}

// --- Certificate generation (unchanged) ---

fn generate_self_signed_cert() -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>)> {
    let subject_alt_names = vec!["localhost".to_string()];

    let certified_key = generate_simple_self_signed(subject_alt_names)
        .context("Failed to generate self-signed certificate using rcgen")?;

    let cert_der = certified_key.cert.der().to_vec();
    let key_der = certified_key.signing_key.serialize_der();

    Ok((
        CertificateDer::from(cert_der),
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der)),
    ))
}

// --- Connection handlers ---

async fn handle_connection(mut stream: TcpStream, ctx: SessionContext) -> Result<()> {
    let mut session = MessageSession::default();

    let protocol_result = async {
        let (read_half, write_half) = tokio::io::split(&mut stream);
        let reader = tokio::io::BufReader::new(read_half);
        let writer = tokio::io::BufWriter::new(write_half);
        let mut protocol = SmtpProtocol::new(reader, writer, ctx.max_message_size_bytes);

        protocol.send_greeting().await?;

        loop {
            trace!("SMTP({:?}): Waiting for command...", protocol.get_state());
            let line = protocol.read_line().await?;
            trace!(
                "SMTP({:?}): Received line (len {}): {:?}",
                protocol.get_state(),
                line.len(),
                line
            );

            if protocol.get_state() != SmtpState::Data && line.is_empty() {
                info!(
                    "Connection closed by client (EOF). State: {:?}",
                    protocol.get_state()
                );
                return Ok(());
            }

            let result = protocol.process_command(&line).await?;

            match step(&mut protocol, &ctx, &mut session, result).await? {
                StepOutcome::Continue => {}
                StepOutcome::Quit => return Ok(()),
                StepOutcome::StartTls => return Err(anyhow::anyhow!("STARTTLS")),
            }
        }
    }
    .await;

    match protocol_result {
        Ok(()) => Ok(()),
        Err(e) if e.to_string() == "STARTTLS" => handle_starttls(stream, ctx).await,
        Err(e) => Err(e),
    }
}

async fn handle_starttls(stream: TcpStream, ctx: SessionContext) -> Result<()> {
    let (cert, key) = generate_self_signed_cert()
        .context("Failed to generate self-signed certificate for STARTTLS")?;

    let tls_config = RustlsServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .map_err(|e| anyhow::anyhow!("Failed to create rustls config: {}", e))?;

    let acceptor = TlsAcceptor::from(Arc::new(tls_config));

    match acceptor.accept(stream).await {
        Ok(tls_stream) => {
            info!("STARTTLS handshake successful.");
            handle_secure_session(tls_stream, ctx).await
        }
        Err(e) => {
            error!("STARTTLS handshake failed: {:?}", e);
            Err(anyhow::Error::new(e).context("STARTTLS handshake failed"))
        }
    }
}

async fn handle_secure_session<T>(tls_stream: T, ctx: SessionContext) -> Result<()>
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (read_half, write_half) = tokio::io::split(tls_stream);
    let reader = tokio::io::BufReader::new(read_half);
    let writer = tokio::io::BufWriter::new(write_half);
    let mut protocol = SmtpProtocol::new(reader, writer, ctx.max_message_size_bytes);

    let mut session = MessageSession::default();

    loop {
        trace!(
            "SMTP(TLS/{:?}): Waiting for command...",
            protocol.get_state()
        );
        let line = protocol.read_line().await?;
        trace!(
            "SMTP(TLS/{:?}): Received line (len {}): {:?}",
            protocol.get_state(),
            line.len(),
            line
        );

        if protocol.get_state() != SmtpState::Data && line.is_empty() {
            info!("Connection closed by client (EOF) during secure session.");
            break;
        }

        let result = protocol.process_command(&line).await?;

        match step(&mut protocol, &ctx, &mut session, result).await? {
            StepOutcome::Continue => {}
            StepOutcome::Quit => break,
            StepOutcome::StartTls => {
                warn!("Received STARTTLS command within secure session. Sending error.");
                protocol.write_line("503 STARTTLS already active").await?;
            }
        }
    }
    Ok(())
}

/// Per-message mutable bookkeeping shared across DataLine/DataEnd ticks.
#[derive(Default)]
struct MessageSession {
    sender: String,
    accepted_recipient: String,
    email_data: String,
    collecting_data: bool,
    size_exceeded: bool,
    data_size_bytes: u64,
    /// Most recent HELO/EHLO domain advertised by the client. Persists across
    /// messages within the same connection (a client may send multiple
    /// transactions without resending HELO).
    helo: String,
}

impl MessageSession {
    fn reset_message(&mut self) {
        self.sender.clear();
        self.accepted_recipient.clear();
        self.email_data.clear();
        self.collecting_data = false;
        self.size_exceeded = false;
        self.data_size_bytes = 0;
        // Deliberately do not clear `helo` — it's a session-wide fact.
    }
}

enum StepOutcome {
    Continue,
    Quit,
    StartTls,
}

/// Advances the SMTP session by one command tick, writing whatever response
/// is needed. Centralizes the logic so the plaintext and TLS loops stay in
/// lockstep.
async fn step<R, W>(
    protocol: &mut SmtpProtocol<R, W>,
    ctx: &SessionContext,
    session: &mut MessageSession,
    result: SmtpCommandResult,
) -> Result<StepOutcome>
where
    R: tokio::io::AsyncBufReadExt + Unpin,
    W: tokio::io::AsyncWriteExt + Unpin,
{
    match result {
        SmtpCommandResult::Continue => Ok(StepOutcome::Continue),
        SmtpCommandResult::Quit => {
            info!("Client quit.");
            Ok(StepOutcome::Quit)
        }
        SmtpCommandResult::StartTls => {
            info!("Client initiated STARTTLS.");
            Ok(StepOutcome::StartTls)
        }
        SmtpCommandResult::Helo(domain) => {
            session.helo = domain;
            Ok(StepOutcome::Continue)
        }
        SmtpCommandResult::MailFrom(email) => {
            if ctx.policy.can_send(&email) {
                session.sender = email;
                protocol.write_line("250 OK").await?;
            } else {
                warn!("Cedar denied SendMail for sender '{}'", email);
                protocol
                    .write_line("550 5.7.1 Sender not authorized")
                    .await?;
                session.sender.clear();
                protocol.reset_state();
            }
            Ok(StepOutcome::Continue)
        }
        SmtpCommandResult::RcptTo(email) => {
            let received_email_lower = email.to_lowercase();
            if ctx
                .target_emails
                .iter()
                .any(|t| t.to_lowercase() == received_email_lower)
            {
                session.accepted_recipient = email;
                protocol.write_line("250 OK").await?;
            } else {
                protocol.write_line("550 No such user here").await?;
                session.accepted_recipient.clear();
            }
            Ok(StepOutcome::Continue)
        }
        SmtpCommandResult::DataStart => {
            // Protocol layer has already written "354 Start mail input..." when
            // transitioning to Data state; we just reset message bookkeeping.
            session.collecting_data = true;
            session.email_data.clear();
            session.size_exceeded = false;
            session.data_size_bytes = 0;
            Ok(StepOutcome::Continue)
        }
        SmtpCommandResult::DataLine(line_content) => {
            if !session.collecting_data {
                warn!("Received DataLine result when not in Data state.");
                return Ok(StepOutcome::Continue);
            }
            // RFC 5321 §4.5.2: a receiving MTA strips a single leading dot
            // from each DATA line. Not doing so breaks DKIM body-hash
            // verification for any body line that starts with `.`.
            let unstuffed: &str = if let Some(rest) = line_content.strip_prefix('.') {
                rest
            } else {
                line_content.as_str()
            };
            let added = (unstuffed.len() as u64).saturating_add(2); // include CRLF
            let next_total = session.data_size_bytes.saturating_add(added);
            if next_total > ctx.max_message_size_bytes {
                if !session.size_exceeded {
                    warn!(
                        "Message from '{}' exceeds max_message_size_bytes ({} > {}); continuing to drain until end-of-data.",
                        session.sender, next_total, ctx.max_message_size_bytes
                    );
                    session.size_exceeded = true;
                }
            } else {
                session.email_data.push_str(unstuffed);
                session.email_data.push_str("\r\n");
                session.data_size_bytes = next_total;
            }
            Ok(StepOutcome::Continue)
        }
        SmtpCommandResult::DataEnd => {
            session.collecting_data = false;
            let response = finalize_message(ctx, session).await;
            protocol.write_line(&response).await?;
            session.reset_message();
            Ok(StepOutcome::Continue)
        }
    }
}

/// Parses, authorizes, and forwards the collected message. Returns the SMTP
/// reply to write back to the client.
async fn finalize_message(ctx: &SessionContext, session: &MessageSession) -> String {
    if session.size_exceeded {
        return "552 5.3.4 Message size exceeds fixed limit".to_string();
    }
    if session.sender.is_empty() || session.accepted_recipient.is_empty() {
        return "503 5.5.1 Bad sequence: no MAIL FROM or RCPT TO".to_string();
    }

    // DMARC gate — runs before parse so we can 550/451 on fail without burning
    // the parse + policy budget. When ctx.dmarc is None (mode=off) this is a
    // no-op returning an "off" accept decision.
    let (dmarc_result, authenticated_from) = match run_dmarc(ctx, session).await {
        DmarcDecision::Reject { code, status } => {
            warn!(
                "DMARC {}: sender={} helo={} peer={}",
                status, session.sender, session.helo, ctx.peer_addr
            );
            return format!("{} {}", code, status);
        }
        DmarcDecision::Accept {
            dmarc_result,
            authenticated_from,
        } => {
            if dmarc_result == "off" {
                // DMARC disabled — omit the payload fields entirely.
                (None, None)
            } else {
                info!(
                    "DMARC result={} sender={} helo={} peer={} authed_from={:?}",
                    dmarc_result,
                    session.sender,
                    session.helo,
                    ctx.peer_addr,
                    authenticated_from
                );
                (Some(dmarc_result.to_string()), authenticated_from)
            }
        }
    };

    let parsed =
        match EmailParser::parse(session.email_data.as_bytes(), &ctx.header_prefixes) {
            Ok(p) => p,
            Err(e) => {
                error!(
                    "Failed to parse email data from {}: {:#}",
                    session.sender, e
                );
                return "451 4.3.0 Could not parse message".to_string();
            }
        };

    info!(
        "Received email from {} to {} (Subject: '{}') with {} attachment(s)",
        session.sender,
        session.accepted_recipient,
        parsed.subject,
        parsed.attachments.len()
    );

    // Per-attachment size and policy enforcement (defense-in-depth with Cedar).
    for att in &parsed.attachments {
        if att.size_bytes > ctx.max_attachment_size_bytes {
            warn!(
                "Attachment '{:?}' from {} exceeds max_attachment_size_bytes ({} > {})",
                att.filename, session.sender, att.size_bytes, ctx.max_attachment_size_bytes
            );
            return format!(
                "552 5.3.4 Attachment '{}' exceeds size limit",
                att.filename.as_deref().unwrap_or("(unnamed)")
            );
        }
        let check = AttachmentCheck {
            filename: att.filename.as_deref(),
            content_type: &att.content_type,
            size_bytes: att.size_bytes,
        };
        if !ctx.policy.can_attach(&session.sender, &check) {
            warn!(
                "Cedar denied Attach (type={}, name={:?}) for sender {}",
                att.content_type, att.filename, session.sender
            );
            return "550 5.7.1 Attachment not permitted by policy".to_string();
        }
    }

    // Run surviving attachments through the configured delivery backend.
    let mut serialized = Vec::with_capacity(parsed.attachments.len());
    for att in parsed.attachments {
        match ctx.backend.prepare(att).await {
            Ok(s) => serialized.push(s),
            Err(e) => {
                error!(
                    "Attachment backend prepare failed for {}: {:#}",
                    session.sender, e
                );
                return "451 4.7.0 Attachment delivery backend failed".to_string();
            }
        }
    }

    let headers = if parsed.matched_headers.is_empty() {
        None
    } else {
        Some(parsed.matched_headers)
    };
    let attachments = if serialized.is_empty() {
        None
    } else {
        Some(serialized)
    };
    let email_payload = EmailPayload {
        sender: session.sender.clone(),
        sender_name: parsed.from_name,
        recipient: session.accepted_recipient.clone(),
        subject: parsed.subject,
        body: parsed.text_body,
        html_body: parsed.html_body,
        headers,
        attachments,
        dmarc_result,
        authenticated_from,
    };
    ctx.webhook_handle
        .send(ForwardEmail {
            payload: email_payload,
        })
        .await;

    "250 OK: Message accepted for delivery".to_string()
}

/// Runs the DMARC check when the validator is configured, otherwise returns an
/// `Accept` decision carrying the sentinel `"off"` result that the caller
/// translates into "no payload annotation".
async fn run_dmarc(ctx: &SessionContext, session: &MessageSession) -> DmarcDecision {
    let Some(validator) = ctx.dmarc.as_ref() else {
        return DmarcDecision::Accept {
            dmarc_result: "off",
            authenticated_from: None,
        };
    };

    let helo = if session.helo.is_empty() {
        "unknown"
    } else {
        session.helo.as_str()
    };

    let outcome = validator
        .validate(
            session.email_data.as_bytes(),
            ctx.peer_addr,
            helo,
            &session.sender,
        )
        .await;

    decide(&outcome, ctx.dmarc_mode, ctx.dmarc_temperror_action)
}
