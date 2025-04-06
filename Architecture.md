# Architecture

## Introduction

**Purpose:** MailLaser is an asynchronous SMTP server designed to receive emails for one or more specific target addresses and forward the essential content (sender, recipient, subject, plain text body, and optionally the original HTML body) as a JSON payload to a pre-configured webhook URL via HTTPS.

**Rationale:** This application provides a simple mechanism to integrate email reception into other systems or workflows that primarily operate with webhooks. It acts as a bridge, converting incoming emails into structured HTTP events, eliminating the need for complex email client libraries or full mail server setups within the receiving application.

**Use Cases:**

*   Triggering serverless functions or automated workflows based on received emails (e.g., reports, alerts).
*   Integrating legacy systems that send email notifications with modern webhook-based platforms (e.g., chat applications, issue trackers).
*   Creating simple email-to-API gateways.
*   Providing a lightweight endpoint for testing email sending functionality.

**Architecture Overview:**

The application is built using Rust and the Tokio asynchronous runtime. It consists of several key modules:

1.  **Entry Point (`main.rs`):** Initializes logging (`env_logger`), sets up a panic hook for better error reporting, starts the Tokio runtime, and calls the main library function (`mail_laser::run`). It handles the final process exit code based on the success or failure of the core logic.
2.  **Orchestration (`lib.rs`):** Loads configuration, then concurrently starts the SMTP server and a separate Health Check server using `tokio::spawn`. It uses `tokio::select!` to monitor both tasks, shutting down the application if either server encounters a fatal error.
3.  **Configuration (`config`):** Defines a `Config` struct and loads settings (target emails, webhook URL, server bind addresses/ports) from environment variables (with `.env` file support via `dotenv`).
4.  **SMTP Server (`smtp`):** Listens for TCP connections on the configured SMTP port. For each connection, it spawns a task that uses:
    *   `smtp_protocol`: A state machine implementation handling SMTP commands (HELO, MAIL FROM, RCPT TO, DATA, QUIT) and responses.
    *   `email_parser`: Parses the email's DATA section to extract the Subject header, a plain text representation of the body (using `html2text` to strip HTML), and the original HTML body if present.
    *   It validates the recipient against the configured list of `target_emails`. Upon successful reception and parsing, it creates an `EmailPayload` (containing sender, recipient, subject, text body, and optional HTML body) and invokes the `WebhookClient`.
5.  **Webhook Client (`webhook`):** Uses `hyper` and `hyper-rustls` to create an asynchronous HTTPS client. It takes the parsed `EmailPayload` (sender, recipient, subject, body), serializes it to JSON, and sends it via POST request to the configured `webhook_url`. Webhook failures are logged but do not cause the SMTP transaction to fail.
6.  **Health Check (`health`):** Runs a minimal `hyper` HTTP server on a separate port, responding with `200 OK` to requests on the `/health` path, allowing external monitoring systems to check if the service is running.

The application leverages asynchronous I/O throughout, primarily using Tokio, Hyper, and related ecosystem crates.

---

## Modules

### `src/config`

**Purpose:** Manages application configuration.

**Key Components:**

*   **`Config` struct:** Defines the structure for holding all configuration settings, including:
    *   `target_emails`: A list (`Vec<String>`) of email addresses the service will accept mail for.
    *   `webhook_url`: The URL to which incoming emails will be forwarded via POST request.
    *   `smtp_bind_address` / `smtp_port`: Network address and port for the SMTP server listener.
    *   `health_check_bind_address` / `health_check_port`: Network address and port for the health check endpoint.
*   **`from_env()` function:** Loads configuration values from environment variables (prefixed with `MAIL_LASER_`). It uses `dotenv` to optionally load from a `.env` file, provides defaults for bind addresses and ports, validates required variables (`MAIL_LASER_TARGET_EMAILS`, `MAIL_LASER_WEBHOOK_URL`), parses the comma-separated `MAIL_LASER_TARGET_EMAILS` into a `Vec<String>`, and handles potential errors (e.g., missing required variables, invalid ports, empty target email list). Logging is used to trace the configuration loading process.
*   **Tests:** Includes unit tests to verify the configuration loading logic under various conditions (defaults, missing required variables, invalid values).

