use super::{
    MarketApi,
    error::{ApiError, ErrorResponse, Result},
    models::*,
};
use actix_web::{HttpResponse, web};
#[utoipa::path(get,path="/healthz",tag="Health",summary="Process health",responses((status=200,description="Process is running")))]
pub(crate) async fn health() -> HttpResponse {
    HttpResponse::Ok().finish()
}
#[utoipa::path(get,path="/readyz",tag="Health",summary="Replica readiness",responses((status=200,description="A complete gift replica is available"),(status=503,body=ErrorResponse)))]
pub(crate) async fn ready(state: web::Data<MarketApi>) -> Result<HttpResponse> {
    state.ready().await?;
    Ok(HttpResponse::Ok().finish())
}
#[utoipa::path(get,path="/sync",tag="Health",summary="Synchronization status",responses((status=200,body=SyncStatus)))]
pub(crate) async fn sync(state: web::Data<MarketApi>) -> Result<web::Json<SyncStatus>> {
    Ok(web::Json(state.sync().await?))
}
#[utoipa::path(get,path="/catalog",tag="Catalog",summary="Collections, currencies and venues",responses((status=200,body=GiftCatalog),(status=503,body=ErrorResponse)))]
pub(crate) async fn catalog(state: web::Data<MarketApi>) -> Result<web::Json<GiftCatalog>> {
    Ok(web::Json(state.catalog().await?))
}
#[utoipa::path(get,path="/collections/{id}",tag="Catalog",summary="Collection models, backdrops and patterns",params(("id"=String,Path,description="Gift collection ID")),responses((status=200,body=GiftCollectionCatalog),(status=404,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn collection(
    state: web::Data<MarketApi>,
    id: web::Path<String>,
) -> Result<web::Json<GiftCollectionCatalog>> {
    let id =
        CollectionId::new(id.into_inner()).map_err(|error| ApiError::invalid(error.to_string()))?;
    Ok(web::Json(state.collection(id).await?))
}
#[utoipa::path(post,path="/orders/query",tag="Orders",summary="Query current asks or bids",request_body=GiftOrdersQuery,responses((status=200,body=Page<GiftOrder>),(status=400,body=ErrorResponse),(status=409,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn orders(
    state: web::Data<MarketApi>,
    query: web::Json<GiftOrdersQuery>,
) -> Result<web::Json<Page<GiftOrder>>> {
    Ok(web::Json(state.orders(query.into_inner()).await?))
}
#[utoipa::path(post,path="/history/query",tag="History",summary="Query confirmed executions",request_body=GiftHistoryQuery,responses((status=200,body=Page<GiftOrder>),(status=400,body=ErrorResponse),(status=409,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn executions(
    state: web::Data<MarketApi>,
    query: web::Json<GiftHistoryQuery>,
) -> Result<web::Json<Page<GiftOrder>>> {
    Ok(web::Json(state.executions(query.into_inner()).await?))
}
#[utoipa::path(post,path="/portfolio/value",tag="Portfolio",summary="Value a model portfolio at current ask floors",request_body=market_gifts::valuation::ValuationRequest,responses((status=200,body=market_gifts::valuation::Valuation),(status=400,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn portfolio(
    state: web::Data<MarketApi>,
    query: web::Json<market_gifts::valuation::ValuationRequest>,
) -> Result<web::Json<market_gifts::valuation::Valuation>> {
    Ok(web::Json(state.value_models(query.into_inner()).await?))
}
pub fn configure(cfg: &mut web::ServiceConfig) {
    configure_with_limit(cfg, crate::config::ApiLimits::default().json_bytes);
}
pub(crate) fn configure_with_limit(cfg: &mut web::ServiceConfig, json_bytes: usize) {
    crate::error::configure_extractors(cfg, json_bytes);
    cfg.route("/healthz", web::get().to(health))
        .route("/readyz", web::get().to(ready))
        .route("/sync", web::get().to(sync))
        .route("/catalog", web::get().to(catalog))
        .route("/collections/{id}", web::get().to(collection))
        .route("/orders/query", web::post().to(orders))
        .route("/history/query", web::post().to(executions))
        .route("/portfolio/value", web::post().to(portfolio))
        .route(
            "/media/models/{id}",
            web::get().to(crate::media::model_thumbnail),
        )
        .route(
            "/media/models/{id}/animation",
            web::get().to(crate::media::model_animation),
        )
        .route(
            "/media/patterns/{id}",
            web::get().to(crate::media::pattern_thumbnail),
        )
        .route("/ws", web::get().to(crate::websocket::connect))
        .route("/openapi.json", web::get().to(crate::schema::schema))
        .route("/asyncapi.json", web::get().to(crate::schema::async_schema));
    #[cfg(feature = "miniapp")]
    cfg.configure(crate::miniapp::configure);
}
