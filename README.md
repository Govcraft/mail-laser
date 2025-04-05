# MailLaser

A focused Rust-based SMTP server designed solely to receive emails at a single address and forward them to a webhook.

## Overview

MailLaser is a lightweight, efficient SMTP server built with Rust that serves a single purpose: to receive emails at a configured email address and forward them to a webhook endpoint. It's designed to be minimal, secure, and reliable.

Key features:
- Accepts emails at a single email address (configured via environment variable)
- Uses Tokio runtime for asynchronous processing
- Forwards received emails to a webhook via HTTP POST requests using only the Hyper crate
- Includes only the sender, subject, and plain text body in the forwarded request
- Excludes attachments and HTML content
- Doesn't store any emails locally

## Architecture

MailLaser is built with a modular architecture consisting of three main components:

1. **SMTP Server**: Handles incoming SMTP connections, processes SMTP commands, and extracts email content.
2. **Email Parser**: Parses raw email data to extract the subject and plain text body, filtering out HTML content and attachments.
3. **Webhook Client**: Forwards the extracted email information to the configured webhook endpoint.

The application uses the following Rust crates:
- `tokio`: For asynchronous runtime and networking
- `hyper`: For making HTTP requests to the webhook endpoint
- `serde` and `serde_json`: For serializing email data to JSON
- `anyhow` and `thiserror`: For error handling
- `log` and `env_logger`: For logging
- `dotenv`: For loading environment variables from a .env file (optional)

## Installation

### Prerequisites
- Rust and Cargo (1.70.0 or newer)

### Building from source

1. Clone the repository:
```bash
git clone https://github.com/yourusername/mail-laser.git
cd mail-laser
```

2. Build the project:
```bash
cargo build --release
```

The compiled binary will be available at `target/release/mail-laser`.

## Configuration

MailLaser is configured using environment variables:

| Variable | Description | Required | Default |
|----------|-------------|----------|---------|
| `MAIL_LASER_TARGET_EMAIL` | The email address to accept mail for | Yes | - |
| `MAIL_LASER_WEBHOOK_URL` | The webhook URL to forward emails to | Yes | - |
| `MAIL_LASER_BIND_ADDRESS` | The address to bind the SMTP server to | No | 0.0.0.0 |
| `MAIL_LASER_PORT` | The port to bind the SMTP server to | No | 2525 |

You can set these variables directly in your environment or use a `.env` file in the same directory as the binary.

Example `.env` file:
```
MAIL_LASER_TARGET_EMAIL=inbox@yourdomain.com
MAIL_LASER_WEBHOOK_URL=https://your-webhook-endpoint.com/email
MAIL_LASER_BIND_ADDRESS=0.0.0.0
MAIL_LASER_PORT=2525
```

## Usage

### Running the server

```bash
./mail-laser
```

Or with environment variables:

```bash
MAIL_LASER_TARGET_EMAIL=inbox@yourdomain.com MAIL_LASER_WEBHOOK_URL=https://your-webhook-endpoint.com/email ./mail-laser
```

### Webhook Format

When an email is received, MailLaser will make a POST request to the configured webhook URL with the following JSON payload:

```json
{
  "sender": "sender@example.com",
  "subject": "Email Subject",
  "body": "Plain text body of the email."
}
```

### DNS Configuration

To receive emails from the internet, you'll need to:

1. Configure your DNS with an MX record pointing to your server
2. Ensure port 25 is open and forwarded to your server
3. Configure your firewall to allow incoming connections on port 25

## Development

### Running tests

```bash
cargo test
```

### Building in debug mode

```bash
cargo build
```

## License

This project is licensed under the MIT License - see the LICENSE file for details.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

1. Fork the repository
2. Create your feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add some amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request
