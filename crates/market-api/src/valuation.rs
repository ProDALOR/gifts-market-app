//! One public report with one snapshot revision and evaluation time.
use serde::Serialize;
use utoipa::ToSchema;
#[derive(Debug, Serialize, ToSchema)]
pub struct GiftValuationResponse<'a> {
    pub strategy: &'a str,
    pub revision: &'a str,
    #[schema(value_type = String, format = DateTime)]
    pub evaluated_at: jiff::Timestamp,
    #[serde(flatten)]
    pub data: &'a market_valuation::GiftFloorValuation,
    pub display: market_valuation::ValuationDisplay<'a>,
}
impl<'a> From<&'a market_valuation::GiftFloorMark> for GiftValuationResponse<'a> {
    fn from(value: &'a market_valuation::GiftFloorMark) -> Self {
        Self {
            strategy: value.strategy,
            revision: &value.revision,
            evaluated_at: value.evaluated_at,
            data: &value.data,
            display: value.data.display(),
        }
    }
}

/// Public presentation adds HTTP links to transport-independent strategy metadata.
#[derive(Debug, Serialize, ToSchema)]
pub struct StrategyResponse {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub documentation_url: String,
    pub documentation_revision: String,
}
impl From<&market_valuation::StrategyMetadata> for StrategyResponse {
    fn from(metadata: &market_valuation::StrategyMetadata) -> Self {
        Self {
            id: metadata.id,
            name: metadata.name,
            description: metadata.description,
            documentation_revision: content_revision(metadata.documentation.as_bytes()),
            documentation_url: format!("/valuations/strategies/{}/documentation", metadata.id),
        }
    }
}
fn content_revision(bytes: &[u8]) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hash = DefaultHasher::new();
    bytes.hash(&mut hash);
    format!("{:016x}", hash.finish())
}
struct Content {
    body: actix_web::web::Bytes,
    etag: String,
    mime: &'static str,
}
impl Content {
    fn new(body: actix_web::web::Bytes, mime: &'static str) -> Self {
        Self {
            etag: format!("\"{}\"", content_revision(&body)),
            body,
            mime,
        }
    }
    fn response(&self, req: &actix_web::HttpRequest) -> actix_web::HttpResponse {
        use actix_web::{HttpResponse, http::header};
        let unchanged = crate::http::etag_matches(req, &self.etag);
        let mut response = if unchanged {
            HttpResponse::NotModified()
        } else {
            HttpResponse::Ok()
        };
        response
            .insert_header((header::ETAG, self.etag.as_str()))
            .insert_header((header::CACHE_CONTROL, "public, no-cache"));
        if unchanged {
            response.finish()
        } else {
            response.content_type(self.mime).body(self.body.clone())
        }
    }
}
struct StrategyCatalog {
    index: Content,
    guides: std::collections::BTreeMap<&'static str, Content>,
}
impl StrategyCatalog {
    fn new(metadata: &[market_valuation::StrategyMetadata]) -> Self {
        let rows = metadata
            .iter()
            .map(StrategyResponse::from)
            .collect::<Vec<_>>();
        Self {
            index: Content::new(
                serde_json::to_vec(&rows)
                    .expect("strategy metadata contains only strings")
                    .into(),
                "application/json",
            ),
            guides: metadata
                .iter()
                .map(|metadata| {
                    (
                        metadata.id,
                        Content::new(
                            actix_web::web::Bytes::from_static(metadata.documentation.as_bytes()),
                            "text/markdown; charset=utf-8",
                        ),
                    )
                })
                .collect(),
        }
    }
}
fn catalog() -> &'static StrategyCatalog {
    static CATALOG: std::sync::OnceLock<StrategyCatalog> = std::sync::OnceLock::new();
    CATALOG.get_or_init(|| StrategyCatalog::new(market_valuation::strategies()))
}
#[utoipa::path(get,path="/valuations/strategies",tag="Valuations",summary="Discover available gift-valuation strategies",description="IDs match the strategy field in gift-valuation responses. Each strategy owns its display name, description and Markdown documentation with LaTeX math. Follow documentation_url to fetch the guide. documentation_revision changes with guide content. Both discovery and documentation support ETag revalidation.",responses((status=200,body=Vec<StrategyResponse>,headers(("ETag"=String,description="Strategy catalog content revision"))),(status=304,description="Strategy catalog is unchanged")))]
pub(crate) async fn strategies(req: actix_web::HttpRequest) -> actix_web::HttpResponse {
    catalog().index.response(&req)
}

