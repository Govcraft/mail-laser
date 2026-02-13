---
title: Architecture
nextjs:
  metadata:
    title: Architecture
    description: MailLaser's actor-based architecture, module structure, and graceful shutdown design.
---

MailLaser is built in Rust using the `acton-reactive` actor framework and the Tokio asynchronous runtime. The architecture separates concerns into independent actors that communicate through message passing.

---

## Actor model

MailLaser uses three actors, each responsible for a distinct concern:

| Actor | Module | Restart policy | Responsibility |
|-------|--------|----------------|----------------|
| `SmtpListenerState` | `smtp` | Permanent | Accepts TCP connections and spawns per-connection handlers. |
| `WebhookState` | `webhook` | Permanent | Delivers email payloads to the webhook with retry and circuit breaker logic. |
| `HealthState` | `health` | Permanent | Serves the `/health` HTTP endpoint. |

All actors use the `Permanent` restart policy, meaning the `acton-reactive` framework automatically restarts them if they fail unexpectedly.

### Message flow

```text
[SMTP Client] --> SmtpListenerState --> (per-connection task)
                                              |
                                              | ForwardEmail message
                                              v
                                        WebhookState --> [Webhook URL]
                                              |
                                              | WebhookResult message (self)
                                              v
                                        (circuit breaker state update)
```

The `SmtpListenerState` actor spawns a Tokio task for each TCP connection. When a connection handler finishes parsing an email, it sends a `ForwardEmail` message to the `WebhookState` actor's handle. The webhook actor delivers the email and sends a `WebhookResult` message to itself to update circuit breaker state.

---

## Module structure

```text
src/
  main.rs           Entry point: logging, panic hooks, runtime
  lib.rs            Orchestration: config loading, actor creation, shutdown
  config/
    mod.rs          Config struct, from_env() loading
    tests.rs        Configuration unit tests
  smtp/
    mod.rs          SmtpListenerState actor, connection handlers, STARTTLS
    smtp_protocol.rs  SMTP state machine, command parsing
    email_parser.rs   MIME parsing, body extraction, header matching
    tests/
      smtp_protocol_tests.rs
      email_parser_tests.rs
  webhook/
    mod.rs          WebhookState actor, WebhookClient, EmailPayload, resilience
    tests.rs        Webhook delivery unit tests
  health/
    mod.rs          HealthState actor, HTTP handler
```

---

## Startup sequence

The `lib.rs` `run()` function orchestrates startup:

1. Load configuration from environment variables (`Config::from_env()`)
2. Launch the `acton-reactive` runtime (`ActonApp::launch_async()`)
3. Create the `WebhookState` actor (produces an `ActorHandle`)
4. Create the `SmtpListenerState` actor, passing it the webhook handle
5. Create the `HealthState` actor
6. Wait for `SIGTERM` or `SIGINT` via `tokio::signal::ctrl_c()`

Actors are created in dependency order: the webhook actor must exist before the SMTP actor, because the SMTP actor needs the webhook handle to forward emails.

---

## Graceful shutdown

When MailLaser receives a shutdown signal:

1. `tokio::signal::ctrl_c()` returns
2. `runtime.shutdown_all()` is called on the `acton-reactive` runtime
3. Each actor's `before_stop` handler fires:
   - `SmtpListenerState`: Cancels the accept loop via `CancellationToken`, stopping new connections
   - `HealthState`: Cancels the health server accept loop
   - `WebhookState`: Logs final forwarded/failed counts
4. In-flight webhook deliveries (already in the actor's message queue) complete before the actor fully stops
5. The application exits

The cancellation token pattern (`tokio_util::sync::CancellationToken`) ensures that each actor's background task stops cleanly. The `tokio::select!` in each listener loop checks the cancellation token alongside new connections, so shutdown is responsive even when no connections are pending.

---

## Connection handling

Each incoming SMTP connection follows this path:

1. `SmtpListenerState`'s background task accepts a TCP connection
2. A new Tokio task is spawned for the connection (`handle_connection`)
3. The connection handler creates an `SmtpProtocol` instance for state machine management
4. Commands are processed in a loop until the client disconnects or quits
5. If the client sends `STARTTLS`, the connection is upgraded:
   - A self-signed certificate is generated (`rcgen`)
   - The TCP stream is wrapped with `tokio-rustls`
   - Processing continues via `handle_secure_session`
6. When an email is fully received and parsed, a `ForwardEmail` message is sent to the webhook actor

Multiple connections are handled concurrently because each runs in its own Tokio task.

---

## Key dependencies

| Crate | Purpose |
|-------|---------|
| `acton-reactive` | Actor framework with supervision and restart policies |
| `tokio` | Async runtime, TCP networking, signal handling |
| `hyper` / `hyper-util` | HTTP client (webhook) and server (health check) |
| `hyper-rustls` | HTTPS for webhook delivery using native root certificates |
| `rustls` / `tokio-rustls` | TLS for SMTP STARTTLS |
| `rcgen` | Self-signed certificate generation |
| `mailparse` | MIME parsing and header extraction |
| `html2text` | HTML-to-plain-text conversion |
| `serde` / `serde_json` | JSON serialization for webhook payloads |
| `dotenv` | `.env` file loading |
| `tracing` / `tracing-subscriber` | Structured logging with env-filter support |

---

## Design decisions

**Why actors?** The actor model provides natural isolation between the SMTP listener, webhook delivery, and health check. Each actor manages its own state (especially the circuit breaker in the webhook actor) without shared mutable state or locks.

**Why no authentication?** MailLaser is designed as an internal bridge component, not a public-facing mail server. Adding SMTP AUTH would increase complexity without serving the primary use case. Network-level security (firewalls, VPNs, bind addresses) provides access control.

**Why fire-and-forget?** The SMTP session acknowledges email receipt before webhook delivery completes. This prevents slow webhooks from causing SMTP timeouts and keeps the SMTP protocol flow simple. The resilience patterns (retry + circuit breaker) handle delivery reliability independently.

**Why self-signed TLS?** STARTTLS support allows encrypted connections without requiring certificate management. For internal deployments, self-signed certificates provide transport encryption. For internet-facing deployments where certificate validation matters, terminate TLS at a reverse proxy.
