---
title: API reference
nextjs:
  metadata:
    title: API reference
    description: Complete MailLaser EmailPayload JSON schema and supported SMTP commands.
---

This page provides the complete reference for MailLaser's webhook payload format and SMTP protocol behavior.

---

## EmailPayload JSON schema

Every webhook delivery sends a JSON object with this structure:

```json
{
  "sender": "string (required)",
  "sender_name": "string (optional)",
  "recipient": "string (required)",
  "subject": "string (required)",
  "body": "string (required)",
  "html_body": "string (optional)",
  "headers": "object (optional)"
}
```

### Field reference

| Field | Type | Required | Serialization | Description |
|-------|------|----------|---------------|-------------|
| `sender` | `String` | Yes | Always present | Email address from the SMTP `MAIL FROM` command. |
| `sender_name` | `Option<String>` | No | Omitted when `None` | Display name from the `From:` header. For example, `"John Doe"` from `John Doe <john@example.com>`. `None` when the `From:` header contains only an address or is absent. |
| `recipient` | `String` | Yes | Always present | Email address from the SMTP `RCPT TO` command that matched a configured target. |
| `subject` | `String` | Yes | Always present | Value of the `Subject:` header. Empty string if the header is missing. |
| `body` | `String` | Yes | Always present | Plain text email body. If the email has a `text/html` part, this is generated from that HTML using `html2text` (80-character width). If the email has a `text/plain` part and no HTML, this contains the raw text. Empty string if neither is found. |
| `html_body` | `Option<String>` | No | Omitted when `None` | Raw HTML content from the `text/html` MIME part. `None` when the email has no HTML content. |
| `headers` | `Option<HashMap<String, String>>` | No | Omitted when `None` | Key-value map of email headers matching the configured `MAIL_LASER_HEADER_PREFIX` prefixes. `None` when no prefixes are configured or no headers match. |

### Serialization behavior

Optional fields use `#[serde(skip_serializing_if = "Option::is_none")]`. When a field has no value, it is absent from the JSON rather than set to `null`.

**Minimal payload** (plain text email, no display name, no header passthrough):

```json
{
  "sender": "user@example.com",
  "recipient": "alerts@myapp.com",
  "subject": "Test",
  "body": "Hello world"
}
```

**Maximal payload** (all optional fields present):

```json
{
  "sender": "user@example.com",
  "sender_name": "Jane Smith",
  "recipient": "alerts@myapp.com",
  "subject": "Monthly Report",
  "body": "Please review the attached report.\n\nBest regards,\nJane",
  "html_body": "<html><body><p>Please review the attached report.</p><p>Best regards,<br>Jane</p></body></html>",
  "headers": {
    "X-Priority": "1",
    "X-Request-Id": "abc-123-def"
  }
}
```

---

## HTTP request details

| Property | Value |
|----------|-------|
| Method | `POST` |
| Content-Type | `application/json` |
| User-Agent | `MailLaser/2.0.0` |
| Body | JSON-serialized `EmailPayload` |

The `User-Agent` value is derived from `Cargo.toml` at compile time using `env!("CARGO_PKG_NAME")` and `env!("CARGO_PKG_VERSION")`.

---

## SMTP command reference

### Greeting

| Server response | Meaning |
|-----------------|---------|
| `220 MailLaser SMTP Server Ready` | Connection accepted, waiting for EHLO/HELO. |

### EHLO / HELO

| Command | Server response | Next state |
|---------|-----------------|------------|
| `EHLO domain` | `250-MailLaser greets domain` then `250 STARTTLS` | Greeted |
| `HELO domain` | `250 MailLaser` | Greeted |

`EHLO` without a domain uses `client` as the default.

### STARTTLS

| Command | State required | Server response | Effect |
|---------|---------------|-----------------|--------|
| `STARTTLS` | Greeted | `220 Go ahead` | TLS handshake begins. Client must re-send EHLO/HELO after handshake. |
| `STARTTLS` | Any other state | `503 Bad sequence of commands` | No effect. |
| `STARTTLS` | Already in TLS | `503 STARTTLS already active` | No effect. |

### MAIL FROM

| Command | State required | Server response | Effect |
|---------|---------------|-----------------|--------|
| `MAIL FROM:<user@example.com>` | Greeted | `250 OK` | Sender recorded. Transitions to MailFrom state. |
| `MAIL FROM:` (empty) | Greeted | `501 Syntax error in MAIL FROM parameters` | No state change. |

### RCPT TO

| Command | State required | Server response | Effect |
|---------|---------------|-----------------|--------|
| `RCPT TO:<match@target.com>` | MailFrom or RcptTo | `250 OK` | Recipient accepted. Transitions to RcptTo state. |
| `RCPT TO:<unknown@other.com>` | MailFrom or RcptTo | `550 No such user here` | Recipient rejected. Accepted recipient cleared. |

### DATA

| Command | State required | Server response | Effect |
|---------|---------------|-----------------|--------|
| `DATA` | RcptTo (with valid sender and recipient) | `354 Start mail input; end with <CRLF>.<CRLF>` | Transitions to Data state. |
| `DATA` | Without valid MAIL FROM/RCPT TO | `503 Bad sequence of commands` | No state change. |
| `.` (end of data) | Data | `250 OK: Message accepted for delivery` | Email parsed and forwarded. State resets to Greeted. |

### QUIT

| Command | State required | Server response | Effect |
|---------|---------------|-----------------|--------|
| `QUIT` | Any (except Data) | `221 Bye` | Connection closed. |
| `QUIT` | Data | Treated as a data line | Part of email body. |

---

## Health check endpoint

| Property | Value |
|----------|-------|
| Path | `GET /health` (any HTTP method accepted) |
| Success response | `200 OK` (empty body) |
| Other paths | `404 Not Found` (body: `Not Found`) |
| Default port | `8080` |
