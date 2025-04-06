//! Implements the state machine and command handling logic for the SMTP protocol.
//!
//! This module defines the states of an SMTP conversation (`SmtpState`),
//! manages reading commands and writing responses over a `TcpStream`,
//! and parses basic SMTP commands, transitioning the state accordingly.

use anyhow::Result;
use log::debug;
// Keep only used IO traits/types
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
// Remove unused TcpStream import

/// Represents the possible states during an SMTP session.
///
/// The protocol handler transitions between these states based on the commands received.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum SmtpState {
    /// Initial state immediately after connection, before any greeting.
    Initial,
    /// State after the server has sent the initial greeting (220). Client should send HELO/EHLO.
    Greeted,
    /// State after a valid `MAIL FROM` command has been received. Client should send RCPT TO.
    MailFrom,
    /// State after at least one valid `RCPT TO` command has been received. Client can send more RCPT TO or DATA.
    RcptTo,
    /// State after a `DATA` command has been received and acknowledged (354). Client sends email content.
    Data,
}

/// Manages the state and I/O for a single SMTP client connection.
///
/// Encapsulates buffered reading and writing on the underlying `TcpStream`
/// and tracks the current `SmtpState` of the conversation.
///
/// It's generic over the Reader (`R`) and Writer (`W`) types to allow
/// for testing with mocks.
pub struct SmtpProtocol<R, W>
where
    R: AsyncBufReadExt + Unpin, // Reader must support buffered async reading
    W: AsyncWriteExt + Unpin,   // Writer must support async writing
{
    reader: R, // Use the generic reader type
    writer: W, // Use the generic writer type
    state: SmtpState,
}

