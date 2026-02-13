use http_body::Body;
use http_body_util::Full;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;

use crate::config::Config;
use acton_reactive::prelude::*;
use anyhow::Result;
use bytes::Bytes;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

async fn health_check_handler<B>(req: Request<B>) -> Result<Response<Full<Bytes>>, hyper::Error>
where
    B: Body,
{
    if req.uri().path() == "/health" {
        Ok(Response::builder()
            .status(StatusCode::OK)
            .body(Full::new(Bytes::from("")))
            .unwrap())
    } else {
        Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Full::new(Bytes::from("Not Found")))
            .unwrap())
    }
}

async fn health_check_adapter(
    req: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    health_check_handler(req).await
}

// --- HealthActor ---

#[acton_actor]
pub struct HealthState;

impl HealthState {
    pub async fn create(
        runtime: &mut ActorRuntime,
        config: &Config,
    ) -> anyhow::Result<ActorHandle> {
        let actor_config = ActorConfig::new(Ern::with_root("health-check")?, None, None)?
            .with_restart_policy(RestartPolicy::Permanent);

        let mut builder = runtime.new_actor_with_config::<Self>(actor_config);

        let cancel = CancellationToken::new();
        let cancel_for_loop = cancel.clone();
        let cancel_for_stop = cancel.clone();
        let health_config = config.clone();

        builder.after_start(move |_| {
            let config = health_config.clone();
            let cancel = cancel_for_loop.clone();

            tokio::spawn(async move {
                let addr_str = format!(
                    "{}:{}",
                    config.health_check_bind_address, config.health_check_port
                );
                let listener = match TcpListener::bind(&addr_str).await {
                    Ok(l) => {
                        tracing::info!("Health check server listening on {}", addr_str);
                        l
                    }
                    Err(e) => {
                        tracing::error!("Failed to bind health check server to {}: {}", addr_str, e);
                        return;
                    }
                };

                loop {
                    tokio::select! {
                        result = listener.accept() => {
                            match result {
                                Ok((stream, _)) => {
                                    let io = TokioIo::new(stream);
                                    let service = hyper::service::service_fn(health_check_adapter);

                                    tokio::spawn(async move {
                                        if let Err(err) = Builder::new(TokioExecutor::new())
                                            .serve_connection(io, service)
                                            .await
                                        {
                                            tracing::error!("Error serving health connection: {:?}", err);
                                        }
                                    });
                                }
                                Err(e) => {
                                    tracing::error!("Health accept error: {}", e);
                                    break;
                                }
                            }
                        }
                        _ = cancel.cancelled() => {
                            tracing::info!("Health server shutting down");
                            break;
                        }
                    }
                }
            });

            Reply::ready()
        });

        builder.before_stop(move |_| {
            cancel_for_stop.cancel();
            Reply::ready()
        });

        Ok(builder.start().await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use http_body_util::Empty;
    use hyper::Request;
    use hyper::StatusCode;

    #[tokio::test]
    async fn test_health_check_handler() {
        let req = Request::builder()
            .uri("/health")
            .body(Empty::<Bytes>::new())
            .unwrap();
        let response = health_check_handler(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let req = Request::builder()
            .uri("/wrong")
            .body(Empty::<Bytes>::new())
            .unwrap();
        let response = health_check_handler(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_health_check_post_method() {
        let req = Request::builder()
            .method(hyper::Method::POST)
            .uri("/health")
            .body(Empty::<Bytes>::new())
            .unwrap();
        let response = health_check_handler(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_health_check_put_method() {
        let req = Request::builder()
            .method(hyper::Method::PUT)
            .uri("/health")
            .body(Empty::<Bytes>::new())
            .unwrap();
        let response = health_check_handler(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_health_check_head_method() {
        let req = Request::builder()
            .method(hyper::Method::HEAD)
            .uri("/health")
            .body(Empty::<Bytes>::new())
            .unwrap();
        let response = health_check_handler(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_health_check_root_path_returns_404() {
        let req = Request::builder()
            .uri("/")
            .body(Empty::<Bytes>::new())
            .unwrap();
        let response = health_check_handler(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_health_check_various_paths_return_404() {
        let paths = vec!["/healthz", "/status", "/api/health", "/ready", "/healthcheck"];
        for path in paths {
            let req = Request::builder()
                .uri(path)
                .body(Empty::<Bytes>::new())
                .unwrap();
            let response = health_check_handler(req).await.unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "GET {} should return 404", path);
        }
    }

    #[tokio::test]
    async fn test_health_check_post_wrong_path_returns_404() {
        let req = Request::builder()
            .method(hyper::Method::POST)
            .uri("/wrong")
            .body(Empty::<Bytes>::new())
            .unwrap();
        let response = health_check_handler(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
