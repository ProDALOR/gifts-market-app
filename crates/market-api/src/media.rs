//! Public catalog keys resolve through actors; only validated image bytes leave the cache.
use crate::{
    MarketApi,
    error::{ApiError, ErrorCode, Result},
    models::{MediaTarget, ModelId, PatternId},
};
use actix_web::{HttpRequest, HttpResponse, http::header, web};
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
#[utoipa::path(get,tag="Media",path="/media/models/{id}",params(("id"=String,Path,description="Catalog model ID")),responses((status=200,description="PNG, JPEG or WebP thumbnail"),(status=304,description="Cached thumbnail unchanged"),(status=400,body=crate::error::ErrorResponse),(status=404,body=crate::error::ErrorResponse),(status=429,body=crate::error::ErrorResponse),(status=503,body=crate::error::ErrorResponse)))]
pub(crate) async fn model_thumbnail(
    market: web::Data<MarketApi>,
    media: Option<web::Data<MediaApi>>,
    id: web::Path<ModelId>,
    request: HttpRequest,
) -> Result<HttpResponse> {
    let target = MediaTarget::Model(id.into_inner());
    catalog_media(market, media, target, request, MediaKind::Thumbnail).await
}
#[utoipa::path(get,tag="Media",path="/media/models/{id}/animation",params(("id"=String,Path,description="Catalog model ID")),responses((status=200,description="Gzip-compressed TGS (application/x-tgsticker)"),(status=304,description="Cached animation unchanged"),(status=400,body=crate::error::ErrorResponse),(status=404,body=crate::error::ErrorResponse),(status=429,body=crate::error::ErrorResponse),(status=503,body=crate::error::ErrorResponse)))]
pub(crate) async fn model_animation(
    market: web::Data<MarketApi>,
    media: Option<web::Data<MediaApi>>,
    id: web::Path<ModelId>,
    request: HttpRequest,
) -> Result<HttpResponse> {
    let target = MediaTarget::Model(id.into_inner());
    catalog_media(market, media, target, request, MediaKind::Animation).await
}
#[utoipa::path(get,tag="Media",path="/media/patterns/{id}",params(("id"=String,Path,description="Catalog pattern ID")),responses((status=200,description="PNG, JPEG or WebP gift pattern thumbnail"),(status=304,description="Cached pattern unchanged"),(status=400,body=crate::error::ErrorResponse),(status=404,body=crate::error::ErrorResponse),(status=429,body=crate::error::ErrorResponse),(status=503,body=crate::error::ErrorResponse)))]
pub(crate) async fn pattern_thumbnail(
    market: web::Data<MarketApi>,
    media: Option<web::Data<MediaApi>>,
    id: web::Path<PatternId>,
    request: HttpRequest,
) -> Result<HttpResponse> {
    let target = MediaTarget::Pattern(id.into_inner());
    catalog_media(market, media, target, request, MediaKind::Thumbnail).await
}
async fn catalog_media(
    market: web::Data<MarketApi>,
    media: Option<web::Data<MediaApi>>,
    target: MediaTarget,
    request: HttpRequest,
    kind: MediaKind,
) -> Result<HttpResponse> {
    let media = media.ok_or_else(|| ApiError::unavailable("Media cache is unavailable"))?;
    let _slot = media
        .slots
        .try_acquire()
        .map_err(|_| ApiError::capacity("Media service is busy"))?;
    let document = market.media_document(target).await?;
    let document = market_media::DocumentId::new(document.get()).ok_or_else(ApiError::not_found)?;
    let key = match kind {
        MediaKind::Thumbnail => MediaKey::thumbnail(document),
        MediaKind::Animation => MediaKey::animation(document),
    };
    let image = match media.cache.get(key).await {
        Ok(image) => image,
        Err(error) => {
            let (code, message, retry) = match error {
                MediaError::NotFound | MediaError::InvalidMedia => {
                    (ErrorCode::NotFound, "Gift media is unavailable", None)
                }
                MediaError::Busy => (
                    ErrorCode::CapacityExceeded,
                    "Media service is busy",
                    Some(2),
                ),
                MediaError::RetryAfter(seconds) => (
                    ErrorCode::Unavailable,
                    "Media download is cooling down",
                    Some(seconds),
                ),
                MediaError::Unavailable => {
                    (ErrorCode::Unavailable, "Media is not cached yet", Some(2))
                }
            };
            let error = ApiError {
                details: None,
                code,
                message: message.into(),
            };
            use actix_web::ResponseError;
            let mut response = error.error_response();
            if let Some(seconds) = retry {
                response.headers_mut().insert(
                    header::RETRY_AFTER,
                    header::HeaderValue::from_str(&seconds.to_string()).unwrap(),
                );
            }
            return Ok(response);
        }
    };
    let unchanged = request
        .headers()
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|tag| tag.trim().trim_start_matches("W/") == image.etag() || tag.trim() == "*")
        });
    let mut response = if unchanged {
        HttpResponse::NotModified()
    } else {
        HttpResponse::Ok()
    };
    response
        .insert_header((header::ETAG, image.etag()))
        .insert_header((header::CACHE_CONTROL, "public,max-age=86400"));
    if unchanged {
        Ok(response.finish())
    } else {
        Ok(response
            .content_type(image.content_type())
            .body(web::Bytes::from_owner(image)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{App, test};
    use market_hub_protocol::wire as w;

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
        let data = market_actors::MarketDataService::start(writer.reader(), 2, 2)
            .await
            .unwrap();
        let valuation = market_valuation::Service::start(
            market_actors::valuation::ModelFloorStrategy::new(data.handle.clone()),
            Default::default(),
        )
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
        let app = test::init_service(
            App::new()
                .wrap(crate::security_headers())
                .app_data(web::Data::new(MarketApi::new(
                    data.handle.clone(),
                    market_actors::valuation::ValuationHandle::new([valuation.handle.clone()])
                        .unwrap(),
                )))
                .app_data(web::Data::new(MediaApi::new(cache.clone())))
                .configure(crate::configure),
        )
        .await;
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/models/model")
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
                .uri("/media/models/model")
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
                    .uri(&format!("/media/models/{id}"))
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
                .uri("/media/patterns/pattern")
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
                .uri("/media/patterns/pattern")
                .insert_header((header::IF_NONE_MATCH, etag))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 304);
        for (path, status) in [
            ("/media/patterns/model", 404),
            ("/media/patterns/backdrop", 404),
            ("/media/models/pattern", 404),
            ("/media/models/pattern/animation", 404),
            ("/media/patterns/missing", 404),
            ("/media/patterns/uncached-pattern", 503),
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
                .uri("/media/models/model%2F%F0%9F%92%8E%3F")
                .to_request(),
        )
        .await;
        assert_eq!(encoded.status(), 200);
        assert_eq!(test::read_body(encoded).await.as_ref(), bytes);
        // Conditional hits do not reread or revalidate their cached bodies.
        let reads = cache.stats().disk_reads;
        for _ in 0..4 {
            assert_eq!(
                test::call_service(
                    &app,
                    test::TestRequest::get()
                        .uri("/media/models/model")
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
        valuation.task.shutdown().await.unwrap();
        let stop = tokio_util::sync::CancellationToken::new();
        stop.cancel();
        data.task.run(stop).await.unwrap();
    }
}
