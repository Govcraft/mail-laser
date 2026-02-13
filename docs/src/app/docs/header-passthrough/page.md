---
title: Header passthrough
nextjs:
  metadata:
    title: Header passthrough
    description: Forward specific email headers to your webhook using MailLaser's prefix-based header matching.
---

MailLaser can forward selected email headers to your webhook endpoint. This is useful when emails contain custom headers that carry metadata your application needs, such as tracking IDs, priority flags, or source identifiers.

---

## How it works

Set `MAIL_LASER_HEADER_PREFIX` to a comma-separated list of header name prefixes. MailLaser scans all headers in the incoming email and forwards any header whose name starts with one of the configured prefixes.

Matching is **case-insensitive**: a prefix of `x-custom` matches headers named `X-Custom-Id`, `x-custom-source`, and `X-CUSTOM-PRIORITY`.

```shell
MAIL_LASER_HEADER_PREFIX="X-Custom,X-My-App"
```

With this configuration, an email containing these headers:

```text
X-Custom-Id: 12345
X-Custom-Source: crm
X-My-App-Priority: high
X-Unrelated: ignored
Content-Type: text/plain
```

produces this `headers` field in the webhook payload:

```json
{
  "headers": {
    "X-Custom-Id": "12345",
    "X-Custom-Source": "crm",
    "X-My-App-Priority": "high"
  }
}
```

The `X-Unrelated` and `Content-Type` headers are not forwarded because they do not match any configured prefix.

---

## Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `MAIL_LASER_HEADER_PREFIX` | *(none)* | Comma-separated header name prefixes. Whitespace around commas is trimmed. |

When `MAIL_LASER_HEADER_PREFIX` is not set or is empty, header matching is skipped entirely and the `headers` field is omitted from the webhook payload.

---

## Payload behavior

The `headers` field in the JSON payload is an object mapping header names to their decoded values:

```json
{
  "sender": "user@example.com",
  "recipient": "alerts@myapp.com",
  "subject": "Report",
  "body": "See attached.",
  "headers": {
    "X-Request-Id": "abc-123",
    "X-Source": "monitoring"
  }
}
```

If no headers match the configured prefixes (or no prefixes are configured), the `headers` field is omitted from the payload entirely -- it is not present as an empty object.

---

## Use cases

### Correlation IDs

Forward tracking or request IDs from upstream systems so your webhook handler can correlate the email with other events:

```shell
MAIL_LASER_HEADER_PREFIX="X-Request-Id,X-Correlation"
```

### Application routing

Use custom headers to route emails to different processing pipelines:

```shell
MAIL_LASER_HEADER_PREFIX="X-Route,X-Priority"
```

Your webhook handler can inspect the `headers` object to determine how to process each email.

### Preserving sender metadata

Some email systems add custom headers with sender metadata that may not appear in standard fields:

```shell
MAIL_LASER_HEADER_PREFIX="X-Mailer,X-Originating"
```

{% callout title="Standard headers" %}
You can match standard headers too. A prefix of `from` would match `From`, and a prefix of `reply` would match `Reply-To`. However, the `sender` field in the payload already provides the `MAIL FROM` address, and `sender_name` provides the `From:` display name, so matching standard headers is rarely necessary.
{% /callout %}
