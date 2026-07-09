//! Workspace / user の deny policy(P1)。
//! `.origin/policy.toml`(workspace)と `~/.origin/policy.toml`(user global)の
//! read/write deny を読み、origin の検証関門で強制するための型とマッチャ。
//!
//! 安全契約(D-POL-1〜5 / P-1〜P-5):
//! - deny-only。allow・除外・override 構文は存在しない(permission を増やせない)。
//! - built-in の safe-path 規則より弱い(そちらが先に走る)。deny はその追加分。
//! - deny は user の明示 intent より強い。解除はファイル編集のみ。
//! - パターン一覧は provider prompt にも transcript にも出さない(denied になった
//!   当該 path のみ表示可)。エラー文言には source ファイル名を含める。
//! - 壊れた policy ファイルは黙って無視せず warning として保持する(気づかない緩和が最悪)。

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// policy ファイルの最大サイズ(instruction ファイル群と同じ 64 KiB)。
const MAX_POLICY_FILE_BYTES: u64 = 64 * 1024;
/// workspace 側 policy の場所(cwd 相対)。
pub const WORKSPACE_POLICY_PATH: &str = ".origin/policy.toml";
/// user 側 policy の場所(~/.origin 相対の表示名)。
pub const USER_POLICY_LABEL: &str = "~/.origin/policy.toml";

/// deny 規則の出所。エラー文言の「どのファイルが拒否したか」に使う。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum PolicySource {
    Workspace,
    User,
}

impl PolicySource {
    /// エラー文言に埋める source ファイル名。
    pub fn label(self) -> &'static str {
        match self {
            Self::Workspace => ".origin/policy.toml",
            Self::User => "~/.origin/policy.toml",
        }
    }
}

/// 1つの deny 規則(検証済みパターン + 出所)。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PolicyRule {
    pub pattern: String,
    pub source: PolicySource,
}

/// 合成済み policy。workspace と user の deny の union(deny-only なので衝突しない)。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PolicySet {
    pub read_deny: Vec<PolicyRule>,
    pub write_deny: Vec<PolicyRule>,
    pub workspace_rules: usize,
    pub user_rules: usize,
    /// parse 失敗・無効パターンの警告(表示用)。空でなければ /status と起動時に見せる。
    pub warnings: Vec<String>,
}

impl PolicySet {
    /// deny なしの空 policy(テスト・policy ファイル不在時の既定)。
    pub fn empty() -> Self {
        Self::default()
    }

    /// workspace 相対 path が read deny に該当するか。
    pub fn denies_read(&self, relative_path: &str) -> Option<PolicySource> {
        deny_match(&self.read_deny, relative_path)
    }

    /// workspace 相対 path が write deny に該当するか。
    pub fn denies_write(&self, relative_path: &str) -> Option<PolicySource> {
        deny_match(&self.write_deny, relative_path)
    }

    /// write 拒否のエラー文言(source ファイル名入り・パターンは含めない)。
    pub fn write_denial_reason(&self, relative_path: &str) -> Option<String> {
        self.denies_write(relative_path).map(|source| {
            format!(
                "denied by {} policy ({}): {relative_path}",
                match source {
                    PolicySource::Workspace => "workspace",
                    PolicySource::User => "user",
                },
                source.label()
            )
        })
    }

    /// read 拒否のエラー文言(同上)。
    pub fn read_denial_reason(&self, relative_path: &str) -> Option<String> {
        self.denies_read(relative_path).map(|source| {
            format!(
                "denied by {} policy ({})",
                match source {
                    PolicySource::Workspace => "workspace",
                    PolicySource::User => "user",
                },
                source.label()
            )
        })
    }
}

fn deny_match(rules: &[PolicyRule], relative_path: &str) -> Option<PolicySource> {
    rules
        .iter()
        .find(|rule| pattern_matches_path(&rule.pattern, relative_path))
        .map(|rule| rule.source)
}

/// workspace と user の policy.toml を読み、union で合成する。
/// どちらのファイルも「無い」はエラーではなく空。壊れている場合は
/// そのファイル分を無視して warning を残す(黙って緩和しない)。
pub fn load_policy(workspace_root: &Path, user_origin_dir: Option<&Path>) -> PolicySet {
    let mut set = PolicySet::empty();

    let workspace_path = workspace_root.join(WORKSPACE_POLICY_PATH);
    load_policy_file(
        &workspace_path,
        PolicySource::Workspace,
        WORKSPACE_POLICY_PATH,
        &mut set,
    );

    if let Some(dir) = user_origin_dir {
        let user_path = dir.join("policy.toml");
        load_policy_file(&user_path, PolicySource::User, USER_POLICY_LABEL, &mut set);
    }

    set
}

/// HOME から `~/.origin` を解決する(無ければ None = user policy なし)。
pub fn user_origin_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(|home| PathBuf::from(home).join(".origin"))
}

