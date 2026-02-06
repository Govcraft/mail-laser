# MailLaser: Simple Email-to-Webhook Forwarder

MailLaser is a lightweight, dedicated server application designed for one specific task: receiving emails sent to one or more designated email addresses and instantly forwarding the essential content (sender, recipient, subject, plain text body, and optionally the original HTML body) to a webhook URL you configure.

Think of it as a bridge: it converts incoming emails into structured HTTP POST requests (JSON format), making it easy to integrate email reception with modern web services, automation workflows, or custom applications without needing complex email handling libraries or a full mail server setup.

## Why Use MailLaser?

*   **Integrate Email with Webhooks:** Easily connect email events to systems like Slack, Discord, Zapier, IFTTT, custom APIs, serverless functions, or issue trackers.
*   **Automate Workflows:** Trigger actions based on email content (e.g., process reports, alerts, or notifications).
*   **Simplify Email Reception:** Avoid the complexity of managing mailboxes or parsing diverse email formats directly in your application.
*   **Lightweight & Focused:** Runs as a minimal, efficient background service.
*   **Multiple Deployment Options:** Run via Docker, pre-compiled binaries, or build from source.
*   **Testing:** Provides a simple endpoint for testing email sending functionality in other applications.

## Key Features

*   **Multiple Address Support:** Listens for email directed to one or more target email addresses configured via a comma-separated list.
*   **Webhook Forwarding:** Sends a POST request with a JSON payload to your configured webhook URL for each valid email received.
*   **Content Extraction:** Parses and forwards the `sender`, `recipient`, `subject`, a plain text `body` (HTML is stripped), and optionally the original `html_body` if the email contained HTML. Attachments are ignored.
*   **No Local Storage:** Emails are processed and forwarded immediately; nothing is stored on the MailLaser server itself.
*   **Health Check Endpoint:** Includes a simple `/health` HTTP endpoint for monitoring service status.
*   **Configurable Logging:** Control log verbosity using the `RUST_LOG` environment variable.
*   **Configurable:** Uses environment variables for easy setup.
*   **Cross-Platform:** Pre-compiled binaries for Linux, macOS (x86_64, Apple Silicon), and Windows.
*   **Dockerized:** Official images available on GitHub Packages for easy deployment.

## How It Works (High-Level)

1.  **Listen:** MailLaser listens for incoming email connections (SMTP protocol) on a configured network port (default: 2525).
2.  **Receive & Validate:** When an email arrives, it checks if the recipient matches one of the configured `MAIL_LASER_TARGET_EMAILS`.
3.  **Parse:** If the recipient matches, it uses the `mailparse` crate to parse the email structure (handling multipart messages), extracts the sender address, the specific recipient address, subject line, the plain text body (preferring `text/plain` part, otherwise generating from `text/html` via `html2text`), and the HTML body (if a `text/html` part exists).
4.  **Forward:** It packages this information (`sender`, `recipient`, `subject`, `body` (plain/generated text), `html_body` (optional raw HTML)) into a JSON object and sends it via an HTTPS POST request to the configured `MAIL_LASER_WEBHOOK_URL`.
5.  **Monitor:** A separate, simple HTTP server runs (default port: 8080) providing a `/health` endpoint that returns `200 OK` if MailLaser is running.

## Getting Started

There are several ways to run MailLaser. Choose the one that best suits your needs.

### Prerequisites (All Methods)

*   **Webhook Endpoint:** You need a URL endpoint ready to receive POST requests with a JSON payload.

### Method 1: Running with Docker (Recommended)

This is the easiest way for automated deployments or if you already use Docker.

