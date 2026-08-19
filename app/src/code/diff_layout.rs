/// Diff layout mode for code review.
/// Stored in memory only (not persisted to settings).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DiffLayout {
    #[default]
    Inline,
    SideBySide,
}

impl DiffLayout {
    pub fn label(self) -> &'static str {
        match self {
            Self::Inline => "Inline",
            Self::SideBySide => "Side by side",
        }
    }

    pub fn is_side_by_side(self) -> bool {
        matches!(self, Self::SideBySide)
    }
}