/// 実行環境の標準位置(workspace の `.origin/policy.toml` + `~/.origin/policy.toml`)から
/// policy を読む本番入口。テストは `load_policy` に root を注入する。
pub fn load_policy_from_env(workspace_root: &Path) -> PolicySet {
    load_policy(workspace_root, user_origin_dir().as_deref())
}

fn load_policy_file(path: &Path, source: PolicySource, label: &str, set: &mut PolicySet) {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(_) => {
            set.warnings
                .push(format!("{label} was ignored: not readable"));
            return;
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        set.warnings
            .push(format!("{label} was ignored: not a regular file"));
        return;
    }
    if metadata.len() > MAX_POLICY_FILE_BYTES {
        set.warnings
            .push(format!("{label} was ignored: larger than 64 KiB"));
        return;
    }
    let body = match fs::read_to_string(path) {
        Ok(body) => body,
        Err(_) => {
            set.warnings
                .push(format!("{label} was ignored: not valid UTF-8 text"));
            return;
        }
    };
    let value: toml::Value = match body.parse() {
        Ok(value) => value,
        Err(_) => {
            set.warnings
                .push(format!("{label} was ignored: TOML parse error"));
            return;
        }
    };

    let mut added = 0usize;
    for (section, bucket) in [("read", 0usize), ("write", 1usize)] {
        let Some(list) = value
            .get(section)
            .and_then(|table| table.get("deny"))
            .and_then(|deny| deny.as_array())
        else {
            continue;
        };
        for entry in list {
            let Some(pattern) = entry.as_str() else {
                set.warnings.push(format!(
                    "{label}: non-string pattern in [{section}] was ignored"
                ));
                continue;
            };
            match validate_pattern(pattern) {
                Ok(()) => {
                    let rule = PolicyRule {
                        pattern: pattern.to_string(),
                        source,
                    };
                    if bucket == 0 {
                        set.read_deny.push(rule);
                    } else {
                        set.write_deny.push(rule);
                    }
                    added += 1;
                }
                Err(reason) => set
                    .warnings
                    .push(format!("{label}: pattern was ignored ({reason})")),
            }
        }
    }
    match source {
        PolicySource::Workspace => set.workspace_rules += added,
        PolicySource::User => set.user_rules += added,
    }
}

/// サポートする glob は4形のみ(契約 §3)。それ以外は無効として弾く。
/// 1. `NAME`(`/` なし。`*` 可): 任意階層のファイル/ディレクトリ名に一致
/// 2. `*.ext`: 1 の特殊形
/// 3. `dir/sub/**`: サブツリー全部
/// 4. `dir/sub/name`(`/` あり。各セグメントで `*` 可): 相対パス一致
fn validate_pattern(pattern: &str) -> Result<(), &'static str> {
    if pattern.is_empty() {
        return Err("empty pattern");
    }
    if pattern.contains(['{', '}', '[', ']', '!']) {
        return Err("unsupported glob syntax");
    }
    if pattern.starts_with('/') {
        return Err("absolute patterns are not supported");
    }
    if pattern.contains("**") && !pattern.ends_with("/**") {
        return Err("`**` is only supported as a trailing `/**`");
    }
    if pattern == "/**" || pattern == "**" {
        return Err("deny-everything patterns are not supported");
    }
    Ok(())
}

/// 検証済みパターンを workspace 相対 path(`/` 区切り)に適用する。
fn pattern_matches_path(pattern: &str, relative_path: &str) -> bool {
    let path = relative_path.trim_start_matches("./");

    if let Some(prefix) = pattern.strip_suffix("/**") {
        let prefix_segments: Vec<&str> = prefix.split('/').collect();
        let path_segments: Vec<&str> = path.split('/').collect();
        if path_segments.len() < prefix_segments.len() {
            return false;
        }
        return prefix_segments
            .iter()
            .zip(path_segments.iter())
            .all(|(pattern_segment, path_segment)| segment_matches(pattern_segment, path_segment));
    }

    if pattern.contains('/') {
        let pattern_segments: Vec<&str> = pattern.split('/').collect();
        let path_segments: Vec<&str> = path.split('/').collect();
        return pattern_segments.len() == path_segments.len()
            && pattern_segments.iter().zip(path_segments.iter()).all(
                |(pattern_segment, path_segment)| segment_matches(pattern_segment, path_segment),
            );
    }

    // `/` を含まないパターンは任意階層の1セグメントに一致する。
    path.split('/')
        .any(|segment| segment_matches(pattern, segment))
}

