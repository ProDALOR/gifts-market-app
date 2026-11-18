//! WebSocket handshake, with session state and API work kept separate.
mod operations;
mod queue;
mod registry;
mod session;
use crate::{
    MarketApi, config::WebSocketLimits, connections::ConnectionSupervisor, error::ApiError,
    origin::OriginPolicy,
};
use actix_web::{HttpRequest, HttpResponse, web};
#[derive(Clone)]
pub(crate) struct WebSocketService {
    pub market: MarketApi,
    pub connections: ConnectionSupervisor,
    pub origins: OriginPolicy,
    pub limits: WebSocketLimits,
}
#[utoipa::path(get,path="/ws",tag="WebSocket",summary="Open the typed WebSocket API",description="Typed requests, order subscriptions and catalog watches share one connection. Interactive documentation and a console are available at /docs/ws. The AsyncAPI specification is at /asyncapi.json.",responses((status=101,description="WebSocket upgraded; Command and ServerFrame define the message protocol"),(status=400,body=crate::error::ErrorResponse),(status=403,body=crate::error::ErrorResponse),(status=429,body=crate::error::ErrorResponse),(status=503,body=crate::error::ErrorResponse)))]
pub(crate) async fn connect(
    req: HttpRequest,
    payload: web::Payload,
    service: Option<web::Data<WebSocketService>>,
) -> actix_web::Result<HttpResponse> {
    let service =
        service.ok_or_else(|| ApiError::unavailable("WebSocket service is not configured"))?;
    service.origins.check(&req)?;
    let (response, session, messages) = actix_ws::handle(&req, payload)?;
    let messages = messages
        .max_frame_size(service.limits.frame_bytes)
        .aggregate_continuations()
        .max_continuation_size(service.limits.frame_bytes);
    service.connections.spawn_local({
        let service = service.clone();
        move |stop| async move {
            session::run(
                session,
                messages,
                service.market.clone(),
                service.limits.clone(),
                stop,
            )
            .await
        }
    })?;
    Ok(response)
}
