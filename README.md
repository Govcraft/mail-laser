# MailLaser

Turn incoming emails into webhook calls -- no mail server required.

MailLaser is a lightweight SMTP server that receives emails and instantly forwards them as JSON payloads to any HTTP endpoint. Connect email to Slack, Discord, Zapier, or your own API with a single Docker command and two environment variables.

- **Zero complexity** -- No mailbox, no storage, no email parsing libraries in your app. MailLaser handles SMTP and delivers clean JSON.
- **Deploy in minutes** -- One Docker command or a single binary. Configure with two required environment variables and you are running.
- **Built-in resilience** -- Automatic retries with exponential backoff and a circuit breaker protect your webhook from cascading failures.
- **Lightweight** -- A statically linked Rust binary under 5 MB on a scratch Docker image. No runtime dependencies.
- **STARTTLS support** -- Encrypted SMTP connections with auto-generated certificates.

> **[Read the full documentation](https://govcraft.github.io/mail-laser)** for installation options, configuration reference, webhook payload details, and production deployment guides.

## Quick start

Start MailLaser with Docker. Replace the two `-e` values with your target email address and webhook URL:

```shell
docker run -d \
  --name mail-laser \
  -p 2525:2525 \
  -p 8080:8080 \
  -e MAIL_LASER_TARGET_EMAILS="alerts@example.com" \
  -e MAIL_LASER_WEBHOOK_URL="https://your-api.com/webhook" \
  ghcr.io/govcraft/mail-laser:latest
```

Send a test email with [swaks](https://www.jetmore.org/john/code/swaks/):

```shell
swaks \
  --to alerts@example.com \
  --from test@sender.com \
  --server localhost:2525 \
  --header "Subject: Test from swaks" \
  --body "Hello from MailLaser!"
```

Your webhook receives a JSON POST:

```json
{
  "sender": "test@sender.com",
  "recipient": "alerts@example.com",
  "subject": "Test from swaks",
  "body": "Hello from MailLaser!"
}
```

Other installation methods (pre-compiled binaries, Nix, building from source) are covered in the [Installation guide](https://govcraft.github.io/mail-laser/docs/installation).

## How it works

1. **Listen** -- Accepts SMTP connections on port 2525 (configurable).
2. **Validate** -- Checks each recipient against your configured target emails. Non-matching addresses are rejected.
3. **Parse** -- Extracts sender, recipient, subject, plain text body, and optional HTML body and headers.
4. **Forward** -- Serializes the payload to JSON and sends it to your webhook URL with automatic retries and circuit breaker protection.

A separate health check server on port 8080 responds to `GET /health` for monitoring integration. See the [Architecture](https://govcraft.github.io/mail-laser/docs/architecture) page for the full actor-based design.

## Documentation

Visit **[govcraft.github.io/mail-laser](https://govcraft.github.io/mail-laser)** for comprehensive guides:

- [Installation](https://govcraft.github.io/mail-laser/docs/installation) -- Docker, binaries, Nix, or build from source
- [Configuration](https://govcraft.github.io/mail-laser/docs/configuration) -- Environment variables, `.env` files, and defaults
- [Docker deployment](https://govcraft.github.io/mail-laser/docs/docker) -- Compose, Kubernetes, and production setup
- [Webhook delivery](https://govcraft.github.io/mail-laser/docs/webhook-delivery) -- JSON payload format and delivery behavior
- [API reference](https://govcraft.github.io/mail-laser/docs/api-reference) -- Full payload schema and SMTP command reference
- [Header passthrough](https://govcraft.github.io/mail-laser/docs/header-passthrough) -- Forward custom email headers to your webhook
- [Resilience](https://govcraft.github.io/mail-laser/docs/resilience) -- Retry backoff and circuit breaker details
- [DNS and network setup](https://govcraft.github.io/mail-laser/docs/dns-network-setup) -- MX records, firewalls, and port forwarding
- [Health check](https://govcraft.github.io/mail-laser/docs/health-check) -- Monitoring and orchestration integration
- [Testing](https://govcraft.github.io/mail-laser/docs/testing) -- swaks examples and the built-in test suite

## Development

```shell
cargo build           # Debug build
cargo test            # Run tests
cargo build --release # Optimized release build
```

See [Architecture](https://govcraft.github.io/mail-laser/docs/architecture) for the module structure and design decisions.

## Contributing

Contributions are welcome.

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/your-feature`)
3. Commit your changes
4. Push to your branch and open a pull request

## License

MIT -- see [LICENSE](LICENSE) for details.

## Sponsor

Govcraft is a one-person shop -- no corporate backing, no investors, just me building useful tools. If this project helps you, [sponsoring](https://github.com/sponsors/Govcraft) keeps the work going.

[![Sponsor on GitHub](https://img.shields.io/badge/Sponsor-%E2%9D%A4-%23db61a2?logo=GitHub)](https://github.com/sponsors/Govcraft)
