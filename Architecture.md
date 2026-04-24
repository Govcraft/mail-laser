# Architecture

## Introduction

**Purpose:** MailLaser is an asynchronous SMTP server that receives mail for one or more configured target addresses and forwards the extracted content — sender, recipient, subject, plain-text body, optional HTML body, matched headers, and any attachments — as a JSON payload to a configured HTTPS webhook.

**Rationale:** MailLaser bridges email reception into webhook-driven systems without requiring a full mail server or embedded email libraries in the consumer. Incoming SMTP becomes structured HTTP events with authorization enforced at the policy layer and attachment bytes delivered either inline or through an S3-compatible object store.

**Use cases:**

*   Triggering serverless functions or automated workflows from incoming email.
*   Bridging legacy systems that emit email notifications into modern webhook platforms.
*   Building email-to-API gateways with attachment handoff to object storage.
*   Authorizing senders and attachments with a declarative policy layer (Cedar).

**Architecture overview:**

The application is built in Rust on the Tokio asynchronous runtime and organized around the [`acton-reactive`](https://crates.io/crates/acton-reactive) actor framework. Each long-running component is an actor with its own state and lifecycle; the entry point starts the actor runtime, wires actors together in dependency order, and blocks on a shutdown signal.

1.  **Entry point (`main.rs`)** — initializes `tracing` (with the `log` crate bridged into `tracing` via `tracing-log`), installs a panic hook that routes panic payloads into `tracing::error!`, then calls `mail_laser::run()`.
2.  **Orchestration (`lib.rs`)** — loads configuration, constructs the `PolicyEngine`, the optional `DmarcValidator` (`None` when DMARC is off), and the attachment `AttachmentBackend`, launches the acton runtime, and creates the `WebhookState`, `SmtpListenerState`, and `HealthState` actors in that order. Blocks on `Ctrl-C`; on shutdown, `runtime.shutdown_all()` drains in-flight work before exit.
3.  **Configuration (`config`)** — `Config` struct loaded from environment variables (with `.env` support). Covers SMTP/health bind addresses and ports, target emails, webhook URL and resilience settings, header-prefix passthrough, Cedar policy/entity paths, size caps, and the attachment-delivery mode.
4.  **Policy (`policy`)** — Cedar-based authorization engine evaluated at two points, both at end-of-DATA after DMARC has run: `can_send` (principal selected from DMARC-aligned From in Enforce mode, otherwise envelope sender) and `can_attach` per attachment after parsing. Both evaluations receive the full DMARC outcome as Cedar context.
5.  **DMARC (`dmarc`)** — optional RFC 7489 SPF + DKIM + DMARC gate evaluated at end-of-DATA, before Cedar. Off by default; when `Monitor` or `Enforce` is configured, rejects or annotates messages from spoofed senders using the `mail-auth` crate and a `hickory-resolver`-backed DNS client. The outcome feeds Cedar's authorization context regardless of mode.
6.  **SMTP server (`smtp`)** — `SmtpListenerState` actor owns a `tokio::net::TcpListener`, gates accept via a per-source-IP concurrency cap (`IpLimiter`), and spawns per-connection tasks that run a STARTTLS-capable SMTP state machine, evaluate DMARC, run Cedar `SendMail`, parse the DATA segment into a `ParsedEmail`, run Cedar `Attach` per attachment, pass attachments through the selected `AttachmentBackend`, and dispatch a `ForwardEmail` message to the webhook actor.
7.  **Attachment backends (`attachment`)** — `AttachmentBackend` trait with two implementations: `InlineBackend` (base64-encodes into the JSON payload) and `S3Backend` (uploads to any S3-compatible bucket and emits an `s3://` URL plus an optional presigned GET URL).
8.  **Webhook client (`webhook`)** — `WebhookState` actor wrapping a `hyper` + `hyper-rustls` HTTPS client. Handles JSON serialization, retries with exponential backoff, and a circuit breaker that drops deliveries when consecutive failures exceed the configured threshold.
9.  **Health check (`health`)** — `HealthState` actor running a minimal `hyper` HTTP server that answers `GET /health` with `200 OK` and all other paths with `404`.

All actors are supervised by the acton runtime with `RestartPolicy::Permanent`; each owns a `CancellationToken` so `before_stop` can cleanly cancel its accept loop during shutdown.

---

## Modules

### `src/config`

**Purpose:** Loads the `Config` struct from environment variables.

**Key components:**

*   **`Config` struct** — full runtime configuration. Notable fields:
    *   `target_emails: Vec<String>` — addresses the server accepts mail for.
    *   `webhook_url: String` — target HTTPS endpoint.
    *   `smtp_bind_address` / `smtp_port` / `health_check_bind_address` / `health_check_port` — listener configuration.
    *   `header_prefixes: Vec<String>` — case-insensitive header-name prefixes captured and forwarded as a `headers` map.
    *   `webhook_timeout_secs`, `webhook_max_retries`, `circuit_breaker_threshold`, `circuit_breaker_reset_secs` — delivery resilience.
    *   `cedar_policies_path: PathBuf` (required) and `cedar_entities_path: Option<PathBuf>` — Cedar policy + optional entity store paths.
    *   `max_message_size_bytes`, `max_attachment_size_bytes` — hard caps enforced during SMTP DATA ingest.
    *   `attachment_delivery: AttachmentDelivery` — `Inline` or `S3(S3Settings)`.
    *   `dmarc_mode`, `dmarc_dns_timeout_secs`, `dmarc_dns_servers`, `dmarc_temperror_action` — DMARC validator configuration; see `src/dmarc`.
    *   `max_concurrent_per_ip: u32` — per-source-IP concurrent connection cap (`0` disables).
*   **`AttachmentDelivery` enum** — tagged by `mode` (`"inline"` / `"s3"`) for serde round-trips.
*   **`S3Settings` struct** — `bucket`, `region`, optional `endpoint` (for MinIO/R2/Wasabi), `key_prefix`, optional `presign_ttl_secs`.
*   **`Config::from_env()`** — loads `.env` via `dotenv`, validates required variables, parses ports as `u16` and size caps as `u64`, and dispatches into `parse_attachment_delivery` / `parse_s3_settings` for the delivery mode.

**Environment variables:**

| Variable | Required | Default | Notes |
|---|---|---|---|
| `MAIL_LASER_TARGET_EMAILS` | yes | — | Comma-separated, whitespace-trimmed, non-empty. |
| `MAIL_LASER_WEBHOOK_URL` | yes | — | HTTPS endpoint. |
| `MAIL_LASER_CEDAR_POLICIES` | yes | — | Path to a Cedar policy file (text format). |
| `MAIL_LASER_CEDAR_ENTITIES` | no | — | Path to a Cedar entities JSON file. |
| `MAIL_LASER_BIND_ADDRESS` | no | `0.0.0.0` | SMTP bind address. |
| `MAIL_LASER_PORT` | no | `2525` | SMTP port (`u16`). |
| `MAIL_LASER_HEALTH_BIND_ADDRESS` | no | `0.0.0.0` | Health check bind address. |
| `MAIL_LASER_HEALTH_PORT` | no | `8080` | Health check port (`u16`). |
| `MAIL_LASER_HEADER_PREFIX` | no | empty | Comma-separated, case-insensitive header-name prefixes to forward. |
| `MAIL_LASER_WEBHOOK_TIMEOUT` | no | `30` | Per-attempt timeout (seconds). |
| `MAIL_LASER_WEBHOOK_MAX_RETRIES` | no | `3` | Retry attempts on delivery failure. |
| `MAIL_LASER_CIRCUIT_BREAKER_THRESHOLD` | no | `5` | Consecutive failures that open the circuit. |
| `MAIL_LASER_CIRCUIT_BREAKER_RESET` | no | `60` | Seconds before the breaker half-opens. |
| `MAIL_LASER_MAX_MESSAGE_SIZE` | no | `26_214_400` | SMTP DATA cap (bytes). |
| `MAIL_LASER_MAX_ATTACHMENT_SIZE` | no | `10_485_760` | Per-attachment cap (bytes). |
| `MAIL_LASER_ATTACHMENT_DELIVERY` | no | `inline` | `inline` or `s3`. |
| `MAIL_LASER_S3_BUCKET` | iff `=s3` | — | Target bucket. |
| `MAIL_LASER_S3_REGION` | iff `=s3` | — | AWS region string (e.g. `us-east-1`). |
| `MAIL_LASER_S3_ENDPOINT` | no | — | Endpoint override for non-AWS S3-compatible stores. |
| `MAIL_LASER_S3_KEY_PREFIX` | no | empty | Prepended to every generated key. |
| `MAIL_LASER_S3_PRESIGN_TTL` | no | — | When set (non-zero `u64`), each uploaded object gets a presigned GET URL with this TTL. |
| `MAIL_LASER_DMARC_MODE` | no | `off` | `off` / `monitor` / `enforce`. See the `src/dmarc` module for semantics. |
| `MAIL_LASER_DMARC_DNS_TIMEOUT` | no | `5` | Seconds; overall budget for SPF + DKIM + DMARC DNS lookups. Timeout → `TempError`. |
| `MAIL_LASER_DMARC_DNS_SERVERS` | no | empty | Comma-separated `ip:port` list. When unset, the system resolver is used. |
| `MAIL_LASER_DMARC_TEMPERROR_ACTION` | no | `reject` | `reject` (451) / `accept`. Only consulted in `enforce` mode. |
| `MAIL_LASER_MAX_CONCURRENT_PER_IP` | no | `10` | Max concurrent SMTP sessions per peer IP. `0` disables. Over-cap connections are dropped at TCP accept without an SMTP greeting. |
| `MAIL_LASER_MAX_UNKNOWN_RCPTS_PER_SESSION` | no | `3` | Max unknown `RCPT TO` recipients per session. Nth unknown → `421 4.7.0` + socket close. Bounds in-session enumeration of `target_emails`. `0` disables. |
| `RUST_LOG` | no | `info` | Consumed by `tracing-subscriber::EnvFilter`. |

**Dependencies:** `anyhow`, `serde`, `dotenv`, `log`, `std::env`, `std::path`.

### `src/policy`

**Purpose:** Cedar-based authorization for sender and attachment decisions.

**Key components:**

*   **`PolicyEngine` struct** — wraps a `cedar_policy::PolicySet`, an `Entities` store (empty when no entities file is supplied), and an `Authorizer`. Cheap to `Arc`-clone and safe to share across tasks.
*   **`PolicyEngine::load(policies_path, entities_path)`** — reads policy text and (optionally) entities JSON from disk; returns a fully constructed engine.
*   **`PolicyEngine::from_strings(...)`** — in-memory constructor used by tests.
*   **`DmarcContext` struct** — per-request DMARC facts surfaced to Cedar as context. Constructed once in `finalize_message` after DMARC runs and reused across `SendMail` and `Attach` so both evaluations see a consistent view. Fields: `result` (`"pass"|"fail"|"none"|"temperror"|"off"`), `aligned: bool`, `authenticated_from: Option<String>`, `envelope_from: String`, `helo: String`, `peer_ip: IpAddr`.
*   **`can_send(principal: &str, recipient: &str, &DmarcContext) -> bool`** — builds a `User::"<principal>"` principal, action `Action::"SendMail"`, resource `Recipient::"<recipient>"`, and the DMARC context (`context.dmarc_result`, `context.dmarc_aligned`, `context.authenticated_from`, `context.envelope_from`, `context.helo`, `context.peer_ip`). Invoked at end-of-DATA after DMARC runs; the caller selects the principal (DMARC-aligned From in Enforce mode when DMARC passed, otherwise envelope sender). Rejection returns `550 5.7.1 Sender not authorized`.
*   **`can_attach(principal: &str, att: &AttachmentCheck<'_>, &DmarcContext)`** — builds the request for `Action::"Attach"`, merging attachment-specific fields (`filename`, `content_type`, `size_bytes`) into the same DMARC context so policies can gate attachments on authentication state too. Invoked once per parsed attachment.
*   **`AttachmentCheck<'a>` struct** — lightweight view of an attachment used only for policy evaluation (no bytes).

**Dependencies:** `cedar-policy`, `anyhow`, `tracing`, `std::fs`, `std::net`.

### `src/dmarc`

**Purpose:** Optional RFC 7489 authentication gate — validates SPF + DKIM + DMARC on inbound messages and, in `enforce` mode, rejects spoofed From headers at the SMTP layer before the payload reaches the webhook. Off by default so existing deployments see no behavior change.

**Why this matters for public-MX deployments:** mail-laser's envelope `MAIL FROM` is trivially spoofable. With Cedar alone, an attacker on the open internet can impersonate any allow-listed sender — driving reflected mail to the real victim's inbox, poisoning downstream attribution (federal records, FOIA logs, LLM-driven workflows), and burning paid-API budget. DMARC is the industry-standard defense: most federal domains publish `p=reject` under BOD 18-01, and a receiving MTA that verifies SPF + DKIM + DMARC alignment rejects spoofed mail before it ever reaches the application layer.

**Key components:**

*   **`DmarcMode` enum** (`config`) — `Off` (default), `Monitor`, `Enforce`.
*   **`DmarcTempErrorAction` enum** (`config`) — `Reject` (451 SMTP reply, default) or `Accept` (fail-open). Only consulted in `Enforce`.
*   **`DmarcValidator` struct** — wraps a `mail_auth::MessageAuthenticator` (hickory-resolver-backed) and a `Duration` timeout. `Arc`-cloned into each SMTP session.
    *   `DmarcValidator::load(&Config) -> Result<Option<Arc<Self>>>` — returns `None` when `mode = Off`, so the actor and per-session paths never touch DNS in the default configuration.
    *   `validate(raw_bytes, peer_ip, helo, envelope_from).await -> DmarcOutcome` — wraps SPF + DKIM + DMARC in a single `tokio::time::timeout`; a timeout always maps to `TempError`.
*   **`DmarcOutcome` enum** — `Pass { authenticated_from }`, `Fail`, `TempError`, `NoPolicy`.
*   **`DmarcDecision` enum** — the SMTP-facing mapping. `Accept { dmarc_result, authenticated_from }` or `Reject { code, status }`.
*   **`decide(outcome, mode, temperror_action) -> DmarcDecision`** — pure function, unit-tested. Decides both the SMTP reply and the webhook-payload annotation.
*   **Organizational-domain helper** — `psl::domain_str` reduces subdomains to their registrable domain for alignment checks (handles nested TLDs like `.co.uk` correctly).

**Hook point:** `finalize_message` in `src/smtp/mod.rs` runs `run_dmarc` after the size-exceeded and empty-sender guards and before Cedar `can_send` and `EmailParser::parse`. DMARC evaluates first so a DMARC rejection returns an SMTP reply without burning parse + policy + backend budget. Cedar `can_send` then runs with the DMARC outcome in context and the principal selected per mode; Cedar `can_attach` runs per attachment after parsing with the same DMARC context plus attachment metadata.

**SMTP reply codes:**

| Outcome | Mode | Action | SMTP reply |
|---|---|---|---|
| `Pass` / `NoPolicy` | any | — | `250 OK: Message accepted for delivery` |
| `Fail` | `Monitor` | — | `250 OK` (logged, `dmarc_result=fail` in payload) |
| `Fail` | `Enforce` | — | `550 5.7.1 DMARC policy violation` |
| `TempError` | `Monitor` | — | `250 OK` (logged) |
| `TempError` | `Enforce` | `Reject` | `451 4.7.0 DMARC temporary error` |
| `TempError` | `Enforce` | `Accept` | `250 OK` (logged) |

**Webhook payload additions:**

*   `dmarc_result: Option<String>` — `"pass" | "fail" | "none" | "temperror"` when `Monitor` or `Enforce`; omitted when `Off`.
*   `authenticated_from: Option<String>` — DMARC-aligned From-header address when `dmarc_result == "pass"`; omitted otherwise.

Both fields use `#[serde(skip_serializing_if = "Option::is_none")]`, so existing consumers that ignore unknown JSON fields are unaffected.

**Dot-unstuffing fix:** adding DMARC surfaced a latent bug in the SMTP DATA loop — a line beginning with `.` on the wire (doubled per RFC 5321 §4.5.2) was being stored in the accumulation buffer with the leading dot still doubled. DKIM body-hash verification would have failed for any such message. The fix lives in the `DataLine` arm of `step` in `src/smtp/mod.rs`: strip exactly one leading dot before pushing into `email_data`.

**Recommended rollout:** start with `MAIL_LASER_DMARC_MODE=off` (no change). Flip to `monitor` and let logs accumulate for a week — review the distribution of `dmarc_result` values for your actual senders. Flip to `enforce` when the baseline looks clean.

**Dependencies:** `mail-auth`, `psl`, `anyhow`, `tracing`, `tokio`.

### `src/attachment`

**Purpose:** Delivery strategies for binary MIME attachments extracted from inbound mail.

**Key components:**

*   **`AttachmentBackend` trait** — `async fn prepare(&self, att: Attachment) -> Result<SerializedAttachment>`. Each backend owns the attachment bytes and returns a serializable representation for the webhook payload.
*   **`SerializedAttachment` struct** — metadata (`filename`, `content_type`, `size_bytes`, `content_id`) plus a flattened `AttachmentPayload`. Serde `skip_serializing_if = "Option::is_none"` keeps optional fields out of the payload when absent.
*   **`AttachmentPayload` enum** — serde-tagged by `delivery`:
    *   `Inline { data_base64 }` — bytes embedded in the JSON.
    *   `S3 { url, presigned_url }` — `s3://bucket/key` plus optional presigned GET URL.
*   **`build(config) -> Arc<dyn AttachmentBackend>`** — dispatches on `config.attachment_delivery`.

#### `src/attachment/inline.rs`

*   **`InlineBackend`** — standard-base64 encodes the attachment bytes with `base64::engine::general_purpose::STANDARD`.

#### `src/attachment/s3.rs`

*   **`S3Backend`** — wraps an `aws_sdk_s3::Client`. Constructed via `S3Backend::new(S3Settings)`:
    *   Uses `aws_config::defaults(BehaviorVersion::latest())` to pick up credentials from the default provider chain.
    *   When `S3Settings::endpoint` is set, applies `endpoint_url(...)` and forces path-style addressing (`force_path_style(true)`) — the compatible-store idiom.
    *   `prepare` generates a key of the form `{key_prefix}{uuid}-{sanitized_filename}`, uploads via `put_object`, and (when `presign_ttl_secs` is `Some`) generates a presigned GET URL via `client.get_object().presigned(...)`.
    *   `sanitize_filename` restricts keys to `[A-Za-z0-9._-]`, replacing other characters with `_`.

**Dependencies:** `aws-config`, `aws-sdk-s3`, `base64`, `uuid`, `async-trait`, `anyhow`.

### `src/smtp`

**Purpose:** The SMTP server. Accepts connections, drives the SMTP state machine (including STARTTLS), parses inbound messages, enforces policy, prepares attachments, and dispatches the resulting payload to the webhook actor.

**Key components:**

*   **`SmtpListenerState`** — acton actor declared with `#[acton_actor]`. `RestartPolicy::Permanent`.
    *   `create(runtime, config, webhook_handle, policy, backend, dmarc)` builds the actor, spawns the accept loop in `after_start`, and registers `before_stop` to cancel the loop via a `CancellationToken`.
    *   The accept loop binds the `TcpListener`, consults the `IpLimiter` for every accepted socket, and per-permitted connection spawns a task running `handle_connection`. The `IpConnGuard` is moved into the spawned task so its drop releases the slot when the session ends.
*   **`IpLimiter`** (in `src/smtp/ip_limiter.rs`) — bounds concurrent sessions per source IP via `Arc<Mutex<HashMap<IpAddr, u32>>>`. `try_acquire(ip)` returns an RAII `IpConnGuard` on success or `None` when the cap is reached; on `None`, the socket is dropped at accept with no SMTP greeting. `max_per_ip == 0` disables the limiter entirely.
*   **`SessionContext` struct** — per-connection bundle: webhook handle, target emails, header prefixes, `Arc<PolicyEngine>`, `Arc<dyn AttachmentBackend>`, size caps, the connecting peer IP (for SPF + Cedar context), and `Option<Arc<DmarcValidator>>` plus the DMARC mode / temperror action. Cheap to clone into each session.
*   **`handle_connection`** — runs the plaintext SMTP dialogue; on `STARTTLS`, swaps the stream for a `tokio_rustls` server session (with a self-signed cert generated at startup by `rcgen::generate_simple_self_signed`) and continues with the same state machine. Enforces recipient validation (case-insensitive match against `target_emails`), provisionally accepts MAIL FROM (Cedar eval is deferred), streams DATA into a bounded buffer (drops the transaction on `max_message_size_bytes`), and on `DataEnd` invokes `finalize_message`.
*   **`finalize_message`** — end-of-DATA pipeline: run DMARC → build `DmarcContext` → select principal (DMARC-aligned From when `Enforce` + `Pass`, otherwise envelope sender) → Cedar `can_send(principal, recipient, &dmarc_ctx)` → parse MIME → per-attachment `can_attach(principal, att, &dmarc_ctx)` → backend prepare → dispatch `ForwardEmail`. Any step's rejection emits the appropriate SMTP reply and short-circuits.
*   **Sub-modules:** `email_parser` (MIME parsing), `smtp_protocol` (state machine), `ip_limiter` (per-IP connection cap).

**Dependencies:** `acton-reactive`, `tokio`, `tokio-util` (for `CancellationToken`), `tokio-rustls`, `rustls`, `rcgen`, `anyhow`, `tracing`/`log`.

#### `src/smtp/email_parser.rs`

**Purpose:** Parses raw RFC 2822 / MIME bytes into a structured `ParsedEmail`.

**Key components:**

*   **`Attachment` struct** — `filename: Option<String>`, `content_type: String`, `size_bytes: u64`, `content_id: Option<String>`, `data: Vec<u8>`.
*   **`ParsedEmail` struct** — `subject`, `from_name: Option<String>`, `text_body`, `html_body: Option<String>`, `matched_headers: HashMap<String, String>`, `attachments: Vec<Attachment>`.
*   **`EmailParser::parse(raw_data, header_prefixes)`** — delegates to `mailparse::parse_mail`, walks the MIME tree, extracts:
    *   `Subject`, `From` display name.
    *   The first `text/plain` and `text/html` parts (with `html2text` used only to derive a text body when the message is HTML-only).
    *   Headers whose names case-insensitively match any configured prefix.
    *   Every remaining leaf part — plus any part marked `Content-Disposition: attachment` — as an `Attachment`, with bytes decoded via `mailparse::ParsedMail::get_body_raw`.

**Dependencies:** `mailparse`, `html2text`, `anyhow`, `log`.

#### `src/smtp/smtp_protocol.rs`

**Purpose:** Implements the SMTP state machine and protocol I/O, including STARTTLS negotiation.

**Key components:**

*   **`SmtpState` enum** — `Initial`, `Greeted`, `MailFrom`, `RcptTo`, `Data`.
*   **`SmtpProtocol` struct** — buffered reader/writer over any `AsyncRead + AsyncWrite` stream (plaintext `TcpStream` or TLS-wrapped stream).
*   **`process_command(line: &str) -> SmtpCommandResult`** — parses and dispatches SMTP verbs. `EHLO` advertises `STARTTLS`; `STARTTLS` itself returns `SmtpCommandResult::StartTls` so the connection handler can upgrade the stream.
*   **`SmtpCommandResult` enum** — `Continue`, `Quit`, `Helo(String)`, `MailFrom(String)`, `RcptTo(String)`, `DataStart`, `DataLine(String)`, `DataEnd`, `StartTls`. The `Helo` variant carries the HELO/EHLO domain (or the `"client"` fallback) so the SMTP layer can stash it for SPF verification.
*   **I/O helpers** — CRLF-terminated `read_line` / `write_line` and an `extract_email` helper for angle-addr parsing.

**Dependencies:** `tokio`, `anyhow`, `log`.

### `src/webhook`

**Purpose:** Delivers the parsed email as JSON to the configured webhook URL, with retry and circuit-breaker resilience.

**Key components:**

*   **`EmailPayload` struct** — serde-serialized payload:
    *   `sender: String`, `recipient: String`, `subject: String`, `body: String` (text body) — always present.
    *   `sender_name: Option<String>`, `html_body: Option<String>`, `headers: Option<HashMap<String, String>>`, `attachments: Option<Vec<SerializedAttachment>>` — omitted when empty/absent via `skip_serializing_if`.
    *   `dmarc_result: Option<String>`, `authenticated_from: Option<String>` — populated only when DMARC is enabled (`Monitor` or `Enforce`); see `src/dmarc`.
*   **`ForwardEmail` message** — acton message (`#[acton_message]`) carrying an `EmailPayload` from the SMTP actor to the webhook actor.
*   **`WebhookState`** — acton actor. Holds a `WebhookClient` (a `hyper_util::client::legacy::Client` over `hyper_rustls::HttpsConnector<HttpConnector>` serving `Full<Bytes>`), and circuit-breaker state.
    *   On receipt of `ForwardEmail`, attempts delivery up to `webhook_max_retries + 1` times with exponential backoff, honoring `webhook_timeout_secs` per attempt.
    *   On consecutive failures exceeding `circuit_breaker_threshold`, the breaker opens and subsequent `ForwardEmail` messages are dropped with a warning until `circuit_breaker_reset_secs` elapses and a single probe is attempted.
    *   In debug builds the connector is `https_or_http` so local tests can target HTTP endpoints; release builds are `https_only`.
*   **`WebhookResult` message** — internal actor message carrying per-attempt outcome for the circuit-breaker state machine.

**Dependencies:** `acton-reactive`, `hyper`, `hyper-rustls`, `hyper-util`, `http-body-util`, `bytes`, `serde`, `serde_json`, `tokio`, `tracing`/`log`.

### `src/health`

**Purpose:** Minimal HTTP health check endpoint for liveness monitoring.

**Key components:**

*   **`HealthState`** — acton actor with `RestartPolicy::Permanent`.
    *   `create(runtime, config)` binds a `TcpListener` in `after_start` and serves connections through `hyper_util::server::conn::auto::Builder`.
    *   `before_stop` cancels the accept loop via a `CancellationToken`.
*   **`health_check_handler`** — returns `200 OK` for `/health` (any method) and `404 Not Found` otherwise.

**Dependencies:** `acton-reactive`, `hyper`, `hyper-util`, `http-body-util`, `http-body`, `bytes`, `tokio`, `tokio-util`.

### `src/lib.rs`

**Purpose:** Library entry point. Composes configuration, policy, attachment backend, and the three actors into a running system.

**Key components:**

*   **Module declarations:** `attachment`, `config`, `dmarc`, `health`, `policy`, `smtp`, `webhook`.
*   **`run()` async function:**
    1.  Logs startup banner (crate name + version).
    2.  Loads `Config` via `Config::from_env()`.
    3.  Builds `Arc<PolicyEngine>` via `PolicyEngine::load(...)`.
    4.  Builds `Option<Arc<DmarcValidator>>` via `DmarcValidator::load(&config)` — `None` when `dmarc_mode = Off`.
    5.  Builds `Arc<dyn AttachmentBackend>` via `attachment::build(&config).await`.
    6.  Launches the acton runtime: `ActonApp::launch_async().await`.
    7.  Creates actors in dependency order: `WebhookState` → `SmtpListenerState` (injected webhook handle, policy, backend, optional DMARC validator) → `HealthState`.
    8.  Awaits `tokio::signal::ctrl_c()`, then calls `runtime.shutdown_all().await` to drain in-flight work.
*   Propagates errors at every step with `tracing::error!` and returns them from `run()`.

**Dependencies:** `acton-reactive`, `anyhow`, `log`, `tokio`.

### `src/main.rs`

**Purpose:** Binary entry point.

**Key components:**

*   **`#[tokio::main]`** — starts the Tokio runtime.
*   **Tracing initialization** — `tracing_subscriber::fmt` with an `EnvFilter` that defaults to `info` and respects `RUST_LOG`. `tracing_log::LogTracer::init()` bridges the `log` crate into `tracing` so transitive code using `log` macros still shows up.
*   **Panic hook** — routes panic payloads and source locations into `tracing::error!`.
*   **Execution** — invokes `mail_laser::run().await`; on `Err`, logs and exits with status `1`.

**Dependencies:** `tokio`, `tracing`, `tracing-subscriber`, `tracing-log`, `mail_laser`.

---

## Tests

*   **Unit tests** live alongside each module (`src/*/tests.rs` or `#[cfg(test)] mod tests` blocks). They cover config parsing, policy evaluation (including DMARC-context-gated permits and `Recipient` resource matching), attachment serialization + key generation, email parsing (including multipart/mixed with mixed encodings), the SMTP state machine, webhook payload shape, DMARC decision logic (outcome × mode × temperror-action combinations via the pure `decide` helper), and the `IpLimiter` (cap, release on drop, disabled mode, per-IP isolation).
*   **Integration tests** under `tests/`:
    *   `tests/integration.rs` — end-to-end SMTP → parse → webhook path with a `mockserver/mockserver` container. Covers the happy path, webhook retry on failure, circuit-breaker opening, oversize-message (552) rejection, DMARC monitor-mode annotation (using a `.invalid` TLD so the DMARC lookup is deterministically NXDOMAIN), Cedar end-of-DATA denial when a `context.dmarc_result == "pass"` policy meets DMARC-off traffic, and the per-IP connection cap dropping an over-cap connection without a greeting.
    *   `tests/s3_attachment.rs` — end-to-end with a real MinIO container. Covers both `presign_ttl_secs = None` and `Some(_)` paths: uploads a multipart/mixed message, asserts the webhook payload shape (`delivery: "s3"`, `url`, optional `presigned_url`, `size_bytes`), and round-trips the uploaded bytes via the SDK or the presigned URL.

Both integration test files use `testcontainers` to spin up dependencies; Docker is required to run them.

---

## Build and environment

### `Dockerfile`

**Purpose:** Multi-stage build producing a minimal, statically linked production image.

**Key aspects:**

*   **Multi-stage:** `rust:slim` builder stage; final stage is `scratch`.
*   **Static linking:** `x86_64-unknown-linux-musl` target + `musl-tools` yields a fully static binary.
*   **Non-root builder user:** build steps run as a dedicated `builder` user.
*   **Dependency caching:** `Cargo.toml` + `Cargo.lock` are copied first and dependencies are pre-built so source changes don't invalidate the dependency layer.
*   **Minimal final image:** only the binary and CA certificates (for outbound HTTPS) land in the `scratch` image.

### `flake.nix`

**Purpose:** Reproducible development environments via Nix Flakes, `flake-parts`, and `dev-environments`.

**Key aspects:**

*   **Inputs:** `flake-parts`, `nixpkgs`, `dev-environments`.
*   **Rust environment:** stable toolchain enabled by default.
*   **Default shell (`devShells.default`):** the Rust env plus `openssl` and `swaks` (SMTP test tool).
*   **Reproducibility:** aligns developer toolchains regardless of host OS.

### `LICENSE`

MIT — permissive use, modification, and distribution with attribution.
