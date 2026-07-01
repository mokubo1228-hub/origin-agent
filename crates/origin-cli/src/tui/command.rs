//! slash コマンドの解析と suggestion。
//! daily command と advanced/direct command を同じ parser に置きつつ、write 系は
//! ここでは実行せず submit へ分類するだけに留める。
use super::*;

/// suggestion tray に出す 1 slash command の表示メタ。
/// edit_loop は proposal/apply 系(/diff /check /apply 等)かどうかで、pending proposal 時の
/// 並べ替えに使う。実行はせず、候補表示のためのデータに過ぎない。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SlashCommandSuggestion {
    pub(super) command: &'static str,
    pub(super) description: &'static str,
    pub(super) edit_loop: bool,
}

/// suggestion 列挙と補完の元になる slash command の単一カタログ。
/// daily command と advanced/direct command を同居させるが、ここは表示・候補のための表で、
/// どれも実行はしない(実行は後段の分類 → command 処理が担う)。
pub(super) const TUI_SLASH_COMMANDS: &[SlashCommandSuggestion] = &[
    SlashCommandSuggestion {
        command: "/help",
        description: "show commands",
        edit_loop: false,
    },
    SlashCommandSuggestion {
        command: "/status",
        description: "show provider/model/session state",
        edit_loop: false,
    },
    SlashCommandSuggestion {
        command: "/mode",
        description: "show Ask/Agent mode",
        edit_loop: false,
    },
    SlashCommandSuggestion {
        command: "/mode ask",
        description: "read-only chat with explicit file context",
        edit_loop: false,
    },
    SlashCommandSuggestion {
        command: "/mode agent",
        description: "single-file proposal mode",
        edit_loop: false,
    },
    SlashCommandSuggestion {
        command: "/models",
        description: "pick model route",
        edit_loop: false,
    },
    SlashCommandSuggestion {
        command: "/model",
        description: "show current model",
        edit_loop: false,
    },
    SlashCommandSuggestion {
        command: "/model list",
        description: "list model routes",
        edit_loop: false,
    },
    SlashCommandSuggestion {
        command: "/model set <model-id>",
        description: "switch model route for this session",
        edit_loop: false,
    },
    SlashCommandSuggestion {
        command: "/provider",
        description: "show current provider",
        edit_loop: false,
    },
    SlashCommandSuggestion {
        command: "/provider list",
        description: "list providers",
        edit_loop: false,
    },
    SlashCommandSuggestion {
        command: "/provider set <provider-id>",
        description: "switch provider for this session",
        edit_loop: false,
    },
    SlashCommandSuggestion {
        command: "/propose <file> <task>",
        description: "preview an explicit file edit",
        edit_loop: true,
    },
    SlashCommandSuggestion {
        command: "/diff",
        description: "review pending proposal",
        edit_loop: true,
    },
    SlashCommandSuggestion {
        command: "/check",
        description: "validate pending proposal",
        edit_loop: true,
    },
    SlashCommandSuggestion {
        command: "/apply",
        description: "apply after explicit approval",
        edit_loop: true,
    },
    SlashCommandSuggestion {
        command: "/clear",
        description: "clear messages and pending proposal",
        edit_loop: true,
    },
    SlashCommandSuggestion {
        command: "/sessions",
        description: "show session info",
        edit_loop: false,
    },
    SlashCommandSuggestion {
        command: "/resume",
        description: "resume a past session (planned)",
        edit_loop: false,
    },
    SlashCommandSuggestion {
        command: "/exit",
        description: "quit TUI",
        edit_loop: false,
    },
    SlashCommandSuggestion {
        command: "/quit",
        description: "quit TUI",
        edit_loop: false,
    },
];

/// suggestion tray を出してよいかの可否判定。
/// composer が単一行の `/` 始まりで、かつ apply 承認待ちでない時だけ true。
pub(super) fn should_show_slash_suggestions(state: &TuiState) -> bool {
    state.pending_apply.is_none()
        && !state.input.contains('\n')
        && state.input.trim_start().starts_with('/')
}

