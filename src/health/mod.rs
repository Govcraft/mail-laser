use hyper::{Request, Response, StatusCode}; // Removed unused HttpError alias
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use http_body_util::Full; // For creating full response bodies
use http_body::Body; // Import the Body trait

use tokio::net::TcpListener;
use log::{info, error};
use anyhow::Result;
use crate::config::Config;
use std::net::SocketAddr;
use bytes::Bytes;

/// Simple handler for the health check endpoint.
// Make the handler generic over the Body type.
// We only need the Body trait bound as we don't interact with the body's data or error type.
// Return http::Error directly, as produced by Response::builder().body()
// Revert to non-generic handler expecting Incoming body and returning hyper::Error
// Make the handler generic over the Body type again
async fn health_check_handler<B>(req: Request<B>) -> Result<Response<Full<Bytes>>, hyper::Error>
where
    B: Body, // Use the http_body::Body trait
{
    if req.uri().path() == "/health" {
        // Build response, unwrap the Result assuming it won't fail for Full<Bytes>,
        // and wrap in Ok() for the function's return type.
        Ok(Response::builder()
            .status(StatusCode::OK)
            .body(Full::new(Bytes::from("")))
            .unwrap()) // Expect success
    } else {
        // Build response, unwrap the Result assuming it won't fail for Full<Bytes>,
        // and wrap in Ok() for the function's return type.
        Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Full::new(Bytes::from("Not Found")))
            .unwrap()) // Expect success
    }
}

/// Adapter function to bridge the generic handler with the concrete `Incoming` body type
/// expected by `service_fn`.
async fn health_check_adapter(req: Request<hyper::body::Incoming>) -> Result<Response<Full<Bytes>>, hyper::Error> {
    // Call the generic handler
    health_check_handler(req).await
}

/// Runs the health check HTTP server.
///
/// Binds to the address specified in the config and serves the `/health` endpoint.
pub async fn run_health_server(config: Config) -> Result<()> {
    // Construct the bind address
    let addr_str = format!(
        "{}:{}",
        config.health_check_bind_address, config.health_check_port
    );
    let addr: SocketAddr = addr_str.parse()
        .map_err(|e| {
            error!("Invalid bind address {}: {}", addr_str, e);
            anyhow::anyhow!("Invalid bind address: {}", e)
        })?;

    // Create a TCP listener
    let listener = TcpListener::bind(&addr).await
        .map_err(|e| {
            error!("Failed to bind health check server to {}: {}", addr_str, e);
            anyhow::anyhow!("Failed to bind health check server: {}", e)
        })?;

    info!("Health check server listening on {}", addr_str);

    // Run the server
    loop {
        let (stream, _) = listener.accept().await
            .map_err(|e| {
                error!("Failed to accept connection: {}", e);
                anyhow::anyhow!("Failed to accept connection: {}", e)
            })?;

        let io = TokioIo::new(stream);
        // Use the adapter function which takes Request<Incoming>
        // Use the non-generic handler directly
        // Use the adapter function for the server
        let service = hyper::service::service_fn(health_check_adapter);

        tokio::spawn(async move {
            if let Err(err) = Builder::new(TokioExecutor::new())
                .serve_connection(io, service)
                .await
            {
                error!("Error serving connection: {:?}", err);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::Request;
    use http_body_util::Empty; // Use Empty from http-body-util
    use bytes::Bytes;
    use hyper::StatusCode; // Ensure StatusCode is imported for asserts

    #[tokio::test]
    async fn test_health_check_handler() {
        // Test successful health check
        let req = Request::builder()
            .uri("/health")
            .body(Empty::<Bytes>::new()) // Revert to Empty<Bytes>
            .unwrap();
        // No need for explicit type annotation if inference works
        let response = health_check_handler(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Test 404 for wrong path
        let req = Request::builder()
            .uri("/wrong")
            .body(Empty::<Bytes>::new()) // Revert to Empty<Bytes>
            .unwrap();
        // No need for explicit type annotation if inference works
        let response = health_check_handler(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}