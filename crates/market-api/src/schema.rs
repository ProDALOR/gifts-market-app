use crate::models::*;
use actix_web::HttpResponse;
use utoipa::OpenApi;
#[derive(OpenApi)]
#[openapi(
    paths(
        crate::routes::catalog,
        crate::routes::collection,
        crate::routes::valuation,
        crate::valuation::documentation,
        crate::valuation::strategies,
        crate::routes::read_orders,
        crate::routes::release_read,
        crate::media::get,
        crate::websocket::connect
    ),
    tags(
        (name="Catalog",description="Discover gift collections and their traits."),
        (name="Orders",description="Asks, bids and confirmed executions with exact prices and stable pagination."),
        (name="Valuations",description="Catalog, collection, trait and gift targets with explicitly selected typed strategies, coherent reports and independent evidence/errors."),
        (name="Media",description="Cached gift thumbnails, first-frame SVGs, original TGS animations and transparent loading silhouettes."),
        (name="WebSocket",description="See /docs/ws for live-message documentation and a test console.")
    ),
    components(schemas(Command, ServerFrame, crate::media::MediaCategory, crate::media::MediaVariant)),
    info(title = "Market App", description = "Browse gift markets and calculate estimates. Start with /catalog for collection and currency IDs, then use /collections/{id} for model, backdrop and pattern choices. Prices are exact decimal strings. One orders/query model covers asks, bids and history; the connection-level market.status message reports market availability. Live message documentation and an interactive subscription console are available at /docs/ws. Public market reads do not require sign-in.")
)]
struct MarketDoc;
pub struct ApiDoc;
impl OpenApi for ApiDoc {
    fn openapi() -> utoipa::openapi::OpenApi {
        let doc = MarketDoc::openapi();
        #[cfg(feature = "miniapp")]
        let doc = {
            let mut doc = doc;
            doc.merge(crate::miniapp::MiniAppDoc::openapi());
            doc
        };
        doc
    }
}
fn encoded(
    cache: &'static std::sync::OnceLock<actix_web::web::Bytes>,
    build: fn() -> serde_json::Result<String>,
) -> actix_web::web::Bytes {
    cache
        .get_or_init(|| build().expect("public schema serializes").into())
        .clone()
}
pub(crate) async fn schema() -> HttpResponse {
    static SCHEMA: std::sync::OnceLock<actix_web::web::Bytes> = std::sync::OnceLock::new();
    HttpResponse::Ok()
        .content_type("application/json")
        .body(encoded(&SCHEMA, crate::schema_json))
}
pub(crate) async fn async_schema() -> HttpResponse {
    static SCHEMA: std::sync::OnceLock<actix_web::web::Bytes> = std::sync::OnceLock::new();
    HttpResponse::Ok()
        .content_type("application/json")
        .body(encoded(&SCHEMA, crate::asyncapi_json))
}