// Implementation block now needs the generic parameters and bounds.
impl<R, W> SmtpProtocol<R, W>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    /// Creates a new `SmtpProtocol` handler using the provided reader and writer.
    ///
    /// Initializes the state to `SmtpState::Initial`.
    /// The reader and writer should typically be buffered for efficiency.
    pub fn new(reader: R, writer: W) -> Self {
        SmtpProtocol {
            reader, // Store the provided reader
            writer, // Store the provided writer
            state: SmtpState::Initial, // Start in the initial state.
        }
    }

    /// Sends the initial SMTP greeting (220) to the client.
    ///
    /// This should be called immediately after establishing a connection.
    /// Transitions the state implicitly (caller should expect `Greeted` state next).
    pub async fn send_greeting(&mut self) -> Result<()> {
        self.write_line("220 MailLaser SMTP Server Ready").await // Informative greeting.
    }

    /// Processes a single command line received from the client.
    ///
    /// Parses the command based on the current `SmtpState`, sends the appropriate
    /// response code, updates the internal state, and returns an `SmtpCommandResult`
    /// indicating the outcome or necessary follow-up action.
    ///
    /// # Arguments
    ///
    /// * `line` - The command line string received from the client (excluding CRLF).
    ///
    /// # Returns
    ///
    /// A `Result` containing an `SmtpCommandResult` on success, or an error if
    /// writing the response fails.
    pub async fn process_command(&mut self, line: &str) -> Result<SmtpCommandResult> {
        // Log the command being processed and the state *before* processing.
        debug!("SMTP({:?}): Processing command: {:?}", self.state, line);

        match self.state {
            SmtpState::Initial => {
                // Expect HELO or EHLO after connection.
                let upper_line = line.to_uppercase(); // Avoid repeated conversions
                if upper_line.starts_with("HELO") {
                    // Respond to HELO
                    self.write_line("250 MailLaser").await?;
                    self.state = SmtpState::Greeted;
                    Ok(SmtpCommandResult::Continue)
                } else if upper_line.starts_with("EHLO") {
                    // Respond to EHLO, advertising STARTTLS
                    // Extract the domain provided by the client (optional, but good practice)
                    let domain = line.split_whitespace().nth(1).unwrap_or("client");
                    self.write_line(&format!("250-MailLaser greets {}", domain)).await?;
                    self.write_line("250 STARTTLS").await?; // Advertise STARTTLS capability
                    self.state = SmtpState::Greeted;
                    Ok(SmtpCommandResult::Continue)
                } else if line.to_uppercase().starts_with("QUIT") {
                    self.write_line("221 Bye").await?;
                    Ok(SmtpCommandResult::Quit)
                } else {
                    // Command out of sequence or unrecognized.
                    self.write_line("500 Command not recognized or out of sequence").await?;
                    Ok(SmtpCommandResult::Continue)
                }
            },
            SmtpState::Greeted => {
                // Expect MAIL FROM or STARTTLS after greeting.
                let upper_line = line.to_uppercase(); // Avoid repeated conversions
                if upper_line.starts_with("MAIL FROM:") {
                    if let Some(email) = self.extract_email(line) {
                        self.write_line("250 OK").await?;
                        self.state = SmtpState::MailFrom;
                        Ok(SmtpCommandResult::MailFrom(email))
                    } else {
                        self.write_line("501 Syntax error in MAIL FROM parameters").await?;
                        Ok(SmtpCommandResult::Continue)
                    }
                } else if upper_line.starts_with("STARTTLS") {
                    // Handle STARTTLS command
                    self.write_line("220 Go ahead").await?;
                    // State remains Greeted; the caller handles the TLS upgrade.
                    Ok(SmtpCommandResult::StartTls)
                } else if upper_line.starts_with("QUIT") {
                    self.write_line("221 Bye").await?;
                    Ok(SmtpCommandResult::Quit)
                } else {
                    self.write_line("503 Bad sequence of commands (expected MAIL FROM or STARTTLS)").await?;
                    Ok(SmtpCommandResult::Continue)
                }
            },
            SmtpState::MailFrom => {
                // Expect RCPT TO after MAIL FROM.
                if line.to_uppercase().starts_with("RCPT TO:") {
                    if let Some(email) = self.extract_email(line) {
                        // Response (250 or 550) is handled by the caller based on validation.
                        self.state = SmtpState::RcptTo;
                        Ok(SmtpCommandResult::RcptTo(email))
                    } else {
                        self.write_line("501 Syntax error in RCPT TO parameters").await?;
                        Ok(SmtpCommandResult::Continue)
                    }
                } else if line.to_uppercase().starts_with("QUIT") {
                    self.write_line("221 Bye").await?;
                    Ok(SmtpCommandResult::Quit)
                } else {
                    self.write_line("503 Bad sequence of commands (expected RCPT TO)").await?;
                    Ok(SmtpCommandResult::Continue)
                }
            },
            SmtpState::RcptTo => {
                // Expect DATA or another RCPT TO after RCPT TO.
                if line.to_uppercase().starts_with("DATA") {
                    self.write_line("354 Start mail input; end with <CRLF>.<CRLF>").await?;
                    self.state = SmtpState::Data;
                    Ok(SmtpCommandResult::DataStart)
                } else if line.to_uppercase().starts_with("RCPT TO:") {
                     // Allow multiple recipients.
                     if let Some(email) = self.extract_email(line) {
                        // Response handled by caller. State remains RcptTo.
                        Ok(SmtpCommandResult::RcptTo(email))
                    } else {
                        self.write_line("501 Syntax error in RCPT TO parameters").await?;
                        Ok(SmtpCommandResult::Continue)
                    }
                } else if line.to_uppercase().starts_with("QUIT") {
                    self.write_line("221 Bye").await?;
                    Ok(SmtpCommandResult::Quit)
                } else {
                    self.write_line("503 Bad sequence of commands (expected DATA or RCPT TO)").await?;
                    Ok(SmtpCommandResult::Continue)
                }
            },
            SmtpState::Data => {
                // Expect email content lines or the end-of-data marker ".".
                if line == "." {
                    self.write_line("250 OK: Message accepted for delivery").await?;
                    self.state = SmtpState::Greeted; // Reset state for next potential email.
                    Ok(SmtpCommandResult::DataEnd)
                } else {
                    // Pass the line content up to the caller.
                    // Handle potential leading "." (dot-stuffing) if needed, though not implemented here.
                    Ok(SmtpCommandResult::DataLine(line.to_string()))
                }
            }
        }
    }

    /// Reads a single line (terminated by CRLF) from the client stream.
    ///
    /// Returns an empty string if the connection is closed (EOF).
    /// Trims the trailing CRLF from the returned string.
    pub async fn read_line(&mut self) -> Result<String> {
        let mut buffer = String::new();
        // Read until \n, including the delimiter.
        let bytes_read = self.reader.read_line(&mut buffer).await?;

        if bytes_read == 0 {
            // Connection closed by peer.
            Ok(String::new())
        } else {
            // Trim trailing CRLF or LF before returning.
            // Use array pattern suggested by clippy for conciseness
            let line = buffer.trim_end_matches(['\r', '\n']).to_string();
            debug!("SMTP Read: {}", line);
            Ok(line)
        }
    }

    /// Writes a single line (appending CRLF) to the client stream.
    ///
    /// Flushes the write buffer to ensure the line is sent immediately.
    pub async fn write_line(&mut self, line: &str) -> Result<()> {
        debug!("SMTP Write: {}", line);
        self.writer.write_all(format!("{}\r\n", line).as_bytes()).await?;
        self.writer.flush().await?; // Ensure data is sent over the network.
        Ok(())
    }

    /// Extracts an email address enclosed in angle brackets (`< >`) from a command line.
    ///
    /// Performs a simple string search. Returns `None` if brackets are not found
    /// or are malformed.
    fn extract_email(&self, line: &str) -> Option<String> {
        // Find the positions of the angle brackets.
        let start = line.find('<');
        let end = line.find('>');

        match (start, end) {
            (Some(s), Some(e)) if s < e => {
                // Extract the substring between the brackets.
                Some(line[s + 1..e].to_string())
            }
            _ => None, // Brackets not found or in wrong order.
        }
    }

    /// Returns the current `SmtpState` of the protocol handler.
    pub fn get_state(&self) -> SmtpState {
        self.state
    }

    /// Resets the protocol state back to `SmtpState::Greeted`.
    ///
    /// Typically used after a complete email transaction (MAIL FROM -> RCPT TO -> DATA -> .)
    /// or after encountering an error that requires resetting the transaction state.
    /// Note: This is currently handled implicitly by `process_command` after `DataEnd`.
    #[allow(dead_code)] // Kept for potential future use or explicit resets.
    pub fn reset_state(&mut self) {
        debug!("Resetting SMTP state to Greeted");
        self.state = SmtpState::Greeted;
    }
}

