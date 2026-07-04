//! provider / model route のヒントと選択肢の組み立て。
//! route 表示は session-local switching の案内に留め、config 自動書き換えは行わない。
use super::*;
use origin_protocol::model::select_fallback_model;

/// provider を選ぶ判断材料(route 種別・privacy・quota コスト・向き)をまとめた表示用ヒント。
/// session-local 切替の案内であって、provider を自動選択したり config を書き換えたりはしない。
pub(super) struct ProviderRoutingHint {
    pub(super) route: &'static str,
    pub(super) privacy: &'static str,
    pub(super) quota_cost: &'static str,
    pub(super) best_for: &'static str,
}

/// `/models` で 1 行 = 1 route として並べる選択肢の view データ。
/// current は現在の session route、selectable は実際に切替できるか、availability は可否の理由。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ModelRouteEntry {
    pub(super) provider_id: String,
    pub(super) model_id: String,
    pub(super) route: String,
    pub(super) current: bool,
    pub(super) recommended: bool,
    pub(super) route_label: &'static str,
    pub(super) source: &'static str,
    pub(super) availability: String,
    pub(super) selectable: bool,
}

/// 1 preset 分の route 候補(models 群)と、使えない場合の理由。entry へ平坦化する前の中間集約。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ModelRouteProvider {
    pub(super) provider_id: String,
    pub(super) route_label: &'static str,
    pub(super) models: Vec<ModelRouteProviderModel>,
    pub(super) unavailable_reason: Option<String>,
}

/// provider 内の 1 model 候補。source(live/config/builtin)と availability で由来と可否を区別する。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ModelRouteProviderModel {
    pub(super) model_id: String,
    pub(super) source: &'static str,
    pub(super) availability: String,
    pub(super) selectable: bool,
    pub(super) recommended: bool,
}

/// 現在の provider id から上の表示ヒントを引く。preset 固有(lmstudio=local / openai=external)で、
/// 未知 provider には provider 依存の中立な説明を返す。判断材料を見せるだけで選択はしない。
pub(super) fn provider_routing_hint(status: &StatusSnapshot) -> ProviderRoutingHint {
    match status.provider_id.as_str() {
        "lmstudio" => ProviderRoutingHint {
            route: "local",
            privacy: "highest; runs on local machine",
            quota_cost: "no cloud AI quota; uses local compute",
            best_for: "light edits, docs, private notes, routine work",
        },
        "openai" => ProviderRoutingHint {
            route: "external OpenAI-compatible API",
            privacy: "external provider",
            quota_cost: "uses API/provider quota",
            best_for: "tasks needing more capability than local, below Codex/Claude Code",
        },
        _ => ProviderRoutingHint {
            route: "OpenAI-compatible provider",
            privacy: "depends on provider",
            quota_cost: "depends on provider",
            best_for: "manually selected provider-specific work",
        },
    }
}

/// provider を route の性質(local/private・external/API 等)で 1 語ラベル化する表示用ヘルパ。
pub(super) fn model_route_label(provider_id: &str) -> &'static str {
    match provider_id {
        "lmstudio" => "local/private",
        "openai" => "external/API",
        _ => "custom/OpenAI-compatible",
    }
}

/// preset 単位の builtin 既定 model id を返す拡張点。現在はどの preset も既定を
/// ハードコードせず、すべて config / live 解決に委ねる(lmstudio と同じ挙動)。
/// よって API key も config も無い provider は既定 model を推測されず、unavailable
/// として表示される。表示用のみで config を読み書きしない。
pub(super) fn builtin_model_route_hint(_provider_id: &str) -> Option<&'static str> {
    None
}

/// registry の全 preset を辿って `/models` 用の route entry 一覧を作る入口。
pub(super) fn model_route_entries(runtime: &RuntimeState) -> Vec<ModelRouteEntry> {
    let registry = builtin_registry();
    let providers = registry
        .presets()
        .iter()
        .map(model_route_provider_from_preset)
        .collect::<Vec<_>>();
    model_route_entries_from_providers(runtime, &providers)
}

