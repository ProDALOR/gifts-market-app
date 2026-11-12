//! Read-only HTTP transport for the shared local query service.
use crate::{
    error::{ApiError, ErrorCode, ErrorResponse, Result},
    query::*,
    status::{ResponseMeta, StatusData},
};
use actix_web::{HttpResponse, web};
use serde::{Deserialize, Serialize};
use utoipa::{OpenApi, ToSchema};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoQuery {}
#[derive(Serialize, ToSchema)]
struct Liveness {
    alive: bool,
}
#[utoipa::path(get,path="/healthz",responses((status=200,body=Liveness)))]
async fn healthz(_: web::Query<NoQuery>) -> HttpResponse {
    HttpResponse::Ok().json(Liveness { alive: true })
}
#[utoipa::path(get,path="/status",responses((status=200,body=StatusResponse)))]
async fn status(state: web::Data<ApiState>, _: web::Query<NoQuery>) -> web::Json<StatusResponse> {
    web::Json(state.status())
}
#[utoipa::path(get,path="/readyz",responses((status=200,body=StatusResponse),(status=503,body=StatusResponse)))]
async fn readyz(state: web::Data<ApiState>, _: web::Query<NoQuery>) -> HttpResponse {
    let response = state.status();
    if response.data.ready {
        HttpResponse::Ok().json(response)
    } else {
        HttpResponse::ServiceUnavailable().json(response)
    }
}
#[utoipa::path(get,path="/catalog",params(CatalogQuery),responses((status=200,body=CatalogResponse),(status=400,body=ErrorResponse),(status=409,body=ErrorResponse),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
async fn catalog(
    state: web::Data<ApiState>,
    query: web::Query<CatalogQuery>,
) -> Result<web::Json<CatalogResponse>> {
    Ok(web::Json(state.catalog(&query)?))
}
#[utoipa::path(get,path="/items",params(ItemsQuery),responses((status=200,body=ItemsResponse),(status=400,body=ErrorResponse),(status=409,body=ErrorResponse),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
async fn items(
    state: web::Data<ApiState>,
    query: web::Query<ItemsQuery>,
) -> Result<web::Json<ItemsResponse>> {
    Ok(web::Json(state.items(&query)?))
}
#[utoipa::path(get,path="/feeds",params(FeedsQuery),responses((status=200,body=FeedsResponse),(status=400,body=ErrorResponse),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
async fn feeds(
    state: web::Data<ApiState>,
    query: web::Query<FeedsQuery>,
) -> Result<web::Json<FeedsResponse>> {
    Ok(web::Json(state.feeds(&query)?))
}
#[utoipa::path(get,path="/orders",params(OrdersQuery),responses((status=200,body=OrdersResponse),(status=400,body=ErrorResponse),(status=409,body=ErrorResponse),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
async fn orders(
    state: web::Data<ApiState>,
    query: web::Query<OrdersQuery>,
) -> Result<web::Json<OrdersResponse>> {
    Ok(web::Json(state.orders(&query)?))
}
#[utoipa::path(get,path="/orderbook",params(BookQuery),responses((status=200,body=BookResponse),(status=400,body=ErrorResponse),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
async fn orderbook(
    state: web::Data<ApiState>,
    query: web::Query<BookQuery>,
) -> Result<web::Json<BookResponse>> {
    Ok(web::Json(state.orderbook(&query)?))
}
#[utoipa::path(get,path="/openapi.json",responses((status=200,description="OpenAPI document")))]
async fn openapi(_: web::Query<NoQuery>) -> web::Json<utoipa::openapi::OpenApi> {
    web::Json(ApiDoc::openapi())
}
#[derive(OpenApi)]
#[openapi(
    paths(
        healthz,
        status,
        readyz,
        catalog,
        items,
        feeds,
        orders,
        orderbook,
        crate::websocket::connect,
        openapi
    ),
    components(schemas(
        Liveness,
        StatusData,
        ResponseMeta,
        StatusResponse,
        CatalogResponse,
        ItemsResponse,
        FeedsResponse,
        OrdersResponse,
        BookResponse,
        BookData,
        BookId,
        OrderPage,
        Orders,
        Items,
        CatalogEntries,
        Page,
        Included,
        crate::status::ApiCursor,
        crate::status::ReplicaHealth,
        crate::status::SyncState,
        market_json::schema::Item,
        market_json::schema::Order,
        market_json::schema::FeedStatus,
        market_json::schema::CatalogEntry,
        market_json::schema::Record,
        market_json::schema::OrderSide,
        ErrorResponse
    )),
    info(title = "Market API", version = "0.1.0")
)]
pub struct ApiDoc;
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.app_data(
        web::QueryConfig::default().error_handler(|error, _| ApiError::invalid_query(error).into()),
    );
    macro_rules! read {
        ($path:literal,$handler:expr) => {
            cfg.service(
                web::resource($path)
                    .route(web::get().to($handler))
                    .route(web::head().to($handler))
                    .default_service(web::to(read_only)),
            );
        };
    }
    read!("/healthz", healthz);
    read!("/readyz", readyz);
    read!("/status", status);
    read!("/catalog", catalog);
    read!("/items", items);
    read!("/feeds", feeds);
    read!("/ws", crate::websocket::connect);
    read!("/orders", orders);
    read!("/orderbook", orderbook);
    read!("/openapi.json", openapi);
    cfg.default_service(web::to(fallback));
}
async fn read_only() -> Result<HttpResponse> {
    Err(ApiError::new(ErrorCode::ReadOnly, "API is read-only"))
}
async fn fallback(request: actix_web::HttpRequest) -> Result<HttpResponse> {
    if matches!(
        *request.method(),
        actix_web::http::Method::GET | actix_web::http::Method::HEAD
    ) {
        Err(ApiError::new(ErrorCode::NotFound, "route not found"))
    } else {
        read_only().await
    }
}
