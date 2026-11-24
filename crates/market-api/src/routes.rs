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
#[utoipa::path(post,path="/valuations/query",tag="Valuations",summary="Evaluate explicit valuation strategies",description="Choose explicit strategy IDs from /valuations/strategies using their input contracts, supported targets and period requirements. Send a batched catalog target, unified selection with collection_id and criteria, or gift lookup; the service resolves domain facts once in one pinned read. The same query can be subscribed with valuation.watch. Cached reports and collection/currency floors are held for at least 60 seconds; changes and listing expiry are combined until the next refresh. Availability changes update the status independently. Cold selections calculate on demand. Unknown or repeated IDs, legacy targets and periods unused by every selected strategy fail the request. Unsupported targets and calculation failures are reported independently per strategy; missing evidence remains a typed metric issue. Each result contains typed data or an independent strategy error and a definition revision. Static display fields, formatting and notes are discovered once from /valuations/strategies; evidence stays in the typed data. Follow discovery documentation links for formulas and sample requirements. HTTP and WebSocket use the same handler, read budgets and cancellation.",request_body=market_valuation::ValuationQuery,responses((status=200,body=crate::valuation::ValuationResponse),(status=400,body=ErrorResponse),(status=404,body=ErrorResponse),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn valuation(
    state: web::Data<MarketApi>,
    query: web::Json<market_valuation::ValuationQuery>,
) -> Result<HttpResponse> {
    let report = state.valuation(query.into_inner()).await?;
    let body = state
        .encode_response(move |writer| {
            serde_json::to_writer(
                writer,
                &crate::valuation::ValuationResponse::from(report.as_ref()),
            )
        })
        .await?;
    Ok(HttpResponse::Ok()
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .content_type("application/json")
        .body(body))
}
#[utoipa::path(post,path="/orders/query",tag="Orders",summary="Read a latest or frozen ordered slice",description="Latest is the default and retains no view. Set view.kind to snapshot to open or reuse a snapshot lease. Ordering keys are portable across latest, frozen and WebSocket reads. The server enforces read budgets. Expired leases return restart_read.",request_body=GiftOrderQuery,responses((status=200,body=crate::orders::OrderReadResponse),(status=400,body=ErrorResponse),(status=409,body=ErrorResponse),(status=429,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub(crate) async fn read_orders(
    state: web::Data<MarketApi>,
    query: web::Json<GiftOrderQuery>,
) -> Result<HttpResponse> {
    let result = state.query_orders(query.into_inner()).await?;
    let body = state
        .encode_response(move |writer| {
            serde_json::to_writer(writer, &crate::orders::OrderReadResponse::from(&result))
        })
        .await?;
    Ok(HttpResponse::Ok()
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .content_type("application/json")
        .body(body))
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
    .route(
        "/portfolios/{user_id}",
        web::get().to(crate::portfolio::query),
    )
    .route(
        "/portfolios/{user_id}/refresh",
        web::post().to(crate::portfolio::refresh),
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
    .route("/valuations/query", web::post().to(valuation))
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