/// 1 preset を route 候補集合へ解決する:config 解決 → auth 判定 → live 一覧 or 既定 model の順。
/// config は読み取りのみで、ここで設定を書き換えることはない(session-local の案内に徹する)。
pub(super) fn model_route_provider_from_preset(preset: &ProviderPreset) -> ModelRouteProvider {
    let resolved = resolve_config(CliOverrides {
        provider: Some(preset.id.as_str().to_string()),
        base_url: None,
    })
    .ok();

    let configured_or_default_model = resolved
        .as_ref()
        .and_then(|config| config.default_model.clone())
        .or_else(|| builtin_model_route_hint(preset.id.as_str()).map(str::to_string));
    let configured_or_default_source = if resolved
        .as_ref()
        .and_then(|config| config.default_model.as_ref())
        .is_some()
    {
        "config"
    } else {
        "builtin"
    };
    let api_key_env_override = resolved
        .as_ref()
        .and_then(|config| config.api_key_env_override.as_deref());
    let provider_id = preset.id.as_str().to_string();
    let route_label = model_route_label(&provider_id);

    if let Some(reason) = model_route_auth_unavailable_reason(&preset.auth, api_key_env_override) {
        let models = configured_or_default_model
            .map(|model_id| ModelRouteProviderModel {
                model_id,
                source: configured_or_default_source,
                availability: format!("unavailable: {reason}"),
                selectable: false,
                recommended: false,
            })
            .into_iter()
            .collect();

        return ModelRouteProvider {
            provider_id,
            route_label,
            models,
            unavailable_reason: Some(reason),
        };
    }

    let mut models = Vec::new();
    let mut unavailable_reason = None;

    if let Some(provider) = builtin_registry().openai_compatible_provider(
        preset.id.as_str(),
        resolved
            .as_ref()
            .and_then(|config| config.base_url_override.clone()),
        resolved
            .as_ref()
            .and_then(|config| config.api_key_env_override.clone()),
    ) {
        match provider.list_models() {
            Ok(live_models) => {
                let recommended_model_id =
                    select_fallback_model(&live_models).map(|model| model.id.clone());
                for model in live_models {
                    push_provider_model(
                        &mut models,
                        &model.id,
                        "live",
                        "available".to_string(),
                        true,
                        recommended_model_id.as_deref() == Some(model.id.as_str()),
                    );
                }
                if models.is_empty() {
                    unavailable_reason = Some("no live models returned".to_string());
                }
            }
            Err(error) => {
                unavailable_reason = Some(safe_model_route_failure_reason(&error.user_message()));
            }
        }
    } else {
        unavailable_reason = Some("provider is not configured".to_string());
    }

    if let Some(model_id) = configured_or_default_model {
        let available = unavailable_reason.is_none();
        push_provider_model(
            &mut models,
            &model_id,
            configured_or_default_source,
            if let Some(reason) = &unavailable_reason {
                format!("unavailable: {reason}")
            } else {
                "available".to_string()
            },
            available,
            false,
        );
    }

    ModelRouteProvider {
        provider_id,
        route_label,
        models,
        unavailable_reason,
    }
}

/// model 候補を重複なく積む。同一 id は最も強い source(live)/availability(available)/selectable へ
/// 格上げして 1 件に畳む。空 id は捨てる。
pub(super) fn push_provider_model(
    models: &mut Vec<ModelRouteProviderModel>,
    model_id: &str,
    source: &'static str,
    availability: String,
    selectable: bool,
    recommended: bool,
) {
    if model_id.trim().is_empty() {
        return;
    }

    if let Some(existing) = models.iter_mut().find(|model| model.model_id == model_id) {
        existing.selectable |= selectable;
        existing.recommended |= recommended;
        if existing.source != "live" && source == "live" {
            existing.source = source;
        }
        if existing.availability != "available" && availability == "available" {
            existing.availability = availability;
        }
        return;
    }

    models.push(ModelRouteProviderModel {
        model_id: model_id.to_string(),
        source,
        availability,
        selectable,
        recommended,
    });
}

/// auth 設定と env override から、必要な API key が実際に設定済みかだけを確認する。
/// 未設定なら理由文を返す。key の値そのものは読まない・返さない(secret は表示しない)。
pub(super) fn model_route_auth_unavailable_reason(
    auth: &ProviderAuth,
    api_key_env_override: Option<&str>,
) -> Option<String> {
    let api_key_env = match (auth, api_key_env_override) {
        (ProviderAuth::None, None) => return None,
        (ProviderAuth::None, Some(api_key_env)) => api_key_env,
        (ProviderAuth::BearerEnv { api_key_env }, None) => api_key_env,
        (ProviderAuth::BearerEnv { .. }, Some(api_key_env)) => api_key_env,
    };

    if env::var(api_key_env)
        .ok()
        .is_some_and(|value| !value.trim().is_empty())
    {
        None
    } else {
        Some("configured API key environment variable is not set".to_string())
    }
}

