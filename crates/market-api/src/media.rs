//! Public catalog keys resolve through actors; only validated image bytes leave the cache.
use crate::{
    MarketApi,
    error::{ApiError, ErrorCode, Result},
};
use actix_web::{HttpRequest, HttpResponse, http::header, web};
use market_gifts::{MediaTarget, ModelId, PatternId};
use market_media::{MediaCache, MediaError, MediaKey, MediaKind};
use std::sync::Arc;
use tokio::sync::Semaphore;
#[derive(Clone)]
pub(crate) struct MediaApi {
    pub(crate) cache: MediaCache,
    slots: Arc<Semaphore>,
}
impl MediaApi {
    pub(crate) fn new(cache: MediaCache) -> Self {
        Self {
            cache,
            slots: Arc::new(Semaphore::new(64)),
        }
    }
}
#[derive(Debug, Clone, Copy, serde::Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MediaCategory {
    Models,
    Patterns,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MediaVariant {
    Thumbnail,
    Preview,
    Animation,
    Placeholder,
}
#[derive(serde::Deserialize)]
pub(crate) struct MediaPath {
    kind: MediaCategory,
    id: String,
    variant: MediaVariant,
}
impl MediaPath {
    fn target(self) -> Result<(MediaTarget, MediaKind)> {
        let target = match self.kind {
            MediaCategory::Models => MediaTarget::Model(
                ModelId::new(self.id).map_err(|_| ApiError::invalid("Invalid model ID"))?,
            ),
            MediaCategory::Patterns => {
                if self.variant != MediaVariant::Thumbnail {
                    return Err(ApiError::not_found());
                }
                MediaTarget::Pattern(
                    PatternId::new(self.id).map_err(|_| ApiError::invalid("Invalid pattern ID"))?,
                )
            }
        };
        let kind = match self.variant {
            MediaVariant::Thumbnail => MediaKind::Thumbnail,
            MediaVariant::Preview => MediaKind::Preview,
            MediaVariant::Animation => MediaKind::Animation,
            MediaVariant::Placeholder => MediaKind::Placeholder,
        };
        Ok((target, kind))
    }
}
#[utoipa::path(get,tag="Media",path="/media/{kind}/{id}/{variant}",
    params(("kind"=MediaCategory,Path,description="Catalog media category"),
           ("id"=String,Path,description="Catalog model or pattern ID; never an arbitrary Telegram file ID"),
           ("variant"=MediaVariant,Path,description="Models support all variants; patterns support thumbnail only. Missing outlines return 404.")),
    responses((status=200,description="Thumbnail (PNG/JPEG/WebP), preview/placeholder (passive SVG), or animation (validated, expression-free gzip-compressed TGS)",
               content(("image/png"),("image/jpeg"),("image/webp"),("image/svg+xml"),("application/x-tgsticker"))),
              (status=304,description="Cached media unchanged"),(status=400,body=crate::error::ErrorResponse),
              (status=404,body=crate::error::ErrorResponse),(status=429,description="Media capacity exceeded; Retry-After is included",body=crate::error::ErrorResponse),
              (status=503,description="Media temporarily unavailable; Retry-After is included",body=crate::error::ErrorResponse)))]
pub(crate) async fn get(
    market: web::Data<MarketApi>,
    media: Option<web::Data<MediaApi>>,
    path: web::Path<MediaPath>,
    request: HttpRequest,
) -> Result<HttpResponse> {
    let (target, kind) = path.into_inner().target()?;
    let Some(media) = media else {
        return Ok(failure(MediaError::Unavailable));
    };
    let Ok(_slot) = media.slots.try_acquire() else {
        return Ok(failure(MediaError::Busy));
    };
    let document = match market.media_document(target).await {
        Ok(document) => document,
        Err(error) => return Ok(error_response(error, None)),
    };
    let document = market_media::DocumentId::new(document.get()).ok_or_else(ApiError::not_found)?;
    let asset = match kind {
        MediaKind::Thumbnail => media.cache.get(MediaKey::thumbnail(document)).await,
        MediaKind::Animation => media.cache.get(MediaKey::animation(document)).await,
        MediaKind::Preview => media.cache.preview(document).await,
        MediaKind::Placeholder => media.cache.get(MediaKey::placeholder(document)).await,
    };
    let image = match asset {
        Ok(image) => image,
        Err(error) => return Ok(failure(error)),
    };
    let unchanged = crate::http::etag_matches(&request, image.etag());
    let mut response = if unchanged {
        HttpResponse::NotModified()
    } else {
        HttpResponse::Ok()
    };
    response
        .insert_header((header::ETAG, image.etag()))
        .insert_header((header::CACHE_CONTROL, "public,max-age=86400"));
    if image.content_type() == "image/svg+xml" {
        response
            .insert_header((header::VARY, "Accept-Encoding"))
            .insert_header((
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; style-src 'unsafe-inline'; sandbox",
            ));
    }
    if unchanged {
        Ok(response.finish())
    } else {
        Ok(response
            .content_type(image.content_type())
            .body(web::Bytes::from_owner(image)))
    }
}

