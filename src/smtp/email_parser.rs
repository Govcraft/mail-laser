//! Parses raw SMTP message data into a structured [`ParsedEmail`].
//!
//! The parser extracts Subject, From display name, the first `text/plain` and
//! `text/html` body parts, any headers matching the configured prefixes, and
//! every remaining leaf part (or any leaf marked `Content-Disposition:
//! attachment`) as a binary [`Attachment`]. Attachment bytes are decoded to
//! raw form by `mailparse::ParsedMail::get_body_raw`.

use anyhow::{anyhow, Result};
use log::{debug, warn};
use mailparse::{addrparse, MailAddr, MailHeaderMap, ParsedMail};
use std::collections::HashMap;

/// A binary attachment extracted from a MIME message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub filename: Option<String>,
    pub content_type: String,
    pub size_bytes: u64,
    pub content_id: Option<String>,
    pub data: Vec<u8>,
}

/// Result of parsing a raw email.
#[derive(Debug, Clone)]
pub struct ParsedEmail {
    pub subject: String,
    pub from_name: Option<String>,
    pub text_body: String,
    pub html_body: Option<String>,
    pub matched_headers: HashMap<String, String>,
    pub attachments: Vec<Attachment>,
}

/// Namespace struct for parsing logic.
pub struct EmailParser;

