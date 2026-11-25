//! Public visible portfolios. Numeric IDs identify subjects; Telegram still enforces visibility.
pub type PortfolioReadQuery = market_portfolio::PortfolioReadQuery<
    market_actors::input::CollectionRef,
    market_actors::input::ModelRef,
    market_actors::input::BackdropRef,
    market_actors::input::PatternRef,
>;
use crate::{
    MarketApi,
    error::{ApiError, ErrorCode, Result},
};
use actix_web::{HttpResponse, web};
use market_portfolio::{PortfolioQuery, PortfolioState, UserId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Public watches carry progress and complete totals, never the full inventory.
#[derive(Debug, Serialize, ToSchema)]
pub struct PortfolioResponse<'a> {
    pub user_id: UserId,
    pub refresh_id: u64,
    pub progress: &'a market_portfolio::PortfolioProgress,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inventory: &'a Option<market_portfolio::InventorySummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valuation: Option<PortfolioValuationSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub refresh_after: Option<jiff::Timestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<&'a str>,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct PortfolioValuationSummary {
    pub version: market_portfolio::PortfolioVersion,
    pub currency_id: market_gifts::CurrencyId,
    #[schema(value_type = String, format = DateTime)]
    pub evaluated_at: jiff::Timestamp,
    pub stale: bool,
    #[serde(flatten)]
    pub selection: market_portfolio::PortfolioSelection,
}
impl<'a> From<&'a PortfolioState> for PortfolioResponse<'a> {
    fn from(state: &'a PortfolioState) -> Self {
        Self {
            user_id: state.user_id,
            refresh_id: state.refresh_id,
            progress: &state.progress,
            inventory: &state.inventory,
            refresh_after: state.refresh_after,
            error: state.error.as_deref(),
            valuation: state.valuation.as_ref().zip(state.version.as_ref()).map(
                |(report, version)| PortfolioValuationSummary {
                    version: version.clone(),
                    currency_id: report.currency_id.clone(),
                    evaluated_at: report.evaluated_at,
                    stale: report.stale,
                    selection: market_portfolio::PortfolioSelection::full(report),
                },
            ),
        }
    }
}
fn failure(error: market_portfolio::PortfolioError) -> ApiError {
    use market_portfolio::PortfolioError as E;
    let code = match &error {
        E::RestartRead => ErrorCode::RestartRead,
        E::Busy | E::Cooldown(_) => ErrorCode::CapacityExceeded,
        E::InvalidData(_) => ErrorCode::InvalidQuery,
        E::Inventory(
            market_portfolio::InventoryError::Forbidden
            | market_portfolio::InventoryError::Authorization,
        ) => ErrorCode::Forbidden,
        _ => ErrorCode::Unavailable,
    };
    ApiError {
        code,
        message: error.to_string(),
    }
}
impl MarketApi {
    fn portfolio_service(&self) -> Result<&market_portfolio::PortfolioHandle> {
        self.portfolio
            .as_ref()
            .ok_or_else(|| ApiError::unavailable("Telegram portfolios are not configured"))
    }
    pub async fn portfolio_state(
        &self,
        query: PortfolioQuery,
    ) -> Result<std::sync::Arc<PortfolioState>> {
        self.portfolio_service()?
            .query(query)
            .await
            .map_err(failure)
    }
    pub async fn portfolio_query(
        &self,
        query: PortfolioReadQuery,
    ) -> Result<market_portfolio::PortfolioPage> {
        query.validate().map_err(failure)?;
        let input = market_actors::input::GiftFilters {
            collection_id: query.filters.collection_id,
            currency_id: query.currency_id.clone(),
            venue_id: None,
            models: query.filters.models,
            backdrops: query.filters.backdrops,
            patterns: query.filters.patterns,
            number: None,
            price: None,
        };
        let uses_aliases = input.has_aliases();
        let filters = if uses_aliases {
            self.data.resolve_filters(input).await?
        } else {
            input
                .into_resolved()
                .map_err(|e| ApiError::invalid(e.to_string()))?
        };
        let mut query = market_portfolio::ResolvedPortfolioReadQuery {
            user_id: query.user_id,
            currency_id: query.currency_id,
            filters: market_portfolio::ResolvedPortfolioFilters {
                collection_id: filters.collection_id,
                models: filters.models,
                backdrops: filters.backdrops,
                patterns: filters.patterns,
                number: query.filters.number,
            },
            sort: query.sort,
            offset: query.offset,
            limit: query.limit,
            version: query.version,
        };
        query.validate().map_err(failure)?;
        let mut state = self
            .portfolio_service()?
            .query(query.subject())
            .await
            .map_err(failure)?;
        if state.valuation.is_none() {
            return Err(ApiError::unavailable("Portfolio is still being checked"));
        }
        if uses_aliases {
            use sha2::{Digest, Sha256};
            let base = state
                .version
                .as_ref()
                .ok_or_else(|| failure(market_portfolio::PortfolioError::RestartRead))?;
            let base_text: String = base.clone().into();
            let mut digest = Sha256::new();
            digest.update(b"portfolio-alias-selection-v1\0");
            digest.update(base_text.as_bytes());
            digest.update(
                serde_json::to_vec(&query.filters)
                    .map_err(|_| ApiError::unavailable("Cannot encode portfolio selection"))?,
            );
            let mut token = format!("{:x}", digest.finalize());
            token.truncate(32);
            let version = market_portfolio::PortfolioVersion::try_from(token)
                .map_err(|_| ApiError::unavailable("Cannot version portfolio selection"))?;
            if query.offset == 0 && query.version.as_ref() == Some(base) {
                query.version = Some(version.clone());
            }
            let mut selected = state.as_ref().clone();
            selected.version = Some(version);
            state = std::sync::Arc::new(selected);
        }
        let reads = self.portfolio_reads.clone();
        self.prepare_response(move |stop| {
            reads
                .read(&state, query, || {
                    if stop.is_cancelled() {
                        Err(market_portfolio::PortfolioError::Stopped)
                    } else {
                        Ok(())
                    }
                })
                .map_err(failure)
        })
        .await
    }
    pub async fn portfolio_refresh(
        &self,
        query: PortfolioQuery,
    ) -> Result<std::sync::Arc<PortfolioState>> {
        self.portfolio_service()?
            .refresh(query)
            .await
            .map_err(failure)
    }
    pub(crate) async fn portfolio_watch(
        &self,
        query: PortfolioQuery,
    ) -> Result<market_portfolio::PortfolioWatch> {
        self.portfolio_service()?
            .watch(query)
            .await
            .map_err(failure)
    }
}
#[utoipa::path(get,path="/portfolios/{user_id}",tag="Portfolios",summary="Portfolio progress and totals without gift rows",params(("user_id"=UserId,Path),CurrencyBody),responses((status=200,body=PortfolioResponse),(status=400,body=crate::error::ErrorResponse),(status=503,body=crate::error::ErrorResponse)))]
pub(crate) async fn query(
    state: web::Data<MarketApi>,
    user: web::Path<UserId>,
    currency: web::Query<CurrencyBody>,
) -> Result<HttpResponse> {
    let query = PortfolioQuery {
        user_id: user.into_inner(),
        currency_id: currency.into_inner().currency_id,
    };
    let result = state.portfolio_state(query).await?;
    Ok(HttpResponse::Ok()
        .insert_header(("cache-control", "no-store"))
        .content_type("application/json")
        .body(
            state
                .encode_response(move |writer| {
                    serde_json::to_writer(writer, &PortfolioResponse::from(result.as_ref()))
                })
                .await?,
        ))
}
#[utoipa::path(post,path="/portfolios/{user_id}/refresh",tag="Portfolios",params(("user_id"=UserId,Path)),request_body=CurrencyBody,responses((status=200,body=PortfolioResponse),(status=400,body=crate::error::ErrorResponse),(status=429,body=crate::error::ErrorResponse),(status=503,body=crate::error::ErrorResponse)))]
pub(crate) async fn refresh(
    state: web::Data<MarketApi>,
    user: web::Path<UserId>,
    body: web::Json<CurrencyBody>,
) -> Result<HttpResponse> {
    let query = PortfolioQuery {
        user_id: user.into_inner(),
        currency_id: body.into_inner().currency_id,
    };
    let result = state.portfolio_refresh(query).await?;
    Ok(HttpResponse::Ok()
        .insert_header(("cache-control", "no-store"))
        .content_type("application/json")
        .body(
            state
                .encode_response(move |writer| {
                    serde_json::to_writer(writer, &PortfolioResponse::from(result.as_ref()))
                })
                .await?,
        ))
}
#[utoipa::path(post,path="/portfolios/query",tag="Portfolios",summary="Page through a filtered and sorted portfolio",description="Defaults to 100 gifts, at most 100 per request. Filters, sorting and selection totals cover the entire inventory. Reuse the returned version for subsequent pages; changed item data returns restart_read. Progress and totals come from portfolio.watch or GET /portfolios/{user_id} without downloading gift rows.",request_body=PortfolioReadQuery,responses((status=200,body=market_portfolio::PortfolioPage),(status=400,body=crate::error::ErrorResponse),(status=409,body=crate::error::ErrorResponse),(status=503,body=crate::error::ErrorResponse)))]
pub(crate) async fn read(
    state: web::Data<MarketApi>,
    query: web::Json<PortfolioReadQuery>,
) -> Result<HttpResponse> {
    let result = state.portfolio_query(query.into_inner()).await?;
    let bytes = state
        .encode_response(move |writer| serde_json::to_writer(writer, &result))
        .await?;
    Ok(HttpResponse::Ok()
        .insert_header(("cache-control", "no-store"))
        .content_type("application/json")
        .body(bytes))
}
#[derive(Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct CurrencyBody {
    pub currency_id: market_gifts::CurrencyId,
}
