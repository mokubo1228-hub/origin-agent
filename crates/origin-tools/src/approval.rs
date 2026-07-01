#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalDecision {
    Approved,
    Rejected,
}

pub fn parse_approval_decision(input: &str) -> ApprovalDecision {
    match input.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => ApprovalDecision::Approved,
        _ => ApprovalDecision::Rejected,
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_approval_decision, ApprovalDecision};

    #[test]
    fn approves_y_and_yes() {
        assert_eq!(parse_approval_decision("y\n"), ApprovalDecision::Approved);
        assert_eq!(parse_approval_decision("YES"), ApprovalDecision::Approved);
    }

    #[test]
    fn rejects_enter_no_and_other_values() {
        assert_eq!(parse_approval_decision("\n"), ApprovalDecision::Rejected);
        assert_eq!(parse_approval_decision("n"), ApprovalDecision::Rejected);
        assert_eq!(parse_approval_decision("maybe"), ApprovalDecision::Rejected);
    }
}