/// provider 群を平坦な entry 列へ変換する。現在の session route を必ず 1 件含め、
/// provider 順 → current → selectable → model_id で安定ソートして選びやすく並べる。
pub(super) fn model_route_entries_from_providers(
    runtime: &RuntimeState,
    providers: &[ModelRouteProvider],
) -> Vec<ModelRouteEntry> {
    let mut entries = Vec::new();

    for provider in providers {
        for model in &provider.models {
            push_model_route_entry(
                &mut entries,
                &provider.provider_id,
                &model.model_id,
                runtime,
                provider.route_label,
                model.source,
                model.availability.clone(),
                model.selectable,
                model.recommended,
            );
        }

        if provider.models.is_empty() {
            if let Some(reason) = &provider.unavailable_reason {
                push_unavailable_model_route_provider(
                    &mut entries,
                    &provider.provider_id,
                    provider.route_label,
                    reason,
                );
            }
        }
    }

    if let Some(model_id) = runtime.model_id.as_deref() {
        push_model_route_entry(
            &mut entries,
            &runtime.provider_id,
            model_id,
            runtime,
            model_route_label(&runtime.provider_id),
            "current",
            "available".to_string(),
            true,
            false,
        );
    }

    entries.sort_by_key(|entry| {
        (
            model_route_provider_order(&entry.provider_id),
            entry.provider_id.clone(),
            !entry.current,
            !entry.recommended,
            !entry.selectable,
            entry.model_id.clone(),
        )
    });
    entries
}

/// route 文字列(`provider/model`)を key に重複排除しつつ entry を積む。
/// 既存に当たれば current/source/selectable/availability をより強い側へ更新する。
pub(super) fn push_model_route_entry(
    entries: &mut Vec<ModelRouteEntry>,
    provider_id: &str,
    model_id: &str,
    runtime: &RuntimeState,
    route_label: &'static str,
    source: &'static str,
    availability: String,
    selectable: bool,
    recommended: bool,
) {
    let route = format!("{provider_id}/{model_id}");
    let current =
        runtime.provider_id == provider_id && runtime.model_id.as_deref() == Some(model_id);

    if let Some(existing) = entries.iter_mut().find(|entry| entry.route == route) {
        existing.current |= current;
        existing.recommended |= recommended;
        if existing.source != "current" && source == "current" {
            existing.source = source;
        }
        existing.selectable |= selectable;
        if existing.availability != "available" && availability == "available" {
            existing.availability = availability;
        }
        return;
    }

    entries.push(ModelRouteEntry {
        provider_id: provider_id.to_string(),
        model_id: model_id.to_string(),
        route,
        current,
        recommended,
        route_label,
        source,
        availability,
        selectable,
    });
}

/// model を 1 つも出せなかった provider を「使用不可」行として entry に残す。
/// 診断のため route ごと消さず、理由は redact してから載せる。
pub(super) fn push_unavailable_model_route_provider(
    entries: &mut Vec<ModelRouteEntry>,
    provider_id: &str,
    route_label: &'static str,
    reason: &str,
) {
    entries.push(ModelRouteEntry {
        provider_id: provider_id.to_string(),
        model_id: String::new(),
        route: provider_id.to_string(),
        current: false,
        recommended: false,
        route_label,
        source: "provider",
        availability: format!("unavailable: {}", redact_sensitive_text(reason)),
        selectable: false,
    });
}

/// registry の preset 並び順を sort key に使うための index(未知 provider は末尾)。
pub(super) fn model_route_provider_order(provider_id: &str) -> usize {
    builtin_registry()
        .presets()
        .iter()
        .position(|preset| preset.id.as_str() == provider_id)
        .unwrap_or(usize::MAX)
}

/// provider 失敗メッセージを redact し、既知パターンを短く安全な理由語へ正規化する。
/// 生のエラー全文を UI に流さないための関門で、未知パターンは先頭行だけ返す。
pub(super) fn safe_model_route_failure_reason(message: &str) -> String {
    let redacted = redact_sensitive_text(message);
    let first_line = redacted.lines().next().unwrap_or("provider unavailable");

    if first_line.contains("Could not connect to LM Studio") {
        "local server not reachable".to_string()
    } else if first_line.contains("Timed out waiting for provider") {
        "provider did not respond in time".to_string()
    } else if first_line.contains("API key is missing")
        || first_line.contains("requires an API key")
    {
        "configured API key environment variable is not set".to_string()
    } else if first_line.contains("does not expose a models endpoint") {
        "models endpoint unavailable".to_string()
    } else {
        first_line.to_string()
    }
}

