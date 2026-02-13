---
title: SMTP server
nextjs:
  metadata:
    title: SMTP server
    description: How MailLaser's SMTP server receives emails, validates recipients, and supports STARTTLS encryption.
---

MailLaser runs a lightweight SMTP server that accepts incoming email connections, validates recipients against your configured list, and passes parsed email data to the webhook delivery system.

---

## Supported SMTP commands

MailLaser implements the essential SMTP commands needed to receive email:

| Command | Description |
|---------|-------------|
| `EHLO` / `HELO` | Initiates the SMTP session. `EHLO` advertises STARTTLS capability. |
| `STARTTLS` | Upgrades the connection to TLS encryption. |
| `MAIL FROM` | Specifies the sender's email address. |
| `RCPT TO` | Specifies the recipient. Validated against `MAIL_LASER_TARGET_EMAILS`. |
| `DATA` | Begins the email content transfer. Ends with a line containing only `.` |
| `QUIT` | Closes the connection. |

Commands are case-insensitive. `MAIL FROM`, `mail from`, and `Mail From` are all accepted.

---

## Session lifecycle

A typical SMTP session follows this sequence:

```text
Client connects
Server: 220 MailLaser SMTP Server Ready
Client: EHLO mail.example.com
Server: 250-MailLaser greets mail.example.com
Server: 250 STARTTLS
Client: MAIL FROM:<sender@example.com>
Server: 250 OK
Client: RCPT TO:<alerts@myapp.com>
Server: 250 OK
Client: DATA
Server: 354 Start mail input; end with <CRLF>.<CRLF>
Client: (email headers and body)
Client: .
Server: 250 OK: Message accepted for delivery
Client: QUIT
Server: 221 Bye
```

After the `DATA` phase completes, the state resets to `Greeted`, allowing the client to send additional emails on the same connection without reconnecting.

---

## Recipient validation

When a `RCPT TO` command arrives, MailLaser compares the recipient address against the list in `MAIL_LASER_TARGET_EMAILS`. The comparison is **case-insensitive**: `Alerts@Example.com` matches `alerts@example.com`.

- **Match**: Responds with `250 OK` and accepts the recipient.
- **No match**: Responds with `550 No such user here` and rejects the recipient.

If no valid recipient has been accepted, the `DATA` command is rejected with `503 Bad sequence of commands`.

---

## STARTTLS

MailLaser supports STARTTLS to encrypt SMTP connections. When a client sends `EHLO`, MailLaser advertises STARTTLS as a capability. The client can then issue `STARTTLS` to upgrade the connection.

MailLaser generates a **self-signed TLS certificate** at runtime using the `rcgen` crate. No certificate files need to be configured or managed. The certificate uses `localhost` as the subject alternative name.

{% callout type="warning" title="Self-signed certificates" %}
Because the certificate is self-signed, sending mail clients must either accept self-signed certificates or skip certificate verification. This is appropriate for internal deployments but not suitable for receiving mail from arbitrary internet senders that enforce strict TLS validation.
{% /callout %}

The STARTTLS flow:

1. Client sends `STARTTLS`
2. Server responds with `220 Go ahead`
3. TLS handshake occurs using `tokio-rustls`
4. After successful handshake, the session continues over the encrypted connection
5. The client must send `EHLO` or `HELO` again to re-establish the session

If a client attempts `STARTTLS` when TLS is already active, the server responds with `503 STARTTLS already active`.

---

## Email parsing

Once the `DATA` phase completes, MailLaser parses the raw email using the `mailparse` crate. The parser handles:

- **Subject**: Extracted from the `Subject:` header. Defaults to an empty string if not present.
- **Sender name**: The display name from the `From:` header (e.g., "John Doe" from `John Doe <john@example.com>`). Omitted from the payload if not present.
- **Plain text body**: Preferred from the `text/plain` MIME part. If only `text/html` is available, `html2text` generates a plain-text conversion.
- **HTML body**: The raw `text/html` MIME part, if present. Omitted from the payload if the email is plain text only.
- **Matched headers**: Any headers matching the configured `MAIL_LASER_HEADER_PREFIX` prefixes. See [Header passthrough](/docs/header-passthrough).

The parser handles both simple single-part emails and multipart/alternative messages. It processes MIME subparts recursively, extracting the first `text/plain` and `text/html` parts it finds.

{% callout title="Attachments" %}
MailLaser ignores email attachments. Only the text and HTML body parts are extracted and forwarded. Binary attachments, images, and other non-text MIME parts are silently discarded.
{% /callout %}

---

## Connection handling

Each incoming TCP connection is handled in a separate Tokio task, so concurrent connections do not block each other. The SMTP listener actor uses the `acton-reactive` framework with a `Permanent` restart policy, meaning it automatically recovers from unexpected failures.

The server uses `tokio::select!` to listen for new connections while also monitoring a cancellation token, enabling graceful shutdown when the application receives a termination signal.

---

## No authentication

MailLaser does not implement SMTP authentication (SMTP AUTH). Security relies on network-level controls:

- Bind to a specific interface using `MAIL_LASER_BIND_ADDRESS` (e.g., `127.0.0.1` for local-only access)
- Use firewall rules to restrict which hosts can connect to the SMTP port
- Place MailLaser behind a reverse proxy or VPN for production deployments exposed to the internet

The recipient validation provides application-level filtering: only emails addressed to your configured targets are processed.
