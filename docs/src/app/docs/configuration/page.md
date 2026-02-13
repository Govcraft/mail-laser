---
title: Configuration
nextjs:
  metadata:
    title: Configuration
    description: All MailLaser environment variables, default values, and .env file support.
---

MailLaser is configured entirely through environment variables. Set them directly in your shell, pass them via Docker's `-e` flag, or place them in a `.env` file in the working directory.

---

## Required variables

These must be set for MailLaser to start. If either is missing, the application exits with an error.

| Variable | Description |
|----------|-------------|
| `MAIL_LASER_TARGET_EMAILS` | Comma-separated list of email addresses to accept. At least one address is required. Whitespace around commas is trimmed. |
| `MAIL_LASER_WEBHOOK_URL` | The URL where email payloads are forwarded via HTTP POST. |

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

### Circuit breaker settings

| Variable | Default | Description |
|----------|---------|-------------|
| `MAIL_LASER_CIRCUIT_BREAKER_THRESHOLD` | `5` | Consecutive failures before the circuit breaker opens. |
| `MAIL_LASER_CIRCUIT_BREAKER_RESET` | `60` | Seconds before an open circuit breaker transitions to half-open. |

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
- **Invalid numeric values**: Timeout, retry, and circuit breaker settings must be valid integers of their expected types.

All loaded configuration values are logged at `info` level during startup, making it straightforward to verify what settings are in effect.

---

## Example: minimal configuration

The smallest viable configuration requires only the two required variables:

```shell
MAIL_LASER_TARGET_EMAILS="inbox@myapp.com" \
MAIL_LASER_WEBHOOK_URL="https://myapp.com/email-hook" \
./mail_laser
```

This starts the SMTP server on `0.0.0.0:2525` and the health check on `0.0.0.0:8080` with all default resilience settings.
