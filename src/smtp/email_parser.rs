//! Provides parsing functionality to extract Subject, plain text Body (by stripping HTML),
//! and the original HTML Body from raw email data received during an SMTP transaction.

use anyhow::Result;
use log::debug;

/// A namespace struct for email parsing logic.
///
/// This parser focuses on extracting the `Subject:` header and processing the body.
/// It uses the `html2text` crate to convert HTML content into a plain text representation
/// while also preserving the original HTML content separately.
/// It does not handle complex MIME structures or different encodings beyond basic UTF-8.
pub struct EmailParser;

impl EmailParser {
    /// Parses raw email data (headers and body) to extract the Subject header
    /// and both a plain text representation (HTML stripped) and the original HTML content of the body.
    ///
    /// Iterates through lines, identifying the `Subject:` header (case-insensitive).
    /// After encountering the first empty line (separating headers from body),
    /// it accumulates subsequent lines. If the content appears to be HTML (basic check),
    /// it uses `html2text` to generate the plain text version and stores the original HTML.
    /// Otherwise, the accumulated text is treated as plain text directly.
    ///
    /// # Arguments
    ///
    /// * `raw_data` - A string slice containing the raw email content (headers and body).
    ///
    /// # Returns
    ///
    /// A `Result` containing a tuple `(String, String, Option<String>)` representing
    /// `(subject, text_body, html_body)`.
    /// - `subject`: The extracted subject line. Empty if not found.
    /// - `text_body`: The plain text representation of the body. HTML tags are stripped,
    ///   and basic formatting (like links) might be converted.
    /// - `html_body`: An `Option<String>` containing the original HTML body, if detected.
    ///   `None` if the body was treated as plain text.
    ///
    /// Returns `Ok` even if the subject is not found. Errors are generally not expected
    /// from this parsing logic itself, but the `Result` signature is kept for consistency.
    pub fn parse(raw_data: &str) -> Result<(String, String, Option<String>)> {
        let mut subject = String::new();
        // text_body will be assigned within the if/else block below
        let mut raw_body_lines: Vec<String> = Vec::new(); // Collect raw lines for potential HTML parsing
        let mut is_html = false; // Flag to indicate if HTML content is detected
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
                // Now processing the body section. Collect all lines first.
                raw_body_lines.push(line.to_string());
                // Simple heuristic to detect HTML content. A more robust check might look
                // for Content-Type header, but this parser works on raw data post-header separation.
                if line.trim_start().starts_with('<') && line.trim_end().ends_with('>') {
                    // Check for common tags, case-insensitive
                    let lower_line = line.to_lowercase();
                    if lower_line.contains("<html") || lower_line.contains("<body") || lower_line.contains("<p") || lower_line.contains("<div") || lower_line.contains("<a href") {
                         debug!("Detected potential HTML content.");
                         is_html = true;
                    }
                }
            }
        }

        // Process the collected body lines
        let raw_body = raw_body_lines.join("\r\n");
        let html_body: Option<String>;

        if is_html {
            debug!("Processing body as HTML.");
            // Use html2text to convert HTML to plain text.
            // We use from_read_rich to get more formatting options like links.
            // Let's keep it simple for now with from_read and default width.
            // Consider using `html2text::from_read_rich` if more sophisticated text conversion is needed.
            let text_body = match html2text::from_read(raw_body.as_bytes(), 80) { // Assign here
                Ok(text) => {
                    // Explicitly ensure it's a String
                    let converted_text: String = text;
                    converted_text
                },
                Err(e) => {
                    log::warn!("Failed to parse HTML body, falling back to raw body: {}", e);
                    // Explicitly ensure it's a String
                    let fallback_text: String = raw_body.clone();
                    fallback_text // Use the original raw body as fallback text
                }
            };
            html_body = Some(raw_body); // Store the original HTML
        } else {
            debug!("Processing body as plain text.");
            let text_body = raw_body; // Assign here. Treat the collected body as plain text
            html_body = None;
        }

        // Return the extracted subject, text body, and optional HTML body.
        Ok((subject, text_body, html_body))
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

        let (subject, text_body, html_body) = EmailParser::parse(email).expect("Parsing failed for simple email");
        assert_eq!(subject, "Test Email");
        assert_eq!(text_body, "This is a test email.\r\nIt has multiple lines.");
        assert!(html_body.is_none(), "HTML body should be None for plain text email");
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

        let (subject, text_body, html_body) = EmailParser::parse(email).expect("Parsing failed for HTML email");
        assert_eq!(subject, "HTML Email");

        // Expected text output from html2text might differ slightly based on its conversion logic.

    #[test]
    fn test_parse_html_with_links_and_formatting() {
        let email = "Subject: Complex HTML\r\n\r\n<html><body><h1>Title</h1><p>This is <strong>bold</strong> text and a <a href=\"http://example.com\">link</a>.</p><div>Another section</div></body></html>";

        let (subject, text_body, html_body) = EmailParser::parse(email).expect("Parsing failed for complex HTML");
        assert_eq!(subject, "Complex HTML");

        // Check text body for key elements converted by html2text
        assert!(text_body.contains("Title"), "Text body missing title. Got: {}", text_body);
        assert!(text_body.contains("bold"), "Text body missing bold text. Got: {}", text_body);
        // html2text usually formats links like: link [http://example.com]
        assert!(text_body.contains("link [http://example.com]"), "Text body missing formatted link. Got: {}", text_body);
        assert!(text_body.contains("Another section"), "Text body missing div content. Got: {}", text_body);

        assert!(html_body.is_some(), "HTML body should be Some for complex HTML");
        let html_content = html_body.unwrap();
        assert!(html_content.contains("<h1>Title</h1>"), "HTML body missing h1 tag");
        assert!(html_content.contains("<a href=\"http://example.com\">link</a>"), "HTML body missing link tag");
    }

        // Let's check for key content and structure. html2text often adds line breaks.
        // Example: "<p>Hello</p>" might become "Hello\n".
        // The raw email has "Plain text part.\r\n<html><body>..."
        // html2text will process the whole body part.
        let expected_text_fragment_1 = "Plain text part.";
        let expected_text_fragment_2 = "HTML content that should be ignored."; // html2text extracts text from tags
        let expected_text_fragment_3 = "Another plain line.";

        assert!(text_body.contains(expected_text_fragment_1), "Text body missing first plain part. Got: {}", text_body);
        assert!(text_body.contains(expected_text_fragment_2), "Text body missing HTML content part. Got: {}", text_body);
        assert!(text_body.contains(expected_text_fragment_3), "Text body missing second plain part. Got: {}", text_body);

        assert!(html_body.is_some(), "HTML body should be Some for HTML email");
        let html_content = html_body.unwrap();
        // Check if the original HTML structure is preserved in the html_body
        assert!(html_content.contains("<html>"), "HTML body missing <html> tag");
        assert!(html_content.contains("<p>HTML content that should be ignored.</p>"), "HTML body missing <p> tag content");
        assert!(html_content.contains("</html>"), "HTML body missing </html> tag");
        assert!(html_content.contains("Plain text part."), "HTML body missing plain text part");
    }

    #[test]
    fn test_parse_no_subject() {
        let email = "From: sender@example.com\r\n\
                     To: recipient@example.com\r\n\
                     \r\n\
                     Body only.\r\n";

        let (subject, text_body, html_body) = EmailParser::parse(email).expect("Parsing failed for no-subject email");
        assert!(subject.is_empty(), "Subject should be empty when not present");
        assert_eq!(text_body, "Body only.");
        assert!(html_body.is_none(), "HTML body should be None for plain text email");
    }

    #[test]
    fn test_parse_empty_body() {
        let email = "From: sender@example.com\r\n\
                     Subject: Empty Body Test\r\n\
                     \r\n"; // Headers end, but no body follows

        let (subject, text_body, html_body) = EmailParser::parse(email).expect("Parsing failed for empty-body email");
        assert_eq!(subject, "Empty Body Test");
        assert!(text_body.is_empty(), "Text body should be empty");
        assert!(html_body.is_none(), "HTML body should be None for empty body email");
    }
}