/// Strategy documentation is static and needs no actor, replica read or Telegram session.
#[utoipa::path(get,path="/valuations/strategies/{strategy}/documentation",tag="Valuations",summary="Read a strategy's calculation guide",params(("strategy"=String,Path,description="ID from GET /valuations/strategies")),responses((status=200,body=String,content_type="text/markdown",description="Markdown with LaTeX formulas, sample requirements and rounding",headers(("ETag"=String,description="Documentation content revision"))),(status=304,description="Guide is unchanged"),(status=400,body=crate::error::ErrorResponse),(status=404,body=crate::error::ErrorResponse)))]
pub(crate) async fn documentation(
    strategy: actix_web::web::Path<market_valuation::StrategyId>,
    req: actix_web::HttpRequest,
) -> crate::error::Result<actix_web::HttpResponse> {
    let guide = catalog()
        .guides
        .get(strategy.as_str())
        .ok_or_else(|| crate::error::ApiError {
            code: crate::error::ErrorCode::NotFound,
            message: "Strategy documentation is unavailable".into(),
            details: None,
        })?;
    Ok(guide.response(&req))
}

#[cfg(test)]
mod tests {
    use actix_web::{App, http::header, test};
    #[actix_web::test]
    async fn catalog_revalidation_tracks_documentation_changes_and_each_guide_has_its_own_revision()
    {
        use super::*;
        fn entries(documentation: &'static str) -> [market_valuation::StrategyMetadata; 2] {
            [
                market_valuation::StrategyMetadata {
                    id: "one",
                    name: "One",
                    description: "first",
                    documentation,
                },
                market_valuation::StrategyMetadata {
                    id: "two",
                    name: "Two",
                    description: "second",
                    documentation: "other guide",
                },
            ]
        }
        let old = StrategyCatalog::new(&entries("first version"));
        let changed = StrategyCatalog::new(&entries("second version"));
        assert_ne!(old.index.etag, changed.index.etag);
        assert_ne!(old.guides["one"].etag, changed.guides["one"].etag);
        assert_eq!(old.guides["two"].etag, changed.guides["two"].etag);
        let rows: serde_json::Value = serde_json::from_slice(&changed.index.body).unwrap();
        assert_eq!(
            changed.guides["one"].etag,
            format!(
                "\"{}\"",
                rows[0]["documentation_revision"].as_str().unwrap()
            )
        );
        let app = test::init_service(App::new().configure(crate::configure)).await;
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/valuations/strategies")
                .to_request(),
        )
        .await;
        let etag = response.headers().get(header::ETAG).unwrap().clone();
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/valuations/strategies")
                .insert_header((header::IF_NONE_MATCH, etag))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 304);
        assert!(test::read_body(response).await.is_empty());
    }
    #[actix_web::test]
    async fn discovered_metadata_links_to_each_strategys_own_guide_without_market_state() {
        let app = test::init_service(App::new().configure(crate::configure)).await;
        let rows: serde_json::Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::get()
                .uri("/valuations/strategies")
                .to_request(),
        )
        .await;
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), market_valuation::strategies().len());
        for row in rows {
            let metadata = market_valuation::strategy(row["id"].as_str().unwrap()).unwrap();
            assert_eq!(row["name"], metadata.name);
            assert_eq!(row["description"], metadata.description);
            let response = test::call_service(
                &app,
                test::TestRequest::get()
                    .uri(row["documentation_url"].as_str().unwrap())
                    .to_request(),
            )
            .await;
            assert_eq!(response.status(), 200);
            assert_eq!(
                test::read_body(response).await.as_ref(),
                metadata.documentation.as_bytes()
            );
        }
    }
    #[actix_web::test]
    async fn strategy_markdown_is_available_without_market_state_and_revalidates_by_content() {
        let app = test::init_service(App::new().configure(crate::configure)).await;
        let uri = "/valuations/strategies/gift_floor/documentation";
        let response =
            test::call_service(&app, test::TestRequest::get().uri(uri).to_request()).await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/markdown; charset=utf-8"
        );
        let etag = response
            .headers()
            .get(header::ETAG)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let body = test::read_body(response).await;
        assert_eq!(
            body.as_ref(),
            market_valuation::gift_floor::METADATA
                .documentation
                .as_bytes()
        );
        for condition in [etag.clone(), format!("\"older\", W/{etag}"), "*".into()] {
            let response = test::call_service(
                &app,
                test::TestRequest::get()
                    .uri(uri)
                    .insert_header((header::IF_NONE_MATCH, condition))
                    .to_request(),
            )
            .await;
            assert_eq!(response.status(), 304);
            assert!(test::read_body(response).await.is_empty());
        }
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri(uri)
                .insert_header((header::IF_NONE_MATCH, "\"older\""))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        for (id, status) in [("unknown_strategy", 404), ("Invalid", 400)] {
            let response = test::call_service(
                &app,
                test::TestRequest::get()
                    .uri(&format!("/valuations/strategies/{id}/documentation"))
                    .to_request(),
            )
            .await;
            assert_eq!(response.status(), status);
        }
    }
}
