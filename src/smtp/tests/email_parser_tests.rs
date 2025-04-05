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
    
    #[test]
    fn test_parse_email_no_subject() {
        let email = "From: sender@example.com\r\n\
                     To: recipient@example.com\r\n\
                     \r\n\
                     This is a test email with no subject.\r\n";
                     
        let (subject, body) = EmailParser::parse(email).expect("Email parsing failed in test_parse_no_subject");
        assert_eq!(subject, "");
        assert_eq!(body, "This is a test email with no subject.");
    }
    
    #[test]
    fn test_parse_email_multiline_subject() {
        let email = "From: sender@example.com\r\n\
                     To: recipient@example.com\r\n\
                     Subject: This is a very long subject\r\n\
                      that spans multiple lines\r\n\
                     \r\n\
                     Email body here.\r\n";
                     
        let (subject, body) = EmailParser::parse(email).expect("Email parsing failed in test_parse_long_subject");
        assert_eq!(subject, "This is a very long subject");
        assert_eq!(body, "Email body here.");
    }
    
    #[test]
    fn test_parse_email_with_complex_html() {
        let email = "From: sender@example.com\r\n\
                     To: recipient@example.com\r\n\
                     Subject: Complex HTML Email\r\n\
                     Content-Type: multipart/alternative; boundary=\"boundary\"\r\n\
                     \r\n\
                     --boundary\r\n\
                     Content-Type: text/plain\r\n\
                     \r\n\
                     This is the plain text part.\r\n\
                     --boundary\r\n\
                     Content-Type: text/html\r\n\
                     \r\n\
                     <html>\r\n\
                     <body>\r\n\
                     <div>This is HTML content that should be ignored.</div>\r\n\
                     </body>\r\n\
                     </html>\r\n\
                     --boundary--\r\n";
                     
        let (subject, body) = EmailParser::parse(email).expect("Email parsing failed in test_parse_complex_html");
        assert_eq!(subject, "Complex HTML Email");
        assert!(body.contains("This is the plain text part."));
        assert!(!body.contains("<html>"));
        assert!(!body.contains("<div>"));
    }
}
