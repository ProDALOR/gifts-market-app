use super::{
    MarketApi,
    error::{ApiError, ErrorResponse, Result},
    models::*,
};
use actix_web::{HttpResponse, web};
use serde::Deserialize;
#[utoipa::path(get,path="/healthz",responses((status=200,description="Process is running")))]
pub(crate) async fn health() -> HttpResponse {
    HttpResponse::Ok().finish()
}
#[utoipa::path(get,path="/readyz",responses((status=200,description="A complete market replica is available"),(status=503,body=ErrorResponse)))]
pub(crate) async fn ready(state: web::Data<MarketApi>) -> Result<HttpResponse> {
    state.ready().await?;
    Ok(HttpResponse::Ok().finish())
}
#[utoipa::path(get,path="/sync",responses((status=200,body=SyncInfo)))]
pub(crate) async fn sync(state: web::Data<MarketApi>) -> Result<web::Json<SyncInfo>> {
    Ok(web::Json(state.sync().await?))
}
#[derive(Deserialize)]
pub(crate) struct BooksParams {
    #[serde(default)]
    limit: u32,
    after: Option<String>,
}
#[utoipa::path(get,path="/books",params(("limit"=Option<u32>,Query),("after"=Option<String>,Query,description="JSON BookKey returned in next_after")),responses((status=200,body=BooksPage),(status=400,body=ErrorResponse),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn books(
    state: web::Data<MarketApi>,
    q: web::Query<BooksParams>,
) -> Result<web::Json<BooksPage>> {
    let after = q
        .after
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| ApiError::invalid("invalid book continuation"))?;
    Ok(web::Json(
        state
            .books(BooksQuery {
                limit: q.limit,
                after,
            })
            .await?,
    ))
}
#[utoipa::path(post,path="/catalog/query",request_body=CatalogQuery,responses((status=200,body=Page<CatalogEntry>),(status=400,body=ErrorResponse),(status=413,body=ErrorResponse),(status=415,body=ErrorResponse),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn catalog(
    state: web::Data<MarketApi>,
    q: web::Json<CatalogQuery>,
) -> Result<web::Json<Page<CatalogEntry>>> {
    Ok(web::Json(state.catalog(q.into_inner()).await?))
}
#[derive(Deserialize)]
pub(crate) struct CatalogParams {
    #[serde(default = "crate::models::default_limit")]
    limit: u32,
    #[serde(default)]
    page_token: String,
}
#[utoipa::path(get,path="/catalog/{kind}",params(("kind"=CatalogKind,Path),("limit"=Option<u32>,Query),("page_token"=Option<String>,Query)),responses((status=200,body=Page<CatalogEntry>),(status=400,body=ErrorResponse),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn list_catalog(
    state: web::Data<MarketApi>,
    kind: web::Path<CatalogKind>,
    q: web::Query<CatalogParams>,
) -> Result<web::Json<Page<CatalogEntry>>> {
    Ok(web::Json(
        state
            .catalog(CatalogQuery {
                kind: kind.into_inner(),
                ids: Vec::new(),
                limit: q.limit,
                page_token: q.page_token.clone(),
            })
            .await?,
    ))
}
#[utoipa::path(post,path="/orders/query",request_body=OrdersQuery,responses((status=200,body=Page<Order>),(status=400,body=ErrorResponse),(status=409,body=ErrorResponse),(status=413,body=ErrorResponse),(status=415,body=ErrorResponse),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn orders(
    state: web::Data<MarketApi>,
    q: web::Json<OrdersQuery>,
) -> Result<web::Json<Page<Order>>> {
    Ok(web::Json(state.orders(q.into_inner()).await?))
}
#[utoipa::path(post,path="/executions/query",request_body=ExecutionsQuery,responses((status=200,body=Page<Order>),(status=400,body=ErrorResponse),(status=409,body=ErrorResponse),(status=413,body=ErrorResponse),(status=415,body=ErrorResponse),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn executions(
    state: web::Data<MarketApi>,
    q: web::Json<ExecutionsQuery>,
) -> Result<web::Json<Page<Order>>> {
    Ok(web::Json(state.executions(q.into_inner()).await?))
}
pub fn configure(cfg: &mut web::ServiceConfig) {
    configure_with_limit(cfg, crate::config::ApiLimits::default().json_bytes);
}
pub(crate) fn configure_with_limit(cfg: &mut web::ServiceConfig, json_bytes: usize) {
    crate::error::configure_extractors(cfg, json_bytes);
    cfg.route("/healthz", web::get().to(health))
        .route("/readyz", web::get().to(ready))
        .route("/sync", web::get().to(sync))
        .route("/books", web::get().to(books))
        .route("/catalog/query", web::post().to(catalog))
        .route("/catalog/{kind}", web::get().to(list_catalog))
        .route("/orders/query", web::post().to(orders))
        .route("/executions/query", web::post().to(executions))
        .route("/ws", web::get().to(crate::websocket::connect))
        .route("/openapi.json", web::get().to(crate::schema::schema));
    #[cfg(feature = "miniapp")]
    cfg.configure(crate::miniapp::configure);
}
