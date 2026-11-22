//! Public valuation metadata and strategy-owned calculations.
use serde::Serialize;
use utoipa::ToSchema;
#[derive(Debug, Serialize, ToSchema)]
pub struct ValuationResponse<'a> {
    pub target: &'a market_valuation::ResolvedTarget,
    pub currency_id: &'a market_gifts::CurrencyId,
    #[schema(value_type=String,format=DateTime)]
    pub evaluated_at: jiff::Timestamp,
    pub stale: bool,
    pub results: Vec<StrategyOutcomeResponse<'a>>,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct StrategyOutcomeResponse<'a> {
    #[serde(flatten)]
    pub result: &'a market_valuation::StrategyOutcome,
    pub revision: &'static str,
}
impl<'a> From<&'a market_valuation::ValuationReport> for ValuationResponse<'a> {
    fn from(report: &'a market_valuation::ValuationReport) -> Self {
        Self {
            target: &report.target,
            currency_id: &report.currency_id,
            evaluated_at: report.evaluated_at,
            stale: report.stale,
            results: report
                .results
                .iter()
                .map(|result| StrategyOutcomeResponse {
                    result,
                    revision: catalog().revisions[result.strategy()].as_str(),
                })
                .collect(),
        }
    }
}
#[derive(Debug, Serialize, ToSchema)]
pub struct InputRequirements {
    pub targets: &'static [market_valuation::ValuationTargetKind],
    pub period: PeriodRequirement,
}
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PeriodRequirement {
    Required,
    Unsupported,
}
impl From<market_valuation::InputContract> for InputRequirements {
    fn from(input: market_valuation::InputContract) -> Self {
        Self {
            targets: input.targets(),
            period: if input.requires_period() {
                PeriodRequirement::Required
            } else {
                PeriodRequirement::Unsupported
            },
        }
    }
}
/// Public presentation adds HTTP links to transport-independent strategy metadata.
#[derive(Debug, Serialize, ToSchema)]
pub struct StrategyResponse {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub revision: String,
    pub display: market_valuation::DisplayDefinition,
    pub documentation_url: String,
    pub documentation_revision: String,
    pub input: InputRequirements,
    pub purpose: market_valuation::StrategyPurpose,
}
impl From<&market_valuation::StrategyMetadata> for StrategyResponse {
    fn from(metadata: &market_valuation::StrategyMetadata) -> Self {
        Self {
            id: metadata.id,
            name: metadata.name,
            description: metadata.description,
            revision: content_revision(
                &serde_json::to_vec(&(
                    InputRequirements::from(metadata.input),
                    metadata.display,
                    metadata.purpose,
                ))
                .expect("static strategy contract serializes"),
            ),
            display: metadata.display,
            input: metadata.input.into(),
            purpose: metadata.purpose,
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
    revisions: std::collections::BTreeMap<&'static str, String>,
    guides: std::collections::BTreeMap<&'static str, Content>,
}
impl StrategyCatalog {
    fn new<'a>(metadata: impl IntoIterator<Item = &'a market_valuation::StrategyMetadata>) -> Self {
        let mut rows = Vec::new();
        let mut guides = std::collections::BTreeMap::new();
        let mut revisions = std::collections::BTreeMap::new();
        for metadata in metadata {
            let row = StrategyResponse::from(metadata);
            revisions.insert(metadata.id, row.revision.clone());
            rows.push(row);
            guides.insert(
                metadata.id,
                Content::new(
                    actix_web::web::Bytes::from_static(metadata.documentation.as_bytes()),
                    "text/markdown; charset=utf-8",
                ),
            );
        }
        Self {
            index: Content::new(
                serde_json::to_vec(&rows)
                    .expect("strategy metadata serializes")
                    .into(),
                "application/json",
            ),
            guides,
            revisions,
        }
    }
}
fn catalog() -> &'static StrategyCatalog {
    static CATALOG: std::sync::OnceLock<StrategyCatalog> = std::sync::OnceLock::new();
    CATALOG.get_or_init(|| StrategyCatalog::new(market_valuation::strategies()))
}
#[utoipa::path(get,path="/valuations/strategies",tag="Valuations",summary="Discover valuation strategies and supported targets",description="IDs match strategy fields in valuation responses. Each entry advertises one input descriptor with targets and a required or unsupported period. These capabilities are derived from the internal typed input contract. Each strategy owns its display name, description, static display definition and Markdown documentation with LaTeX math. Display field IDs select top-level typed metrics by default. An optional RFC 6901 path selects another metric; an empty path selects the flat root metric used by ask_floor. Discovery classifies strategies by purpose as estimate or reference. Field formats define decimal precision (midpoint-to-even), currency or numeric suffix and primary emphasis. Evidence paths are RFC 6901 JSON pointers into strategy data; absent evidence is omitted. revision identifies the input/display/purpose contract and matches each query/watch result. Follow documentation_url to fetch the guide. documentation_revision changes with guide content. Both discovery and documentation support ETag revalidation.",responses((status=200,body=Vec<StrategyResponse>,headers(("ETag"=String,description="Strategy catalog content revision"))),(status=304,description="Strategy catalog is unchanged")))]
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
        })?;
    Ok(guide.response(&req))
}

