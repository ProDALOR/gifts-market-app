use actix_web::{HttpResponse, web};
use std::path::PathBuf;
#[derive(Clone)]
pub(crate) struct WebRoot(pub PathBuf);
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

pub(crate) fn configure(cfg: &mut web::ServiceConfig) {
    cfg.route("/", web::get().to(index))
        .route("/assets/{name}", web::get().to(asset));
}
