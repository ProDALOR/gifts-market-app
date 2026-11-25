use super::*;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PortfolioMode {
    #[default]
    Private,
    Inline,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PortfolioAction {
    #[default]
    Open,
    Navigate,
    Refresh,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortfolioRequest {
    pub user_id: UserId,
    pub currency_id: BotCurrency,
    pub action: PortfolioAction,
    pub page: usize,
    pub sort: PortfolioSort,
    pub mode: PortfolioMode,
    pub layout: GridMode,
}
pub(super) const SORTS: [(PortfolioSort, &str, &str); 6] = [
    (PortfolioSort::PositionAsc, "position_asc", "pa"),
    (PortfolioSort::PositionDesc, "position_desc", "pd"),
    (PortfolioSort::NumberAsc, "number_asc", "na"),
    (PortfolioSort::NumberDesc, "number_desc", "nd"),
    (PortfolioSort::ValueAsc, "price_asc", "va"),
    (PortfolioSort::ValueDesc, "price_desc", "vd"),
];
pub(super) fn parse_sort(value: &str) -> Option<PortfolioSort> {
    SORTS
        .iter()
        .find(|(_, name, code)| *name == value || *code == value)
        .map(|(sort, _, _)| *sort)
}
impl PortfolioRequest {
    pub(super) fn data(&self, page: usize, action: PortfolioAction, sort: PortfolioSort) -> String {
        let code = SORTS
            .iter()
            .find(|(candidate, _, _)| *candidate == sort)
            .unwrap()
            .2;
        let data = format!(
            "p:{}:{}:{}:{page}:{code}",
            self.user_id.get(),
            self.currency_id,
            if action == PortfolioAction::Refresh {
                "r"
            } else {
                "v"
            }
        );
        if self.layout == GridMode::Gallery {
            format!("{data}:g")
        } else {
            data
        }
    }
    pub(super) fn inline_query(&self) -> String {
        let name = SORTS
            .iter()
            .find(|(candidate, _, _)| *candidate == self.sort)
            .unwrap()
            .1;
        let query = format!(
            "portfolio {} {} sort={name}",
            self.user_id.get(),
            self.currency_id
        );
        if self.layout == GridMode::Gallery {
            format!("{query} view=gallery")
        } else {
            query
        }
    }
    pub(super) fn page_size(&self) -> usize {
        if self.layout == GridMode::Gallery {
            market_media::MAX_GALLERY_CELLS
        } else {
            PAGE_SIZE
        }
    }
    pub(super) fn check_command(&self) -> String {
        format!("/portfolio {} {}", self.user_id.get(), self.currency_id)
    }
}
pub(in crate::commands) fn parse_args(
    text: &str,
    user_id: u64,
    mode: PortfolioMode,
) -> Option<PortfolioRequest> {
    if text.len() > 512 {
        return None;
    }
    let mut request = PortfolioRequest {
        user_id: UserId::new(user_id).ok()?,
        currency_id: BotCurrency::Gram,
        action: PortfolioAction::Open,
        page: 0,
        sort: PortfolioSort::PositionAsc,
        mode,
        layout: GridMode::Linked,
    };
    let (mut user, mut currency, mut sort, mut layout) = (false, false, false, false);
    for word in text.split_whitespace() {
        if let Some(value) = word.strip_prefix("view=") {
            if layout {
                return None;
            }
            request.layout = GridMode::parse(value)?;
            layout = true;
        } else if let Some(value) = word.strip_prefix("sort=") {
            if sort {
                return None;
            }
            request.sort = parse_sort(value)?;
            sort = true;
        } else if word.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '+') {
            if user {
                return None;
            }
            request.user_id = UserId::new(word.parse().ok()?).ok()?;
            user = true;
        } else {
            if currency {
                return None;
            }
            request.currency_id = BotCurrency::parse(word)?;
            currency = true;
        }
    }
    Some(request)
}
pub(in crate::commands) fn inline(text: &str, user: u64) -> Option<PortfolioRequest> {
    let text = text.trim();
    if text.is_empty() {
        return parse_args("", user, PortfolioMode::Inline);
    }
    let (name, args) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
    if !matches!(name, "portfolio" | "/portfolio") {
        return None;
    }
    parse_args(args, user, PortfolioMode::Inline)
}
pub(in crate::commands) fn callback(text: &str) -> Option<PortfolioRequest> {
    let mut p = text.strip_prefix("p:")?.split(':');
    if text.len() > 64 {
        return None;
    }
    let user_id = UserId::new(p.next()?.parse().ok()?).ok()?;
    let currency_id = BotCurrency::parse(p.next()?)?;
    let action = p.next()?;
    let page = p.next()?.parse::<usize>().ok()?;
    let sort = p
        .next()
        .map(parse_sort)
        .unwrap_or(Some(PortfolioSort::PositionAsc))?;
    let layout = match p.next() {
        None => GridMode::Linked,
        Some("g") => GridMode::Gallery,
        _ => return None,
    };
    if page > 999 {
        return None;
    }
    if p.next().is_some() || !matches!(action, "r" | "v") {
        return None;
    }
    Some(PortfolioRequest {
        user_id,
        currency_id,
        action: if action == "r" {
            PortfolioAction::Refresh
        } else {
            PortfolioAction::Navigate
        },
        page,
        sort,
        mode: PortfolioMode::Private,
        layout,
    })
}
