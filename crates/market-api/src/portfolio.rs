//! Public visible portfolios. Numeric IDs identify subjects; Telegram still enforces visibility.
use crate::{
    MarketApi,
    error::{ApiError, ErrorCode, Result},
};
use actix_web::{HttpResponse, web};
use market_portfolio::{PortfolioQuery, PortfolioState, UserId};
use serde::Deserialize;
fn failure(error: market_portfolio::PortfolioError) -> ApiError {
    use market_portfolio::PortfolioError as E;
    let code = match &error {
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
    pub async fn portfolio_query(
        &self,
        query: PortfolioQuery,
    ) -> Result<std::sync::Arc<PortfolioState>> {
        self.portfolio_service()?
            .query(query)
            .await
            .map_err(failure)
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
#[utoipa::path(get,path="/portfolios/{user_id}",tag="Portfolios",params(("user_id"=UserId,Path),CurrencyBody),responses((status=200,body=PortfolioState),(status=400,body=crate::error::ErrorResponse),(status=503,body=crate::error::ErrorResponse)))]
pub(crate) async fn query(
    state: web::Data<MarketApi>,
    user: web::Path<UserId>,
    currency: web::Query<CurrencyBody>,
) -> Result<HttpResponse> {
    let query = PortfolioQuery {
        user_id: user.into_inner(),
        currency_id: currency.into_inner().currency_id,
    };
    let result = state.portfolio_query(query).await?;
    Ok(HttpResponse::Ok()
        .insert_header(("cache-control", "no-store"))
        .content_type("application/json")
        .body(
            state
                .encode_response(move |writer| serde_json::to_writer(writer, result.as_ref()))
                .await?,
        ))
}
#[utoipa::path(post,path="/portfolios/{user_id}/refresh",tag="Portfolios",params(("user_id"=UserId,Path)),request_body=CurrencyBody,responses((status=200,body=PortfolioState),(status=400,body=crate::error::ErrorResponse),(status=429,body=crate::error::ErrorResponse),(status=503,body=crate::error::ErrorResponse)))]
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
                .encode_response(move |writer| serde_json::to_writer(writer, result.as_ref()))
                .await?,
        ))
}
#[derive(Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct CurrencyBody {
    pub currency_id: market_gifts::CurrencyId,
}
