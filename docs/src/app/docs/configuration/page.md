---
title: Configuration
nextjs:
  metadata:
    title: Configuration
    description: All MailLaser environment variables, default values, and .env file support.
---

MailLaser is configured entirely through environment variables. Set them directly in your shell, pass them via Docker's `-e` flag, or place them in a `.env` file in the working directory.

{% callout type="warning" title="v3.0 breaking change" %}
v3.0 requires `MAIL_LASER_CEDAR_POLICIES` pointing to a Cedar policy file. v2.0 deployments that omit this will fail to start. See [Upgrading to v3](/docs/upgrading-to-v3) for a drop-in policy that preserves v2 behavior.
{% /callout %}

---

## Required variables

These must be set for MailLaser to start. If any is missing, the application exits with an error.

| Variable | Description |
|----------|-------------|
| `MAIL_LASER_TARGET_EMAILS` | Comma-separated list of email addresses to accept. At least one address is required. Whitespace around commas is trimmed. |
| `MAIL_LASER_WEBHOOK_URL` | The URL where email payloads are forwarded via HTTP POST. |
| `MAIL_LASER_CEDAR_POLICIES` | Path to a Cedar policy file that decides which senders may send to which recipients and which attachments are allowed. See [Authorization](/docs/authorization). |

---

## Optional variables

These have sensible defaults and can be left unset for most deployments.

### Server settings

| Variable | Default | Description |
|----------|---------|-------------|
| `MAIL_LASER_BIND_ADDRESS` | `0.0.0.0` | IP address the SMTP server binds to. |
| `MAIL_LASER_PORT` | `2525` | Port the SMTP server listens on. Must be a valid port number (1-65535). |
| `MAIL_LASER_HEALTH_BIND_ADDRESS` | `0.0.0.0` | IP address the health check server binds to. |
| `MAIL_LASER_HEALTH_PORT` | `8080` | Port the health check server listens on. |

### Webhook settings

| Variable | Default | Description |
|----------|---------|-------------|
| `MAIL_LASER_WEBHOOK_TIMEOUT` | `30` | Seconds to wait for a webhook response before timing out. |
| `MAIL_LASER_WEBHOOK_MAX_RETRIES` | `3` | Maximum retry attempts after a failed webhook delivery. |
| `MAIL_LASER_WEBHOOK_SIGNING_SECRET` | *(none)* | Shared secret for HMAC-SHA256 request signing. When set, each delivery carries `X-MailLaser-Timestamp` and `X-MailLaser-Signature-256` headers. See [Webhook signing](/docs/webhook-signing). |

### Circuit breaker settings

| Variable | Default | Description |
|----------|---------|-------------|
| `MAIL_LASER_CIRCUIT_BREAKER_THRESHOLD` | `5` | Consecutive failures before the circuit breaker opens. |
| `MAIL_LASER_CIRCUIT_BREAKER_RESET` | `60` | Seconds before an open circuit breaker transitions to half-open. |

### Authorization (Cedar)

| Variable | Default | Description |
|----------|---------|-------------|
| `MAIL_LASER_CEDAR_ENTITIES` | *(none)* | Path to an optional Cedar entities JSON file (users, groups, attributes referenced by policies). See [Authorization](/docs/authorization). |

### Attachments

| Variable | Default | Description |
|----------|---------|-------------|
| `MAIL_LASER_ATTACHMENT_DELIVERY` | `inline` | How attachments reach the webhook: `inline` embeds base64 bytes in the JSON payload, `s3` uploads to an S3-compatible bucket and emits a URL. See [Attachments](/docs/attachments). |
| `MAIL_LASER_MAX_MESSAGE_SIZE` | `26214400` | Maximum total SMTP message size in bytes (default 25 MiB). Advertised via the EHLO `SIZE` extension. Messages that exceed the limit are rejected with `552 5.3.4`. |
| `MAIL_LASER_MAX_ATTACHMENT_SIZE` | `10485760` | Maximum size in bytes for any single attachment (default 10 MiB). |
| `MAIL_LASER_S3_BUCKET` | *(none)* | Target bucket. Required when `MAIL_LASER_ATTACHMENT_DELIVERY=s3`. |
| `MAIL_LASER_S3_REGION` | *(none)* | Bucket region. Required when `MAIL_LASER_ATTACHMENT_DELIVERY=s3`. |
| `MAIL_LASER_S3_ENDPOINT` | *(AWS)* | Custom endpoint for S3-compatible stores (MinIO, R2, Wasabi). Omit to use AWS. |
| `MAIL_LASER_S3_KEY_PREFIX` | *(empty)* | Prefix prepended to every object key. Useful for namespacing (e.g. `mail-laser/inbound/`). |
| `MAIL_LASER_S3_PRESIGN_TTL` | *(none)* | When set, each uploaded object gets a presigned GET URL valid for this many seconds, in addition to the bare object URL. |

### DMARC validation

