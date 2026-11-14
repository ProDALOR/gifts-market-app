use crate::{
    api::*,
    error::{ApiError, ErrorResponse, Result},
    gateway::Gateway,
};
use actix_web::{HttpResponse, web};
use serde::Deserialize;
use std::path::PathBuf;
use utoipa::OpenApi;
#[derive(Clone)]
pub struct WebRoot(pub PathBuf);
#[utoipa::path(get,path="/healthz",responses((status=200,description="Process is running")))]
async fn health() -> HttpResponse {
    HttpResponse::Ok().finish()
}
#[utoipa::path(get,path="/readyz",responses((status=200,description="Hub is reachable"),(status=503,body=ErrorResponse)))]
async fn ready(state: web::Data<Gateway>) -> Result<HttpResponse> {
    state.ready().await?;
    Ok(HttpResponse::Ok().finish())
}
#[utoipa::path(get,path="/sync",responses((status=200,body=SyncInfo)))]
async fn sync(state: web::Data<Gateway>) -> web::Json<SyncInfo> {
    web::Json(state.sync())
}
#[derive(Deserialize)]
struct BooksParams {
    #[serde(default)]
    limit: u32,
    after: Option<String>,
}
#[utoipa::path(get,path="/books",params(("limit"=Option<u32>,Query),("after"=Option<String>,Query,description="JSON BookKey returned in next_after")),responses((status=200,body=BooksPage)))]
async fn books(
    state: web::Data<Gateway>,
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
#[utoipa::path(post,path="/catalog/query",request_body=CatalogQuery,responses((status=200,body=Page<CatalogEntry>),(status=400,body=ErrorResponse)))]
async fn catalog(
    state: web::Data<Gateway>,
    q: web::Json<CatalogQuery>,
) -> Result<web::Json<Page<CatalogEntry>>> {
    Ok(web::Json(state.catalog(q.into_inner()).await?))
}
#[derive(Deserialize)]
struct CatalogParams {
    #[serde(default = "default_limit")]
    limit: u32,
    #[serde(default)]
    page_token: String,
}
fn default_limit() -> u32 {
    50
}
#[utoipa::path(get,path="/catalog/{kind}",params(("kind"=CatalogKind,Path),("limit"=Option<u32>,Query),("page_token"=Option<String>,Query)),responses((status=200,body=Page<CatalogEntry>)))]
async fn list_catalog(
    state: web::Data<Gateway>,
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
#[utoipa::path(post,path="/orders/query",request_body=OrdersQuery,responses((status=200,body=Page<Order>),(status=400,body=ErrorResponse),(status=409,body=ErrorResponse)))]
async fn orders(
    state: web::Data<Gateway>,
    q: web::Json<OrdersQuery>,
) -> Result<web::Json<Page<Order>>> {
    Ok(web::Json(state.orders(q.into_inner()).await?))
}
#[utoipa::path(post,path="/executions/query",request_body=ExecutionsQuery,responses((status=200,body=Page<Order>),(status=400,body=ErrorResponse),(status=409,body=ErrorResponse)))]
async fn executions(
    state: web::Data<Gateway>,
    q: web::Json<ExecutionsQuery>,
) -> Result<web::Json<Page<Order>>> {
    Ok(web::Json(state.executions(q.into_inner()).await?))
}
#[derive(OpenApi)]
#[openapi(
    paths(health, ready, sync, books, catalog, list_catalog, orders, executions),
    components(schemas(Command, ServerFrame, AttributeValue)),
    info(title = "Market App", version = "0.1.0")
)]
pub struct ApiDoc;
async fn schema() -> HttpResponse {
    HttpResponse::Ok().json(ApiDoc::openapi())
}
async fn index(root: web::Data<WebRoot>) -> HttpResponse {
    match tokio::fs::read(root.0.join("index.html")).await {
        Ok(bytes) => HttpResponse::Ok()
            .content_type("text/html; charset=utf-8")
            .insert_header(("Cache-Control", "no-cache"))
            .body(bytes),
        Err(_) => HttpResponse::ServiceUnavailable().body("The market interface is unavailable."),
    }
}
async fn asset(root: web::Data<WebRoot>, name: web::Path<String>) -> HttpResponse {
    if !name
        .bytes()
        .all(|v| v.is_ascii_alphanumeric() || b"-_.".contains(&v))
        || name.contains("..")
    {
        return HttpResponse::NotFound().finish();
    }
    let content = if name.ends_with(".js") {
        "text/javascript"
    } else if name.ends_with(".css") {
        "text/css"
    } else if name.ends_with(".svg") {
        "image/svg+xml"
    } else {
        return HttpResponse::NotFound().finish();
    };
    match tokio::fs::read(root.0.join("assets").join(name.as_str())).await {
        Ok(bytes) => HttpResponse::Ok()
            .content_type(content)
            .insert_header(("Cache-Control", "public,max-age=31536000,immutable"))
            .body(bytes),
        Err(_) => HttpResponse::NotFound().finish(),
    }
}
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.route("/", web::get().to(index))
        .route("/assets/{name}", web::get().to(asset))
        .route("/healthz", web::get().to(health))
        .route("/readyz", web::get().to(ready))
        .route("/sync", web::get().to(sync))
        .route("/books", web::get().to(books))
        .route("/catalog/query", web::post().to(catalog))
        .route("/catalog/{kind}", web::get().to(list_catalog))
        .route("/orders/query", web::post().to(orders))
        .route("/executions/query", web::post().to(executions))
        .route("/ws", web::get().to(crate::websocket::connect))
        .route("/openapi.json", web::get().to(schema));
}
