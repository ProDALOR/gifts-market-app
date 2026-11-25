/// Presentation changes never change the underlying query or gift identity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GridMode {
    #[default]
    Linked,
    Gallery,
}
impl GridMode {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "linked" | "list" => Some(Self::Linked),
            "gallery" => Some(Self::Gallery),
            _ => None,
        }
    }
}