/// 現在の model/provider 状態の表示文を組む。末尾で session 内切替の導線
/// (/models, /model set)を案内する。config 変更ではなく route 切替の案内に留める。
pub(super) fn format_tui_model(status: &StatusSnapshot) -> String {
    let mut output = String::new();
    output.push_str("Current model:\n\n");
    output.push_str("Provider:\n");
    output.push_str(&format!("  id:      {}\n\n", status.provider_id));
    output.push_str("Model:\n");
    output.push_str(&format!(
        "  id:      {}\n",
        status.model_id.as_deref().unwrap_or("(none)")
    ));
    output.push_str(&format!("  source:  {}\n", status.model_source));
    if let Some(error) = &status.model_error {
        output.push_str(&format!("  error:   {}\n", redact_sensitive_text(error)));
    }
    output.push_str("\nUse /models to pick a model route.");
    output.push_str("\nUse /model set <provider>/<model-id> to switch route for this TUI session.");
    output
}

/// 現在の provider 詳細の表示文を組む。/provider list / set で session 内切替できる旨を案内する。
pub(super) fn format_tui_provider(status: &StatusSnapshot) -> String {
    format!(
        "Current provider:\n\nProvider:\n  id:       {}\n  kind:     {}\n  base_url: {}\n  auth:     {}\n\nUse /provider list to see available providers.\nUse /provider set <provider-id> to switch for this TUI session.",
        status.provider_id, status.provider_kind, status.base_url, status.auth_label
    )
}

/// model route 切替成功の表示文。「session-local で config は未変更」を明示するのが要点(安全境界)。
pub(super) fn format_tui_model_set_success(status: &StatusSnapshot) -> String {
    format!(
        "Model route set to {}/{}.\nprovider: {}\nmodel: {}\nsource: {}\n\nThis change is session-local. Config was not modified.",
        status.provider_id,
        status.model_id.as_deref().unwrap_or("(none)"),
        status.provider_id,
        status.model_id.as_deref().unwrap_or("(none)"),
        status.model_source
    )
}

/// model route 切替失敗の表示文。API key 不足なら env 設定を促すが、key の値は決して表示しない。
pub(super) fn format_tui_model_set_error(error: &str) -> String {
    if error.contains("requires an API key") {
        format!(
            "{}\nNext: set the configured API key environment variable and try again.\nThe API key value is never displayed by origin.",
            redact_sensitive_text(error)
        )
    } else {
        redact_sensitive_text(error)
    }
}

/// 選んだ route が使用不可の時の説明文。target と理由は redact してから出す。
pub(super) fn format_tui_model_route_unavailable(route: &ModelRouteEntry) -> String {
    let target = if route.model_id.is_empty() {
        route.provider_id.as_str()
    } else {
        route.route.as_str()
    };
    format!(
        "Model route is unavailable.\nroute: {}\nreason: {}\n\nChoose another route or use /provider list for diagnostics.",
        redact_sensitive_text(target),
        redact_sensitive_text(route.availability.trim_start_matches("unavailable: ").trim())
    )
}

/// provider 切替成功の表示文。これも session-local で config は未変更であることを明示する(安全境界)。
pub(super) fn format_tui_provider_set_success(status: &StatusSnapshot) -> String {
    format!(
        "Provider set to {}.\nbase_url: {}\nauth: {}\nmodel re-resolved to: {}\nmodel_source: {}\n\nThis change is session-local. Config was not modified.",
        status.provider_id,
        status.base_url,
        status.auth_label,
        status.model_id.as_deref().unwrap_or("(none)"),
        status.model_source
    )
}

/// provider 切替失敗の表示文。未知 provider / key 不足 / その他で文面を分け、redact する。key 値は出さない。
pub(super) fn format_tui_provider_set_error(error: &str) -> String {
    if error.starts_with("Unknown provider `") {
        format!("{error}\nUse /provider list to see available providers.")
    } else if error.contains("requires an API key") {
        format!("{error}\nNext: set the configured environment variable and try again.\nThe API key value is never displayed by origin.")
    } else {
        format!(
            "Could not switch provider.\nReason: {}\nNext: run /provider list and try again.",
            redact_sensitive_text(error)
        )
    }
}

