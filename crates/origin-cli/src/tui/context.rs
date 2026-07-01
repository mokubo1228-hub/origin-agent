//! Ask / Agent の安全な file context と bounded workspace overview の構築。
//! workspace 外・secret/binary/large file は内容を載せず、provider へ渡す文脈を狭く保つ。
use super::*;

// workspace overview を provider へ渡す際の bound 群。件数・深さ・バイト数を固定上限で
// 抑えることで、巨大 workspace でも文脈が無制限に膨らまないようにする安全境界。
/// 木を辿る再帰の深さ上限。
pub(super) const WORKSPACE_OVERVIEW_MAX_DEPTH: usize = 3;
/// 列挙するエントリ(パス)総数の上限。
pub(super) const WORKSPACE_OVERVIEW_MAX_ENTRIES: usize = 100;
/// 内容まで読み込む overview ファイル数の上限。
pub(super) const WORKSPACE_OVERVIEW_MAX_FILES: usize = 10;
/// overview として provider に載せる合計バイト数の上限(文脈を狭く保つ)。
pub(super) const WORKSPACE_OVERVIEW_MAX_BYTES: usize = 64 * 1024;
/// ユーザーに表示する skip 警告の最大件数(ノイズ抑制の表示上限)。
pub(super) const WORKSPACE_OVERVIEW_MAX_WARNINGS: usize = 3;

/// prompt 由来の file context 構築結果。安全に取り込めた本文 file_contexts(provider へ渡す)、
/// 採否を残した records、任意の bounded overview、UI 表示用 messages を束ねる。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AskFileContextResult {
    pub(super) file_contexts: Vec<ChatFileContext>,
    pub(super) file_context_records: Vec<ChatFileContextRecord>,
    pub(super) workspace_overview: Option<ChatWorkspaceOverviewContext>,
    pub(super) messages: Vec<TuiMessage>,
}

/// bounded workspace overview の構築結果。provider 用の overview context と UI メッセージを返す。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AskWorkspaceOverviewResult {
    pub(super) context: Option<ChatWorkspaceOverviewContext>,
    pub(super) messages: Vec<TuiMessage>,
}

/// bound 内で列挙した workspace の木。entries はパス列挙のみ(内容は持たない)で、
/// skipped_items / warnings に「載せなかったもの」を残して可視化する。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct WorkspaceOverviewTree {
    pub(super) entries: Vec<String>,
    pub(super) skipped_items: usize,
    pub(super) warnings: Vec<(String, String)>,
}

/// planner が出した新規作成パスが安全に作れるかを分類する検証段。
/// 危険パス・あいまいなパス・既存ファイルとの衝突は Rejected、親解決不能は Missing、
/// 単一の安全な相対パスで親ディレクトリが実在する時だけ NewFile を返す(narrow safe exception)。
pub(super) fn agent_new_file_target_for_path(workspace: &Workspace, path: &str) -> AgentPathTarget {
    if let Some(reason) = unsafe_agent_edit_target_path_reason(path) {
        return AgentPathTarget::Rejected {
            path: path.to_string(),
            reason,
        };
    }
    if !is_clear_prompt_file_path(path) {
        return AgentPathTarget::Rejected {
            path: path.to_string(),
            reason: "new file targets must look like explicit editable file paths".to_string(),
        };
    }
    if workspace.resolve_existing_file(path).is_ok() {
        return AgentPathTarget::Rejected {
            path: path.to_string(),
            reason: "file already exists".to_string(),
        };
    }

    // 親が無い/空(= current directory 直下)なら作成可。親があるなら、それが実在する
    // ディレクトリである時だけ許可し、勝手にディレクトリを作らせない。
    let relative = Path::new(path);
    let Some(parent) = relative.parent() else {
        return AgentPathTarget::NewFile {
            path: path.to_string(),
        };
    };
    if parent.as_os_str().is_empty() {
        return AgentPathTarget::NewFile {
            path: path.to_string(),
        };
    }
    let parent_path = parent.to_string_lossy().to_string();
    match workspace.resolve_existing_file(&parent_path) {
        Ok(resolved) if resolved.is_dir() => AgentPathTarget::NewFile {
            path: path.to_string(),
        },
        Ok(_) => AgentPathTarget::Rejected {
            path: path.to_string(),
            reason: "new file parent path is not a directory".to_string(),
        },
        Err(WorkspaceError::CanonicalizePath { .. }) => AgentPathTarget::Missing {
            path: path.to_string(),
        },
        Err(error) => AgentPathTarget::Rejected {
            path: path.to_string(),
            reason: error.to_string(),
        },
    }
}