*   **Requirement:** Docker installed ([docker.com](https://www.docker.com/get-started)).

1.  **Pull the Image:**
    ```bash
    docker pull ghcr.io/govcraft/mail-laser:latest
    # Or pull a specific version tag if needed
    # docker pull ghcr.io/govcraft/mail-laser:vX.Y.Z
    ```

2.  **Run the Container:**
    Provide environment variables and map ports.

    ```bash
    docker run -d \
      --name mail-laser \
      -p 2525:2525 \
      -p 8080:8080 \
      -e MAIL_LASER_TARGET_EMAILS="target1@example.com,target2@example.com" \
      -e MAIL_LASER_WEBHOOK_URL="https://your-webhook-url.com/path" \
      -e MAIL_LASER_PORT="2525" \
      -e MAIL_LASER_HEALTH_PORT="8080" \
      -e RUST_LOG="info" \
      --restart unless-stopped \
      ghcr.io/govcraft/mail-laser:latest
    ```
    *(See Configuration section below for variable details)*

### Method 2: Running with Pre-compiled Binaries

This is a simple way to run MailLaser without needing Docker or the Rust toolchain.

*   **Requirement:** Ability to download and run executables on your system.

1.  **Download:** Go to the [**GitHub Releases page**](https://github.com/Govcraft/mail-laser/releases). Find the latest release and download the binary matching your operating system and architecture (e.g., `mail_laser-linux-x86_64`, `mail_laser-macos-aarch64`, `mail_laser-windows-x86_64.exe`).
2.  **(Linux/macOS Only) Make Executable:** Open your terminal, navigate to the download location, and run:
    ```bash
    chmod +x ./mail_laser-<your_platform_suffix>
    ```
3.  **Run:** Open your terminal, navigate to the download location, set the required environment variables (see Configuration), and execute the binary:

    *   **Linux/macOS:**
        ```bash
        MAIL_LASER_TARGET_EMAILS="target1@example.com,target2@example.com" \
        MAIL_LASER_WEBHOOK_URL="https://your-webhook-url.com/path" \
        ./mail_laser-<your_platform_suffix>
        ```
    *   **Windows (Command Prompt):**
        ```cmd
        set MAIL_LASER_TARGET_EMAILS=target1@example.com,target2@example.com
        set MAIL_LASER_WEBHOOK_URL=https://your-webhook-url.com/path
        .\mail_laser-windows-x86_64.exe
        ```
    *   **Windows (PowerShell):**
        ```powershell
        $env:MAIL_LASER_TARGET_EMAILS = "target1@example.com,target2@example.com"
        $env:MAIL_LASER_WEBHOOK_URL = "https://your-webhook-url.com/path"
        .\mail_laser-windows-x86_64.exe
        ```
    *(Alternatively, use a `.env` file as described in Configuration)*

### Method 3: Building from Source (Alternative)

Use this method if you want to modify the code or prefer building it yourself.

*   **Requirement:** Rust toolchain (Rust and Cargo 1.70.0+) installed ([rustup.rs](https://rustup.rs/)).

1.  **Clone the Repository:**
    ```bash
    git clone https://github.com/Govcraft/mail-laser.git
    cd mail-laser
    ```
2.  **Build the Application:**
    ```bash
    cargo build --release
    ```
    The executable binary will be located at `target/release/mail-laser` (or `mail-laser.exe` on Windows).
3.  **Run:** Navigate to the project directory, set environment variables, and run the compiled binary from the `target/release` directory:
    ```bash
    MAIL_LASER_TARGET_EMAILS="target1@example.com,target2@example.com" \
    MAIL_LASER_WEBHOOK_URL="https://your-webhook-url.com/path" \
    ./target/release/mail-laser
    ```
    *(Alternatively, use a `.env` file)*

### Configuration

MailLaser is configured entirely through environment variables, regardless of the running method. You can set these directly in your shell, pass them via Docker (`-e` or `--env-file`), or place them in a `.env` file in the same directory where you run the binary.

| Variable                       | Description                                       | Required | Default   |
| :----------------------------- | :------------------------------------------------ | :------- | :-------- |
| `MAIL_LASER_TARGET_EMAILS`     | Comma-separated list of email addresses to accept. | **Yes**  | -         |
| `MAIL_LASER_WEBHOOK_URL`       | The URL to forward the email payload to.          | **Yes**  | -         |
| `MAIL_LASER_BIND_ADDRESS`      | IP address for the server to listen on.           | No       | `0.0.0.0` |
| `MAIL_LASER_PORT`              | Port for the SMTP server.                         | No       | `2525`    |
| `MAIL_LASER_HEALTH_BIND_ADDRESS` | IP address for the Health Check server.           | No       | `0.0.0.0` |
| `MAIL_LASER_HEALTH_PORT`       | Port for the Health Check server.                 | No       | `8080`    |
| `MAIL_LASER_HEADER_PREFIX`     | Comma-separated header name prefixes to forward.  | No       | *(none)*  |
| `RUST_LOG`                     | Controls logging level (see Logging section).     | No       | `info`    |

**Note on Docker:** When running in Docker, `MAIL_LASER_BIND_ADDRESS` and `MAIL_LASER_HEALTH_BIND_ADDRESS` should generally be left as `0.0.0.0` to listen on all interfaces *within* the container. Port mapping (`-p`) handles exposing the service *outside* the container.

**Example `.env` file:**

```dotenv
MAIL_LASER_TARGET_EMAILS=alerts@mydomain.com,support@mydomain.com
MAIL_LASER_WEBHOOK_URL=https://hooks.example.com/services/T000/B001/XXX
# MAIL_LASER_PORT=2525 # Optional
# MAIL_LASER_HEALTH_PORT=8080 # Optional
# MAIL_LASER_HEADER_PREFIX=X-Custom,X-My-App # Optional, forward matching headers
RUST_LOG=debug # Optional, set desired log level
```

### Logging

MailLaser uses the `env_logger` crate for logging. You can control the verbosity and scope of the logs using the `RUST_LOG` environment variable.

*   **Default Level:** `info` (shows informational messages, warnings, and errors).
*   **Set Log Level:** Set the `RUST_LOG` variable when running the binary or pass it via the `-e` flag in `docker run`.

**Common `RUST_LOG` values:**

*   `error`: Show only critical errors.
*   `warn`: Show errors and warnings.
*   `info`: Show informational messages, warnings, and errors (Default).
*   `debug`: Show detailed debugging information.
*   `trace`: Show very verbose tracing information.

**Module-Specific Logging:** You can also enable specific levels for MailLaser's internal modules (e.g., `smtp`, `webhook`):

*   `RUST_LOG=mail_laser=debug`: Show debug messages only from MailLaser code.
*   `RUST_LOG=info,mail_laser::smtp=debug`: Show info level globally, but debug level for the SMTP module.

Refer to the [`env_logger` documentation](https://docs.rs/env_logger/latest/env_logger/) for more advanced filtering options. Logs are written to standard output (stdout), which can be viewed using `docker logs mail-laser` when running in Docker, or directly in your terminal otherwise.

## Webhook Integration Details

When MailLaser successfully receives and parses an email for one of the configured target addresses, it will send an HTTPS POST request to your `MAIL_LASER_WEBHOOK_URL`.

*   **Method:** `POST`
*   **Content-Type:** `application/json`
*   **User-Agent:** `MailLaser/<version>` (e.g., `MailLaser/0.1.0`)
*   **Body (JSON Payload):**

```json
{
  "sender": "sender@example.com",
  "recipient": "target1@example.com",
  "subject": "Example Email Subject",
  "body": "This is the plain text body content of the email.\\nLines are preserved, HTML tags are removed.",
  "html_body": "<html><body><p>This is the <b>original</b> HTML content.</p></body></html>",
  "headers": {
    "X-Custom-Id": "12345",
    "X-Custom-Source": "crm"
  }
}
```
*(Note: The `html_body` field will only be present if the incoming email contained HTML content. The `headers` field will only be present if `MAIL_LASER_HEADER_PREFIX` is configured and matching headers are found in the email.)*

**Note:** MailLaser logs the status code of the webhook response (at `info` level or higher) but considers its job done once the request is sent. A failure response from your webhook (e.g., 4xx or 5xx) will be logged by MailLaser but will *not* cause the original email transaction to fail.

## Receiving Email from the Internet (DNS Setup)

To receive emails from external sources (not just locally), you typically need to:

1.  **Configure DNS:** Set up an `MX` (Mail Exchanger) record for your domain (or subdomain) that points to the public IP address of the server where MailLaser is running.
2.  **Firewall Rules:** Ensure the port MailLaser is listening on for SMTP (default `2525`, or the port you mapped in Docker) is open for incoming TCP connections on your server's firewall and any network firewalls. You might need port forwarding if the server is behind NAT.
3.  **Server Accessibility:** The server hosting MailLaser must have a stable public IP address accessible from the internet on the configured port.

*Disclaimer: Properly configuring DNS and firewalls for public email reception can be complex and depends heavily on your hosting provider and network setup. Consult relevant documentation for your specific environment.*

## Development

*   **Build (Debug):** `cargo build`
*   **Run Tests:** `cargo test`
*   **Architecture:** For a detailed look at the internal components and design, see [Architecture.md](Architecture.md).

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

1.  Fork the repository.
2.  Create your feature branch (`git checkout -b feature/your-feature`).
3.  Commit your changes (`git commit -m 'Add some feature'`).
4.  Push to the branch (`git push origin feature/your-feature`).
5.  Open a Pull Request.
## Sponsor

Govcraft is a one-person shop—no corporate backing, no investors, just me building useful tools. If this project helps you, [sponsoring](https://github.com/sponsors/Govcraft) keeps the work going.

[![Sponsor on GitHub](https://img.shields.io/badge/Sponsor-%E2%9D%A4-%23db61a2?logo=GitHub)](https://github.com/sponsors/Govcraft)
