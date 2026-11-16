use crate::models::*;
use actix_web::HttpResponse;
use utoipa::OpenApi;
#[derive(OpenApi)]
#[openapi(
    paths(
        crate::routes::health,
        crate::routes::ready,
        crate::routes::sync,
        crate::routes::books,
        crate::routes::catalog,
        crate::routes::list_catalog,
        crate::routes::orders,
        crate::routes::executions,
        crate::websocket::connect
    ),
    components(schemas(Command, ServerFrame, AttributeValue)),
    info(title = "Market App", version = "0.1.0")
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
pub(crate) async fn schema() -> HttpResponse {
    HttpResponse::Ok().json(ApiDoc::openapi())
}