/// パスを「Agent が安全に何をできるか」で分類した結果。書き込み可否はこの種別で決まる。
/// Included=既存を読める / NewFile=新規作成できる / External=workspace 外 /
/// Rejected=安全境界違反 / Missing=解決不能。後段はこの分類を見て review か clarification を選ぶ。
pub(super) enum AgentPathTarget {
    Included { path: String, bytes: u64 },
    NewFile { path: String },
    External { path: String, reason: String },
    Rejected { path: String, reason: String },
    Missing { path: String },
}

/// planner が出した既存ファイル編集ターゲットを解決・安全検証する。
/// absolute / 危険パス / workspace 外は弾き、解決後の相対パスも再検証してから内容を読む。
/// 存在しない(canonicalize 失敗)場合は新規作成判定に委譲する。
pub(super) fn agent_edit_target_for_path(workspace: &Workspace, path: &str) -> AgentPathTarget {
    if Path::new(path).is_absolute() {
        return AgentPathTarget::External {
            path: path.to_string(),
            reason: "absolute paths are outside the current workspace edit boundary".to_string(),
        };
    }

    if let Some(reason) = unsafe_agent_edit_target_path_reason(path) {
        return AgentPathTarget::Rejected {
            path: path.to_string(),
            reason,
        };
    }

    let resolved = match workspace.resolve_existing_file(path) {
        Ok(resolved) => resolved,
        Err(WorkspaceError::CanonicalizePath { .. }) => {
            return agent_new_file_target_for_path(workspace, path);
        }
        Err(WorkspaceError::OutsideWorkspace { .. }) => {
            return AgentPathTarget::External {
                path: path.to_string(),
                reason: "path resolves outside the current working directory".to_string(),
            };
        }
        Err(error) => {
            return AgentPathTarget::Rejected {
                path: path.to_string(),
                reason: error.to_string(),
            };
        }
    };

    // 解決後の相対パスでも再検証する。入力時点では安全に見えても、解決先が危険な実体を
    // 指している可能性があるため、内容を読む前にもう一度安全境界を通す。
    let relative_path = workspace
        .relative_path(&resolved)
        .to_string_lossy()
        .to_string();
    if let Some(reason) = unsafe_agent_edit_target_path_reason(&relative_path) {
        return AgentPathTarget::Rejected {
            path: relative_path,
            reason,
        };
    }

    match read_workspace_file(workspace, path) {
        Ok(file) => AgentPathTarget::Included {
            bytes: file.contents.len() as u64,
            path: file.relative_path,
        },
        Err(error) => AgentPathTarget::Rejected {
            path: relative_path,
            reason: error.to_string(),
        },
    }
}

/// current directory を workspace として prompt 由来の file context を集める入口。
/// workspace を確立できなければ何も載せず、エラーメッセージだけ返す(外部参照に開かない)。
pub(super) fn ask_file_context_from_prompt(prompt: &str) -> AskFileContextResult {
    match Workspace::from_current_dir() {
        Ok(workspace) => ask_file_context_from_prompt_in_workspace(prompt, &workspace),
        Err(error) => AskFileContextResult {
            file_contexts: Vec::new(),
            file_context_records: Vec::new(),
            workspace_overview: None,
            messages: vec![TuiMessage::Error(format!(
                "Ask mode could not inspect mentioned file paths: {}",
                redact_sensitive_text(&error.to_string())
            ))],
        },
    }
}

