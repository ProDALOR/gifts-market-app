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
#[utoipa::path(get,path="/sync",tag="Health",summary="Synchronization status",responses((status=200,body=SyncStatus),(status=503,body=ErrorResponse)))]
pub(crate) async fn sync(state: web::Data<MarketApi>) -> Result<web::Json<SyncStatus>> {
    Ok(web::Json(state.sync().await?))
}
#[utoipa::path(get,path="/catalog",tag="Catalog",summary="Collections, currencies and venues",description="Catalog data. Use GET /sync for replica readiness and freshness.",responses((status=200,body=GiftCatalog,headers(("ETag"=String,description="Catalog content revision"))),(status=304,description="Catalog content is unchanged"),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn catalog(state: web::Data<MarketApi>, req: HttpRequest) -> Result<HttpResponse> {
    Ok(catalog_response(
        state.catalog_snapshot().await?,
        &req,
        GiftCatalog::from,
    ))
}
#[utoipa::path(get,path="/collections/{id}",tag="Catalog",summary="Collection models, backdrops and patterns",description="Collection choices. Use GET /sync for replica readiness and freshness.",params(("id"=String,Path,description="Gift collection ID")),responses((status=200,body=GiftCollectionCatalog,headers(("ETag"=String,description="Collection content revision"))),(status=304,description="Collection content is unchanged"),(status=400,body=ErrorResponse),(status=404,body=ErrorResponse),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn collection(
    state: web::Data<MarketApi>,
    id: web::Path<CollectionId>,
    req: HttpRequest,
) -> Result<HttpResponse> {
    Ok(catalog_response(
        state.collection_snapshot(id.into_inner()).await?,
        &req,
        GiftCollectionCatalog::from,
    ))
}
fn catalog_response<T, U: serde::Serialize>(
    snapshot: market_actors::CatalogSnapshot<T>,
    req: &HttpRequest,
    present: impl FnOnce(T) -> U,
) -> HttpResponse {
    let etag = format!("\"{}\"", crate::catalog::revision(snapshot.revision));
    let unchanged = crate::http::etag_matches(req, &etag);
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
        response.json(present(snapshot.data))
    }
}
#[utoipa::path(post,path="/orders/query",tag="Orders",summary="Query current asks or bids",request_body=GiftOrdersQuery,responses((status=200,body=OrderPageResponse),(status=400,body=ErrorResponse),(status=409,body=ErrorResponse),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn orders(
    state: web::Data<MarketApi>,
    query: web::Json<GiftOrdersQuery>,
) -> Result<HttpResponse> {
    let page = state.orders(query.into_inner()).await?;
    Ok(HttpResponse::Ok().json(OrderPageResponse::from(&page)))
}
#[utoipa::path(post,path="/history/query",tag="History",summary="Query confirmed executions",request_body=GiftHistoryQuery,responses((status=200,body=OrderPageResponse),(status=400,body=ErrorResponse),(status=409,body=ErrorResponse),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn executions(
    state: web::Data<MarketApi>,
    query: web::Json<GiftHistoryQuery>,
) -> Result<HttpResponse> {
    let page = state.executions(query.into_inner()).await?;
    Ok(HttpResponse::Ok().json(OrderPageResponse::from(&page)))
}
#[utoipa::path(post,path="/valuations/gift",tag="Valuations",summary="Value a gift with one coherent floor report",description="Accepts a Telegram gift URL, slug, or gift object with collection_id and number, plus currency_id. Resolves gift facts and all metrics from one pinned replica view. Returns collection, model, backdrop and matching model+backdrop floors; two-decimal dimensionless model/backdrop multipliers; and final_mark = round2(min(matching floor, collection floor * unrounded multipliers)). Rounding uses midpoint-to-even. Floor prices retain exact precision. Multipliers require at least three eligible listing rows for the collection and selected trait. Each metric has value or a structured error; a resolved gift returns 200 even when some metrics are unavailable. A strategy-owned display projection supplies ordered fields, labels, formatted values, emphasis, summary and note for generic clients. Typed calculation fields remain in the response. One strategy, revision, evaluated_at and sync status apply to the whole report. The response slug is null for catalog references. Unknown/unlisted gifts return 404. No Telegram account or external lookup is used.",request_body=market_valuation::GiftValuationRequest,responses((status=200,body=GiftValuationResponse),(status=400,body=ErrorResponse),(status=404,body=ErrorResponse),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn gift_valuation(
    state: web::Data<MarketApi>,
    query: web::Json<market_valuation::GiftValuationRequest>,
) -> Result<HttpResponse> {
    let result = state.gift_valuation(query.into_inner()).await?;
    Ok(HttpResponse::Ok()
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .json(GiftValuationResponse::from(result.as_ref())))
}

