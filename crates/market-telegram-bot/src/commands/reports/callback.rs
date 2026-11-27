//! Compact native callback encoding: report identity plus an absolute typed destination.
use super::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportCallback {
    pub(in crate::commands) id: u64,
    pub(in crate::commands) action: ReportAction,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::commands) enum ReportAction {
    Gift(views::Destination),
    Market(browse::Destination),
    Portfolio {
        page: usize,
        sort: market_portfolio::PortfolioSort,
        layout: GridMode,
        action: PortfolioAction,
    },
}
impl ReportCallback {
    pub(in crate::commands) fn new(id: u64, action: ReportAction) -> Self {
        Self { id, action }
    }
}
fn mode(value: &str) -> Option<GridMode> {
    match value {
        "l" => Some(GridMode::Linked),
        "g" => Some(GridMode::Gallery),
        _ => None,
    }
}
fn flag(value: &str) -> Option<bool> {
    match value {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}
impl std::fmt::Display for ReportCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "r:{:016x}:", self.id)?;
        let mode = |mode| if mode == GridMode::Linked { "l" } else { "g" };
        match self.action {
            ReportAction::Gift(d) => write!(
                f,
                "g:{}:{}:{}:{}:{}",
                if d.tab == Tab::Floors { "f" } else { "s" },
                match d.scope {
                    Scope::Pair => "p",
                    Scope::Model => "m",
                    Scope::Backdrop => "b",
                    Scope::Collection => "c",
                },
                d.page,
                mode(d.mode),
                u8::from(d.refresh)
            ),
            ReportAction::Market(d) => {
                write!(f, "m:{}:{}:{}", d.page, mode(d.mode), u8::from(d.refresh))
            }
            ReportAction::Portfolio {
                page,
                sort,
                layout,
                action,
            } => write!(
                f,
                "p:{}:{page}:{}:{}",
                portfolio::sort_code(sort),
                mode(layout),
                match action {
                    PortfolioAction::Refresh => "r",
                    PortfolioAction::Preview => "g",
                    _ => "v",
                }
            ),
        }
    }
}
impl std::str::FromStr for ReportCallback {
    type Err = ();
    fn from_str(data: &str) -> Result<Self, ()> {
        fn decode(data: &str) -> Option<ReportCallback> {
            if data.len() > 64 {
                return None;
            }
            let mut p = data.strip_prefix("r:")?.split(':');
            let id = interactions::callback_id(p.next()?)?;
            let action = match p.next()? {
                "g" => {
                    let tab = match p.next()? {
                        "f" => Tab::Floors,
                        "s" => Tab::Sales,
                        _ => return None,
                    };
                    let scope = match p.next()? {
                        "p" => Scope::Pair,
                        "m" => Scope::Model,
                        "b" => Scope::Backdrop,
                        "c" => Scope::Collection,
                        _ => return None,
                    };
                    let page = p.next()?.parse().ok()?;
                    if !(1..=history::MAX_PAGES).contains(&page) {
                        return None;
                    }
                    ReportAction::Gift(views::Destination {
                        tab,
                        scope,
                        page,
                        mode: mode(p.next()?)?,
                        refresh: flag(p.next()?)?,
                    })
                }
                "m" => {
                    let page = p.next()?.parse().ok()?;
                    if page >= browse::MAX_PAGES {
                        return None;
                    }
                    ReportAction::Market(browse::Destination {
                        page,
                        mode: mode(p.next()?)?,
                        refresh: flag(p.next()?)?,
                    })
                }
                "p" => {
                    let sort = portfolio::parse_sort(p.next()?)?;
                    let page = p.next()?.parse().ok()?;
                    if page > 999 {
                        return None;
                    }
                    let layout = mode(p.next()?)?;
                    let action = match p.next()? {
                        "v" => PortfolioAction::Navigate,
                        "r" => PortfolioAction::Refresh,
                        "g" => PortfolioAction::Preview,
                        _ => return None,
                    };
                    ReportAction::Portfolio {
                        page,
                        sort,
                        layout,
                        action,
                    }
                }
                _ => return None,
            };
            p.next().is_none().then_some(ReportCallback { id, action })
        }
        decode(data).ok_or(())
    }
}