/// prompt 内で明示的に言及されたパスだけを安全検査して read-only context に取り込む。
/// 言及ファイルを載せられない/無い時に限り bounded workspace overview を補う。採否は
/// record と UI メッセージに残し、provider へ渡す文脈を言及範囲に絞る。
pub(super) fn ask_file_context_from_prompt_in_workspace(
    prompt: &str,
    workspace: &Workspace,
) -> AskFileContextResult {
    let paths = detect_prompt_file_paths(prompt);
    let mut file_contexts = Vec::new();
    let mut file_context_records = Vec::new();
    let mut messages = Vec::new();

    for path in &paths {
        match ask_context_for_path(workspace, path) {
            AskPathContext::Included {
                file_context,
                record,
            } => {
                messages.push(TuiMessage::Action(format!(
                    "Ask mode included `{}` as read-only context ({} bytes).",
                    redact_sensitive_text(&record.path),
                    record.bytes.unwrap_or(0)
                )));
                file_contexts.push(file_context);
                file_context_records.push(record);
            }
            AskPathContext::Rejected { record } => {
                messages.push(TuiMessage::Error(format!(
                    "Ask mode skipped `{}`: {}",
                    redact_sensitive_text(&record.path),
                    redact_sensitive_text(record.reason.as_deref().unwrap_or("not safe to read"))
                )));
                file_context_records.push(record);
            }
            AskPathContext::Missing { record } => {
                messages.push(TuiMessage::Action(format!(
                    "Ask mode could not find `{}`; continuing without it.",
                    redact_sensitive_text(&record.path)
                )));
                file_context_records.push(record);
            }
        }
    }

    let workspace_overview = if should_include_workspace_overview(prompt, &paths, &file_contexts) {
        let overview = ask_workspace_overview_context(workspace);
        messages.extend(overview.messages);
        overview.context
    } else {
        None
    };

    AskFileContextResult {
        file_contexts,
        file_context_records,
        workspace_overview,
        messages,
    }
}

/// 1つのパスを Ask context として評価した結果。Included=安全に読めた本文+record、
/// Rejected=安全境界で除外(record のみ)、Missing=見つからない。本文は Included だけが持つ。
pub(super) enum AskPathContext {
    Included {
        file_context: ChatFileContext,
        record: ChatFileContextRecord,
    },
    Rejected {
        record: ChatFileContextRecord,
    },
    Missing {
        record: ChatFileContextRecord,
    },
}

/// 単一パスを Ask context へ解決する。secret-like / 危険パス / workspace 外は読まず、
/// 解決後の相対パスでも再検証してから本文と sha256 を取る(内容を載せる手前の最後の関門)。
pub(super) fn ask_context_for_path(workspace: &Workspace, path: &str) -> AskPathContext {
    if let Some(reason) = unsafe_prompt_context_path_reason(path) {
        return AskPathContext::Rejected {
            record: rejected_file_context_record(path, reason),
        };
    }

    let resolved = match workspace.resolve_existing_file(path) {
        Ok(resolved) => resolved,
        Err(WorkspaceError::CanonicalizePath { .. }) => {
            return AskPathContext::Missing {
                record: ChatFileContextRecord {
                    path: path.to_string(),
                    status: "missing".to_string(),
                    bytes: None,
                    sha256: None,
                    reason: Some("path not found".to_string()),
                },
            };
        }
        Err(error) => {
            return AskPathContext::Rejected {
                record: rejected_file_context_record(path, error.to_string()),
            };
        }
    };

    let relative_path = workspace
        .relative_path(&resolved)
        .to_string_lossy()
        .to_string();
    if let Some(reason) = unsafe_prompt_context_path_reason(&relative_path) {
        return AskPathContext::Rejected {
            record: rejected_file_context_record(&relative_path, reason),
        };
    }

    match read_workspace_file(workspace, path) {
        Ok(file) => {
            let bytes = file.contents.as_bytes().len() as u64;
            let sha256 = sha256_hex(file.contents.as_bytes());
            AskPathContext::Included {
                file_context: ChatFileContext {
                    path: file.relative_path.clone(),
                    contents: file.contents,
                },
                record: ChatFileContextRecord {
                    path: file.relative_path,
                    status: "included".to_string(),
                    bytes: Some(bytes),
                    sha256: Some(sha256),
                    reason: None,
                },
            }
        }
        Err(error) => AskPathContext::Rejected {
            record: rejected_file_context_record(&relative_path, error.to_string()),
        },
    }
}

/// 除外したパスの監査用 record を作る小ヘルパ。本文は持たず、理由だけを残す。
pub(super) fn rejected_file_context_record(path: &str, reason: String) -> ChatFileContextRecord {
    ChatFileContextRecord {
        path: path.to_string(),
        status: "rejected".to_string(),
        bytes: None,
        sha256: None,
        reason: Some(reason),
    }
}

