---
title: Testing
nextjs:
  metadata:
    title: Testing
    description: Test MailLaser with swaks, write integration tests, and run the built-in test suite.
---

MailLaser can be tested at multiple levels: manual SMTP testing with `swaks`, automated integration testing, and the built-in Rust test suite.

---

## Testing with swaks

[swaks](https://www.jetmore.org/john/code/swaks/) (Swiss Army Knife for SMTP) is the quickest way to send test emails to MailLaser.

### Install swaks

```shell
# macOS
brew install swaks

# Debian/Ubuntu
sudo apt install swaks

# Nix (included in MailLaser's flake.nix)
nix develop
```

### Send a basic test email

```shell
swaks \
  --to alerts@example.com \
  --from sender@test.com \
  --server localhost:2525 \
  --header "Subject: Test email" \
  --body "This is a test."
```

### Send with custom headers

Test header passthrough by including headers that match your `MAIL_LASER_HEADER_PREFIX`:

```shell
swaks \
  --to alerts@example.com \
  --from sender@test.com \
  --server localhost:2525 \
  --header "Subject: Header test" \
  --header "X-Custom-Id: 12345" \
  --header "X-Custom-Source: testing" \
  --body "Testing header passthrough."
```

### Send with HTML content

```shell
swaks \
  --to alerts@example.com \
  --from sender@test.com \
  --server localhost:2525 \
  --header "Subject: HTML test" \
  --header "Content-Type: text/html" \
  --body "<html><body><h1>Hello</h1><p>This is <strong>HTML</strong> content.</p></body></html>"
```

### Test STARTTLS

```shell
swaks \
  --to alerts@example.com \
  --from sender@test.com \
  --server localhost:2525 \
  --tls-on-connect \
  --tls-verify \
  --header "Subject: TLS test" \
  --body "Sent over TLS."
```

{% callout type="warning" title="Certificate verification" %}
Because MailLaser uses a self-signed certificate, `swaks` may reject the TLS handshake with strict verification. Use `--tls-verify` to see the verification result, or remove it to skip verification. In testing environments, the connection will still be encrypted regardless of certificate validation.
{% /callout %}

### Test rejected recipients

```shell
swaks \
  --to unknown@example.com \
  --from sender@test.com \
  --server localhost:2525 \
  --header "Subject: Should be rejected" \
  --body "This should not arrive."
```

If `unknown@example.com` is not in `MAIL_LASER_TARGET_EMAILS`, MailLaser responds with `550 No such user here` and the email is not forwarded.

---

## Webhook testing endpoints

For testing without a real webhook, use a request inspection service:

### webhook.site

1. Go to [webhook.site](https://webhook.site/) and copy the unique URL
2. Set `MAIL_LASER_WEBHOOK_URL` to that URL
3. Send test emails and inspect the received payloads in your browser

### Local webhook with netcat

For quick local testing, listen for the webhook POST:

```shell
# Terminal 1: Listen for webhook
nc -l -p 9000

# Terminal 2: Start MailLaser
MAIL_LASER_TARGET_EMAILS="test@example.com" \
MAIL_LASER_WEBHOOK_URL="http://localhost:9000" \
./mail_laser

# Terminal 3: Send test email
swaks --to test@example.com --from sender@test.com --server localhost:2525
```

{% callout title="HTTP vs HTTPS" %}
In release builds, MailLaser enforces HTTPS-only for webhook URLs. For local testing with HTTP endpoints, use a debug build (`cargo run` without `--release`).
{% /callout %}

---

## Built-in test suite

MailLaser includes comprehensive unit tests across all modules.

### Run all tests

```shell
cargo test
```

### Run tests for a specific module

```shell
# SMTP protocol tests
cargo test --lib smtp::smtp_protocol::tests

# Email parser tests
cargo test --lib smtp::email_parser::tests

# Config tests
cargo test --lib config::tests

# Health check tests
cargo test --lib health::tests

# Webhook tests
cargo test --lib webhook::tests
```

### Test coverage

The test suite covers:

- **SMTP protocol**: State machine transitions for all commands, case insensitivity, STARTTLS handling in correct and incorrect states, QUIT in every state, command sequence validation, email address extraction.
- **Email parser**: Simple emails, HTML content, multipart/alternative, missing subjects, empty bodies, sender name extraction, header prefix matching (case-insensitive), malformed input handling.
- **Configuration**: Default values, required variable validation, empty target emails, invalid port numbers.
- **Health check**: Correct path returns 200, incorrect paths return 404, multiple HTTP methods.

---

## Integration testing approach

For end-to-end testing, combine MailLaser with a mock webhook server:

1. Start MailLaser with test configuration
2. Start a simple HTTP server that records incoming requests
3. Send emails via `swaks` or a programmatic SMTP client
4. Assert that the mock server received the expected JSON payloads

The `Cargo.toml` includes `testcontainers` as a dev dependency, enabling Docker-based integration tests when needed.

### Example test flow

```shell
# 1. Start a mock webhook (Python)
python3 -c "
from http.server import HTTPServer, BaseHTTPRequestHandler
import json

class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers['Content-Length'])
        body = json.loads(self.rfile.read(length))
        print(json.dumps(body, indent=2))
        self.send_response(200)
        self.end_headers()

HTTPServer(('', 9000), Handler).serve_forever()
" &

# 2. Start MailLaser
MAIL_LASER_TARGET_EMAILS="test@example.com" \
MAIL_LASER_WEBHOOK_URL="http://localhost:9000" \
cargo run &

# 3. Send test email
sleep 2
swaks --to test@example.com --from sender@test.com \
  --server localhost:2525 \
  --header "Subject: Integration test" \
  --body "End-to-end test"
```

The mock webhook prints the received JSON payload, which you can verify contains the expected fields.