/// 入力 prefix に対する候補一覧を作る。`/` 単独なら文脈(pending proposal の有無)で初期セットを
/// 変え、apply 承認待ち中は何も出さない。pending proposal 時は edit_loop 系を上位に寄せ、最大 6 件に絞る。
pub(super) fn slash_suggestions(
    input: &str,
    has_pending_proposal: bool,
    has_pending_apply: bool,
) -> Vec<SlashCommandSuggestion> {
    if has_pending_apply {
        return Vec::new();
    }

    let prefix = input.trim_start();
    if !prefix.starts_with('/') {
        return Vec::new();
    }

    let mut suggestions: Vec<_> = if prefix == "/" {
        let initial_commands: &[&str] = if has_pending_proposal {
            &[
                "/propose <file> <task>",
                "/diff",
                "/check",
                "/apply",
                "/clear",
                "/status",
            ]
        } else {
            &["/models", "/mode", "/resume", "/status", "/help", "/exit"]
        };
        initial_commands
            .into_iter()
            .copied()
            .filter_map(find_slash_suggestion)
            .collect()
    } else {
        TUI_SLASH_COMMANDS
            .iter()
            .copied()
            .filter(|suggestion| suggestion.command.starts_with(prefix))
            .collect()
    };

    if has_pending_proposal {
        suggestions.sort_by_key(|suggestion| if suggestion.edit_loop { 0 } else { 1 });
    }

    suggestions.truncate(6);
    suggestions
}

/// command 文字列の完全一致でカタログから suggestion メタを引く。
pub(super) fn find_slash_suggestion(command: &str) -> Option<SlashCommandSuggestion> {
    TUI_SLASH_COMMANDS
        .iter()
        .copied()
        .find(|suggestion| suggestion.command == command)
}

/// 候補を tray の描画行へ変換する。selected_index 行に marker と強調を付け、
/// 候補が空なら「該当なし・/help を使え」の案内行を 1 つだけ返す。
pub(super) fn slash_suggestion_lines(
    input: &str,
    has_pending_proposal: bool,
    has_pending_apply: bool,
    selected_index: usize,
    width: u16,
) -> Vec<Line<'static>> {
    let suggestions = slash_suggestions(input, has_pending_proposal, has_pending_apply);
    if suggestions.is_empty() {
        return vec![command_tray_row(
            " ",
            "No matching slash commands. Use /help.",
            "",
            false,
            width,
        )];
    }

    suggestions
        .into_iter()
        .enumerate()
        .map(|(index, suggestion)| {
            let marker = if index == selected_index { "›" } else { " " };
            command_tray_row(
                marker,
                suggestion.command,
                suggestion.description,
                index == selected_index,
                width,
            )
        })
        .collect()
}

/// 1 候補行を marker / command / description の 3 列に幅調整して描く。
/// selected は cyan + bold で強調する(選択表現は literal marker 単独に頼らず色も使う)。
pub(super) fn command_tray_row(
    marker: &str,
    command: &str,
    description: &str,
    selected: bool,
    width: u16,
) -> Line<'static> {
    let total_width = width.max(12) as usize;
    let inner_width = total_width.saturating_sub(4);
    let command_width = if description.is_empty() {
        inner_width.saturating_sub(2)
    } else {
        28.min(inner_width.saturating_sub(2))
    };
    let description_width = inner_width.saturating_sub(command_width + 3);
    let command_text = fit_to_width(command, command_width);
    let description_text = fit_to_width(description, description_width);
    let command_padding = " ".repeat(command_width.saturating_sub(command_text.chars().count()));
    let description_separator_width = if description_text.is_empty() { 0 } else { 1 };
    let content_width = display_width(marker)
        + 1
        + command_width
        + description_separator_width
        + display_width(&description_text);
    let trailing_padding = " ".repeat(inner_width.saturating_sub(content_width));
    let command_style = if selected {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    let mut spans = vec![
        Span::raw("  "),
        Span::styled(marker.to_string(), command_style),
        Span::raw(" "),
        Span::styled(command_text, command_style),
        Span::raw(command_padding),
    ];
    if !description_text.is_empty() {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            description_text,
            Style::default().add_modifier(Modifier::DIM),
        ));
    }
    spans.push(Span::raw(trailing_padding));
    Line::from(spans)
}

/// 文字列を max_chars 以内に収め、超過分は `...` 付きで切り詰める表示ヘルパ。
pub(super) fn fit_to_width(value: &str, max_chars: usize) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return value.to_string();
    }
    if max_chars <= 3 {
        return chars.into_iter().take(max_chars).collect();
    }

    let visible = max_chars - 3;
    let prefix = chars.into_iter().take(visible).collect::<String>();
    format!("{prefix}...")
}

