//! HTTP deadlines and conditional requests, independent of domain handlers.
use actix_web::{
    HttpRequest,
    body::MessageBody,
    dev::{ServiceRequest, ServiceResponse},
    http::header,
    middleware::Next,
};
use std::time::Duration;

#[derive(Clone, Copy)]
pub(crate) struct Deadline(pub Duration);
/// Bound extraction and handler execution. Streaming bodies and upgraded sockets have
/// their own lifetime; returning the initial response completes this deadline.
pub(crate) async fn deadline<B: MessageBody + 'static>(
    request: ServiceRequest,
    next: Next<B>,
) -> actix_web::Result<ServiceResponse<B>> {
    let timeout = request.app_data::<Deadline>().map_or(
        crate::config::ApiLimits::default().request_timeout,
        |limit| limit.0,
    );
    match tokio::time::timeout(timeout, next.call(request)).await {
        Ok(result) => result,
        Err(_) => {
            // Keep request ownership with Actix routing. Framework errors also let
            // outer CORS middleware preserve its response headers.
            let mut error: actix_web::Error =
                crate::error::ApiError::timed_out("HTTP request timed out").into();
            error.add_response_mapper(crate::security::apply_headers);
            Err(error)
        }
    }
}

pub(crate) fn etag_matches(req: &HttpRequest, etag: &str) -> bool {
    let etag = etag.trim_start_matches("W/");
    req.headers()
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|value| value.trim().trim_start_matches("W/") == etag || value.trim() == "*")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{App, HttpResponse, middleware::from_fn, test, web};
    use tokio_util::sync::CancellationToken;

    #[actix_web::test]
    async fn handler_deadline_returns_structured_error_cancels_work_and_keeps_server_usable() {
        let cancelled = CancellationToken::new();
        let app = test::init_service(
            App::new()
                .app_data(Deadline(Duration::from_millis(20)))
                .app_data(web::Data::new(cancelled.clone()))
                .wrap(from_fn(deadline))
                .wrap(crate::security_headers())
                .route(
                    "/slow",
                    web::get().to(|cancelled: web::Data<CancellationToken>| async move {
                        let _cancel = cancelled.get_ref().clone().drop_guard();
                        std::future::pending::<HttpResponse>().await
                    }),
                )
                .route(
                    "/healthy",
                    web::get().to(|| async { HttpResponse::Ok().finish() }),
                ),
        )
        .await;
        let error =
            test::try_call_service(&app, test::TestRequest::get().uri("/slow").to_request())
                .await
                .expect_err("The deadline must end a blocked handler");
        let response = error.error_response();
        assert_eq!(response.status(), 503);
        assert!(response.headers().contains_key("x-content-type-options"));
        let bytes = actix_web::body::to_bytes(response.into_body())
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"]["code"], "timed_out");
        assert!(
            cancelled.is_cancelled(),
            "Timing out drops the handler future"
        );
        assert_eq!(
            test::call_service(&app, test::TestRequest::get().uri("/healthy").to_request())
                .await
                .status(),
            200
        );
    }

    #[actix_web::test]
    async fn completed_response_can_stream_beyond_the_handler_deadline() {
        let app = test::init_service(
            App::new()
                .app_data(Deadline(Duration::from_millis(10)))
                .wrap(from_fn(deadline))
                .route(
                    "/stream",
                    web::get().to(|| async {
                        HttpResponse::Ok().streaming(futures_util::stream::once(async {
                            tokio::time::sleep(Duration::from_millis(30)).await;
                            Ok::<_, std::io::Error>(web::Bytes::from_static(b"streamed"))
                        }))
                    }),
                ),
        )
        .await;
        let response =
            test::call_service(&app, test::TestRequest::get().uri("/stream").to_request()).await;
        assert_eq!(response.status(), 200);
        assert_eq!(test::read_body(response).await, "streamed");
    }
}