*   **Environment Variables:** The `from_env()` function reads the following environment variables:
    *   `MAIL_LASER_TARGET_EMAILS` (Required): A comma-separated list of email addresses the server accepts mail for (e.g., `"user1@example.com,user2@example.com"`). Whitespace around commas is trimmed. Must contain at least one valid email address.
    *   `MAIL_LASER_WEBHOOK_URL` (Required): The URL to forward the email payload to.
    *   `MAIL_LASER_BIND_ADDRESS` (Optional): The IP address for the SMTP server to bind to. Defaults to `0.0.0.0`.
    *   `MAIL_LASER_PORT` (Optional): The port for the SMTP server. Defaults to `2525`. Must be a valid u16 port number.
    *   `MAIL_LASER_HEALTH_BIND_ADDRESS` (Optional): The IP address for the health check server to bind to. Defaults to `0.0.0.0`.
    *   `MAIL_LASER_HEALTH_PORT` (Optional): The port for the health check server. Defaults to `8080`. Must be a valid u16 port number.
    *   `RUST_LOG` (Optional, used by `env_logger`): Controls logging level (e.g., `info`, `debug`). Defaults to `info`.

**Dependencies:** `anyhow`, `serde`, `dotenv`, `log`, `std::env`.

### `src/health`

**Purpose:** Provides a simple HTTP health check endpoint.

**Key Components:**

*   **`run_health_server(config: Config)`:** An asynchronous function that starts a TCP listener based on the `health_check_bind_address` and `health_check_port` from the configuration. It runs an infinite loop accepting connections.
*   **HTTP Server:** Uses `hyper` and `tokio` to create a minimal HTTP server.
*   **`health_check_handler` / `health_check_adapter`:** Handles incoming requests. It responds with `200 OK` (empty body) for requests to the `/health` path and `404 Not Found` for all other paths.
*   **Tests:** Includes a unit test (`test_health_check_handler`) to verify the response status codes for valid and invalid paths.

**Dependencies:** `hyper`, `hyper-util`, `http-body-util`, `http-body`, `tokio`, `log`, `anyhow`, `bytes`, `crate::config`.

### `src/smtp`

**Purpose:** Implements the core SMTP server logic for receiving emails and initiating the forwarding process.

**Key Components:**

*   **`Server` struct:** Holds the application `Config` and an `Arc<WebhookClient>` for shared access across connections.
*   **`Server::run()`:** Binds a `tokio::net::TcpListener` to the configured SMTP address and port. It enters a loop, accepting incoming TCP connections. For each connection, it spawns a new asynchronous task using `tokio::spawn` to handle the connection via `handle_connection`.
*   **`handle_connection(stream, webhook_client, target_emails)`:** Manages a single client connection, potentially upgrading to TLS via `handle_starttls`.
    *   Instantiates `SmtpProtocol` to manage the state and communication logic.
    *   Sends the initial SMTP greeting.
    *   Enters a loop reading commands from the client using `protocol.read_line()`.
    *   Processes commands (`MAIL FROM`, `RCPT TO`, `DATA`, `QUIT`, etc.) using `protocol.process_command()`.
    *   Validates `RCPT TO` against the configured `target_emails` list (case-insensitive).
    *   Collects email content during the `DATA` state.
    *   Upon `DATA` completion (`.`), it uses `EmailParser::parse()` to extract the subject and body.
    *   Creates an `EmailPayload` including the specific `recipient` address that was accepted.
    *   Calls `webhook_client.forward_email()` to send the payload to the configured webhook.
    *   Handles connection closure and errors.
*   **Sub-modules:** Relies on `email_parser` for parsing raw email data and `smtp_protocol` for handling the state machine and command parsing of the SMTP protocol itself.

**Dependencies:** `anyhow`, `log`, `tokio`, `std::sync::Arc`, `crate::config`, `crate::webhook`, `self::email_parser`, `self::smtp_protocol`.

#### `src/smtp/email_parser.rs`

**Purpose:** Provides parsing of raw email data to extract the Subject header, a plain-text representation of the body (by stripping HTML), and the original HTML body.

**Key Components:**

