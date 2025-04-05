#[cfg(test)]
mod tests {
    use super::*;
    use tokio::test;
    use tokio::net::TcpStream;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use std::net::SocketAddr;
    use std::str::FromStr;
    use std::time::Duration;

    // Helper function to create a mock SMTP protocol handler
    async fn create_mock_smtp_protocol() -> Result<(SmtpProtocol, TcpStream), Box<dyn std::error::Error>> {
        // Create a pair of connected TCP streams
        let (server_stream, client_stream) = tokio::net::TcpSocket::new_v4()
            ?
            .bind(SocketAddr::from_str("127.0.0.1:0").expect("Failed to parse bind address"))?
            .connect(SocketAddr::from_str("127.0.0.1:0").expect("Failed to parse connect address"))
            .await
            ?;
        
        // Create the SMTP protocol handler with the server stream
        let protocol = SmtpProtocol::new(server_stream);
        
        Ok((protocol, client_stream))
    }
    
    #[test]
    async fn test_smtp_protocol_greeting() -> Result<(), Box<dyn std::error::Error>> {
        let (mut protocol, mut client) = create_mock_smtp_protocol().await;
        
        // Send greeting
        protocol.send_greeting().await?;
        
        // Read the greeting from the client side
        let mut buffer = [0; 1024];
        let n = client.read(&mut buffer).await?;
        let greeting = String::from_utf8_lossy(&buffer[0..n]);
        
        // Verify the greeting
        assert_eq!(greeting, "220 MailLaser SMTP Server\r\n"); // Verify updated greeting
        Ok(())
    }
    
    #[test]
    async fn test_smtp_protocol_ehlo() -> Result<(), Box<dyn std::error::Error>> {
        let (mut protocol, mut client) = create_mock_smtp_protocol().await;
        
        // Send EHLO command from client
        client.write_all(b"EHLO example.com\r\n").await?;
        
        // Read the command in the protocol
        let line = protocol.read_line().await?;
        
        // Process the command
        let result = protocol.process_command(&line).await?;
        
        // Verify the result
        match result {
            SmtpCommandResult::Continue => {},
            _ => panic!("Expected Continue, got {:?}", result),
        }
        
        // Read the response from the client side
        let mut buffer = [0; 1024];
        let n = client.read(&mut buffer).await?;
        let response = String::from_utf8_lossy(&buffer[0..n]);
        
        // Verify the response
        assert_eq!(response, "250 MailLaser\r\n"); // Verify updated HELO/EHLO response
        
        // Verify the state
        assert_eq!(protocol.get_state(), SmtpState::Greeted);
        Ok(())
    }
    
    #[test]
    async fn test_smtp_protocol_mail_from() -> Result<(), Box<dyn std::error::Error>> {
        let (mut protocol, mut client) = create_mock_smtp_protocol().await;
        
        // Set the state to Greeted
        protocol.reset_state();
        assert_eq!(protocol.get_state(), SmtpState::Greeted);
        
        // Send MAIL FROM command from client
        client.write_all(b"MAIL FROM:<sender@example.com>\r\n").await?;
        
        // Read the command in the protocol
        let line = protocol.read_line().await?;
        
        // Process the command
        let result = protocol.process_command(&line).await?;
        
        // Verify the result
        match result {
            SmtpCommandResult::MailFrom(email) => {
                assert_eq!(email, "sender@example.com");
            },
            _ => panic!("Expected MailFrom, got {:?}", result),
        }
        
        // Read the response from the client side
        let mut buffer = [0; 1024];
        let n = client.read(&mut buffer).await?;
        let response = String::from_utf8_lossy(&buffer[0..n]);
        
        // Verify the response
        assert_eq!(response, "250 OK\r\n");
        
        // Verify the state
        assert_eq!(protocol.get_state(), SmtpState::MailFrom);
        Ok(())
    }
    
