#[cfg(test)]
mod tests {
    use super::*;
    use tokio::test;
    use std::sync::Arc;
    use hyper::{Body, Response, StatusCode};
    use hyper::service::{make_service_fn, service_fn};
    use hyper::Server;
    use std::convert::Infallible;
    use std::net::SocketAddr;
    use std::sync::Mutex;
    use std::time::Duration;
    
    // Mock webhook server to test the webhook client
    async fn setup_mock_webhook_server() -> (SocketAddr, Arc<Mutex<Option<EmailPayload>>>) {
        // Create a shared state to capture the received payload
        let received_payload = Arc::new(Mutex::new(None));
        let received_payload_clone = received_payload.clone();
        
        // Create a service function that will handle the webhook request
        let make_svc = make_service_fn(move |_conn| {
            let received_payload = received_payload_clone.clone();
            async {
                Ok::<_, Infallible>(service_fn(move |req| {
                    let received_payload = received_payload.clone();
                    async move {
                        // Read the request body
                        let body_bytes = hyper::body::to_bytes(req.into_body()).await.unwrap();
                        let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
                        
                        // Parse the JSON payload
                        let payload: EmailPayload = serde_json::from_str(&body_str).unwrap();
                        
                        // Store the payload
                        let mut guard = received_payload.lock().unwrap();
                        *guard = Some(payload);
                        
                        // Return a success response
                        Ok::<_, Infallible>(Response::new(Body::from("OK")))
                    }
                }))
            }
        });
        
        // Bind to a random port
        let addr = SocketAddr::from(([127, 0, 0, 1], 0));
        let server = Server::bind(&addr).serve(make_svc);
        let server_addr = server.local_addr();
        
        // Spawn the server
        tokio::spawn(async move {
            if let Err(e) = server.await {
                eprintln!("Server error: {}", e);
            }
        });
        
        // Return the server address and the shared state
        (server_addr, received_payload)
    }
    
    #[test]
    async fn test_webhook_client_forward_email() {
        // Setup mock webhook server
        let (server_addr, received_payload) = setup_mock_webhook_server().await;
        
        // Create a webhook URL pointing to our mock server
        let webhook_url = format!("http://{}", server_addr);
        
        // Create a config with the webhook URL
        let config = Config {
            target_email: "test@example.com".to_string(),
            webhook_url,
            smtp_bind_address: "127.0.0.1".to_string(),
            smtp_port: 2525,
        };
        
        // Create a webhook client
        let webhook_client = WebhookClient::new(config);
        
        // Create an email payload
        let email = EmailPayload {
            sender: "sender@example.com".to_string(),
            subject: "Test Subject".to_string(),
            body: "Test Body".to_string(),
        };
        
        // Forward the email
        let result = webhook_client.forward_email(email.clone()).await;
        
        // Verify the result
        assert!(result.is_ok());
        
        // Wait a bit for the server to process the request
        tokio::time::sleep(Duration::from_millis(100)).await;
        
        // Verify the payload was received correctly
        let received = received_payload.lock().unwrap().clone();
        assert!(received.is_some());
        
        let received = received.unwrap();
        assert_eq!(received.sender, "sender@example.com");
        assert_eq!(received.subject, "Test Subject");
        assert_eq!(received.body, "Test Body");
    }
}
