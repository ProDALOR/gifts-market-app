//! Static assets use the framework's conditional and streamed file responses.
use actix_files::NamedFile;
use actix_web::{HttpRequest, HttpResponse, http::header, middleware::Compress, web};
use std::path::PathBuf;
#[derive(Clone)]
pub(crate) struct WebRoot(pub PathBuf);
async fn index(request: HttpRequest, root: web::Data<WebRoot>) -> HttpResponse {
    match NamedFile::open_async(root.0.join("index.html")).await {
        Ok(file) => response(file, &request, "no-cache"),
        Err(_) => HttpResponse::ServiceUnavailable().body("The market interface is unavailable."),
    }
}
// Documentation libraries use injected styles and data-URL icons. Keep that
// allowance on documentation HTML; the market GUI retains its stricter policy.
const DOCS_POLICY: &str = "default-src 'self'; script-src 'self'; connect-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; worker-src 'self' blob:; frame-ancestors 'self'; object-src 'none'; base-uri 'self'; form-action 'self'";
async fn docs_page(request: HttpRequest, root: web::Data<WebRoot>, path: &str) -> HttpResponse {
    match NamedFile::open_async(root.0.join(path)).await {
        Ok(file) => {
            let mut response = response(file, &request, "no-cache");
            response.headers_mut().insert(
                header::CONTENT_SECURITY_POLICY,
                header::HeaderValue::from_static(DOCS_POLICY),
            );
            response
        }
        Err(_) => HttpResponse::NotFound().finish(),
    }
}
async fn rest_docs(request: HttpRequest, root: web::Data<WebRoot>) -> HttpResponse {
    docs_page(request, root, "docs/index.html").await
}
async fn websocket_docs(request: HttpRequest, root: web::Data<WebRoot>) -> HttpResponse {
    docs_page(request, root, "docs/ws/index.html").await
}
async fn asset(
    request: HttpRequest,
    root: web::Data<WebRoot>,
    name: web::Path<String>,
) -> HttpResponse {
    if !name
        .bytes()
        .all(|v| v.is_ascii_alphanumeric() || b"-_.".contains(&v))
        || name.contains("..")
    {
        return HttpResponse::NotFound().finish();
    }
    match NamedFile::open_async(root.0.join("assets").join(name.as_str())).await {
        Ok(file) => response(file, &request, "public,max-age=31536000,immutable"),
        Err(_) => HttpResponse::NotFound().finish(),
    }
}
fn response(file: NamedFile, request: &HttpRequest, cache: &'static str) -> HttpResponse {
    let mut response = file.disable_content_disposition().into_response(request);
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static(cache),
    );
    response
}
pub(crate) fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource(["/docs", "/docs/"])
            .wrap(Compress::default())
            .route(web::get().to(rest_docs)),
    )
    .service(
        web::resource(["/docs/ws", "/docs/ws/"])
            .wrap(Compress::default())
            .route(web::get().to(websocket_docs)),
    )
    .service(
        web::resource([
            "/",
            "/portfolio",
            "/portfolio/{user_id}",
            "/profile",
            "/profile/",
        ])
        .wrap(Compress::default())
        .route(web::get().to(index)),
    )
    .service(
        web::resource("/assets/{name}")
            .wrap(Compress::default())
            .route(web::get().to(asset)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{App, http::StatusCode, test};
    use std::io::Read;
    #[actix_web::test]
    async fn files_support_conditional_reads_compression_ranges_and_path_boundaries() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("assets")).unwrap();
        std::fs::write(
            directory.path().join("index.html"),
            "<!doctype html><title>Market</title>",
        )
        .unwrap();
        std::fs::create_dir_all(directory.path().join("docs/ws")).unwrap();
        std::fs::write(
            directory.path().join("docs/index.html"),
            "<title>REST reference</title>",
        )
        .unwrap();
        std::fs::write(
            directory.path().join("docs/ws/index.html"),
            "<title>WebSocket reference</title>",
        )
        .unwrap();
        let javascript = "console.log('market');\n".repeat(1024);
        std::fs::write(directory.path().join("assets/app-test.js"), &javascript).unwrap();
        std::fs::write(directory.path().join("private.txt"), "private").unwrap();
        let app = test::init_service(
            App::new()
                .wrap(crate::security_headers())
                .app_data(web::Data::new(WebRoot(directory.path().to_owned())))
                .configure(configure),
        )
        .await;
        let index = test::call_service(&app, test::TestRequest::get().uri("/").to_request()).await;
        assert_eq!(index.status(), StatusCode::OK);
        assert_eq!(
            index.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-cache"
        );
        assert!(index.headers().contains_key(header::LAST_MODIFIED));
        let market_policy = index
            .headers()
            .get(header::CONTENT_SECURITY_POLICY)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(!market_policy.contains("unsafe-inline"));
        for path in ["/profile", "/profile/?currency=XTR"] {
            let profile =
                test::call_service(&app, test::TestRequest::get().uri(path).to_request()).await;
            assert_eq!(profile.status(), StatusCode::OK);
            assert_eq!(
                profile.headers().get(header::CACHE_CONTROL).unwrap(),
                "no-cache"
            );
        }
        for path in ["/docs", "/docs/", "/docs/ws", "/docs/ws/"] {
            let page =
                test::call_service(&app, test::TestRequest::get().uri(path).to_request()).await;
            assert_eq!(page.status(), StatusCode::OK);
            assert_eq!(
                page.headers().get(header::CACHE_CONTROL).unwrap(),
                "no-cache"
            );
            assert_eq!(
                page.headers().get(header::CONTENT_SECURITY_POLICY).unwrap(),
                DOCS_POLICY
            );
            let expected = if path.contains("/ws") {
                "<title>WebSocket reference</title>"
            } else {
                "<title>REST reference</title>"
            };
            assert_eq!(test::read_body(page).await.as_ref(), expected.as_bytes());
        }
        let etag = index.headers().get(header::ETAG).unwrap().clone();
        let unchanged = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/")
                .insert_header((header::IF_NONE_MATCH, etag))
                .to_request(),
        )
        .await;
        assert_eq!(unchanged.status(), StatusCode::NOT_MODIFIED);
        assert!(test::read_body(unchanged).await.is_empty());
        let asset = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/assets/app-test.js")
                .insert_header((header::ACCEPT_ENCODING, "gzip"))
                .to_request(),
        )
        .await;
        assert_eq!(
            asset.headers().get(header::CONTENT_ENCODING).unwrap(),
            "gzip"
        );
        assert_eq!(
            asset.headers().get(header::CACHE_CONTROL).unwrap(),
            "public,max-age=31536000,immutable"
        );
        assert!(asset.headers().contains_key(header::ETAG));
        let body = test::read_body(asset).await;
        assert!(body.len() < javascript.len());
        let mut decoded = String::new();
        flate2::read::GzDecoder::new(body.as_ref())
            .read_to_string(&mut decoded)
            .unwrap();
        assert_eq!(decoded, javascript);
        let range = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/assets/app-test.js")
                .insert_header((header::RANGE, "bytes=0-9"))
                .to_request(),
        )
        .await;
        assert_eq!(range.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            test::read_body(range).await.as_ref(),
            &javascript.as_bytes()[..10]
        );
        for path in [
            "/assets/missing.js",
            "/assets/%2e%2e%2fprivate.txt",
            "/assets/..",
            "/private.txt",
            "/docs/private.txt",
            "/docs/ws/private.txt",
            "/.vite/manifest.json",
        ] {
            assert_eq!(
                test::call_service(&app, test::TestRequest::get().uri(path).to_request())
                    .await
                    .status(),
                StatusCode::NOT_FOUND,
                "{path}"
            );
        }
    }
}