#[utoipa::path(post,path="/orders/read",tag="Orders",summary="Read a latest or frozen ordered slice",description="Latest is the default and retains no view. Set consistency to snapshot to open or reuse a snapshot lease. Positions are portable across latest, frozen and WebSocket reads. The server enforces read budgets. Expired leases return restart_read.",request_body=market_gifts::OrderReadQuery,responses((status=200,body=crate::orders::OrderReadResponse),(status=400,body=ErrorResponse),(status=409,body=ErrorResponse),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn read_orders(
    state: web::Data<MarketApi>,
    query: web::Json<market_gifts::OrderReadQuery>,
) -> Result<HttpResponse> {
    let result = state.read_orders(query.into_inner()).await?;
    Ok(HttpResponse::Ok()
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .json(crate::orders::OrderReadResponse::from(&result)))
}
#[utoipa::path(delete,path="/orders/read",tag="Orders",summary="Release a frozen read",request_body=market_gifts::ReleaseReadSnapshot,responses((status=204,description="Released or already expired"),(status=400,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn release_read(
    state: web::Data<MarketApi>,
    query: web::Json<market_gifts::ReleaseReadSnapshot>,
) -> Result<HttpResponse> {
    state
        .release_read_snapshot(query.into_inner().snapshot)
        .await?;
    Ok(HttpResponse::NoContent().finish())
}

/// Register endpoints in an embedding Actix app. The embedder owns transport policy
/// and middleware; `ApiServer` installs deadlines, origins and security headers.
pub fn configure(cfg: &mut web::ServiceConfig) {
    configure_with_limit(cfg, crate::config::ApiLimits::default().json_bytes);
}
pub(crate) fn configure_with_limit(cfg: &mut web::ServiceConfig, json_bytes: usize) {
    crate::error::configure_extractors(cfg, json_bytes);
    cfg.route(
        "/valuations/strategies",
        web::get().to(crate::valuation::strategies),
    )
    .route(
        "/valuations/strategies/{strategy}/documentation",
        web::get().to(crate::valuation::documentation),
    )
    .route("/healthz", web::get().to(health))
    .route("/readyz", web::get().to(ready))
    .route("/sync", web::get().to(sync))
    .route("/catalog", web::get().to(catalog))
    .route("/collections/{id}", web::get().to(collection))
    .route("/orders/query", web::post().to(orders))
    .service(
        web::resource("/orders/read")
            .route(web::post().to(read_orders))
            .route(web::delete().to(release_read)),
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    #[actix_web::test]
    async fn catalog_revalidation_skips_public_projection_and_preserves_response_headers() {
        let snapshot = || market_actors::CatalogSnapshot {
            data: (),
            revision: "content".into(),
        };
        for condition in [
            "\"catalog-v2-content\"",
            "\"old\", W/\"catalog-v2-content\"",
            "*",
        ] {
            let request = actix_web::test::TestRequest::get()
                .insert_header((header::IF_NONE_MATCH, condition))
                .to_http_request();
            let response = catalog_response(snapshot(), &request, |()| -> serde_json::Value {
                panic!("Unchanged catalog must not clone models or build media URLs")
            });
            assert_eq!(response.status(), 304);
            assert_eq!(
                response.headers().get(header::ETAG).unwrap(),
                "\"catalog-v2-content\""
            );
            assert_eq!(
                response.headers().get(header::CACHE_CONTROL).unwrap(),
                "no-cache"
            );
        }
        let request = actix_web::test::TestRequest::get().to_http_request();
        let response =
            catalog_response(snapshot(), &request, |()| serde_json::json!({"models":[]}));
        assert_eq!(response.status(), 200);
        let bytes = actix_web::body::to_bytes(response.into_body())
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
            serde_json::json!({"models":[]})
        );
    }
}