/// overview を補うべきかの判定。overview を望む意図があり、かつ具体ファイルを取り込めて
/// いない時だけ true。言及ファイルで足りる場合に冗長な workspace 文脈を足さないための条件。
pub(super) fn should_include_workspace_overview(
    prompt: &str,
    explicit_paths: &[String],
    included_explicit_files: &[ChatFileContext],
) -> bool {
    has_workspace_overview_intent(prompt)
        && (explicit_paths.is_empty() || included_explicit_files.is_empty())
}

/// prompt に「全体 / 概要 / this project」等の overview を望む語が含まれるかを軽く判定する。
/// 和英のキーワード一致による意図検出で、過剰に overview を載せないためのゲート。
pub(super) fn has_workspace_overview_intent(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();
    [
        "このプロジェクト",
        "このフォルダ",
        "このディレクトリ",
        "この資料群",
        "全体",
        "概要",
        "軽く読んで",
        "確認して",
        "把握して",
        "this project",
        "this folder",
        "overview",
        "summarize",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
}

/// bounded workspace overview の本体。候補パスを件数上限・合計バイト上限で切りながら
/// 安全に読み、tree(パス列挙)・読み込めたファイル・skip 件数を1つの context にまとめる。
/// 上限超過や除外は warnings/skipped に集計し、provider へ渡す overview を bound 内に保つ。
pub(super) fn ask_workspace_overview_context(workspace: &Workspace) -> AskWorkspaceOverviewResult {
    let tree = workspace_overview_tree(workspace);
    let mut file_contexts = Vec::new();
    let mut file_context_records = Vec::new();
    let mut warnings = tree.warnings.clone();
    let mut total_bytes = 0usize;

    for path in workspace_overview_candidate_paths(workspace) {
        if file_contexts.len() >= WORKSPACE_OVERVIEW_MAX_FILES {
            break;
        }

        match ask_workspace_overview_context_for_path(workspace, &path) {
            AskPathContext::Included {
                file_context,
                record,
            } => {
                // 合計バイト上限を超えるファイルは載せず、理由付きで warning/record に回す。
                let bytes = record.bytes.unwrap_or(0) as usize;
                if total_bytes.saturating_add(bytes) > WORKSPACE_OVERVIEW_MAX_BYTES {
                    let reason = "workspace overview context byte limit reached".to_string();
                    warnings.push((record.path.clone(), reason.clone()));
                    file_context_records.push(rejected_file_context_record(&record.path, reason));
                    continue;
                }
                total_bytes += bytes;
                file_contexts.push(file_context);
                file_context_records.push(record);
            }
            AskPathContext::Rejected { record } => {
                warnings.push((
                    record.path.clone(),
                    record
                        .reason
                        .clone()
                        .unwrap_or_else(|| "not safe to read".to_string()),
                ));
                file_context_records.push(record);
            }
            AskPathContext::Missing { .. } => {}
        }
    }

    let skipped_items = tree.skipped_items
        + file_context_records
            .iter()
            .filter(|record| record.status != "included")
            .count();
    let context = ChatWorkspaceOverviewContext {
        root_label: ".".to_string(),
        workspace_kind: workspace_overview_kind(workspace),
        tree: workspace_overview_tree_text(&tree.entries),
        files: file_contexts,
        file_context_records,
        listed_entries: tree.entries.len(),
        skipped_items,
    };

    let mut messages = Vec::new();
    messages.push(TuiMessage::Action(format!(
        "Ask mode included workspace overview: {} files read, {} paths listed, {} skipped.\nScope: .",
        context.files.len(),
        context.listed_entries,
        context.skipped_items
    )));
    for (path, reason) in warnings.into_iter().take(WORKSPACE_OVERVIEW_MAX_WARNINGS) {
        messages.push(TuiMessage::Error(format!(
            "Ask mode skipped `{}`: {}",
            redact_sensitive_text(&path),
            redact_sensitive_text(&reason)
        )));
    }

    AskWorkspaceOverviewResult {
        context: Some(context),
        messages,
    }
}

/// overview 候補1件の安全評価。overview では symlink を辿らず読まないよう先に弾き、
/// それ以外は ask_context_for_path に委譲する(通常検査に symlink 除外を足した薄いラッパ)。
pub(super) fn ask_workspace_overview_context_for_path(
    workspace: &Workspace,
    path: &str,
) -> AskPathContext {
    if let Some(reason) = unsafe_prompt_context_path_reason(path) {
        return AskPathContext::Rejected {
            record: rejected_file_context_record(path, reason),
        };
    }

    let resolved = match workspace.resolve_existing_file(path) {
        Ok(path) => path,
        Err(WorkspaceError::CanonicalizePath { .. }) => {
            return AskPathContext::Missing {
                record: ChatFileContextRecord {
                    path: path.to_string(),
                    status: "missing".to_string(),
                    bytes: None,
                    sha256: None,
                    reason: Some("path not found".to_string()),
                },
            };
        }
        Err(error) => {
            return AskPathContext::Rejected {
                record: rejected_file_context_record(path, error.to_string()),
            };
        }
    };

    match fs::symlink_metadata(&resolved) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return AskPathContext::Rejected {
                record: rejected_file_context_record(
                    path,
                    "symlinks are not read as workspace overview context".to_string(),
                ),
            };
        }
        Ok(_) => {}
        Err(error) => {
            return AskPathContext::Rejected {
                record: rejected_file_context_record(path, error.to_string()),
            };
        }
    }

    ask_context_for_path(workspace, path)
}