fn failure(error: MediaError) -> HttpResponse {
    let (code, message, retry) = match error {
        MediaError::NotFound | MediaError::InvalidMedia => {
            (ErrorCode::NotFound, "Gift media is unavailable", None)
        }
        MediaError::Busy => (ErrorCode::CapacityExceeded, "Media service is busy", None),
        MediaError::RetryAfter(seconds) => (
            ErrorCode::Unavailable,
            "Gift media is temporarily unavailable",
            Some(seconds),
        ),
        MediaError::Unavailable => (
            ErrorCode::Unavailable,
            "Gift media is temporarily unavailable",
            None,
        ),
    };
    error_response(
        ApiError {
            details: None,
            code,
            message: message.into(),
        },
        retry,
    )
}
fn error_response(error: ApiError, retry: Option<u64>) -> HttpResponse {
    use actix_web::ResponseError;
    let retry = retry.or_else(|| {
        matches!(
            error.code,
            ErrorCode::CapacityExceeded | ErrorCode::Unavailable | ErrorCode::TimedOut
        )
        .then_some(2)
    });
    let mut response = error.error_response();
    if let Some(seconds) = retry {
        response.headers_mut().insert(
            header::RETRY_AFTER,
            header::HeaderValue::from_str(&seconds.to_string()).unwrap(),
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{App, test};
    use market_hub_protocol::wire as w;

    #[actix_web::test]
    async fn media_schema_registers_path_enums_and_omits_legacy_routes() {
        use utoipa::OpenApi;
        let schema = serde_json::to_value(crate::ApiDoc::openapi()).unwrap();
        let route = &schema["paths"]["/media/{kind}/{id}/{variant}"]["get"];
        assert_eq!(
            route["responses"]["200"]["content"]
                .as_object()
                .unwrap()
                .len(),
            5
        );
        for parameter in route["parameters"].as_array().unwrap() {
            if let Some(reference) = parameter["schema"]["$ref"].as_str() {
                assert!(
                    schema
                        .pointer(reference.strip_prefix('#').unwrap())
                        .is_some(),
                    "{reference}"
                );
            }
        }
        assert_eq!(
            schema["components"]["schemas"]["MediaCategory"]["enum"],
            serde_json::json!(["models", "patterns"])
        );
        assert_eq!(
            schema["components"]["schemas"]["MediaVariant"]["enum"],
            serde_json::json!(["thumbnail", "preview", "animation", "placeholder"])
        );
        assert!(schema["paths"].get("/media/models/{id}").is_none());
    }
    #[actix_web::test]
    async fn cached_gift_media_uses_catalog_attribute_types_and_works_without_telegram() {
        let directory = tempfile::tempdir().unwrap();
        let mut writer =
            market_replica::ReplicaWriter::open(&directory.path().join("replica"), "test").unwrap();
        let mut snapshot = writer.begin_snapshot().unwrap();
        let entities = [
            ("model", "tg.model", "42"),
            ("model/💎?", "tg.model", "42"),
            ("uncached", "tg.model", "43"),
            ("backdrop", "tg.backdrop", "42"),
            ("pattern", "tg.pattern", "42"),
            ("uncached-pattern", "tg.pattern", "44"),
        ]
        .into_iter()
        .map(|(id, attribute, document)| w::Entity {
            entity: Some(w::entity::Entity::AttributeOption(w::AttributeOption {
                id: id.into(),
                name: id.into(),
                attribute_id: attribute.into(),
                aliases: vec![w::ExternalId {
                    namespace: "tg".into(),
                    id: document.into(),
                }],
                ..Default::default()
            })),
        })
        .collect::<Vec<_>>();
        snapshot
            .append(
                &entities
                    .into_iter()
                    .map(|entity| w::Change {
                        change: Some(w::change::Change::Upsert(entity)),
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        writer
            .install_snapshot(snapshot.finish("test-cursor").unwrap())
            .unwrap();
        let read_runtime = market_replica::ReadRuntime::new(
            writer.reader(),
            2,
            tokio_util::sync::CancellationToken::new(),
        )
        .unwrap();
        let data = market_actors::MarketDataService::start(read_runtime.handle(), 2)
            .await
            .unwrap();
        let (valuation, valuation_task) =
            market_valuation::ValuationHandle::start(&data.arbiter(), read_runtime.handle())
                .await
                .unwrap();
        let media_path = directory.path().join("media");
        let (cache, downloads) = MediaCache::open(&media_path).await.unwrap();
        drop(downloads);
        let bytes = b"\x89PNG\r\n\x1a\nsynthetic-test-image";
        tokio::fs::write(media_path.join("42.thumb"), bytes)
            .await
            .unwrap();
        use std::io::Write;
        let mut tgs = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        tgs.write_all(br#"{"v":"5.7.4","w":512,"h":512,"fr":60,"ip":0,"op":180,"layers":[]}"#)
            .unwrap();
        let tgs = tgs.finish().unwrap();
        tokio::fs::write(media_path.join("42.tgs"), &tgs)
            .await
            .unwrap();
        let placeholder = market_media::MediaAsset::placeholder("M0,0L512,0L512,512z").unwrap();
        tokio::fs::write(media_path.join("42.placeholder.svg"), placeholder.bytes())
            .await
            .unwrap();
        use sha2::Digest;
        let svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512">{}</svg>"##,
            r##"<path d="M0 0L1 1" fill="#ffffff"/>"##.repeat(32)
        );
        tokio::fs::write(
            media_path.join(format!("42.preview-2-{:x}.svg", sha2::Sha256::digest(&tgs))),
            &svg,
        )
        .await
        .unwrap();
        let media = web::Data::new(MediaApi::new(cache.clone()));
        let app = test::init_service(
            App::new()
                .wrap(crate::security_headers())
                .app_data(web::Data::new(MarketApi::new(
                    data.handle.clone(),
                    valuation.clone(),
                )))
                .app_data(media.clone())
                .configure(crate::configure),
        )
        .await;
        let preview = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/models/model/preview")
                .insert_header((header::ACCEPT_ENCODING, "gzip"))
                .to_request(),
        )
        .await;
        assert_eq!(preview.status(), 200);
        assert_eq!(
            preview.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/svg+xml"
        );
        assert_eq!(
            preview.headers().get(header::CONTENT_ENCODING).unwrap(),
            "gzip"
        );
        assert_eq!(
            preview.headers().get(header::VARY).unwrap(),
            "Accept-Encoding"
        );
        let preview_etag = preview
            .headers()
            .get(header::ETAG)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        assert!(preview_etag.starts_with("W/"));
        let compressed = test::read_body(preview).await;
        let mut decoded = String::new();
        std::io::Read::read_to_string(
            &mut flate2::read::GzDecoder::new(compressed.as_ref()),
            &mut decoded,
        )
        .unwrap();
        assert_eq!(decoded, svg);
        let unchanged = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/models/model/preview")
                .insert_header((header::IF_NONE_MATCH, preview_etag))
                .to_request(),
        )
        .await;
        assert_eq!(unchanged.status(), 304);
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/models/model/thumbnail")
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/png"
        );
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "public,max-age=86400"
        );
        let etag = response
            .headers()
            .get(header::ETAG)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        assert_eq!(test::read_body(response).await.as_ref(), bytes);
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/models/model/thumbnail")
                .insert_header((header::IF_NONE_MATCH, format!("W/{etag}")))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 304);
        assert!(test::read_body(response).await.is_empty());
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/models/model/animation")
                .insert_header((header::ACCEPT_ENCODING, "gzip"))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/x-tgsticker"
        );
        assert!(response.headers().get(header::CONTENT_ENCODING).is_none());
        let tgs_etag = response
            .headers()
            .get(header::ETAG)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        assert_ne!(tgs_etag, etag);
        assert_eq!(test::read_body(response).await.as_ref(), tgs);
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/models/model/animation")
                .insert_header((header::IF_NONE_MATCH, tgs_etag))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 304);
        for (id, status) in [("backdrop", 404), ("missing", 404), ("uncached", 503)] {
            assert_eq!(
                test::call_service(
                    &app,
                    test::TestRequest::get()
                        .uri(&format!("/media/models/{id}/animation"))
                        .to_request()
                )
                .await
                .status(),
                status
            );
        }
        for (id, status) in [("backdrop", 404), ("missing", 404), ("uncached", 503)] {
            let response = test::call_service(
                &app,
                test::TestRequest::get()
                    .uri(&format!("/media/models/{id}/thumbnail"))
                    .to_request(),
            )
            .await;
            assert_eq!(response.status(), status);
            assert_eq!(
                response.headers().get(header::CONTENT_TYPE).unwrap(),
                "application/json"
            );
            if status == 503 {
                assert_eq!(response.headers().get(header::RETRY_AFTER).unwrap(), "2");
            }
        }
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/patterns/pattern/thumbnail")
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/png"
        );
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "public,max-age=86400"
        );
        assert_eq!(response.headers().get(header::ETAG).unwrap(), etag.as_str());
        assert_eq!(test::read_body(response).await.as_ref(), bytes);
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/patterns/pattern/thumbnail")
                .insert_header((header::IF_NONE_MATCH, etag))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 304);
        for (path, status) in [
            ("/media/patterns/model/thumbnail", 404),
            ("/media/patterns/backdrop/thumbnail", 404),
            ("/media/models/pattern/thumbnail", 404),
            ("/media/models/pattern/animation", 404),
            ("/media/patterns/missing/thumbnail", 404),
            ("/media/patterns/uncached-pattern/thumbnail", 503),
        ] {
            assert_eq!(
                test::call_service(&app, test::TestRequest::get().uri(path).to_request())
                    .await
                    .status(),
                status
            );
        }
        let encoded = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/models/model%2F%F0%9F%92%8E%3F/thumbnail")
                .to_request(),
        )
        .await;
        assert_eq!(encoded.status(), 200);
        assert_eq!(test::read_body(encoded).await.as_ref(), bytes);
        let outline = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/models/model/placeholder")
                .insert_header((header::ACCEPT_ENCODING, "gzip"))
                .to_request(),
        )
        .await;
        assert_eq!(outline.status(), 200);
        assert_eq!(
            outline.headers().get(header::CONTENT_ENCODING).unwrap(),
            "gzip"
        );
        assert_eq!(
            outline.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/svg+xml"
        );
        assert!(
            outline
                .headers()
                .contains_key(header::CONTENT_SECURITY_POLICY)
        );
        let outline_etag = outline
            .headers()
            .get(header::ETAG)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        assert!(outline_etag.starts_with("W/"));
        let body = test::read_body(outline).await;
        let mut decoded = Vec::new();
        std::io::Read::read_to_end(
            &mut flate2::read::GzDecoder::new(body.as_ref()),
            &mut decoded,
        )
        .unwrap();
        assert_eq!(decoded, placeholder.bytes());
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/models/model/placeholder")
                .insert_header((header::IF_NONE_MATCH, outline_etag))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 304);
        assert_eq!(
            response.headers().get(header::VARY).unwrap(),
            "Accept-Encoding"
        );
        for (path, status) in [
            ("/media/patterns/pattern/preview", 404),
            ("/media/patterns/pattern/placeholder", 404),
            ("/media/patterns/pattern/animation", 404),
            ("/media/backdrops/model/thumbnail", 400),
            ("/media/models/model/video", 400),
            ("/media/models/model", 404),
        ] {
            assert_eq!(
                test::call_service(&app, test::TestRequest::get().uri(path).to_request())
                    .await
                    .status(),
                status,
                "{path}"
            );
        }
        let held = media.slots.acquire_many(64).await.unwrap();
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/models/model/thumbnail")
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 429);
        assert_eq!(response.headers().get(header::RETRY_AFTER).unwrap(), "2");
        drop(held);
        // Conditional hits do not reread or revalidate their cached bodies.
        let reads = cache.stats().disk_reads;
        for _ in 0..4 {
            assert_eq!(
                test::call_service(
                    &app,
                    test::TestRequest::get()
                        .uri("/media/models/model/thumbnail")
                        .insert_header((header::IF_NONE_MATCH, "*"))
                        .to_request()
                )
                .await
                .status(),
                304
            );
        }
        assert_eq!(cache.stats().disk_reads, reads);
        assert_eq!(
            cache.stats().downloads,
            0,
            "Cache-only API nodes do not enqueue downloads"
        );
        cache.shutdown().await;
        valuation_task.shutdown().await.unwrap();
        let stop = tokio_util::sync::CancellationToken::new();
        stop.cancel();
        data.task.run(stop).await.unwrap();
        read_runtime.shutdown().await;
    }
}