*   **`EmailParser` struct:** Namespace for the parsing logic.
*   **`parse(raw_data: &str)` function:** Iterates line by line through the input string. It identifies the `Subject:` header and extracts its value. After the headers (first blank line), it accumulates subsequent lines as the raw body. It uses a simple heuristic to detect if the body contains HTML.
    *   If HTML is detected, it uses `html2text::from_read` to generate a plain text version (`text_body`) and stores the original accumulated lines as `html_body` (in an `Option<String>`).
    *   If HTML is not detected, the accumulated lines are used directly as `text_body`, and `html_body` is `None`.
*   **Return Value:** `Result<(String, String, Option<String>)>` representing `(subject, text_body, html_body)`.
*   **Limitations:** Does not handle complex MIME structures or different encodings beyond basic UTF-8. Relies on `html2text` for HTML-to-text conversion quality.
*   **Tests:** Cover simple emails, emails with basic HTML, and emails with links/formatting.

**Dependencies:** `anyhow`, `log`, `mailparse`, `html2text`.

#### `src/smtp/smtp_protocol.rs`

**Purpose:** Implements the state machine and command handling for the SMTP protocol, including support for STARTTLS.

**Key Components:**

*   **`SmtpState` enum:** (`Initial`, `Greeted`, `MailFrom`, `RcptTo`, `Data`) Defines the stages of an SMTP conversation.
*   **`SmtpProtocol` struct:** Manages the connection state and provides methods for reading/writing lines and processing commands. It holds buffered reader/writer halves of the underlying stream (e.g., `TcpStream` or a TLS stream).
*   **`process_command(line: &str)`:** The core state machine logic. Based on the current `state`, it parses the incoming `line`, validates the command sequence, sends the appropriate SMTP response code (e.g., `220`, `250`, `354`, `503`), updates the internal state, and returns an `SmtpCommandResult`.
    *   Handles `EHLO` by advertising server capabilities, including `STARTTLS`.
    *   Handles `STARTTLS` in the `Greeted` state by responding with `220 Go ahead` and returning `SmtpCommandResult::StartTls`, signaling the connection handler to initiate the TLS handshake. The state remains `Greeted` after this command.
*   **`SmtpCommandResult` enum:** Signals the outcome of `process_command` to the caller (e.g., `Continue`, `Quit`, `MailFrom(String)`, `RcptTo(String)`, `DataStart`, `DataLine(String)`, `DataEnd`, `StartTls`).
*   **I/O Methods:** `read_line()` and `write_line()` handle asynchronous, buffered network I/O with CRLF termination.
*   **Helper Methods:** `extract_email()` provides basic parsing for email addresses within `< >`. `get_state()` and `reset_state()` allow querying and resetting the protocol state.
*   **Tests:** Includes unit tests verifying state transitions for various commands, including STARTTLS handling in correct and incorrect states.

**Dependencies:** `anyhow`, `log`, `tokio`.

### `src/webhook`

**Purpose:** Handles forwarding the processed email data to a configured external webhook URL via HTTPS POST request.

**Key Components:**

*   **`EmailPayload` struct:** Defines the JSON structure (`sender`, `recipient`, `subject`, `body`, `html_body` (optional)) for the data sent to the webhook. Marked with `Serialize`, `Deserialize`, and `Clone`. Includes the specific recipient address the email was accepted for and potentially the original HTML.
*   **`WebhookClient` struct:** Encapsulates the HTTP client logic.
    *   Holds the application `Config` and the configured `hyper` client.
    *   Initializes an asynchronous HTTP client (`hyper_util::client::legacy::Client`) using `hyper-rustls` for HTTPS support, configured to use native system root certificates.
    *   Generates and stores a `User-Agent` string based on the crate's package name and version.
*   **`forward_email(email: EmailPayload)` function:**
    *   Serializes the `EmailPayload` into JSON.
    *   Builds an HTTP POST request using `hyper::Request::builder()`:
        *   Sets the method to POST.
        *   Sets the URI to `config.webhook_url`.
        *   Sets `Content-Type` to `application/json`.
        *   Sets the generated `User-Agent` header.
        *   Sets the JSON string as the request body (`http_body_util::Full<bytes::Bytes>`).
    *   Sends the request using the internal `client`.
    *   Logs the success or failure status code of the webhook response. Importantly, it does *not* return an error on HTTP failure to prevent disrupting the SMTP session; it only logs the issue.