/// 1セグメント内の `*` ワイルドカード一致(`/` を跨がない)。
fn segment_matches(pattern: &str, segment: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == segment;
    }

    let mut rest = segment;
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if index == 0 {
            let Some(stripped) = rest.strip_prefix(part) else {
                return false;
            };
            rest = stripped;
        } else if index == parts.len() - 1 {
            return rest.ends_with(part);
        } else {
            let Some(found) = rest.find(part) else {
                return false;
            };
            rest = &rest[found + part.len()..];
        }
    }
    // 末尾 part が空(パターンが `*` で終わる)なら残りは何でもよい。
    parts.last().is_some_and(|part| part.is_empty()) || rest.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("origin-policy-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_workspace_policy(root: &Path, body: &str) {
        fs::create_dir_all(root.join(".origin")).unwrap();
        fs::write(root.join(WORKSPACE_POLICY_PATH), body).unwrap();
    }

    #[test]
    fn policy_glob_matches_documented_subset_only() {
        // (pattern, path, expected)
        let table = [
            (".env*", ".env", true),
            (".env*", "config/.env.local", true),
            (".env*", "docs/env.md", false),
            ("*.pem", "certs/server.pem", true),
            ("*.pem", "certs/server.pem.txt", false),
            ("secrets/**", "secrets/a.txt", true),
            ("secrets/**", "secrets/deep/b.txt", true),
            ("secrets/**", "secret/a.txt", false),
            ("docs/archive/**", "docs/archive/old.md", true),
            ("docs/archive/**", "docs/archives/old.md", false),
            ("CHANGELOG.md", "CHANGELOG.md", true),
            ("CHANGELOG.md", "docs/CHANGELOG.md", true),
            ("docs/plan.md", "docs/plan.md", true),
            ("docs/plan.md", "plan.md", false),
            ("docs/*.md", "docs/plan.md", true),
            ("docs/*.md", "docs/deep/plan.md", false),
        ];
        for (pattern, path, expected) in table {
            assert_eq!(
                pattern_matches_path(pattern, path),
                expected,
                "pattern {pattern} vs {path}"
            );
        }

        // サポート外の記法は無効。
        for bad in ["{a,b}", "a[0]", "!secrets", "a/**/b", "**", ""] {
            assert!(validate_pattern(bad).is_err(), "should reject {bad}");
        }
    }

    #[test]
    fn policy_load_merges_user_and_workspace_as_union() {
        let workspace = temp_dir("union-workspace");
        let user = temp_dir("union-user");
        write_workspace_policy(
            &workspace,
            "[read]\ndeny=[\"secrets/**\"]\n[write]\ndeny=[\"docs/archive/**\"]\n",
        );
        fs::write(
            user.join("policy.toml"),
            "[write]\ndeny=[\"CHANGELOG.md\"]\n",
        )
        .unwrap();

        let set = load_policy(&workspace, Some(&user));

        assert!(set.warnings.is_empty());
        assert_eq!(set.workspace_rules, 2);
        assert_eq!(set.user_rules, 1);
        assert_eq!(
            set.denies_read("secrets/key.txt"),
            Some(PolicySource::Workspace)
        );
        assert_eq!(
            set.denies_write("docs/archive/x.md"),
            Some(PolicySource::Workspace)
        );
        assert_eq!(set.denies_write("CHANGELOG.md"), Some(PolicySource::User));
        assert_eq!(set.denies_write("README.md"), None);
    }

    #[test]
    fn policy_load_ignores_broken_file_with_warning() {
        let workspace = temp_dir("broken");
        write_workspace_policy(&workspace, "this is not toml [[[");

        let set = load_policy(&workspace, None);

        assert!(set.read_deny.is_empty() && set.write_deny.is_empty());
        assert_eq!(set.warnings.len(), 1);
        assert!(set.warnings[0].contains("TOML parse error"));
    }

    #[test]
    fn policy_missing_files_yield_empty_policy() {
        let workspace = temp_dir("missing");
        let set = load_policy(&workspace, Some(&workspace.join("no-origin")));
        assert_eq!(set, PolicySet::empty());
    }

    #[test]
    fn policy_invalid_patterns_are_ignored_with_warning() {
        let workspace = temp_dir("invalid-pattern");
        write_workspace_policy(
            &workspace,
            "[write]\ndeny=[\"{bad}\", \"docs/archive/**\", 42]\n",
        );

        let set = load_policy(&workspace, None);

        assert_eq!(set.write_deny.len(), 1);
        assert_eq!(set.warnings.len(), 2);
        assert!(set.denies_write("docs/archive/x.md").is_some());
    }

    #[test]
    fn policy_denial_reasons_name_source_file_without_patterns() {
        let workspace = temp_dir("reason");
        write_workspace_policy(&workspace, "[write]\ndeny=[\"docs/archive/**\"]\n");
        let set = load_policy(&workspace, None);

        let reason = set.write_denial_reason("docs/archive/x.md").unwrap();
        assert!(reason.contains(".origin/policy.toml"));
        assert!(reason.contains("docs/archive/x.md"));
        assert!(!reason.contains("**"));
        assert!(set.write_denial_reason("README.md").is_none());
    }
}