/// preset 一覧表を組む(現在 provider・kind・auth・base_url)。auth は label 化のみで key 値は出さない。
pub(super) fn tui_provider_list(runtime: &RuntimeState) -> String {
    let registry = builtin_registry();
    let config = resolve_config(CliOverrides::default()).ok();
    let mut output = String::new();
    output.push_str(&format!(
        "{:<12}  {:<7}  {:<20}  {:<6}  base_url\n",
        "provider", "current", "kind", "auth"
    ));
    for preset in registry.presets() {
        let provider_config = resolve_config(CliOverrides {
            provider: Some(preset.id.as_str().to_string()),
            base_url: None,
        })
        .ok();
        let base_url = provider_config
            .as_ref()
            .and_then(|resolved| resolved.base_url_override.as_deref())
            .unwrap_or(preset.base_url.as_str());
        let api_key_env_override = provider_config
            .as_ref()
            .and_then(|resolved| resolved.api_key_env_override.as_deref())
            .or_else(|| {
                config
                    .as_ref()
                    .and_then(|resolved| resolved.api_key_env_override.as_deref())
            });
        output.push_str(&format!(
            "{:<12}  {:<7}  {:<20}  {:<6}  {}\n",
            preset.id.as_str(),
            if runtime.provider_id == preset.id.as_str() {
                "yes"
            } else {
                "no"
            },
            preset.kind.as_str(),
            auth_label(&preset.auth, api_key_env_override),
            base_url
        ));
    }
    output
}

/// Ask/Agent モード切替の確認文。Agent でも通常は review panel 経由で、
/// /diff /check /apply は手動 fallback である旨を伝える。
pub(super) fn format_tui_mode_set(mode: TuiInteractionMode) -> String {
    match mode {
        TuiInteractionMode::Ask => {
            "Mode switched to ASK.\nAsk mode reads explicitly mentioned safe files or bounded workspace overview context."
                .to_string()
        }
        TuiInteractionMode::Agent => {
            "Mode switched to AGENT.\nNormal Agent edits use review panels; /diff, /check, and /apply remain manual fallbacks."
                .to_string()
        }
    }
}

/// /mode の usage 文(引数なし・不正引数時に出す)。
pub(super) fn tui_mode_usage() -> String {
    "Usage: /mode [ask|agent]\nAsk is the default read-only chat mode. Agent mode previews a plan before chat or single-file proposal work.".to_string()
}

/// 直近 session 一覧表を組む(最大 5 件)。詳細閲覧は plain CLI コマンドへ誘導する。
pub(super) fn format_tui_sessions(sessions: &[SessionSummary]) -> String {
    let mut output = String::new();
    output.push_str("Recent sessions:\n\n");

    if sessions.is_empty() {
        output.push_str("No sessions found.");
        return output;
    }

    output.push_str(&format!(
        "{:<7}  {:<25}  {:<8}  {:<10}  {:<30}  status\n",
        "latest", "session_id", "command", "provider", "model"
    ));
    for (index, summary) in sessions.iter().take(5).enumerate() {
        output.push_str(&format_tui_session_row(summary, index == 0));
    }
    if sessions.len() > 5 {
        output.push_str(&format!("\nShowing 5 of {} sessions.\n", sessions.len()));
    }

    output.push_str("\nTip:\n");
    output.push_str("  Use plain CLI `origin sessions show <session-id>` for details.\n");
    output.push_str("  Use `origin sessions show --latest` for the latest session.");
    output
}

/// session 一覧の 1 行分を整形する。各フィールドは redact 済みで出す。
pub(super) fn format_tui_session_row(summary: &SessionSummary, latest: bool) -> String {
    format!(
        "{:<7}  {:<25}  {:<8}  {:<10}  {:<30}  {}\n",
        if latest { "yes" } else { "" },
        redact_sensitive_text(&summary.session_id),
        redact_sensitive_text(&summary.command),
        redact_sensitive_text(&summary.provider),
        truncate_middle(&redact_sensitive_text(&summary.model), 30),
        redact_sensitive_text(&summary.status)
    )
}

/// 長い文字列を中央省略(`...`)で max_chars に詰める表示ヘルパ。
pub(super) fn truncate_middle(value: &str, max_chars: usize) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= max_chars || max_chars <= 3 {
        return value.to_string();
    }

    let left = (max_chars - 3) / 2;
    let right = max_chars - 3 - left;
    let prefix: String = chars.iter().take(left).collect();
    let suffix: String = chars.iter().skip(chars.len() - right).collect();
    format!("{prefix}...{suffix}")
}