*   **Tests:** Contains a `tests.rs` file (contents not examined for this document).

**Dependencies:** `anyhow`, `hyper`, `hyper-rustls`, `hyper-util`, `http-body-util`, `bytes`, `log`, `serde`, `serde_json`, `crate::config`.

### `src/lib.rs`

**Purpose:** Serves as the main library entry point, orchestrating the startup and concurrent execution of the primary server components (SMTP and Health Check).

**Key Components:**

*   **Module Declarations:** Declares `smtp`, `webhook`, `config`, and `health` as public modules.
*   **`run()` async function:**
    *   Logs the application start message (name and version).
    *   Loads the application configuration using `config::Config::from_env()`.
    *   Instantiates the `smtp::Server`.
    *   Spawns the `health::run_health_server` in a dedicated `tokio` task.
    *   Spawns the `smtp_server.run()` method in a dedicated `tokio` task.
    *   Uses `tokio::select!` to concurrently await the completion of both the health server task and the SMTP server task.
    *   If either task completes (which typically indicates an error in a long-running service), `select!` returns the `Result` from that task.
    *   Logs and propagates any errors returned by the tasks, causing the main `run()` function to terminate with an error.

**Dependencies:** `anyhow`, `log`, `tokio`, `crate::config`, `crate::health`, `crate::smtp`.

### `src/main.rs`

**Purpose:** The executable entry point for the application. Initializes the environment and runs the core application logic from the library crate.

**Key Components:**

*   **`#[tokio::main]`:** Sets up the Tokio asynchronous runtime.
*   **Logging Initialization:** Configures `env_logger` based on the `RUST_LOG` environment variable (defaulting to `info`).
*   **Panic Hook:** Sets a custom panic hook using `std::panic::set_hook` to ensure panics are logged via the `error!` macro, including payload and location information.
*   **Application Execution:** Calls `mail_laser::run().await` to start the main application logic defined in `src/lib.rs`.
*   **Error Handling & Exit:** If `mail_laser::run()` returns an `Err`, logs the error and exits the process with status code 1 using `std::process::exit(1)`.

**Dependencies:** `log`, `env_logger`, `tokio`, `mail_laser` (the library crate itself), `std::panic`, `std::process`.

---

## Build and Environment

### `Dockerfile`

**Purpose:** Defines the container build process for creating a minimal, statically linked production image.

**Key Aspects:**

*   **Multi-Stage Build:** Uses a `builder` stage based on `rust:slim` and a final stage based on `scratch`.
*   **Static Linking:** Adds and uses the `x86_64-unknown-linux-musl` target along with `musl-tools` to produce a statically linked binary.
*   **Non-Root User:** Creates and uses a non-root `builder` user during the build process for improved security.
*   **Dependency Caching:** Copies `Cargo.toml` and `Cargo.lock` first and builds dependencies separately to leverage Docker layer caching.
*   **Minimal Final Image:** Copies only the compiled binary and necessary CA certificates (for HTTPS) into the final `scratch` image.
*   **Execution:** Sets the `CMD` to run the compiled binary.

### `flake.nix`

**Purpose:** Defines reproducible development environments using Nix Flakes, `flake-parts`, and `dev-environments`.

**Key Aspects:**

*   **Inputs:** Declares dependencies on `flake-parts`, `nixpkgs`, and `dev-environments`.
*   **Environment Modules:** Imports modules from `dev-environments` for Rust, Go, Node.js, and Typst (though only Rust is explicitly enabled in the provided configuration).
*   **Rust Environment:** Enables the Rust development environment using the stable toolchain by default.
*   **Default Shell (`devShells.default`):** Creates a combined development shell that includes packages from the enabled environments plus explicitly added packages (`openssl`, `swaks`).
*   **Reproducibility:** Ensures developers have a consistent set of tools and dependencies regardless of their host system setup.

### `LICENSE`

**Purpose:** Specifies the legal terms under which the software can be used, modified, and distributed.

**Content:** Contains the standard text of the MIT License, a permissive open-source license. This allows broad usage with minimal restrictions, requiring only attribution and inclusion of the license text.