/// workspace が git repo か generic かのラベルを返す(overview 表示のメタ情報)。
pub(super) fn workspace_overview_kind(workspace: &Workspace) -> String {
    if discover_git_repo_from(workspace.root()).is_ok() {
        "git".to_string()
    } else {
        "generic".to_string()
    }
}

/// bound(深さ・件数)内で workspace を BFS 列挙し、パスだけの木を作る。内容は読まない。
/// hidden / symlink / 危険パス・読めない dir は載せず skip としてカウントし、overview の
/// 範囲を安全側に絞る。warnings には「ファイルなのに除外した」理由を残す。
pub(super) fn workspace_overview_tree(workspace: &Workspace) -> WorkspaceOverviewTree {
    let mut entries = Vec::new();
    let mut warnings = Vec::new();
    let mut skipped_items = 0usize;
    let mut queue = VecDeque::from([(workspace.root().to_path_buf(), 0usize)]);

    while let Some((dir, depth)) = queue.pop_front() {
        // 深さ・件数の上限に達したらそれ以上は展開しない(木を bound 内に保つ)。
        if depth >= WORKSPACE_OVERVIEW_MAX_DEPTH || entries.len() >= WORKSPACE_OVERVIEW_MAX_ENTRIES
        {
            continue;
        }

        let children = match sorted_read_dir(&dir) {
            Ok(children) => children,
            Err(_) => {
                skipped_items += 1;
                continue;
            }
        };

        for child in children {
            if entries.len() >= WORKSPACE_OVERVIEW_MAX_ENTRIES {
                break;
            }

            let relative_path = workspace
                .relative_path(&child)
                .to_string_lossy()
                .to_string();
            if relative_path.is_empty() {
                continue;
            }

            let Some(name) = child.file_name().and_then(|name| name.to_str()) else {
                skipped_items += 1;
                continue;
            };
            let metadata = match fs::symlink_metadata(&child) {
                Ok(metadata) => metadata,
                Err(_) => {
                    skipped_items += 1;
                    continue;
                }
            };

            if metadata.file_type().is_symlink() {
                skipped_items += 1;
                continue;
            }
            if is_hidden_workspace_entry(name) {
                skipped_items += 1;
                continue;
            }
            if let Some(reason) = unsafe_prompt_context_path_reason(&relative_path) {
                skipped_items += 1;
                if !metadata.is_dir() {
                    warnings.push((relative_path, reason));
                }
                continue;
            }

            if metadata.is_dir() {
                entries.push(format!("{relative_path}/"));
                queue.push_back((child, depth + 1));
            } else if metadata.is_file() {
                entries.push(relative_path);
            } else {
                skipped_items += 1;
            }
        }
    }

    WorkspaceOverviewTree {
        entries,
        skipped_items,
        warnings,
    }
}

