//! Composition adapter: native Telegram pages already use the shared gift inventory contracts.
pub(crate) struct TelegramInventoryProvider(pub market_telegram::InventoryClient);
impl market_portfolio::InventoryProvider for TelegramInventoryProvider {
    fn page(
        &self,
        request: market_portfolio::InventoryPageRequest,
    ) -> impl market_invocations::Invocation<
        Output = market_portfolio::InventoryPage,
        Error = market_portfolio::InventoryError,
    > {
        self.0.page_operation(
            request.user.get(),
            request.cursor.unwrap_or_default(),
            request.limit,
        )
    }
}
