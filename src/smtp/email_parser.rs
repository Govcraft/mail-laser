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
        let text_body: String; // Declare text_body here
        let mut raw_body_lines: Vec<String> = Vec::new();
        let mut content_type: Option<String> = None; // Store the Content-Type header value
        let mut detected_html_tags = false; // Fallback flag if Content-Type is inconclusive
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
                } else if line.to_lowercase().starts_with("content-type:") {
                    // Extract the value part of the Content-Type header.
                    // We only care about the main type (e.g., "text/html"), ignore parameters for now.
                    let value = line[13..].trim();
                    content_type = Some(value.to_lowercase());
                    debug!("Extracted Content-Type: {}", value);
                }
                // Other headers are ignored.
            } else {
                // Now processing the body section. Collect all lines first.
                raw_body_lines.push(line.to_string());
                // Fallback heuristic: check for HTML tags in the body in case Content-Type is missing/ambiguous
                if !detected_html_tags && line.trim_start().starts_with('<') && line.trim_end().ends_with('>') {
                    let lower_line = line.to_lowercase();
                    if lower_line.contains("<html") || lower_line.contains("<body") || lower_line.contains("<p") || lower_line.contains("<div") || lower_line.contains("<a href") {
                        debug!("Detected potential HTML tags via heuristic (fallback).");
                        detected_html_tags = true; // Correctly update detected_html_tags
                    }
                }
            }
        }

        // Process the collected body lines
        let raw_body = raw_body_lines.join("\r\n");
        let html_body: Option<String>;

        // Determine if the body should be treated as HTML
        let treat_as_html = match &content_type {
            Some(ct) => {
                // Check if the main type is text/html (case-insensitive, ignore parameters)
                let main_type = ct.split(';').next().unwrap_or("").trim();
                debug!("Using Content-Type '{}' to determine body type.", main_type);
                main_type == "text/html"
            }
            _none => {
                // If no Content-Type, fall back to the tag detection heuristic
                debug!("No Content-Type header found, falling back to tag detection heuristic.");
                detected_html_tags // Use the flag set by the heuristic
            }
        };

        if treat_as_html {
            debug!("Processing body as HTML based on Content-Type or heuristic.");
            text_body = match html2text::from_read(raw_body.as_bytes(), 80) {
                Ok(text) => text,
                Err(e) => {
                    log::warn!("Failed to parse HTML body, falling back to raw body: {}", e);
                    raw_body.clone()
                }
            };
            html_body = Some(raw_body);
        } else {
            debug!("Processing body as plain text.");
            text_body = raw_body;
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
    fn test_parse_email_with_html_content_type() {
        let email = "From: sender@example.com\r\n\
                     To: recipient@example.com\r\n\
                     Subject: HTML Email\r\n\
                     Content-Type: text/html; charset=utf-8\r\n\
                     \r\n\
                     Plain text part that might be ignored by html2text if not in tags.\r\n\
                     <html><body>\r\n\
                     <p>HTML content that should be ignored.</p>\r\n\
                     </body></html>\r\n\
                     Another plain line.\r\n"; // Added another line to test skipping

        let (subject, text_body, html_body) = EmailParser::parse(email).expect("Parsing failed for HTML email");
        assert_eq!(subject, "HTML Email");

        // Define expected fragments based on html2text output
        let expected_text_fragment_1 = "Plain text part that might be ignored by html2text if not in tags.";
        let expected_text_fragment_2 = "HTML content that should be ignored."; // html2text extracts text from tags
        let expected_text_fragment_3 = "Another plain line.";

        // Check that html2text included all parts
        assert!(text_body.contains(expected_text_fragment_1), "Text body missing first plain part. Got: {}", text_body);
        assert!(text_body.contains(expected_text_fragment_2), "Text body missing HTML content part. Got: {}", text_body);
        assert!(text_body.contains(expected_text_fragment_3), "Text body missing second plain part. Got: {}", text_body);

        // Check the raw HTML body
        assert!(html_body.is_some(), "HTML body should be Some for HTML email");
        let html_content = html_body.unwrap();
        assert!(html_content.contains("<html>"), "HTML body missing <html> tag");
        assert!(html_content.contains("<p>HTML content that should be ignored.</p>"), "HTML body missing <p> tag content");
        assert!(html_content.contains("</html>"), "HTML body missing </html> tag");
        assert!(html_content.contains("Plain text part that might be ignored"), "HTML body missing plain text part"); // Check original plain text too
    }

    #[test] // Add #[test] attribute back
    fn test_parse_html_with_links_and_formatting_no_content_type() {
        // Test that the heuristic *still works* if Content-Type is missing but HTML tags are present
        let email = "Subject: Complex HTML Heuristic\r\n\r\n<html><body><h1>Title</h1><p>This is <strong>bold</strong> text and a <a href=\"http://example.com\">link</a>.</p><div>Another section</div></body></html>";

        let (subject, text_body, html_body) = EmailParser::parse(email).expect("Parsing failed for complex HTML heuristic");
        assert_eq!(subject, "Complex HTML Heuristic");

        // Check text body for key elements converted by html2text
        assert!(text_body.contains("Title"), "Text body missing title. Got: {}", text_body);
        assert!(text_body.contains("bold"), "Text body missing bold text. Got: {}", text_body);
        // html2text formats links like: [link][1] ... [1]: http://example.com
        assert!(text_body.contains("[link][1]"), "Text body missing reference link marker. Got: {}", text_body);
        assert!(text_body.contains("[1]: http://example.com"), "Text body missing reference link definition. Got: {}", text_body);
        assert!(text_body.contains("Another section"), "Text body missing div content. Got: {}", text_body);

        assert!(html_body.is_some(), "HTML body should be Some for complex HTML heuristic"); // Assertion moved from line 200
        let html_content = html_body.unwrap(); // Assertion moved from line 201
        assert!(html_content.contains("<h1>Title</h1>"), "HTML body missing h1 tag"); // Assertion moved from line 202
        assert!(html_content.contains("<a href=\"http://example.com\">link</a>"), "HTML body missing link tag"); // Assertion moved from line 203
    } // End of test_parse_html_with_links_and_formatting_no_content_type

    // --- Assertions below were moved from the end of the file back here ---
    // --- They belong to test_parse_email_with_html_content_type ---
    // --- This block should be removed after applying the diff above ---
    //
    //     // Let's check for key content and structure. html2text often adds line breaks.
    //     // Example: "<p>Hello</p>" might become "Hello\n".
    //     // The raw email has "Plain text part.\r\n<html><body>..."
    //     // html2text will process the whole body part.
    //     let expected_text_fragment_1 = "Plain text part.";
    //     let expected_text_fragment_2 = "HTML content that should be ignored."; // html2text extracts text from tags
    //     let expected_text_fragment_3 = "Another plain line.";
    //
    //     assert!(text_body.contains(expected_text_fragment_1), "Text body missing first plain part. Got: {}", text_body);
    //     assert!(text_body.contains(expected_text_fragment_2), "Text body missing HTML content part. Got: {}", text_body);
    //     assert!(text_body.contains(expected_text_fragment_3), "Text body missing second plain part. Got: {}", text_body);
    //
    //     assert!(html_body.is_some(), "HTML body should be Some for HTML email");
    //     let html_content = html_body.unwrap();
    //     // Check if the original HTML structure is preserved in the html_body
    //     assert!(html_content.contains("<html>"), "HTML body missing <html> tag");
    //     assert!(html_content.contains("<p>HTML content that should be ignored.</p>"), "HTML body missing <p> tag content");
    //     assert!(html_content.contains("</html>"), "HTML body missing </html> tag");
    //     assert!(html_content.contains("Plain text part."), "HTML body missing plain text part");
    // }

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
