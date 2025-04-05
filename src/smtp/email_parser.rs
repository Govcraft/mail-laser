use anyhow::Result;
use log::debug;

/// Parses email content to extract headers and body
pub struct EmailParser;

impl EmailParser {
    /// Parse raw email data to extract subject and plain text body
    pub fn parse(raw_data: &str) -> Result<(String, String)> {
        let mut subject = String::new();
        let mut body = String::new();
        let mut in_headers = true;
        
        for line in raw_data.lines() {
            if in_headers {
                if line.is_empty() {
                    // Empty line marks the end of headers
                    in_headers = false;
                    continue;
                }
                
                // Extract subject from headers
                if line.to_lowercase().starts_with("subject:") {
                    subject = line[8..].trim().to_string();
                    debug!("Found subject: {}", subject);
                }
            } else {
                // Skip HTML content
                if line.contains("<html>") || line.contains("<body>") || 
                   line.contains("<div>") || line.contains("<p>") ||
                   line.contains("</body>") || line.contains("</html>") {
                    continue;
                }
                
                // Add line to body
                if !body.is_empty() {
                    body.push_str("\r\n");
                }
                body.push_str(line);
            }
        }
        
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
                     
        let (subject, body) = EmailParser::parse(email).expect("Email parsing failed in test_parse_simple_email");
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
                     </body></html>\r\n";
                     
        let (subject, body) = EmailParser::parse(email).expect("Email parsing failed in test_parse_html_email");
        assert_eq!(subject, "HTML Email");
        assert_eq!(body, "Plain text part.");
    }
}
