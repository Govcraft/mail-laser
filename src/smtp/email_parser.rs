//! Provides parsing functionality to extract Subject, plain text Body (by stripping HTML),
//! and the original HTML Body from raw email data received during an SMTP transaction.

use std::collections::HashMap;
use anyhow::{anyhow, Result};
use log::{debug, warn};
use mailparse::{addrparse, MailAddr, MailHeaderMap, ParsedMail}; // Import MailHeaderMap trait and addrparse

/// The result of parsing an email: (subject, from_name, text_body, html_body, matched_headers).
pub type ParsedEmail = (String, Option<String>, String, Option<String>, HashMap<String, String>);

/// A namespace struct for email parsing logic.
///
/// This parser focuses on extracting the `Subject:` header and processing the body.
/// It uses the `mailparse` crate to handle MIME structures and extract relevant parts
/// (Subject, text/plain, text/html). It then uses `html2text` to generate a plain text
/// representation *if* only an HTML part is found.
pub struct EmailParser;

impl EmailParser {
    /// Parses raw email data using `mailparse` to extract the Subject header,
    /// the decoded plain text body, the decoded HTML body (if present), and any
    /// headers matching the configured prefixes.
    ///
    /// Handles simple emails and multipart/alternative emails, preferring the
    /// text/plain part for the main `text_body` and text/html for `html_body`.
    /// If only text/html is found, it uses `html2text` to generate the `text_body`.
    ///
    /// # Arguments
    ///
    /// * `raw_data` - A byte slice containing the raw email content (headers and body).
    /// * `header_prefixes` - A slice of header name prefixes to match. Headers whose
    ///   names start with any of these prefixes (case-insensitive) will be included
    ///   in the returned `HashMap`. Pass an empty slice to skip header matching.
    ///
    /// # Returns
    ///
    /// A `Result` containing a tuple
    /// `(String, Option<String>, String, Option<String>, HashMap<String, String>)` representing
    /// `(subject, from_name, text_body, html_body, matched_headers)`.
    /// - `subject`: The extracted subject line. Empty if not found.
    /// - `from_name`: An `Option<String>` containing the display name from the 'From'
    ///   header, if present and parseable. `None` otherwise.
    /// - `text_body`: The plain text representation of the body. HTML tags are stripped,
    ///   and basic formatting (like links) might be converted.
    /// - `html_body`: An `Option<String>` containing the original HTML body, if detected.
    ///   `None` if the body was treated as plain text.
    /// - `matched_headers`: A `HashMap` of header names to their decoded values for
    ///   headers matching the configured prefixes. Empty if no prefixes are configured
    ///   or no headers match.
    ///
    /// Returns `Ok` even if the subject is not found. Errors are generally not expected
    /// from this parsing logic itself, but the `Result` signature is kept for consistency.
    pub fn parse(raw_data: &[u8], header_prefixes: &[String]) -> Result<ParsedEmail> {
        // Use mailparse to parse the raw email data
        let mail = mailparse::parse_mail(raw_data).map_err(|e| anyhow!("Mail parsing failed: {}", e))?;

        // Extract Subject from headers
        // Use the MailHeaderMap trait to get header values
        let subject = mail.headers.get_first_value("Subject")
            .unwrap_or_else(|| {
                debug!("Subject header not found");
                String::new() // Default to empty string if not found
            });
        debug!("Extracted subject: {}", subject);

        // Extract From name using addrparse for robustness
        let from_name = mail.headers.get_first_value("From") // Changed "Reply-To" to "From"
            .and_then(|reply_to_str| {
                match addrparse(&reply_to_str) {
                    Ok(addrs) => {
                        // Get the display name from the first address, if available
                        addrs.first().and_then(|mail_addr| {
                            match mail_addr {
                                MailAddr::Single(spec) => spec.display_name.clone(),
                                // Handle Group if necessary, though less common for Reply-To
                                // Correctly access the group name using tuple indexing (index 0)
                                // Correctly access the group name using the 'group_name' field
                                // Wrap the group name in Some() to match the Option<String> type of the other arm
                                MailAddr::Group(group) => Some(group.group_name.clone()),
                            }
                        })
                    },
                    Err(e) => {
                        warn!("Failed to parse From header '{}': {}", reply_to_str, e); // Updated warning message
                        None // Treat parse failure as no name found
                    }
                }
            });
        debug!("Extracted From name: {:?}", from_name); // Updated debug message

        // Match headers against configured prefixes (case-insensitive prefix matching)
        let matched_headers = if header_prefixes.is_empty() {
            HashMap::new()
        } else {
            let lowercase_prefixes: Vec<String> = header_prefixes
                .iter()
                .map(|p| p.to_lowercase())
                .collect();
            let mut headers_map = HashMap::new();
            for header in &mail.headers {
                let key_lower = header.get_key().to_lowercase();
                if lowercase_prefixes.iter().any(|prefix| key_lower.starts_with(prefix)) {
                    let key = header.get_key();
                    let value = header.get_value();
                    debug!("Matched header: {} = {}", key, value);
                    headers_map.insert(key, value);
                }
            }
            headers_map
        };
        debug!("Matched {} headers against prefixes", matched_headers.len());

        // Variables to store the best plain text and HTML bodies found
        let mut text_body: Option<String> = None;
        let mut html_body: Option<String> = None;

        // Process the main part and subparts recursively
        process_mail_part(&mail, &mut text_body, &mut html_body)?;

        // Determine final text_body:
        // 1. Use text_body if found.
        // 2. If no text_body but html_body exists, generate text_body from html_body using html2text.
        // 3. Otherwise, it's an empty string.
        // (Removing duplicated comment block)
        let final_text_body = if let Some(ref html) = html_body {
             debug!("HTML part found, generating final text body from HTML using html2text.");
             match html2text::from_read(html.as_bytes(), 80) {
                 Ok(converted_text) => converted_text,
                 Err(e) => {
                     warn!("Failed to convert HTML body to text using html2text, falling back to plain text part if available: {}", e);
                     // Fallback: If conversion fails, use the plain text part if it exists
                     text_body.unwrap_or_else(|| {
                         warn!("HTML conversion failed and no plain text part found, using empty string.");
                         String::new()
                     })
                 }
             }
         } else if let Some(text) = text_body {
             debug!("Using found text/plain part for final text body (no HTML part found).");
             text
         } else {
             debug!("No text/plain or text/html body part found.");
             String::new() // No suitable body found
         };

        // Return the subject, from_name, text body, optional HTML body, and matched headers
        Ok((subject, from_name, final_text_body, html_body, matched_headers))
    }
} // End of impl EmailParser