impl EmailParser {
    /// Parses raw email bytes, extracting metadata, bodies, matching headers,
    /// and all attachments.
    ///
    /// * `raw_data` — full SMTP DATA payload (headers + body).
    /// * `header_prefixes` — case-insensitive header name prefixes to capture;
    ///   pass an empty slice to skip header matching.
    ///
    /// Errors only when the underlying `mailparse` call fails or a body part
    /// cannot be decoded.
    pub fn parse(raw_data: &[u8], header_prefixes: &[String]) -> Result<ParsedEmail> {
        let mail = mailparse::parse_mail(raw_data)
            .map_err(|e| anyhow!("Mail parsing failed: {}", e))?;

        let subject = mail
            .headers
            .get_first_value("Subject")
            .unwrap_or_else(|| {
                debug!("Subject header not found");
                String::new()
            });
        debug!("Extracted subject: {}", subject);

        let from_name = mail
            .headers
            .get_first_value("From")
            .and_then(|from_str| match addrparse(&from_str) {
                Ok(addrs) => addrs.first().and_then(|mail_addr| match mail_addr {
                    MailAddr::Single(spec) => spec.display_name.clone(),
                    MailAddr::Group(group) => Some(group.group_name.clone()),
                }),
                Err(e) => {
                    warn!("Failed to parse From header '{}': {}", from_str, e);
                    None
                }
            });
        debug!("Extracted From name: {:?}", from_name);

        let matched_headers = match_headers(&mail, header_prefixes);
        debug!("Matched {} headers against prefixes", matched_headers.len());

        let mut text_body: Option<String> = None;
        let mut html_body: Option<String> = None;
        let mut attachments: Vec<Attachment> = Vec::new();

        process_mail_part(&mail, &mut text_body, &mut html_body, &mut attachments)?;

        let final_text_body = if let Some(ref html) = html_body {
            debug!("HTML part found, generating final text body from HTML using html2text.");
            match html2text::from_read(html.as_bytes(), 80) {
                Ok(converted_text) => converted_text,
                Err(e) => {
                    warn!(
                        "html2text conversion failed, falling back to plain text part if available: {}",
                        e
                    );
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
            String::new()
        };

        Ok(ParsedEmail {
            subject,
            from_name,
            text_body: final_text_body,
            html_body,
            matched_headers,
            attachments,
        })
    }
}

fn match_headers(mail: &ParsedMail<'_>, prefixes: &[String]) -> HashMap<String, String> {
    if prefixes.is_empty() {
        return HashMap::new();
    }
    let lowercase_prefixes: Vec<String> = prefixes.iter().map(|p| p.to_lowercase()).collect();
    let mut headers_map = HashMap::new();
    for header in &mail.headers {
        let key_lower = header.get_key().to_lowercase();
        if lowercase_prefixes
            .iter()
            .any(|prefix| key_lower.starts_with(prefix))
        {
            let key = header.get_key();
            let value = header.get_value();
            debug!("Matched header: {} = {}", key, value);
            headers_map.insert(key, value);
        }
    }
    headers_map
}

/// Recursively walks a parsed MIME tree. The first `text/plain` fills
/// `text_body`, the first `text/html` fills `html_body`, and every other leaf
/// — plus any leaf explicitly marked `Content-Disposition: attachment` — is
/// collected into `attachments`.
fn process_mail_part(
    part: &ParsedMail<'_>,
    text_body: &mut Option<String>,
    html_body: &mut Option<String>,
    attachments: &mut Vec<Attachment>,
) -> Result<()> {
    if !part.subparts.is_empty() {
        debug!(
            "Processing multipart container ({}) with {} subparts.",
            part.ctype.mimetype,
            part.subparts.len()
        );
        for subpart in &part.subparts {
            process_mail_part(subpart, text_body, html_body, attachments)?;
        }
        return Ok(());
    }

    let mimetype = part.ctype.mimetype.clone();
    let disposition = part.get_content_disposition();
    let is_attachment_disposition =
        matches!(disposition.disposition, mailparse::DispositionType::Attachment);

    debug!(
        "Processing leaf part — Content-Type: {}, disposition: {:?}",
        mimetype, disposition.disposition
    );

    if !is_attachment_disposition {
        match mimetype.as_str() {
            "text/plain" if text_body.is_none() => {
                let body_str = part
                    .get_body()
                    .map_err(|e| anyhow!("Failed to get/decode plain text body: {}", e))?;
                debug!("Captured text/plain body ({} bytes).", body_str.len());
                *text_body = Some(body_str);
                return Ok(());
            }
            "text/html" if html_body.is_none() => {
                let body_str = part
                    .get_body()
                    .map_err(|e| anyhow!("Failed to get/decode HTML body: {}", e))?;
                debug!("Captured text/html body ({} bytes).", body_str.len());
                *html_body = Some(body_str);
                return Ok(());
            }
            _ => {}
        }
    }

    // Anything else is treated as an attachment.
    let data = part
        .get_body_raw()
        .map_err(|e| anyhow!("Failed to decode attachment body: {}", e))?;
    if data.is_empty() {
        debug!("Skipping zero-byte part with Content-Type {}", mimetype);
        return Ok(());
    }

    let filename = disposition
        .params
        .get("filename")
        .cloned()
        .or_else(|| part.ctype.params.get("name").cloned());

    let content_id = part
        .headers
        .get_first_value("Content-ID")
        .map(|raw| raw.trim().trim_start_matches('<').trim_end_matches('>').to_string())
        .filter(|s| !s.is_empty());

    let size_bytes = data.len() as u64;
    debug!(
        "Captured attachment filename={:?} type={} size={} bytes",
        filename, mimetype, size_bytes
    );

    attachments.push(Attachment {
        filename,
        content_type: mimetype,
        size_bytes,
        content_id,
        data,
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_email_returns_plain_body() {
        let email = "From: sender@example.com\r\n\
                     To: recipient@example.com\r\n\
                     Subject: Test Email\r\n\
                     \r\n\
                     This is a test email.\r\n\
                     It has multiple lines.\r\n";

        let parsed = EmailParser::parse(email.as_bytes(), &[]).expect("parse failed");
        assert!(parsed.from_name.is_none());
        assert_eq!(parsed.subject, "Test Email");
        assert_eq!(
            parsed.text_body.trim(),
            "This is a test email.\r\nIt has multiple lines.".trim()
        );
        assert!(parsed.html_body.is_none());
        assert!(parsed.attachments.is_empty());
    }

    #[test]
    fn parse_from_name_variants() {
        let cases: &[(&str, Option<&str>)] = &[
            (
                "From: Kangaroo Roo <roo@example.com>\r\nSubject: x\r\n\r\nBody.",
                Some("Kangaroo Roo"),
            ),
            (
                "From: <just_email@example.com>\r\nSubject: x\r\n\r\nBody.",
                None,
            ),
            (
                "From: plain_email@example.com\r\nSubject: x\r\n\r\nBody.",
                None,
            ),
            ("Subject: x\r\n\r\nBody.", None),
        ];
        for (raw, expected) in cases {
            let parsed = EmailParser::parse(raw.as_bytes(), &[]).expect("parse failed");
            assert_eq!(parsed.from_name.as_deref(), *expected, "case: {}", raw);
        }
    }

    #[test]
    fn parse_multipart_alternative_captures_both_bodies() {
        let email_data = "MIME-Version: 1.0\r\n\
Subject: hopefully no html\r\n\
From: Roland Rodriguez <rolandrodriguez@gmail.com>\r\n\
To: design@my.stickerai.shop\r\n\
Content-Type: multipart/alternative; boundary=\"b1\"\r\n\
\r\n\
--b1\r\n\
Content-Type: text/plain; charset=\"UTF-8\"\r\n\
\r\n\
trying to make sure all email is stripped from this message. Thanks!\r\n\
\r\n\
--b1\r\n\
Content-Type: text/html; charset=\"UTF-8\"\r\n\
\r\n\
<div>trying to make sure all email is stripped from this message.</div>\r\n\
\r\n\
--b1--\r\n";
        let parsed = EmailParser::parse(email_data.as_bytes(), &[]).expect("parse failed");
        assert_eq!(parsed.subject, "hopefully no html");
        assert_eq!(parsed.from_name.as_deref(), Some("Roland Rodriguez"));
        assert!(parsed.html_body.is_some());
        assert!(parsed
            .text_body
            .contains("trying to make sure all email is stripped from this message."));
        assert!(parsed.attachments.is_empty());
    }

    #[test]
    fn parse_headers_match_prefixes_case_insensitively() {
        let email = "From: sender@example.com\r\n\
                     Subject: Case Test\r\n\
                     X-Custom-Foo: value1\r\n\
                     x-custom-bar: value2\r\n\
                     X-Other: should-not-match\r\n\
                     \r\n\
                     Body.\r\n";
        let prefixes = vec!["X-Custom".to_string()];
        let parsed = EmailParser::parse(email.as_bytes(), &prefixes).expect("parse failed");
        assert_eq!(parsed.matched_headers.len(), 2);
        assert!(parsed
            .matched_headers
            .values()
            .any(|v| v == "value1"));
        assert!(parsed
            .matched_headers
            .values()
            .any(|v| v == "value2"));
    }

    #[test]
    fn parse_pdf_attachment_captures_bytes_and_metadata() {
        // Single-part attachment with text body + one PDF; minimal but realistic.
        let pdf_bytes = b"%PDF-1.4\n%fake pdf for test\n%%EOF";
        let pdf_b64 = base64_std(pdf_bytes);
        let raw = format!(
            "From: alice@agency.gov\r\n\
             To: intake@example.com\r\n\
             Subject: with pdf\r\n\
             MIME-Version: 1.0\r\n\
             Content-Type: multipart/mixed; boundary=\"b1\"\r\n\
             \r\n\
             --b1\r\n\
             Content-Type: text/plain; charset=UTF-8\r\n\
             \r\n\
             hello\r\n\
             \r\n\
             --b1\r\n\
             Content-Type: application/pdf; name=\"brief.pdf\"\r\n\
             Content-Disposition: attachment; filename=\"brief.pdf\"\r\n\
             Content-Transfer-Encoding: base64\r\n\
             \r\n\
             {}\r\n\
             --b1--\r\n",
            pdf_b64
        );
        let parsed = EmailParser::parse(raw.as_bytes(), &[]).expect("parse failed");
        assert_eq!(parsed.subject, "with pdf");
        assert!(parsed.text_body.contains("hello"));
        assert_eq!(parsed.attachments.len(), 1);
        let att = &parsed.attachments[0];
        assert_eq!(att.content_type, "application/pdf");
        assert_eq!(att.filename.as_deref(), Some("brief.pdf"));
        assert_eq!(att.size_bytes as usize, pdf_bytes.len());
        assert_eq!(att.data, pdf_bytes);
    }

    #[test]
    fn parse_multiple_attachments_with_mixed_encodings() {
        let doc_bytes = b"word doc bytes";
        let doc_b64 = base64_std(doc_bytes);
        let raw = format!(
            "From: alice@agency.gov\r\n\
             To: intake@example.com\r\n\
             Subject: two files\r\n\
             MIME-Version: 1.0\r\n\
             Content-Type: multipart/mixed; boundary=\"b1\"\r\n\
             \r\n\
             --b1\r\n\
             Content-Type: text/plain\r\n\
             \r\n\
             body\r\n\
             \r\n\
             --b1\r\n\
             Content-Type: application/vnd.openxmlformats-officedocument.wordprocessingml.document; name=\"doc.docx\"\r\n\
             Content-Disposition: attachment; filename=\"doc.docx\"\r\n\
             Content-Transfer-Encoding: base64\r\n\
             \r\n\
             {doc}\r\n\
             --b1\r\n\
             Content-Type: text/plain; name=\"notes.txt\"\r\n\
             Content-Disposition: attachment; filename=\"notes.txt\"\r\n\
             Content-Transfer-Encoding: quoted-printable\r\n\
             \r\n\
             line with =E2=9C=93 check mark\r\n\
             --b1--\r\n",
            doc = doc_b64
        );
        let parsed = EmailParser::parse(raw.as_bytes(), &[]).expect("parse failed");
        assert!(parsed.text_body.contains("body"));
        assert_eq!(parsed.attachments.len(), 2);

        let docx = parsed
            .attachments
            .iter()
            .find(|a| a.filename.as_deref() == Some("doc.docx"))
            .expect("docx attachment missing");
        assert_eq!(
            docx.content_type,
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        );
        assert_eq!(docx.data, doc_bytes);

        let notes = parsed
            .attachments
            .iter()
            .find(|a| a.filename.as_deref() == Some("notes.txt"))
            .expect("notes attachment missing");
        assert_eq!(notes.content_type, "text/plain");
        assert!(String::from_utf8_lossy(&notes.data).contains("check mark"));
    }

    #[test]
    fn parse_inline_cid_image_still_captured_as_attachment() {
        let img_bytes = b"\x89PNG\r\n\x1a\nfake";
        let img_b64 = base64_std(img_bytes);
        let raw = format!(
            "From: alice@agency.gov\r\n\
             To: intake@example.com\r\n\
             Subject: cid\r\n\
             MIME-Version: 1.0\r\n\
             Content-Type: multipart/related; boundary=\"b1\"\r\n\
             \r\n\
             --b1\r\n\
             Content-Type: text/html\r\n\
             \r\n\
             <img src=\"cid:logo\">\r\n\
             \r\n\
             --b1\r\n\
             Content-Type: image/png\r\n\
             Content-ID: <logo>\r\n\
             Content-Disposition: inline; filename=\"logo.png\"\r\n\
             Content-Transfer-Encoding: base64\r\n\
             \r\n\
             {}\r\n\
             --b1--\r\n",
            img_b64
        );
        let parsed = EmailParser::parse(raw.as_bytes(), &[]).expect("parse failed");
        assert_eq!(parsed.attachments.len(), 1);
        let att = &parsed.attachments[0];
        assert_eq!(att.content_type, "image/png");
        assert_eq!(att.content_id.as_deref(), Some("logo"));
        assert_eq!(att.filename.as_deref(), Some("logo.png"));
        assert_eq!(att.data, img_bytes);
    }

    #[test]
    fn parse_attachment_without_filename() {
        let bytes = b"opaque blob";
        let b64 = base64_std(bytes);
        let raw = format!(
            "From: alice@agency.gov\r\n\
             To: intake@example.com\r\n\
             Subject: no filename\r\n\
             MIME-Version: 1.0\r\n\
             Content-Type: multipart/mixed; boundary=\"b1\"\r\n\
             \r\n\
             --b1\r\n\
             Content-Type: text/plain\r\n\
             \r\n\
             body\r\n\
             \r\n\
             --b1\r\n\
             Content-Type: application/octet-stream\r\n\
             Content-Disposition: attachment\r\n\
             Content-Transfer-Encoding: base64\r\n\
             \r\n\
             {}\r\n\
             --b1--\r\n",
            b64
        );
        let parsed = EmailParser::parse(raw.as_bytes(), &[]).expect("parse failed");
        assert_eq!(parsed.attachments.len(), 1);
        let att = &parsed.attachments[0];
        assert!(att.filename.is_none());
        assert_eq!(att.content_type, "application/octet-stream");
        assert_eq!(att.data, bytes);
    }

    #[test]
    fn parse_empty_input_is_graceful() {
        if let Ok(parsed) = EmailParser::parse(b"", &[]) {
            assert!(parsed.subject.is_empty());
            assert!(parsed.text_body.is_empty());
            assert!(parsed.attachments.is_empty());
        }
    }

    fn base64_std(bytes: &[u8]) -> String {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;
        STANDARD.encode(bytes)
    }
}
