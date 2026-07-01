pub mod approval;
pub mod diff_extract;
pub mod file_read;
pub mod git;
pub mod patch;
pub mod workspace;

pub use approval::{parse_approval_decision, ApprovalDecision};
pub use diff_extract::{extract_diff_proposal, DiffExtractError};
pub use file_read::{read_workspace_files, ReadFile, ReadFileError, MAX_FILE_BYTES};
pub use git::{
    discover_git_repo, discover_git_repo_from, git_apply, git_apply_args, git_apply_check,
    git_dirty_status, GitApplyMode, GitError, GitRepo, GitStatus,
};
pub use patch::{
    changed_files_from_diff, validate_diff_integrity, validate_diff_targets,
    validate_diff_targets_allowing_new_files, DiffIntegrityError, DiffIntegrityReport,
    DiffValidationError, DiffValidationReport,
};
pub use workspace::{Workspace, WorkspaceError};
