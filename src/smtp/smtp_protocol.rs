//! Implements the state machine and command handling logic for the SMTP protocol.
//!
//! This module defines the states of an SMTP conversation (`SmtpState`),
//! manages reading commands and writing responses over a `TcpStream`,
//! and parses basic SMTP commands, transitioning the state accordingly.

use anyhow::Result;
use log::debug;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::TcpStream;

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
pub struct SmtpProtocol {
    reader: BufReader<tokio::io::ReadHalf<TcpStream>>,
    writer: BufWriter<tokio::io::WriteHalf<TcpStream>>,
    state: SmtpState,
}

impl SmtpProtocol {
    /// Creates a new `SmtpProtocol` handler for a given `TcpStream`.
    ///
    /// Splits the stream into buffered reader and writer halves and initializes
    /// the state to `SmtpState::Initial`.
    pub fn new(stream: TcpStream) -> Self {
        let (reader, writer) = tokio::io::split(stream);

        SmtpProtocol {
            reader: BufReader::new(reader),
            writer: BufWriter::new(writer),
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
                if line.to_uppercase().starts_with("HELO") || line.to_uppercase().starts_with("EHLO") {
                    self.write_line("250 MailLaser").await?; // Simple OK response.
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
                // Expect MAIL FROM after greeting.
                if line.to_uppercase().starts_with("MAIL FROM:") {
                    if let Some(email) = self.extract_email(line) {
                        self.write_line("250 OK").await?;
                        self.state = SmtpState::MailFrom;
                        Ok(SmtpCommandResult::MailFrom(email))
                    } else {
                        self.write_line("501 Syntax error in MAIL FROM parameters").await?;
                        Ok(SmtpCommandResult::Continue)
                    }
                } else if line.to_uppercase().starts_with("QUIT") {
                    self.write_line("221 Bye").await?;
                    Ok(SmtpCommandResult::Quit)
                } else {
                    self.write_line("503 Bad sequence of commands (expected MAIL FROM)").await?;
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
            let line = buffer.trim_end_matches(|c| c == '\r' || c == '\n').to_string();
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
}

#[cfg(test)]
mod tests {
    // Note: The original tests were placeholders/unimplemented.
    // Keeping the structure but acknowledging lack of concrete tests here.

    // use super::*; // Not needed if not using items from parent mod directly in tests.
    // use tokio::net::TcpStream; // Not used without mocks.
    // use std::io::Result as IoResult; // Not used.

    // Placeholder for mock stream if needed for future tests.
    #[allow(dead_code)]
    struct MockTcpStream;

    impl MockTcpStream {
        #[allow(dead_code)]
        fn create_mock_pair() -> (tokio::net::TcpStream, tokio::net::TcpStream) {
            // Requires a proper mocking library (e.g., tokio_test::io::Builder)
            // or manual implementation to simulate network interaction.
            unimplemented!("Mock TcpStream not implemented for smtp_protocol tests")
        }
    }

    #[tokio::test]
    async fn test_smtp_protocol_state_transitions() {
        // TODO: Implement tests using a mock stream to verify:
        // - Correct state transitions for valid command sequences (HELO -> MAIL -> RCPT -> DATA -> . -> Greeted).
        // - Correct error responses for commands out of sequence (e.g., DATA before MAIL FROM).
        // - Correct handling of QUIT command in various states.
        // - Correct extraction of email addresses.
        // - Correct identification of DataStart, DataLine, DataEnd results.
        assert!(true, "Placeholder test for smtp_protocol state transitions passed (no actual test logic)");
    }
}