#[cfg(test)]
mod tests {
    use actix_web::{App, http::header, test};
    #[actix_web::test]
    async fn query_results_contain_typed_data_and_revision_without_display_projection() {
        use utoipa::OpenApi;
        let schema = serde_json::to_value(crate::ApiDoc::openapi()).unwrap();
        let schemas = &schema["components"]["schemas"];
        assert!(schemas.get("ValuationDisplay").is_none());
        assert!(schemas.get("DisplayValue").is_none());
        assert!(
            schemas["StrategyOutcomeResponse"]["allOf"]
                .as_array()
                .unwrap()
                .iter()
                .any(|part| part["properties"]["revision"].is_object())
        );
        assert!(schemas["StrategyResponse"]["properties"]["display"].is_object());
    }
    #[actix_web::test]
    async fn catalog_revalidation_tracks_documentation_changes_and_each_guide_has_its_own_revision()
    {
        use super::*;
        fn entries(documentation: &'static str) -> [market_valuation::StrategyMetadata; 2] {
            [
                market_valuation::StrategyMetadata {
                    purpose: market_valuation::StrategyPurpose::Estimate,
                    id: "one",
                    name: "One",
                    description: "first",
                    documentation,
                    input: market_valuation::InputContract::Selection,
                    display: market_valuation::strategy("ask_floor").unwrap().display,
                },
                market_valuation::StrategyMetadata {
                    purpose: market_valuation::StrategyPurpose::Estimate,
                    id: "two",
                    name: "Two",
                    description: "second",
                    documentation: "other guide",
                    input: market_valuation::InputContract::Selection,
                    display: market_valuation::strategy("ask_floor").unwrap().display,
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
    async fn contract_revisions_track_presentation_and_input_independently_of_guides() {
        use super::*;
        let base = *market_valuation::strategy("ask_floor").unwrap();
        let mut changed = base;
        changed.documentation = "New guide content";
        let original = StrategyResponse::from(&base);
        let docs = StrategyResponse::from(&changed);
        assert_eq!(original.revision, docs.revision);
        assert_ne!(original.documentation_revision, docs.documentation_revision);
        changed = base;
        const FIELDS: &[market_valuation::FieldDefinition] =
            &[market_valuation::FieldDefinition::currency("price", "Estimate", Some(2)).primary()];
        changed.display.fields = FIELDS;
        let display = StrategyResponse::from(&changed);
        assert_ne!(original.revision, display.revision);
        assert_eq!(
            original.documentation_revision,
            display.documentation_revision
        );
        let old_catalog = StrategyCatalog::new([&base]);
        let new_catalog = StrategyCatalog::new([&changed]);
        assert_ne!(old_catalog.index.etag, new_catalog.index.etag);
        assert_eq!(
            old_catalog.guides[base.id].etag,
            new_catalog.guides[base.id].etag
        );
        changed = base;
        changed.input = market_valuation::InputContract::GiftAttributes;
        assert_ne!(original.revision, StrategyResponse::from(&changed).revision);
        changed = base;
        changed.purpose = market_valuation::StrategyPurpose::Estimate;
        assert_ne!(original.revision, StrategyResponse::from(&changed).revision);
        changed = base;
        changed.display.evidence = &[market_valuation::EvidenceDefinition {
            path: "/count",
            label: "unique gifts",
        }];
        assert_ne!(original.revision, StrategyResponse::from(&changed).revision);
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
            assert_eq!(
                row["input"],
                serde_json::to_value(super::InputRequirements::from(metadata.input)).unwrap()
            );
            assert!(row.get("targets").is_none());
            assert!(row.get("requires_period").is_none());
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
            market_valuation::strategy("gift_floor")
                .unwrap()
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
