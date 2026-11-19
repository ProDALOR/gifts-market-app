use crate::models::*;
use actix_web::HttpResponse;
use utoipa::OpenApi;
#[derive(OpenApi)]
#[openapi(
    paths(
        crate::routes::health,
        crate::routes::ready,
        crate::routes::sync,
        crate::routes::catalog,
        crate::routes::collection,
        crate::routes::gift_valuation,
        crate::valuation::documentation,
        crate::valuation::strategies,
        crate::routes::read_orders,
        crate::routes::release_read,
        crate::media::model_thumbnail,
        crate::media::pattern_thumbnail,
        crate::media::model_animation,
        crate::websocket::connect
    ),
    tags(
        (name="Catalog",description="Discover gift collections and their traits."),
        (name="Orders",description="Current asks and bids with exact prices and keyset pagination."),
        (name="History",description="Confirmed executions. Disappearing listings do not imply sales."),
        (name="Valuations",description="One coherent floor valuation for a gift URL, slug or catalog identity, with typed evidence and per-metric errors."),
        (name="Media",description="Cached Telegram gift thumbnails, animations and patterns."),
        (name="Health",description="Process health, replica readiness and freshness."),
        (name="WebSocket",description="See /docs/ws for live-message documentation and a test console.")
    ),
    components(schemas(Command, ServerFrame)),
    info(title = "Market App", description = "Read-only gift markets served from a synchronized local replica. Start with /catalog for collection and currency IDs, then use /collections/{id} for model, backdrop and pattern choices. Prices are exact decimal strings. One orders/query model covers asks, bids and history; /sync and the connection-level replica.status message report freshness. Live message documentation and an interactive subscription console are available at /docs/ws. Public market reads need no hub credentials or Telegram login.")
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
