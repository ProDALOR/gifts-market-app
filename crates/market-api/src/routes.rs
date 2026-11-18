use super::{
    MarketApi,
    error::{ErrorResponse, Result},
    models::*,
};
use actix_web::{HttpRequest, HttpResponse, http::header, web};
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
#[utoipa::path(get,path="/catalog",tag="Catalog",summary="Collections, currencies and venues",description="Catalog data. Use GET /sync for replica readiness and freshness.",responses((status=200,body=GiftCatalog,headers(("ETag"=String,description="Catalog content revision"))),(status=304,description="Catalog content is unchanged"),(status=503,body=ErrorResponse)))]
pub(crate) async fn catalog(state: web::Data<MarketApi>, req: HttpRequest) -> Result<HttpResponse> {
    Ok(catalog_response(state.catalog_snapshot().await?, &req))
}
#[utoipa::path(get,path="/collections/{id}",tag="Catalog",summary="Collection models, backdrops and patterns",description="Collection choices. Use GET /sync for replica readiness and freshness.",params(("id"=String,Path,description="Gift collection ID")),responses((status=200,body=GiftCollectionCatalog,headers(("ETag"=String,description="Collection content revision"))),(status=304,description="Collection content is unchanged"),(status=400,body=ErrorResponse),(status=404,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn collection(
    state: web::Data<MarketApi>,
    id: web::Path<CollectionId>,
    req: HttpRequest,
) -> Result<HttpResponse> {
    Ok(catalog_response(
        state.collection_snapshot(id.into_inner()).await?,
        &req,
    ))
}
fn catalog_response<T: serde::Serialize>(
    snapshot: market_actors::CatalogSnapshot<T>,
    req: &HttpRequest,
) -> HttpResponse {
    let etag = format!("\"{}\"", snapshot.revision);
    let unchanged = req
        .headers()
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|value| value.trim().trim_start_matches("W/") == etag || value.trim() == "*")
        });
    let mut response = if unchanged {
        HttpResponse::NotModified()
    } else {
        HttpResponse::Ok()
    };
    response
        .insert_header((header::ETAG, etag))
        .insert_header((header::CACHE_CONTROL, "no-cache"));
    if unchanged {
        response.finish()
    } else {
        response.json(snapshot.data)
    }
}
#[utoipa::path(post,path="/orders/query",tag="Orders",summary="Query current asks or bids",request_body=GiftOrdersQuery,responses((status=200,body=OrderPageResponse),(status=400,body=ErrorResponse),(status=409,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn orders(
    state: web::Data<MarketApi>,
    query: web::Json<GiftOrdersQuery>,
) -> Result<HttpResponse> {
    let page = state.orders(query.into_inner()).await?;
    Ok(HttpResponse::Ok().json(OrderPageResponse::from(&page)))
}
#[utoipa::path(post,path="/history/query",tag="History",summary="Query confirmed executions",request_body=GiftHistoryQuery,responses((status=200,body=OrderPageResponse),(status=400,body=ErrorResponse),(status=409,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn executions(
    state: web::Data<MarketApi>,
    query: web::Json<GiftHistoryQuery>,
) -> Result<HttpResponse> {
    let page = state.executions(query.into_inner()).await?;
    Ok(HttpResponse::Ok().json(OrderPageResponse::from(&page)))
}
#[utoipa::path(post,path="/valuations/gift",tag="Valuations",summary="Value a gift with one coherent floor report",description="Accepts a Telegram gift URL, slug, or gift object with collection_id and number, plus currency_id. Resolves gift facts and all metrics from one pinned replica view. Returns collection, model, backdrop and matching model+backdrop floors; two-decimal dimensionless model/backdrop multipliers; and final_mark = round2(min(matching floor, collection floor * unrounded multipliers)). Rounding uses midpoint-to-even. Floor prices retain exact precision. Multipliers require at least three eligible listing rows for the collection and selected trait. Each metric has value or a structured error; a resolved gift returns 200 even when some metrics are unavailable. One strategy, revision, evaluated_at and sync status apply to the whole report. The response slug is null for catalog references. Unknown/unlisted gifts return 404. No Telegram account or external lookup is used.",request_body=market_valuation::GiftValuationRequest,responses((status=200,body=GiftValuationResponse),(status=400,body=ErrorResponse),(status=404,body=ErrorResponse),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn gift_valuation(
    state: web::Data<MarketApi>,
    query: web::Json<market_valuation::GiftValuationRequest>,
) -> Result<HttpResponse> {
    let result = state.gift_valuation(query.into_inner()).await?;
    Ok(HttpResponse::Ok()
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .json(GiftValuationResponse::from(result.as_ref())))
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
        .route("/valuations/gift", web::post().to(gift_valuation))
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
