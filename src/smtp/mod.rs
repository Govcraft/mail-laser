mod email_parser;
mod smtp_protocol;

use crate::config::Config;
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
use std::sync::Arc;
use tokio_rustls::TlsAcceptor;

// --- SmtpListenerActor ---

#[acton_actor]
pub struct SmtpListenerState;

impl SmtpListenerState {
    pub async fn create(
        runtime: &mut ActorRuntime,
        config: &Config,
        webhook_handle: ActorHandle,
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
                                    let wh = webhook_handle.clone();
                                    let targets = config.target_emails.clone();
                                    let prefixes = config.header_prefixes.clone();
                                    tokio::spawn(async move {
                                        if let Err(e) = handle_connection(stream, wh, targets, prefixes).await {
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

async fn handle_connection(
    mut stream: TcpStream,
    webhook_handle: ActorHandle,
    target_emails: Vec<String>,
    header_prefixes: Vec<String>,
) -> Result<()> {
    let mut sender = String::new();
    let mut accepted_recipient = String::new();
    let mut email_data = String::new();
    let mut collecting_data = false;

    let protocol_result = async {
        let (read_half, write_half) = tokio::io::split(&mut stream);
        let reader = tokio::io::BufReader::new(read_half);
        let writer = tokio::io::BufWriter::new(write_half);
        let mut initial_protocol = SmtpProtocol::new(reader, writer);

        initial_protocol.send_greeting().await?;

        loop {
            trace!("SMTP({:?}): Waiting for command...", initial_protocol.get_state());
            let line = initial_protocol.read_line().await?;
            trace!("SMTP({:?}): Received line (len {}): {:?}", initial_protocol.get_state(), line.len(), line);

            if initial_protocol.get_state() != SmtpState::Data && line.is_empty() {
                info!("Connection closed by client (EOF). State: {:?}", initial_protocol.get_state());
                return Ok(());
            }

            let result = initial_protocol.process_command(&line).await?;

            match result {
                SmtpCommandResult::StartTls => {
                    info!("Client initiated STARTTLS. Proceeding with handshake.");
                    return Err(anyhow::anyhow!("STARTTLS"));
                }
                SmtpCommandResult::Quit => {
                    info!("Client quit.");
                    return Ok(());
                }
                SmtpCommandResult::MailFrom(email) => {
                    sender = email;
                }
                SmtpCommandResult::RcptTo(email) => {
                    let received_email = email;
                    let received_email_lower = received_email.to_lowercase();
                    if target_emails.iter().any(|target| target.to_lowercase() == received_email_lower) {
                        accepted_recipient = received_email;
                        initial_protocol.write_line("250 OK").await?;
                    } else {
                        initial_protocol.write_line("550 No such user here").await?;
                        accepted_recipient.clear();
                    }
                }
                SmtpCommandResult::DataStart => {
                    if sender.is_empty() || accepted_recipient.is_empty() {
                        warn!("DATA command received without valid MAIL FROM or RCPT TO. Rejecting.");
                        initial_protocol.write_line("503 Bad sequence of commands (MAIL FROM and RCPT TO required first)").await?;
                    } else {
                        collecting_data = true;
                        email_data.clear();
                    }
                }
                SmtpCommandResult::DataLine(line_content) => {
                    if collecting_data {
                        email_data.push_str(&line_content);
                        email_data.push_str("\r\n");
                    } else {
                        warn!("Received DataLine result when not in Data state.");
                    }
                }
                SmtpCommandResult::DataEnd => {
                    collecting_data = false;
                    if sender.is_empty() || accepted_recipient.is_empty() {
                        warn!("DataEnd received but sender or recipient was missing. Message likely not processed.");
                    } else {
                        match EmailParser::parse(email_data.as_bytes(), &header_prefixes) {
                            Ok((subject, from_name, text_body, html_body, matched_headers)) => {
                                info!("Received email from {} to {} (Subject: '{}')", sender, accepted_recipient, subject);
                                let headers = if matched_headers.is_empty() { None } else { Some(matched_headers) };
                                let email_payload = EmailPayload {
                                    sender: sender.clone(),
                                    sender_name: from_name,
                                    recipient: accepted_recipient.clone(),
                                    subject,
                                    body: text_body,
                                    html_body,
                                    headers,
                                };
                                webhook_handle.send(ForwardEmail { payload: email_payload }).await;
                            }
                            Err(e) => {
                                error!("Failed to parse email data from {}: {:#}", sender, e);
                            }
                        }
                    }
                    sender.clear();
                    accepted_recipient.clear();
                    email_data.clear();
                }
                SmtpCommandResult::Continue => {}
            }
        }
    }
    .await;

    match protocol_result {
        Ok(()) => Ok(()),
        Err(e) if e.to_string() == "STARTTLS" => {
            handle_starttls(stream, webhook_handle, target_emails, header_prefixes).await
        }
        Err(e) => Err(e),
    }
}

async fn handle_starttls(
    stream: TcpStream,
    webhook_handle: ActorHandle,
    target_emails: Vec<String>,
    header_prefixes: Vec<String>,
) -> Result<()> {
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
            handle_secure_session(tls_stream, webhook_handle, target_emails, header_prefixes).await
        }
        Err(e) => {
            error!("STARTTLS handshake failed: {:?}", e);
            Err(anyhow::Error::new(e).context("STARTTLS handshake failed"))
        }
    }
}

async fn handle_secure_session<T>(
    tls_stream: T,
    webhook_handle: ActorHandle,
    target_emails: Vec<String>,
    header_prefixes: Vec<String>,
) -> Result<()>
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (read_half, write_half) = tokio::io::split(tls_stream);
    let reader = tokio::io::BufReader::new(read_half);
    let writer = tokio::io::BufWriter::new(write_half);
    let mut protocol = SmtpProtocol::new(reader, writer);

    let mut sender = String::new();
    let mut accepted_recipient = String::new();
    let mut email_data = String::new();
    let mut collecting_data = false;

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

        match result {
            SmtpCommandResult::Quit => break,
            SmtpCommandResult::MailFrom(email) => {
                sender = email;
            }
            SmtpCommandResult::RcptTo(email) => {
                let received_email = email;
                let received_email_lower = received_email.to_lowercase();
                if target_emails
                    .iter()
                    .any(|target| target.to_lowercase() == received_email_lower)
                {
                    accepted_recipient = received_email;
                    protocol.write_line("250 OK").await?;
                } else {
                    protocol.write_line("550 No such user here").await?;
                    accepted_recipient.clear();
                }
            }
            SmtpCommandResult::DataStart => {
                collecting_data = true;
                email_data.clear();
            }
            SmtpCommandResult::DataLine(line_content) => {
                if collecting_data {
                    email_data.push_str(&line_content);
                    email_data.push_str("\r\n");
                }
            }
            SmtpCommandResult::DataEnd => {
                collecting_data = false;
                let (subject, from_name, text_body, html_body, matched_headers) =
                    EmailParser::parse(email_data.as_bytes(), &header_prefixes)?;
                info!(
                    "Received email (TLS) from {} to {} (Subject: '{}')",
                    sender, accepted_recipient, subject
                );

                let headers = if matched_headers.is_empty() {
                    None
                } else {
                    Some(matched_headers)
                };
                let email_payload = EmailPayload {
                    sender: sender.clone(),
                    sender_name: from_name,
                    recipient: accepted_recipient.clone(),
                    subject,
                    body: text_body,
                    html_body,
                    headers,
                };
                webhook_handle
                    .send(ForwardEmail {
                        payload: email_payload,
                    })
                    .await;

                sender.clear();
                accepted_recipient.clear();
                email_data.clear();
            }
            SmtpCommandResult::Continue => {}
            SmtpCommandResult::StartTls => {
                warn!("Received STARTTLS command within secure session. Sending error.");
                protocol.write_line("503 STARTTLS already active").await?;
            }
        }
    }
    Ok(())
}