| Variable | Default | Description |
|----------|---------|-------------|
| `MAIL_LASER_DMARC_MODE` | `off` | `off` disables DMARC entirely. `monitor` validates and annotates the payload without rejecting. `enforce` rejects `fail` at SMTP. See [DMARC validation](/docs/dmarc). |
| `MAIL_LASER_DMARC_DNS_TIMEOUT` | `5` | Overall timeout in seconds for SPF + DKIM + DMARC DNS lookups. |
| `MAIL_LASER_DMARC_DNS_SERVERS` | *(system)* | Optional comma-separated list of explicit DNS servers as `ip:port`. Empty uses the system resolver. |
| `MAIL_LASER_DMARC_TEMPERROR_ACTION` | `reject` | How `enforce` mode handles DNS temperrors: `reject` returns `451 4.7.0` (fail-closed), `accept` accepts the message. |

### Connection limits

| Variable | Default | Description |
|----------|---------|-------------|
| `MAIL_LASER_MAX_CONCURRENT_PER_IP` | `10` | Maximum simultaneous SMTP sessions from a single peer IP. Over-cap connections are dropped without an SMTP greeting. Bounds the bandwidth an abusive client can consume before end-of-DATA authorization runs. Set to `0` to disable. |
| `MAIL_LASER_MAX_UNKNOWN_RCPTS_PER_SESSION` | `3` | Maximum unknown `RCPT TO` recipients tolerated in a single SMTP session. On the Nth unknown, the server replies `421 4.7.0` and closes the connection to bound recipient-address enumeration within a session. Set to `0` to disable. |

### Header passthrough

| Variable | Default | Description |
|----------|---------|-------------|
| `MAIL_LASER_HEADER_PREFIX` | *(none)* | Comma-separated list of header name prefixes to forward. Case-insensitive matching. See [Header passthrough](/docs/header-passthrough). |

### Logging

| Variable | Default | Description |
|----------|---------|-------------|
| `RUST_LOG` | `info` | Controls log verbosity. Common values: `error`, `warn`, `info`, `debug`, `trace`. |

You can target specific modules for granular control:

- `RUST_LOG=mail_laser=debug` -- Debug logging for MailLaser only
- `RUST_LOG=info,mail_laser::smtp=debug` -- Info globally, debug for the SMTP module
- `RUST_LOG=info,mail_laser::webhook=trace` -- Info globally, trace for the webhook module

Logs are written to stdout. Use `docker logs mail-laser` when running in Docker.

---

## Using a .env file

Place a `.env` file in the same directory where the binary runs. MailLaser loads it automatically at startup using the `dotenv` crate.

```shell
# .env
MAIL_LASER_TARGET_EMAILS=alerts@mydomain.com,support@mydomain.com
MAIL_LASER_WEBHOOK_URL=https://hooks.example.com/services/T000/B001/XXX
MAIL_LASER_CEDAR_POLICIES=/etc/mail-laser/policies.cedar
MAIL_LASER_PORT=2525
MAIL_LASER_HEALTH_PORT=8080
MAIL_LASER_HEADER_PREFIX=X-Custom,X-My-App
RUST_LOG=info
```

{% callout type="warning" title="Precedence" %}
Environment variables set in the shell take precedence over values in the `.env` file. If you set `MAIL_LASER_PORT=3000` in both the shell and the `.env` file, the shell value wins.
{% /callout %}

---

## Validation behavior

MailLaser validates configuration at startup:

- **Missing required variables**: The application logs an error and exits immediately.
- **Empty target emails**: If `MAIL_LASER_TARGET_EMAILS` is set but contains no valid addresses after trimming and splitting, startup fails.
- **Invalid port numbers**: If `MAIL_LASER_PORT` or `MAIL_LASER_HEALTH_PORT` cannot be parsed as a valid `u16`, startup fails with a descriptive error.
- **Invalid numeric values**: Timeout, retry, circuit breaker, and size-cap settings must be valid integers of their expected types.
- **Attachment delivery**: `MAIL_LASER_ATTACHMENT_DELIVERY=s3` requires `MAIL_LASER_S3_BUCKET` and `MAIL_LASER_S3_REGION`.
- **Cedar policy**: The file at `MAIL_LASER_CEDAR_POLICIES` must exist and parse as valid Cedar.

All loaded configuration values are logged at `info` level during startup, making it straightforward to verify what settings are in effect. Secret values (such as `MAIL_LASER_WEBHOOK_SIGNING_SECRET`) are never logged; the startup line shows only whether a secret is set.

---

## Example: minimal configuration

The smallest viable configuration sets the three required variables and points to a permissive Cedar policy:

```shell
MAIL_LASER_TARGET_EMAILS="inbox@myapp.com" \
MAIL_LASER_WEBHOOK_URL="https://myapp.com/email-hook" \
MAIL_LASER_CEDAR_POLICIES="/etc/mail-laser/policies.cedar" \
./mail_laser
```

With the matching Cedar policy:

```cedar
permit(principal, action == Action::"SendMail", resource);
permit(principal, action == Action::"Attach", resource);
```

This starts the SMTP server on `0.0.0.0:2525` and the health check on `0.0.0.0:8080`, with DMARC off, attachments delivered inline, and all default resilience settings. See [Authorization](/docs/authorization) for how to tighten the policy.
