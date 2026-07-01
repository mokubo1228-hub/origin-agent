#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffProposal {
    pub diff: String,
}

impl DiffProposal {
    pub fn new(diff: impl Into<String>) -> Self {
        Self { diff: diff.into() }
    }

    pub fn is_empty(&self) -> bool {
        self.diff.trim().is_empty()
    }
}
