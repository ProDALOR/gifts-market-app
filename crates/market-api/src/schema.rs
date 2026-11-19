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
        crate::routes::orders,
        crate::routes::read_orders,
        crate::routes::release_read,
        crate::routes::executions,
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
    info(title = "Market App", description = "Read-only gift markets served from a synchronized local replica. Start with /catalog for collection and currency IDs, then use /collections/{id} for model, backdrop and pattern choices. Prices are exact decimal strings; paged reads include synchronization status. Live message documentation and an interactive subscription console are available at /docs/ws. Public market reads need no hub credentials or Telegram login.")
)]
struct MarketDoc;
pub struct ApiDoc;
impl OpenApi for ApiDoc {
    fn openapi() -> utoipa::openapi::OpenApi {
        let mut doc = MarketDoc::openapi();
        // Publish client-relevant syntax constraints; execution budgets remain internal.
        let limits = market_actors::READ_LIMITS;
        for (name, field, length) in [
            ("GiftOrderQuery", "position", limits.max_position_bytes),
            ("OrderBoundary", "position", limits.max_position_bytes),
            (
                "OrderReadQuery",
                "snapshot",
                limits.max_snapshot_token_bytes,
            ),
            (
                "ReleaseReadSnapshot",
                "snapshot",
                limits.max_snapshot_token_bytes,
            ),
            (
                "OrderedOrderResponse",
                "position",
                limits.max_position_bytes,
            ),
        ] {
            use utoipa::openapi::{RefOr, schema::Schema};
            if let Some(RefOr::T(Schema::Object(schema))) = doc
                .components
                .as_mut()
                .and_then(|c| c.schemas.get_mut(name))
                && let Some(RefOr::T(Schema::Object(property))) = schema.properties.get_mut(field)
            {
                property.max_length = Some(length);
            }
        }
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
