//! Provides basic parsing functionality to extract Subject and plain text Body
//! from raw email data received during an SMTP transaction.

use anyhow::Result;
use log::debug;

/// A namespace struct for email parsing logic.
///
/// This parser is intentionally simple and focuses only on extracting the
/// `Subject:` header and accumulating lines after the headers as the body.
/// It performs a basic attempt to skip common HTML tags but does not handle
/// MIME types, encodings, or complex email structures.
pub struct EmailParser;

impl EmailParser {
    /// Parses raw email data (headers and body) to extract the Subject header
    /// and a best-effort plain text representation of the body.
    ///
    /// Iterates through lines, identifying the `Subject:` header (case-insensitive).
    /// After encountering the first empty line (separating headers from body),
    /// it accumulates subsequent lines into the body string, attempting to skip
    /// lines containing common HTML tags.
    ///
    /// # Arguments
    ///
    /// * `raw_data` - A string slice containing the raw email content (headers and body).
    ///
    /// # Returns
    ///
    /// A `Result` containing a tuple `(String, String)` representing the
    /// extracted subject and body, respectively. Returns `Ok` even if the subject
    /// is not found (subject string will be empty). Errors are generally not expected
    /// from this simple parsing logic itself but the `Result` signature is kept
    /// for potential future enhancements or consistency.
    pub fn parse(raw_data: &str) -> Result<(String, String)> {
        let mut subject = String::new();
        let mut body = String::new();
        let mut in_headers = true; // Flag to track whether we are currently parsing headers.

        for line in raw_data.lines() {
            if in_headers {
                // An empty line signifies the end of the header section.
                if line.is_empty() {
                    in_headers = false;
                    continue; // Move to processing the body in the next iteration.
                }

                // Check for the Subject header (case-insensitive).
                if line.to_lowercase().starts_with("subject:") {
                    // Extract the value part of the Subject header.
                    subject = line[8..].trim().to_string();
                    debug!("Extracted subject: {}", subject);
                }
                // Other headers are ignored.
            } else {
                // Now processing the body section.
                // Basic attempt to skip lines that look like HTML tags.
                // This is not a robust HTML parser.
                if line.contains("<html>") || line.contains("<body>") ||
                   line.contains("<div>") || line.contains("<p>") ||
                   line.contains("</body>") || line.contains("</html>") {
                    debug!("Skipping potential HTML line: {}", line);
                    continue;
                }

                // Append the line to the body buffer, preserving line breaks.
                if !body.is_empty() {
                    body.push_str("\r\n"); // Add CRLF before appending subsequent lines.
                }
                body.push_str(line);
            }
        }

        // Return the extracted subject and body.
        Ok((subject, body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_email() {
        let email = "From: sender@example.com\r\n\
                     To: recipient@example.com\r\n\
                     Subject: Test Email\r\n\
                     \r\n\
                     This is a test email.\r\n\
                     It has multiple lines.\r\n";

        let (subject, body) = EmailParser::parse(email).expect("Parsing failed for simple email");
        assert_eq!(subject, "Test Email");
        assert_eq!(body, "This is a test email.\r\nIt has multiple lines.");
    }

    #[test]
    fn test_parse_email_with_html() {
        let email = "From: sender@example.com\r\n\
                     To: recipient@example.com\r\n\
                     Subject: HTML Email\r\n\
                     Content-Type: text/html\r\n\
                     \r\n\
                     Plain text part.\r\n\
                     <html><body>\r\n\
                     <p>HTML content that should be ignored.</p>\r\n\
                     </body></html>\r\n\
                     Another plain line.\r\n"; // Added another line to test skipping

        let (subject, body) = EmailParser::parse(email).expect("Parsing failed for HTML email");
        assert_eq!(subject, "HTML Email");
        // Expect only the plain text lines, with HTML lines skipped.
        assert_eq!(body, "Plain text part.\r\nAnother plain line.");
    }

    #[test]
    fn test_parse_no_subject() {
        let email = "From: sender@example.com\r\n\
                     To: recipient@example.com\r\n\
                     \r\n\
                     Body only.\r\n";

        let (subject, body) = EmailParser::parse(email).expect("Parsing failed for no-subject email");
        assert!(subject.is_empty(), "Subject should be empty when not present");
        assert_eq!(body, "Body only.");
    }

    #[test]
    fn test_parse_empty_body() {
        let email = "From: sender@example.com\r\n\
                     Subject: Empty Body Test\r\n\
                     \r\n"; // Headers end, but no body follows

        let (subject, body) = EmailParser::parse(email).expect("Parsing failed for empty-body email");
        assert_eq!(subject, "Empty Body Test");
        assert!(body.is_empty(), "Body should be empty");
    }
}
