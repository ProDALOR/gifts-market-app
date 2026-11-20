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
#[utoipa::path(get,path="/readyz",tag="Health",summary="Market data readiness",responses((status=200,description="Market data is available"),(status=503,body=ErrorResponse)))]
pub(crate) async fn ready(state: web::Data<MarketApi>) -> Result<HttpResponse> {
    state.ready().await?;
    Ok(HttpResponse::Ok().finish())
}
#[utoipa::path(get,path="/catalog",tag="Catalog",summary="Collections, currencies and venues",description="Discover available gift markets.",responses((status=200,body=GiftCatalog,headers(("ETag"=String,description="Quoted opaque content token; matches catalog.versions.directory without the quotes"))),(status=304,description="Catalog content is unchanged"),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn catalog(state: web::Data<MarketApi>, req: HttpRequest) -> Result<HttpResponse> {
    Ok(catalog_response(
        state.catalog_snapshot().await?,
        &req,
        GiftCatalog::from,
    ))
}
#[utoipa::path(get,path="/collections/{id}",tag="Catalog",summary="Collection models, backdrops and patterns",description="Models, backdrops and patterns for one collection.",params(("id"=String,Path,description="Gift collection ID")),responses((status=200,body=GiftCollectionCatalog,headers(("ETag"=String,description="Quoted opaque content token; matches the collection entry in catalog.versions without the quotes"))),(status=304,description="Collection content is unchanged"),(status=400,body=ErrorResponse),(status=404,body=ErrorResponse),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
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
    let etag = format!("\"{}\"", crate::catalog::revision(&snapshot.revision));
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
#[utoipa::path(post,path="/valuations/gift",tag="Valuations",summary="Value a gift with one coherent floor report",description="Accepts a Telegram gift URL, slug, or gift object with collection_id and number, plus currency_id. Resolves gift facts and all metrics from one consistent market view. Returns collection, model, backdrop and matching model+backdrop floors; two-decimal dimensionless model/backdrop multipliers; and final_mark = round2(min(matching floor, collection floor * unrounded multipliers)). Rounding uses midpoint-to-even. Floor prices retain exact precision. Multipliers require at least three distinct identifiable gifts for the collection and selected trait. Each metric has value or a structured error; a resolved gift returns 200 even when some metrics are unavailable. A strategy-owned display projection supplies ordered fields, labels, formatted values, emphasis, summary and note for generic clients. Typed calculation fields remain in the response. One strategy, calculation time and stale flag apply to the whole report. The response slug is null for catalog references. Unknown/unlisted gifts return 404. No Telegram account or external lookup is used.",request_body=market_valuation::GiftValuationRequest,responses((status=200,body=GiftValuationResponse),(status=400,body=ErrorResponse),(status=404,body=ErrorResponse),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn gift_valuation(
    state: web::Data<MarketApi>,
    query: web::Json<market_valuation::GiftValuationRequest>,
) -> Result<HttpResponse> {
    let result = state.gift_valuation(query.into_inner()).await?;
    Ok(HttpResponse::Ok()
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .json(GiftValuationResponse::from(result.as_ref())))
}

#[utoipa::path(post,path="/valuations/query",tag="Valuations",summary="Value a collection, trait selection or gift with explicit strategies",description="Strategies are selected explicitly from /valuations/strategies. Targets are collection, model, backdrop, model_backdrop or gift. Execution estimators require explicit inclusive from and exclusive until bounds; at least three observations across three distinct gifts are required for collection/trait targets; an individual-gift target uses its own history. Mean weights execution records equally, independently of quantity. One report uses one pinned view; unavailable evidence or an unsupported target is reported per strategy. Available history is not guaranteed complete. Every typed result includes its strategy-owned display projection. Read budgets and cancellation are shared with the gift endpoint.",request_body=market_valuation::TargetValuationRequest,responses((status=200,body=crate::valuation::TargetValuationResponse),(status=400,body=ErrorResponse),(status=404,body=ErrorResponse),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn target_valuation(
    state: web::Data<MarketApi>,
    query: web::Json<market_valuation::TargetValuationRequest>,
) -> Result<HttpResponse> {
    let report = state.target_valuation(query.into_inner()).await?;
    Ok(HttpResponse::Ok()
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .json(crate::valuation::TargetValuationResponse::from(
            report.as_ref(),
        )))
}

#[utoipa::path(post,path="/orders/query",tag="Orders",summary="Read a latest or frozen ordered slice",description="Latest is the default and retains no view. Set view.kind to snapshot to open or reuse a snapshot lease. Ordering keys are portable across latest, frozen and WebSocket reads. The server enforces read budgets. Expired leases return restart_read.",request_body=GiftOrderQuery,responses((status=200,body=crate::orders::OrderReadResponse),(status=400,body=ErrorResponse),(status=409,body=ErrorResponse),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn read_orders(
    state: web::Data<MarketApi>,
    query: web::Json<GiftOrderQuery>,
) -> Result<HttpResponse> {
    let result = state.query_orders(query.into_inner()).await?;
    Ok(HttpResponse::Ok()
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .json(crate::orders::OrderReadResponse::from(&result)))
}
#[utoipa::path(delete,path="/orders/query",tag="Orders",summary="Release a frozen read",request_body=market_gifts::ReleaseReadSnapshot,responses((status=204,description="Released or already expired"),(status=400,body=ErrorResponse),(status=503,body=ErrorResponse)))]
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
    .route("/catalog", web::get().to(catalog))
    .route("/collections/{id}", web::get().to(collection))
    .service(
        web::resource("/orders/query")
            .route(web::post().to(read_orders))
            .route(web::delete().to(release_read)),
    )
    .route("/valuations/gift", web::post().to(gift_valuation))
    .route("/valuations/query", web::post().to(target_valuation))
    .service(
        web::resource("/media/{kind}/{id}/{variant:preview}")
            .wrap(actix_web::middleware::Compress::default())
            .route(web::get().to(crate::media::get)),
    )
    .route(
        "/media/{kind}/{id}/{variant}",
        web::get().to(crate::media::get),
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
        let etag = format!("\"{}\"", crate::catalog::revision("content"));
        for condition in [etag.clone(), format!("\"old\", W/{etag}"), "*".into()] {
            let request = actix_web::test::TestRequest::get()
                .insert_header((header::IF_NONE_MATCH, condition))
                .to_http_request();
            let response = catalog_response(snapshot(), &request, |()| -> serde_json::Value {
                panic!("Unchanged catalog must not clone models or build media URLs")
            });
            assert_eq!(response.status(), 304);
            assert_eq!(response.headers().get(header::ETAG).unwrap(), etag.as_str());
            assert_eq!(
                response.headers().get(header::CACHE_CONTROL).unwrap(),
                "no-cache"
            );
        }
        for condition in [None, Some(format!("\"catalog-v2-{}\"", "0".repeat(64)))] {
            let mut request = actix_web::test::TestRequest::get();
            if let Some(condition) = condition {
                request = request.insert_header((header::IF_NONE_MATCH, condition));
            }
            let response = catalog_response(
                snapshot(),
                &request.to_http_request(),
                |()| serde_json::json!({"models":[]}),
            );
            assert_eq!(response.status(), 200);
            assert_eq!(response.headers().get(header::ETAG).unwrap(), etag.as_str());
            let bytes = actix_web::body::to_bytes(response.into_body())
                .await
                .unwrap();
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
                serde_json::json!({"models":[]})
            );
        }
    }
}