/// dir 直下を名前順(大小文字無視)で列挙する決定的読み出し。overview の出力を安定させる。
pub(super) fn sorted_read_dir(dir: &Path) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut entries = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect::<Vec<_>>();
    entries.sort_by_key(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_ascii_lowercase())
            .unwrap_or_default()
    });
    Ok(entries)
}

/// 列挙した木を provider/表示用テキストへ整形する。空なら安全エントリ無しの文言を返す。
pub(super) fn workspace_overview_tree_text(entries: &[String]) -> String {
    if entries.is_empty() {
        return "No safe workspace entries listed.".to_string();
    }
    entries.join("\n")
}

/// overview で内容まで読む候補パスを優先順で集める。priority list → `docs/*.md` →
/// root 直下の `.md`/`.txt` の順に、重複を避けて並べる(代表ドキュメントを先に拾う狙い)。
pub(super) fn workspace_overview_candidate_paths(workspace: &Workspace) -> Vec<String> {
    let mut paths = Vec::new();
    for path in WORKSPACE_OVERVIEW_PRIORITY_PATHS {
        push_unique_existing_workspace_path(workspace, &mut paths, path);
    }

    if let Ok(entries) = sorted_read_dir(&workspace.root().join("docs")) {
        for path in entries {
            if path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
            {
                let relative = workspace.relative_path(&path).to_string_lossy().to_string();
                push_unique_workspace_path(&mut paths, &relative);
            }
        }
    }

    if let Ok(entries) = sorted_read_dir(workspace.root()) {
        for path in entries {
            let extension = path.extension().and_then(|extension| extension.to_str());
            if matches!(extension, Some(extension) if extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("txt"))
            {
                let relative = workspace.relative_path(&path).to_string_lossy().to_string();
                push_unique_workspace_path(&mut paths, &relative);
            }
        }
    }

    paths
}

/// 実在する場合だけ候補へ重複なく足すヘルパ(priority list の存在確認用)。
pub(super) fn push_unique_existing_workspace_path(
    workspace: &Workspace,
    paths: &mut Vec<String>,
    path: &str,
) {
    if workspace.resolve_existing_file(path).is_ok() {
        push_unique_workspace_path(paths, path);
    }
}

/// 既出でなければ候補へ足す重複排除ヘルパ。
pub(super) fn push_unique_workspace_path(paths: &mut Vec<String>, path: &str) {
    if !paths.iter().any(|existing| existing == path) {
        paths.push(path.to_string());
    }
}

/// `.` 始まりの隠しエントリ判定。overview の列挙対象から外す基準。
pub(super) fn is_hidden_workspace_entry(name: &str) -> bool {
    name.starts_with('.')
}

/// overview でまず読むべき代表ドキュメント名の優先リスト(README/索引/handoff 系を和英で網羅)。
pub(super) const WORKSPACE_OVERVIEW_PRIORITY_PATHS: &[&str] = &[
    "README.md",
    "README.txt",
    "INDEX.md",
    "index.md",
    "SUMMARY.md",
    "summary.md",
    "START_HERE.md",
    "CHANGELOG.md",
    "CONTRIBUTING.md",
    "ROADMAP.md",
    "TODO.md",
    "AGENTS.md",
    "CLAUDE.md",
    "docs/README.md",
    "docs/index.md",
    "docs/START_HERE.md",
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "go.mod",
    "引き継ぎ.md",
    "引継ぎ.md",
    "手順書.md",
    "運用.md",
    "仕様.md",
    "作業メモ.md",
];

/// prompt 文字列から「明確にファイルパスらしい」トークンだけを抽出する。
/// path 文字の連なりで区切って候補化し、無関係な語を context 対象に取り込まないよう絞る。
pub(super) fn detect_prompt_file_paths(prompt: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut token = String::new();
    for ch in prompt.chars() {
        if is_prompt_path_char(ch) {
            token.push(ch);
        } else {
            push_prompt_path_token(&mut paths, &token);
            token.clear();
        }
    }
    push_prompt_path_token(&mut paths, &token);
    paths
}

/// パストークンを構成しうる文字か(英数 + `/._-`)。これ以外で token を区切る。
pub(super) fn is_prompt_path_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-')
}

/// トークンを末尾 `.` 除去で正規化し、明確なファイルパスなら重複なく候補へ積む。
pub(super) fn push_prompt_path_token(paths: &mut Vec<String>, token: &str) {
    let token = token.trim_end_matches('.');
    if !is_clear_prompt_file_path(token) {
        return;
    }
    if !paths.iter().any(|path| path == token) {
        paths.push(token.to_string());
    }
}