    #[test]
    async fn test_smtp_protocol_rcpt_to() -> Result<(), Box<dyn std::error::Error>> {
        let (mut protocol, mut client) = create_mock_smtp_protocol().await;
        
        // Set the state to MailFrom
        protocol.reset_state();
        protocol.state = SmtpState::MailFrom;
        
        // Send RCPT TO command from client
        client.write_all(b"RCPT TO:<recipient@example.com>\r\n").await?;
        
        // Read the command in the protocol
        let line = protocol.read_line().await?;
        
        // Process the command
        let result = protocol.process_command(&line).await?;
        
        // Verify the result
        match result {
            SmtpCommandResult::RcptTo(email) => {
                assert_eq!(email, "recipient@example.com");
            },
            _ => panic!("Expected RcptTo, got {:?}", result),
        }
        
        // Verify the state
        assert_eq!(protocol.get_state(), SmtpState::RcptTo);
        Ok(())
    }
    
    #[test]
    async fn test_smtp_protocol_data() -> Result<(), Box<dyn std::error::Error>> {
        let (mut protocol, mut client) = create_mock_smtp_protocol().await;
        
        // Set the state to RcptTo
        protocol.reset_state();
        protocol.state = SmtpState::RcptTo;
        
        // Send DATA command from client
        client.write_all(b"DATA\r\n").await?;
        
        // Read the command in the protocol
        let line = protocol.read_line().await?;
        
        // Process the command
        let result = protocol.process_command(&line).await?;
        
        // Verify the result
        match result {
            SmtpCommandResult::DataStart => {},
            _ => panic!("Expected DataStart, got {:?}", result),
        }
        
        // Read the response from the client side
        let mut buffer = [0; 1024];
        let n = client.read(&mut buffer).await?;
        let response = String::from_utf8_lossy(&buffer[0..n]);
        
        // Verify the response
        assert_eq!(response, "354 End data with <CR><LF>.<CR><LF>\r\n");
        
        // Verify the state
        assert_eq!(protocol.get_state(), SmtpState::Data);
        Ok(())
    }
    
    #[test]
    async fn test_smtp_protocol_data_content() -> Result<(), Box<dyn std::error::Error>> {
        let (mut protocol, mut client) = create_mock_smtp_protocol().await;
        
        // Set the state to Data
        protocol.reset_state();
        protocol.state = SmtpState::Data;
        
        // Send data content from client
        client.write_all(b"Subject: Test Subject\r\n").await?;
        client.write_all(b"\r\n").await?;
        client.write_all(b"This is the body.\r\n").await?;
        client.write_all(b".\r\n").await?;
        
        // Read and process the data lines
        let mut data_lines = Vec::new();
        
        for _ in 0..4 {
            let line = protocol.read_line().await?;
            let result = protocol.process_command(&line).await?;
            
            match result {
                SmtpCommandResult::DataLine(content) => {
                    data_lines.push(content);
                },
                SmtpCommandResult::DataEnd => {
                    // Verify we got the end marker
                    assert_eq!(line, ".");
                },
                _ => panic!("Unexpected result: {:?}", result),
            }
        }
        
        // Verify the data lines
        assert_eq!(data_lines.len(), 3);
        assert_eq!(data_lines[0], "Subject: Test Subject");
        assert_eq!(data_lines[1], "");
        assert_eq!(data_lines[2], "This is the body.");
        
        // Read the response from the client side
        let mut buffer = [0; 1024];
        let n = client.read(&mut buffer).await?;
        let response = String::from_utf8_lossy(&buffer[0..n]);
        
        // Verify the response
        assert_eq!(response, "250 OK: Message accepted\r\n");
        
        // Verify the state was reset
        assert_eq!(protocol.get_state(), SmtpState::Greeted);
        Ok(())
    }
    
    #[test]
    async fn test_smtp_protocol_quit() -> Result<(), Box<dyn std::error::Error>> {
        let (mut protocol, mut client) = create_mock_smtp_protocol().await;
        
        // Send QUIT command from client
        client.write_all(b"QUIT\r\n").await?;
        
        // Read the command in the protocol
        let line = protocol.read_line().await?;
        
        // Process the command
        let result = protocol.process_command(&line).await?;
        
        // Verify the result
        match result {
            SmtpCommandResult::Quit => {},
            _ => panic!("Expected Quit, got {:?}", result),
        }
        
        // Read the response from the client side
        let mut buffer = [0; 1024];
        let n = client.read(&mut buffer).await?;
        let response = String::from_utf8_lossy(&buffer[0..n]);
        
        // Verify the response
        assert_eq!(response, "221 Bye\r\n");
        Ok(())
    }
}