/// slash 入力を TuiSubmit へ分類する純粋な parser。
/// ここでは write/apply を実行せず、対応する TuiCommand へ振り分けるだけ(検証・承認・実行は後段)。
/// 例: /propose や /apply は該当 command に分類されるが、ファイル書き込みはここでは起こさない。
pub(super) fn parse_tui_command(input: &str) -> TuiSubmit {
    match input.split_whitespace().collect::<Vec<_>>().as_slice() {
        ["/help"] => TuiSubmit::Command(TuiCommand::Help),
        ["/status"] => TuiSubmit::Command(TuiCommand::Status),
        ["/mode"] => TuiSubmit::Command(TuiCommand::Mode),
        ["/mode", "ask"] => TuiSubmit::Command(TuiCommand::ModeSet(TuiInteractionMode::Ask)),
        ["/mode", "agent"] => TuiSubmit::Command(TuiCommand::ModeSet(TuiInteractionMode::Agent)),
        ["/mode", ..] => TuiSubmit::Command(TuiCommand::Error(tui_mode_usage())),
        ["/models"] => TuiSubmit::Command(TuiCommand::ModelList),
        ["/model"] => TuiSubmit::Command(TuiCommand::Model),
        ["/model", "list"] => TuiSubmit::Command(TuiCommand::ModelList),
        ["/model", "set"] => TuiSubmit::Command(TuiCommand::Error(
            "Model id cannot be empty.\nUsage: /model set <model-id> or /model set <provider>/<model-id>"
                .to_string(),
        )),
        ["/model", "set", model_id] => {
            TuiSubmit::Command(TuiCommand::ModelSet((*model_id).to_string()))
        }
        ["/model", "set", ..] => TuiSubmit::Command(TuiCommand::Error(
            "Usage: /model set <model-id> or /model set <provider>/<model-id>".to_string(),
        )),
        ["/provider"] => TuiSubmit::Command(TuiCommand::Provider),
        ["/provider", "list"] => TuiSubmit::Command(TuiCommand::ProviderList),
        ["/provider", "set"] => TuiSubmit::Command(TuiCommand::Error(
            "Provider id cannot be empty.\nUsage: /provider set <provider-id>".to_string(),
        )),
        ["/provider", "set", provider_id] => {
            TuiSubmit::Command(TuiCommand::ProviderSet((*provider_id).to_string()))
        }
        ["/provider", "set", ..] => TuiSubmit::Command(TuiCommand::Error(
            "Usage: /provider set <provider-id>".to_string(),
        )),
        ["/sessions"] => TuiSubmit::Command(TuiCommand::Sessions),
        ["/resume"] => TuiSubmit::Command(TuiCommand::Resume),
        ["/resume", ..] => TuiSubmit::Command(TuiCommand::Resume),
        ["/propose"] => TuiSubmit::Command(TuiCommand::Error(tui_propose_usage())),
        ["/propose", _path] => TuiSubmit::Command(TuiCommand::Error(tui_propose_usage())),
        ["/propose", path, instruction @ ..] => TuiSubmit::Command(TuiCommand::Propose {
            path: (*path).to_string(),
            instruction: instruction.join(" "),
        }),
        ["/diff"] => TuiSubmit::Command(TuiCommand::Diff),
        ["/clear"] => TuiSubmit::Command(TuiCommand::Clear),
        ["/exit"] | ["/quit"] => TuiSubmit::Exit,
        ["/check"] => TuiSubmit::Command(TuiCommand::Check),
        ["/apply"] => TuiSubmit::Command(TuiCommand::Apply),
        [command, ..] => TuiSubmit::Command(TuiCommand::Unknown((*command).to_string())),
        [] => TuiSubmit::Command(TuiCommand::Unknown(input.to_string())),
    }
}

/// `<provider>/<model-id>` を分解し、provider が registry に存在する時だけ採用する検証付き parser。
pub(super) fn parse_model_route_argument(value: &str) -> Option<(String, String)> {
    let (provider_id, model_id) = value.split_once('/')?;
    let registry = builtin_registry();
    if registry.get_preset(provider_id).is_some() {
        Some((provider_id.to_string(), model_id.to_string()))
    } else {
        None
    }
}

/// 候補から補完で挿入するテキストを作る。`<...>` placeholder は落とし、
/// 引数を取る command(/model・/provider 等)には末尾スペースを足して続きを打ちやすくする。
pub(super) fn slash_completion_text(suggestion: SlashCommandSuggestion) -> String {
    let completion = match suggestion.command.find('<') {
        Some(index) => suggestion.command[..index].trim_end().to_string() + " ",
        None => suggestion.command.to_string(),
    };

    match completion.as_str() {
        "/model" | "/provider" => completion + " ",
        _ => completion,
    }
}
