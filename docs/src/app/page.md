---
title: Getting started
---

MailLaser receives emails via SMTP and instantly forwards them as structured JSON payloads to any webhook URL you configure. No mailbox, no storage, no complexity. {% .lead %}

{% quick-links %}

{% quick-link title="Installation" icon="installation" href="/docs/installation" description="Get MailLaser running in under five minutes with Docker, pre-compiled binaries, or from source." /%}

{% quick-link title="Configuration" icon="presets" href="/docs/configuration" description="All environment variables, defaults, and .env file support explained." /%}

{% quick-link title="Webhook delivery" icon="plugins" href="/docs/webhook-delivery" description="Understand the JSON payload format, HTTP headers, and delivery behavior." /%}

{% quick-link title="API reference" icon="theming" href="/docs/api-reference" description="Complete EmailPayload JSON schema and supported SMTP commands." /%}

{% /quick-links %}

MailLaser bridges the gap between email and modern webhook-based systems. Point incoming emails at MailLaser, and it converts them into HTTP POST requests that services like Slack, Discord, Zapier, or your own API can consume directly.

---

## Why MailLaser?

Most applications that need to react to incoming emails face the same problem: setting up a full mail server or parsing raw SMTP traffic is complex and error-prone. MailLaser eliminates that complexity by doing one thing well.

- **Webhook-native**: Every email becomes a JSON POST request. Integrate with any HTTP endpoint.
- **Zero storage**: Emails are parsed and forwarded immediately. Nothing is written to disk.
- **Resilient delivery**: Built-in retry with exponential backoff and a circuit breaker protect your webhook endpoint from cascading failures.
- **Lightweight**: A single static binary under 5 MB. Runs on scratch Docker images.
- **STARTTLS support**: Accepts encrypted SMTP connections with auto-generated self-signed certificates.

---

## Quick start

The fastest way to run MailLaser is with Docker. You need two things: a list of email addresses to accept and a webhook URL to forward them to.

### Run with Docker

```shell
docker run -d \
  --name mail-laser \
  -p 2525:2525 \
  -p 8080:8080 \
  -e MAIL_LASER_TARGET_EMAILS="alerts@example.com" \
  -e MAIL_LASER_WEBHOOK_URL="https://your-api.com/webhook" \
  ghcr.io/govcraft/mail-laser:latest
```

This starts MailLaser listening for SMTP on port 2525 and serving a health check on port 8080.

### Send a test email

Use `swaks` (the Swiss Army Knife for SMTP) to verify everything works:

```shell
swaks \
  --to alerts@example.com \
  --from test@sender.com \
  --server localhost:2525 \
  --header "Subject: Test from swaks" \
  --body "Hello from MailLaser!"
```

Your webhook endpoint receives a JSON payload like this:

```json
{
  "sender": "test@sender.com",
  "recipient": "alerts@example.com",
  "subject": "Test from swaks",
  "body": "Hello from MailLaser!"
}
```

{% callout title="What about the HTML body?" %}
If the incoming email contains HTML content, the payload includes an `html_body` field with the raw HTML, and the `body` field contains a plain-text conversion. Fields that have no value are omitted from the JSON entirely.
{% /callout %}

---

## How it works

MailLaser operates as a pipeline with four stages:

1. **Listen** -- The SMTP server accepts connections on the configured port (default 2525).
2. **Validate** -- Each `RCPT TO` address is checked against your configured target emails. Non-matching recipients are rejected with a 550 response.
3. **Parse** -- The email data is parsed to extract sender, recipient, subject, plain text body, and optionally the HTML body and matching headers.
4. **Forward** -- The extracted data is serialized to JSON and sent as an HTTP POST to your webhook URL with automatic retries and circuit breaker protection.

A separate health check server runs on port 8080, responding to `GET /health` with a 200 status for monitoring integration.

---

## Next steps

- [Install MailLaser](/docs/installation) using your preferred method
- [Configure environment variables](/docs/configuration) for your deployment
- [Understand the webhook payload](/docs/webhook-delivery) your endpoint will receive