/// Represents the outcome of processing a single SMTP command line.
///
/// This enum signals to the connection handler what action resulted from
/// processing the command and provides any necessary extracted data (like email addresses).
#[derive(Debug)]
pub enum SmtpCommandResult {
    /// Command processed successfully, continue reading next command.
    Continue,
    /// QUIT command received, connection should be closed.
    Quit,
    /// MAIL FROM command processed, contains the sender's email address.
    MailFrom(String),
    /// RCPT TO command processed, contains the recipient's email address.
    RcptTo(String),
    /// DATA command received, client will start sending email content.
    DataStart,
    /// A line of email content received during the DATA state.
    DataLine(String),
    /// End-of-data marker (`.`) received, email content finished.
    DataEnd,
    /// STARTTLS command received, server should initiate TLS handshake.
    StartTls,
}
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{self, BufReader, BufWriter}; // Import necessary IO components

    // Helper to create SmtpProtocol with non-functional IO (Empty reader, Sink writer) for state testing.
    // Explicitly type the reader/writer to satisfy the generic bounds.
    fn create_test_protocol() -> SmtpProtocol<BufReader<io::Empty>, BufWriter<io::Sink>> {
        let reader = BufReader::new(io::empty());
        let writer = BufWriter::new(io::sink());

        // Now calling the generic `new` function
        SmtpProtocol::new(reader, writer)
    }

    // Test existing HELO behavior for baseline
    #[tokio::test]
    async fn test_initial_helo_sets_greeted() {
        let mut protocol = create_test_protocol();
        assert_eq!(protocol.get_state(), SmtpState::Initial);
        // We assume write_line succeeds internally for state tests
        let result = protocol.process_command("HELO example.com").await.unwrap();
        assert!(matches!(result, SmtpCommandResult::Continue));
        assert_eq!(protocol.get_state(), SmtpState::Greeted);
    }

    // Test existing EHLO behavior for baseline
     #[tokio::test]
    async fn test_initial_ehlo_sets_greeted() {
        let mut protocol = create_test_protocol();
        assert_eq!(protocol.get_state(), SmtpState::Initial);
        // The actual response lines for EHLO will be modified later to include STARTTLS
        let result = protocol.process_command("EHLO example.com").await.unwrap();
        assert!(matches!(result, SmtpCommandResult::Continue));
        assert_eq!(protocol.get_state(), SmtpState::Greeted);
    }

    // Test STARTTLS command in the correct state (Greeted)
    #[tokio::test]
    async fn test_greeted_starttls_accepted() {
        let mut protocol = create_test_protocol();
        // Manually set the state to Greeted for this test scenario
        protocol.state = SmtpState::Greeted;
        assert_eq!(protocol.get_state(), SmtpState::Greeted);

        let result = protocol.process_command("STARTTLS").await.unwrap();

        // Expect the StartTls command result
        assert!(matches!(result, SmtpCommandResult::StartTls), "Expected StartTls result, got {:?}", result);
        // The state should remain Greeted, as the handshake happens *after* this command response.
        // The connection handler will take over for the TLS part.
        assert_eq!(protocol.get_state(), SmtpState::Greeted, "State should remain Greeted after STARTTLS command");
    }

    // Test STARTTLS command in an incorrect state (e.g., MailFrom)
    #[tokio::test]
    async fn test_mailfrom_starttls_rejected() {
        let mut protocol = create_test_protocol();
        protocol.state = SmtpState::MailFrom; // Set state manually
        assert_eq!(protocol.get_state(), SmtpState::MailFrom);

        let result = protocol.process_command("STARTTLS").await.unwrap();

        // Expect a rejection (Continue means an error response was sent, loop continues)
        assert!(matches!(result, SmtpCommandResult::Continue), "Expected Continue result for rejected STARTTLS, got {:?}", result);
        // State should not change due to the invalid command sequence
        assert_eq!(protocol.get_state(), SmtpState::MailFrom, "State should remain MailFrom after rejected STARTTLS");
    }

     // Test STARTTLS command in another incorrect state (e.g., RcptTo)
    #[tokio::test]
    async fn test_rcptto_starttls_rejected() {
        let mut protocol = create_test_protocol();
        protocol.state = SmtpState::RcptTo; // Set state manually
        assert_eq!(protocol.get_state(), SmtpState::RcptTo);

        let result = protocol.process_command("STARTTLS").await.unwrap();

        assert!(matches!(result, SmtpCommandResult::Continue), "Expected Continue result for rejected STARTTLS, got {:?}", result);
        assert_eq!(protocol.get_state(), SmtpState::RcptTo, "State should remain RcptTo after rejected STARTTLS");
    }

     // Test STARTTLS command during DATA phase (should be treated as data)
    #[tokio::test]
    async fn test_data_starttls_is_data() {
        let mut protocol = create_test_protocol();
        protocol.state = SmtpState::Data; // Set state manually
        assert_eq!(protocol.get_state(), SmtpState::Data);

        let result = protocol.process_command("STARTTLS").await.unwrap();

        // In DATA state, any line not "." is data
        assert!(matches!(result, SmtpCommandResult::DataLine(ref line) if line == "STARTTLS"), "Expected DataLine result, got {:?}", result);
        assert_eq!(protocol.get_state(), SmtpState::Data);
    }

    // Test QUIT command works in Greeted state (important for STARTTLS flow)
    #[tokio::test]
    async fn test_greeted_quit() {
        let mut protocol = create_test_protocol();
        protocol.state = SmtpState::Greeted;
        let result = protocol.process_command("QUIT").await.unwrap();
        assert!(matches!(result, SmtpCommandResult::Quit));
        // State doesn't technically matter after Quit, but it shouldn't change unexpectedly
        assert_eq!(protocol.get_state(), SmtpState::Greeted);
    }

    // Note: Testing that EHLO *advertises* STARTTLS requires checking the output buffer,
    // which this mock setup doesn't support. This needs an integration test or a more
    // sophisticated mock writer. We will implement the EHLO change and verify manually/later.
}