/// トークンが「明確なファイルパス」と言えるかの判定。空 / `.` / `..` / URL / traversal は
/// 除外し、既知拡張子を要求する。誤検出で無関係な語を context 化しないための足切り。
pub(super) fn is_clear_prompt_file_path(token: &str) -> bool {
    if token.is_empty()
        || token == "."
        || token == ".."
        || token.starts_with("http")
        || token.contains("..")
    {
        return false;
    }

    if token.contains('/') {
        return Path::new(token)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(token_has_known_file_suffix);
    }

    token_has_known_file_suffix(token)
}

/// 既知のテキスト系拡張子か、または `.env`/秘密鍵/`Cargo.lock` のような特別扱い名か。
/// secret-like 名もここでは true にして「検出」させ、後段の安全検査で明示的に弾けるようにする。
pub(super) fn token_has_known_file_suffix(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    if lower == ".env" || lower.starts_with(".env.") || lower == "id_rsa" || lower == "id_ed25519" {
        return true;
    }
    let Some(extension) = Path::new(token)
        .extension()
        .and_then(|extension| extension.to_str())
    else {
        return token == "Cargo.lock";
    };
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "c" | "cpp"
            | "css"
            | "csv"
            | "go"
            | "h"
            | "hpp"
            | "html"
            | "java"
            | "js"
            | "json"
            | "jsx"
            | "kt"
            | "lock"
            | "md"
            | "mjs"
            | "py"
            | "rb"
            | "rs"
            | "sh"
            | "sql"
            | "swift"
            | "toml"
            | "ts"
            | "tsx"
            | "txt"
            | "yaml"
            | "yml"
    )
}

/// パスを Ask context として読んではいけない理由を返す中心的な安全関門。
/// absolute / traversal / `.git`・`target`・`node_modules` 等のディレクトリ、`.env` や
/// secret-like なファイル名を理由付きで拒否する。None なら読んでよいことを意味する。
pub(super) fn unsafe_prompt_context_path_reason(path: &str) -> Option<String> {
    if path.starts_with('/') {
        return Some("absolute paths are not accepted for Ask mode context".to_string());
    }
    if path.contains("..") {
        return Some("path traversal is not accepted for Ask mode context".to_string());
    }

    let components = Path::new(path)
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>();
    for component in &components {
        match component.to_ascii_lowercase().as_str() {
            ".git" | "target" | "node_modules" | "dist" | "build" | "vendor" => {
                return Some(format!("`{component}/` is not read as Ask mode context"));
            }
            _ => {}
        }
    }

    let filename = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
        .to_ascii_lowercase();
    if filename == ".env" || filename.starts_with(".env.") {
        return Some("environment files are not read as Ask mode context".to_string());
    }
    // 鍵・証明書や api_key/secret/token/credential を含む名前は secret-like として読まない。
    if filename.ends_with(".pem")
        || filename.ends_with(".key")
        || filename.ends_with(".p12")
        || filename.ends_with(".pfx")
        || filename == "id_rsa"
        || filename == "id_ed25519"
        || filename.contains("api_key")
        || filename.contains("apikey")
        || filename.contains("private_key")
        || filename.contains("secret")
        || filename.contains("token")
        || filename.contains("credential")
    {
        return Some("secret-like files are not read as Ask mode context".to_string());
    }

    None
}

/// Agent の編集ターゲット用の安全関門。Ask 用検査を流用しつつ文言を edit target 向けに
/// 置換し、さらに hidden file を編集対象から除外する(書き込み境界を Ask より厳しく保つ)。
pub(super) fn unsafe_agent_edit_target_path_reason(path: &str) -> Option<String> {
    if let Some(reason) = unsafe_prompt_context_path_reason(path) {
        return Some(
            reason
                .replace("Ask mode context", "Agent mode edit target")
                .replace(
                    "as workspace overview context",
                    "as an Agent mode edit target",
                ),
        );
    }

    if Path::new(path)
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .any(|component| component.starts_with('.'))
    {
        return Some("hidden files are not accepted as Agent mode edit targets".to_string());
    }

    None
}
