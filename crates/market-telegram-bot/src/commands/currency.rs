use market_gifts::CurrencyId;

/// Currencies supported by Telegram reports and their compact callback codec.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BotCurrency {
    #[default]
    Gram,
    Stars,
}
impl BotCurrency {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_uppercase().as_str() {
            "GRAM" | "TON" => Some(Self::Gram),
            "XTR" | "STARS" => Some(Self::Stars),
            _ => None,
        }
    }
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gram => "GRAM",
            Self::Stars => "XTR",
        }
    }
    pub fn id(self) -> CurrencyId {
        self.as_str().into()
    }
}
impl std::fmt::Display for BotCurrency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aliases_are_case_insensitive_and_unknown_or_unsafe_values_are_rejected() {
        for (words, expected) in [
            (vec!["GRAM", "gram", "Ton", "ton"], BotCurrency::Gram),
            (vec!["XTR", "xtr", "Stars", "stars"], BotCurrency::Stars),
        ] {
            for word in words {
                assert_eq!(BotCurrency::parse(word), Some(expected));
            }
        }
        for word in ["", "USD", "GRAM:foo", " GRAM", "TON\n", &"a".repeat(256)] {
            assert_eq!(BotCurrency::parse(word), None);
        }
    }
}