/// Recursively processes a mail part and its subparts to find
/// the first text/plain and text/html bodies.
fn process_mail_part(part: &ParsedMail, text_body: &mut Option<String>, html_body: &mut Option<String>) -> Result<()> {
    if part.subparts.is_empty() {
        // This is a leaf part, check its content type
        let ctype = &part.ctype;
        let content_type_str = &ctype.mimetype; // Just use the full mimetype string
        debug!("Processing leaf part with Content-Type: {}", content_type_str);

        // Check the full mimetype string
        match content_type_str.as_str() {
            "text/plain" if text_body.is_none() => {
                // Found the first plain text part
                let body_str = part.get_body().map_err(|e| anyhow!("Failed to get/decode plain text body: {}", e))?;
                debug!("Found and decoded text/plain part.");
                *text_body = Some(body_str);
            }
            "text/html" if html_body.is_none() => {
                // Found the first HTML part
                let body_str = part.get_body().map_err(|e| anyhow!("Failed to get/decode HTML body: {}", e))?;
                debug!("Found and decoded text/html part.");
                *html_body = Some(body_str);
            }
            _ => {
                // Other text types or non-text types ignored
                debug!("Ignoring part with Content-Type: {}", content_type_str);
            }
        }
    } else {
        // This is a multipart container, process subparts recursively
        // Often multipart/alternative contains both text/plain and text/html
        debug!("Processing multipart container ({}) with {} subparts.", part.ctype.mimetype, part.subparts.len());
        for subpart in &part.subparts {
            // Stop searching if we've already found both types
            if text_body.is_some() && html_body.is_some() {
                debug!("Found both text and html parts, stopping search in this branch.");
                break;
            }
            process_mail_part(subpart, text_body, html_body)?;
        }
    }
    Ok(())
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

        // Check the from_name field (should be None as From only has email)
        let (subject, from_name, text_body, html_body, _) = EmailParser::parse(email.as_bytes(), &[]).expect("Parsing failed for simple email");
        assert!(from_name.is_none(), "From name should be None for simple email with only address");
        assert_eq!(subject, "Test Email");
        assert_eq!(text_body.trim(), "This is a test email.\r\nIt has multiple lines.".trim());
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

        // Check the from_name field (should be None as From only has email)
        let (subject, from_name, text_body, html_body, _) = EmailParser::parse(email.as_bytes(), &[]).expect("Parsing failed for HTML email");
        assert!(from_name.is_none(), "From name should be None for HTML email with only address");
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

        // Check the from_name field (should be None as From header is missing)
        let (subject, from_name, text_body, html_body, _) = EmailParser::parse(email.as_bytes(), &[]).expect("Parsing failed for complex HTML heuristic");
        assert!(from_name.is_none(), "From name should be None when From header is missing");
        assert_eq!(subject, "Complex HTML Heuristic");

        // Check text body - Since no Content-Type, mailparse likely defaults to text/plain.
        // The final logic block uses this text_body directly because html_body is None.
        // Assert that the text_body contains the raw HTML tags.
        assert!(text_body.contains("<h1>Title</h1>"), "Text body missing raw h1 tag. Got: {}", text_body);
        assert!(text_body.contains("<strong>bold</strong>"), "Text body missing raw strong tag. Got: {}", text_body);
        assert!(text_body.contains("<a href=\"http://example.com\">link</a>"), "Text body missing raw a tag. Got: {}", text_body);
        assert!(text_body.contains("<div>Another section</div>"), "Text body missing raw div tag. Got: {}", text_body);

        // html_body should be None because mailparse likely defaulted the single part to text/plain
        assert!(html_body.is_none(), "HTML body should be None when Content-Type is missing and mailparse defaults to text/plain");
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

        // Check the from_name field (should be None as From only has email)
        let (subject, from_name, text_body, html_body, _) = EmailParser::parse(email.as_bytes(), &[]).expect("Parsing failed for no-subject email");
        assert!(from_name.is_none(), "From name should be None for no-subject email with only address");
        assert!(subject.is_empty(), "Subject should be empty when not present");
        assert_eq!(text_body.trim(), "Body only.".trim());
        assert!(html_body.is_none(), "HTML body should be None for plain text email");
    }

    #[test]
    fn test_parse_empty_body() {
        let email = "From: sender@example.com\r\n\
                     Subject: Empty Body Test\r\n\
                     \r\n"; // Headers end, but no body follows

        // Check the from_name field (should be None as From only has email)
        let (subject, from_name, text_body, html_body, _) = EmailParser::parse(email.as_bytes(), &[]).expect("Parsing failed for empty-body email");
        assert!(from_name.is_none(), "From name should be None for empty-body email with only address");
        assert_eq!(subject, "Empty Body Test");
        assert!(text_body.is_empty(), "Text body should be empty");
        assert!(html_body.is_none(), "HTML body should be None for empty body email");
    }
}


    // Insert the new test case here
    #[test]
    fn test_parse_from_name() {
        // Case 1: From with name and email
        let email_with_name = "From: Kangaroo Roo <roo@example.com>\r\n\
                               Subject: Test With Name\r\n\
                               \r\n\
                               Body.";
        let (subject1, name1, body1, html1, _) = EmailParser::parse(email_with_name.as_bytes(), &[]).expect("Parsing failed for From with name");
        assert_eq!(subject1, "Test With Name");
        assert_eq!(name1.as_deref(), Some("Kangaroo Roo"), "From name mismatch"); // Check the extracted name
        assert_eq!(body1.trim(), "Body.");
        assert!(html1.is_none());

        // Case 2: From with only email address (angle brackets)
        let email_only_addr_angle = "From: <just_email@example.com>\r\n\
                                     Subject: Test Email Only Angle\r\n\
                                     \r\n\
                                     Body.";
        let (subject2, name2, body2, html2, _) = EmailParser::parse(email_only_addr_angle.as_bytes(), &[]).expect("Parsing failed for From email only angle");
        assert_eq!(subject2, "Test Email Only Angle");
        assert!(name2.is_none(), "Name should be None when From only has email (angle)");
        assert_eq!(body2.trim(), "Body.");
        assert!(html2.is_none());

        // Case 3: From with only email address (no angle brackets)
        let email_only_addr_plain = "From: plain_email@example.com\r\n\
                                     Subject: Test Email Only Plain\r\n\
                                     \r\n\
                                     Body.";
        let (subject3, name3, body3, html3, _) = EmailParser::parse(email_only_addr_plain.as_bytes(), &[]).expect("Parsing failed for From email only plain");
        assert_eq!(subject3, "Test Email Only Plain");
        // mailparse::addrparse correctly identifies no display name here
        assert!(name3.is_none(), "Name should be None when From only has email (plain)");
        assert_eq!(body3.trim(), "Body.");
        assert!(html3.is_none());


        // Case 4: No From header (less common, but testable)
        let email_no_from = "Subject: Test No From\r\n\
                             \r\n\
                             Body.";
        let (subject4, name4, body4, html4, _) = EmailParser::parse(email_no_from.as_bytes(), &[]).expect("Parsing failed for no From");
        assert_eq!(subject4, "Test No From");
        assert!(name4.is_none(), "Name should be None when From header is missing");
        assert_eq!(body4.trim(), "Body.");
        assert!(html4.is_none());
    }

    #[test]
    fn test_parse_multipart_alternative() {
        // Example multipart email provided by user
        let email_data = r#"MIME-Version: 1.0
Date: Sun, 6 Apr 2025 02:37:39 -0500
Message-ID: <CALGz_fUk-EJ9wi-VSkZMuAgcHa9bK+kFKnsKdSLrxX62LU1inA@mail.gmail.com>
Subject: hopefully no html
From: Roland Rodriguez <rolandrodriguez@gmail.com>
Reply-To: "Another Name" <another@example.com>
To: design@my.stickerai.shop
Content-Type: multipart/alternative; boundary="0000000000005e994006321734d8"

--0000000000005e994006321734d8
Content-Type: text/plain; charset="UTF-8"

trying to make sure all email is stripped from this message. Thanks!

*Yours truly,*
*ME*
*https://govcraft.ai <https://govcraft.ai>*

--0000000000005e994006321734d8
Content-Type: text/html; charset="UTF-8"

<div dir="ltr">trying to make sure all email is stripped from this message. Thanks!<br><br><b>Yours truly,</b><div><i>ME</i></div><div><i><a href="https://govcraft.ai">https://govcraft.ai</a></i></div><div><i><br></i></div></div>

--0000000000005e994006321734d8--
"#;
        // Check the from_name field and assert it
        let (subject, from_name, text_body, html_body_opt, _) = EmailParser::parse(email_data.as_bytes(), &[]).expect("Parsing multipart failed");

        assert_eq!(subject, "hopefully no html");
        // The From header has "Roland Rodriguez"
        assert_eq!(from_name.as_deref(), Some("Roland Rodriguez"), "From name mismatch in multipart test");

        // Check plain text part (mailparse might normalize line endings)
        // This should match the content of the text/plain part, with normalized newlines (\n)
        // This should match the content of the text/plain part, using \n for normalized newlines
        // This should match the MARKDOWN output generated by html2text from the HTML part
        // This should match the MARKDOWN output generated by html2text from the HTML part, including formatting
        // This should match the MARKDOWN output generated by html2text from the HTML part (matching actual output)
        let expected_markdown = "trying to make sure all email is stripped from this message. Thanks!\n\nYours truly,\nME\n[https://govcraft.ai][1]\n\n\n[1]: https://govcraft.ai";
        // Trim whitespace and compare. html2text might add extra trailing newlines.
        assert_eq!(text_body.trim(), expected_markdown.trim());

        // Check HTML part
        assert!(html_body_opt.is_some(), "HTML body should be present");
        let html_body = html_body_opt.unwrap();
        let expected_html_fragment = "<div dir=\"ltr\">trying to make sure all email is stripped from this message.";
        assert!(html_body.contains(expected_html_fragment), "HTML body missing expected content. Got: {}", html_body);
        assert!(html_body.contains("<b>Yours truly,</b>"), "HTML body missing bold tag");
        assert!(html_body.contains("<a href=\"https://govcraft.ai\">"), "HTML body missing link tag");
    }

    #[test]
    fn test_parse_headers_with_prefixes() {
        let email = "From: sender@example.com\r\n\
                     Subject: Header Test\r\n\
                     X-Custom-Foo: value1\r\n\
                     X-Custom-Bar: value2\r\n\
                     X-Other: should-not-match\r\n\
                     \r\n\
                     Body.\r\n";

        let prefixes = vec!["X-Custom".to_string()];
        let (_, _, _, _, matched) = EmailParser::parse(email.as_bytes(), &prefixes)
            .expect("Parsing failed for header prefix test");
        assert_eq!(matched.len(), 2, "Expected 2 matched headers, got {}", matched.len());
        assert_eq!(matched.get("X-Custom-Foo").map(String::as_str), Some("value1"));
        assert_eq!(matched.get("X-Custom-Bar").map(String::as_str), Some("value2"));
        assert!(!matched.contains_key("X-Other"), "X-Other should not be matched");
    }

    #[test]
    fn test_parse_headers_case_insensitive() {
        let email = "From: sender@example.com\r\n\
                     Subject: Case Test\r\n\
                     X-MY-HEADER: upper-value\r\n\
                     x-my-other: lower-value\r\n\
                     \r\n\
                     Body.\r\n";

        let prefixes = vec!["x-my".to_string()];
        let (_, _, _, _, matched) = EmailParser::parse(email.as_bytes(), &prefixes)
            .expect("Parsing failed for case-insensitive header test");
        assert_eq!(matched.len(), 2, "Expected 2 matched headers (case-insensitive), got {}", matched.len());
        assert!(matched.values().any(|v| v == "upper-value"), "Missing upper-value header");
        assert!(matched.values().any(|v| v == "lower-value"), "Missing lower-value header");
    }

    #[test]
    fn test_parse_headers_no_match() {
        let email = "From: sender@example.com\r\n\
                     Subject: No Match Test\r\n\
                     X-Something: value\r\n\
                     \r\n\
                     Body.\r\n";

        let prefixes = vec!["X-Nonexistent".to_string()];
        let (_, _, _, _, matched) = EmailParser::parse(email.as_bytes(), &prefixes)
            .expect("Parsing failed for no-match header test");
        assert!(matched.is_empty(), "Expected no matched headers, got {}", matched.len());
    }

    #[test]
    fn test_parse_headers_empty_prefixes() {
        let email = "From: sender@example.com\r\n\
                     Subject: Empty Prefixes Test\r\n\
                     X-Custom: value\r\n\
                     \r\n\
                     Body.\r\n";

        let (_, _, _, _, matched) = EmailParser::parse(email.as_bytes(), &[])
            .expect("Parsing failed for empty prefixes test");
        assert!(matched.is_empty(), "Expected no matched headers when prefixes are empty");
    }
