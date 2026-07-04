#[derive(Debug, Eq, PartialEq)]
pub enum Command {
    Help,
    Version,
    Shell,
    Tui,
    Status,
    Providers,
    Models,
    Provider,
    Model,
    Chat,
    Propose,
    Sessions,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HelpTopic {
    Root,
    Shell,
    Tui,
    Status,
    Providers,
    Models,
    Provider,
    Model,
    Chat,
    Propose,
    Sessions,
    SessionsShow,
}

#[derive(Debug, Eq, PartialEq)]
pub enum SessionsAction {
    List,
    ShowLatest,
    Show { session_id: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderAction {
    Show,
    List,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelAction {
    Show,
    List,
}

#[derive(Debug, Eq, PartialEq)]
pub struct Args {
    pub command: Command,
    pub help_topic: HelpTopic,
    pub sessions_action: SessionsAction,
    pub provider_action: ProviderAction,
    pub model_action: ModelAction,
    pub provider: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub resume: Option<String>,
    pub prompt: Option<String>,
    pub paths: Vec<String>,
    pub apply: bool,
    pub check: bool,
    pub retry_invalid: bool,
    pub retry_apply_check: bool,
}

impl Args {
    pub fn parse<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut command = Command::Tui;
        let mut help_topic = HelpTopic::Root;
        let mut sessions_action = SessionsAction::List;
        let mut provider_action = ProviderAction::Show;
        let mut model_action = ModelAction::Show;
        let mut provider = None;
        let mut base_url = None;
        let mut model = None;
        let mut resume = None;
        let mut apply = false;
        let mut check = false;
        let mut retry_invalid = false;
        let mut retry_apply_check = false;
        let mut positionals = Vec::new();
        let mut iter = args.into_iter();

        if let Some(first) = iter.next() {
            if is_help_flag(&first) {
                return Ok(help_args(HelpTopic::Root));
            }
            if is_version_flag(&first) {
                return Ok(version_args());
            }
            if first == "help" {
                let positionals = iter.collect::<Vec<_>>();
                return help_topic_from_positionals(&positionals).map(help_args);
            }

            command = match first.as_str() {
                "shell" => Command::Shell,
                "status" => Command::Status,
                "tui" => Command::Tui,
                "providers" => Command::Providers,
                "models" => Command::Models,
                "provider" => Command::Provider,
                "model" => Command::Model,
                "chat" => Command::Chat,
                "propose" => Command::Propose,
                "sessions" => Command::Sessions,
                _ if first.starts_with("--") => {
                    return Err(format!(
                        "Missing command before option `{first}`.\nTry `origin --help`."
                    ));
                }
                _ => return Err(format!("Unknown command `{first}`.\nTry `origin --help`.")),
            };
        }

        let mut help_requested = false;
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--help" | "-h" => help_requested = true,
                "--provider" => provider = iter.next(),
                "--base-url" => base_url = iter.next(),
                "--model" => model = iter.next(),
                "--resume" => resume = iter.next(),
                "--apply" => apply = true,
                "--check" => check = true,
                "--retry-invalid" => retry_invalid = true,
                "--retry-apply-check" => retry_apply_check = true,
                _ => positionals.push(arg),
            }
        }

        if help_requested {
            help_topic = help_topic_for_command(command, &positionals);
            return Ok(help_args(help_topic));
        }

        if command == Command::Sessions {
            if resume.is_some() {
                return Err("--resume is only supported for chat and propose.".to_string());
            }
            sessions_action = parse_sessions_action(positionals)?;
            positionals = Vec::new();
        }
        if command == Command::Provider {
            provider_action = parse_provider_action(positionals)?;
            positionals = Vec::new();
        }
        if command == Command::Model {
            model_action = parse_model_action(positionals)?;
            positionals = Vec::new();
        }

        if resume.is_some() && !matches!(command, Command::Chat | Command::Propose) {
            return Err("--resume is only supported for chat and propose.".to_string());
        }
        if retry_invalid && command != Command::Propose {
            return Err("--retry-invalid is only supported for propose.".to_string());
        }
        if retry_apply_check && command != Command::Propose {
            return Err("--retry-apply-check is only supported for propose.".to_string());
        }

        let (prompt, paths) = match command {
            Command::Propose => {
                let mut iter = positionals.into_iter();
                (iter.next(), iter.collect())
            }
            _ => {
                let prompt = if positionals.is_empty() {
                    None
                } else {
                    Some(positionals.join(" "))
                };
                (prompt, Vec::new())
            }
        };

        Ok(Self {
            command,
            help_topic,
            sessions_action,
            provider_action,
            model_action,
            provider,
            base_url,
            model,
            resume,
            prompt,
            paths,
            apply,
            check,
            retry_invalid,
            retry_apply_check,
        })
    }
}

fn help_args(help_topic: HelpTopic) -> Args {
    Args {
        command: Command::Help,
        help_topic,
        sessions_action: SessionsAction::List,
        provider_action: ProviderAction::Show,
        model_action: ModelAction::Show,
        provider: None,
        base_url: None,
        model: None,
        resume: None,
        prompt: None,
        paths: Vec::new(),
        apply: false,
        check: false,
        retry_invalid: false,
        retry_apply_check: false,
    }
}

fn version_args() -> Args {
    Args {
        command: Command::Version,
        help_topic: HelpTopic::Root,
        sessions_action: SessionsAction::List,
        provider_action: ProviderAction::Show,
        model_action: ModelAction::Show,
        provider: None,
        base_url: None,
        model: None,
        resume: None,
        prompt: None,
        paths: Vec::new(),
        apply: false,
        check: false,
        retry_invalid: false,
        retry_apply_check: false,
    }
}

fn is_help_flag(value: &str) -> bool {
    matches!(value, "--help" | "-h")
}

fn is_version_flag(value: &str) -> bool {
    matches!(value, "--version" | "-V")
}

fn help_topic_from_positionals(positionals: &[String]) -> Result<HelpTopic, String> {
    match positionals {
        [] => Ok(HelpTopic::Root),
        [topic] if topic == "providers" => Ok(HelpTopic::Providers),
        [topic] if topic == "models" => Ok(HelpTopic::Models),
        [topic] if topic == "shell" => Ok(HelpTopic::Shell),
        [topic] if topic == "tui" => Ok(HelpTopic::Tui),
        [topic] if topic == "status" => Ok(HelpTopic::Status),
        [topic] if topic == "provider" => Ok(HelpTopic::Provider),
        [topic] if topic == "model" => Ok(HelpTopic::Model),
        [topic] if topic == "chat" => Ok(HelpTopic::Chat),
        [topic] if topic == "propose" => Ok(HelpTopic::Propose),
        [topic] if topic == "sessions" => Ok(HelpTopic::Sessions),
        [topic, subtopic] if topic == "sessions" && subtopic == "show" => {
            Ok(HelpTopic::SessionsShow)
        }
        _ => Err("Unknown help topic.\nTry `origin --help`.".to_string()),
    }
}

fn help_topic_for_command(command: Command, positionals: &[String]) -> HelpTopic {
    match command {
        Command::Help => HelpTopic::Root,
        Command::Version => HelpTopic::Root,
        Command::Shell => HelpTopic::Shell,
        Command::Tui => HelpTopic::Tui,
        Command::Status => HelpTopic::Status,
        Command::Providers => HelpTopic::Providers,
        Command::Models => HelpTopic::Models,
        Command::Provider => HelpTopic::Provider,
        Command::Model => HelpTopic::Model,
        Command::Chat => HelpTopic::Chat,
        Command::Propose => HelpTopic::Propose,
        Command::Sessions if matches!(positionals.first().map(String::as_str), Some("show")) => {
            HelpTopic::SessionsShow
        }
        Command::Sessions => HelpTopic::Sessions,
    }
}

fn parse_provider_action(positionals: Vec<String>) -> Result<ProviderAction, String> {
    match positionals.as_slice() {
        [] => Ok(ProviderAction::Show),
        [command] if command == "list" => Ok(ProviderAction::List),
        _ => Err("Usage: origin provider [list]".to_string()),
    }
}

fn parse_model_action(positionals: Vec<String>) -> Result<ModelAction, String> {
    match positionals.as_slice() {
        [] => Ok(ModelAction::Show),
        [command] if command == "list" => Ok(ModelAction::List),
        _ => Err("Usage: origin model [list]".to_string()),
    }
}

fn parse_sessions_action(positionals: Vec<String>) -> Result<SessionsAction, String> {
    match positionals.as_slice() {
        [] => Ok(SessionsAction::List),
        [command, flag] if command == "show" && flag == "--latest" => {
            Ok(SessionsAction::ShowLatest)
        }
        [command, session_id] if command == "show" => Ok(SessionsAction::Show {
            session_id: session_id.clone(),
        }),
        [command] if command == "show" => {
            Err("Missing session id. Usage: origin sessions show --latest|<session-id>".to_string())
        }
        _ => Err("Usage: origin sessions [show --latest|show <session-id>]".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{Args, Command, HelpTopic, ModelAction, ProviderAction};

    #[test]
    fn parses_root_help_flag() {
        let args = Args::parse(["--help".to_string()]).unwrap();

        assert_eq!(args.command, Command::Help);
        assert_eq!(args.help_topic, HelpTopic::Root);
    }

    #[test]
    fn parses_root_version_flag() {
        let args = Args::parse(["--version".to_string()]).unwrap();

        assert_eq!(args.command, Command::Version);
    }

    #[test]
    fn parses_help_command() {
        let args = Args::parse(["help".to_string()]).unwrap();

        assert_eq!(args.command, Command::Help);
        assert_eq!(args.help_topic, HelpTopic::Root);
    }

    #[test]
    fn parses_no_args_as_tui_mode() {
        let args = Args::parse(Vec::<String>::new()).unwrap();

        assert_eq!(args.command, Command::Tui);
    }

    #[test]
    fn parses_shell_command() {
        let args = Args::parse(["shell".to_string()]).unwrap();

        assert_eq!(args.command, Command::Shell);
    }

    #[test]
    fn parses_tui_command() {
        let args = Args::parse(["tui".to_string()]).unwrap();

        assert_eq!(args.command, Command::Tui);
    }

    #[test]
    fn parses_command_help_flag() {
        let args = Args::parse(["propose".to_string(), "--help".to_string()]).unwrap();

        assert_eq!(args.command, Command::Help);
        assert_eq!(args.help_topic, HelpTopic::Propose);
    }

    #[test]
    fn parses_tui_help_flag() {
        let args = Args::parse(["tui".to_string(), "--help".to_string()]).unwrap();

        assert_eq!(args.command, Command::Help);
        assert_eq!(args.help_topic, HelpTopic::Tui);
    }

    #[test]
    fn parses_shell_help_flag() {
        let args = Args::parse(["shell".to_string(), "--help".to_string()]).unwrap();

        assert_eq!(args.command, Command::Help);
        assert_eq!(args.help_topic, HelpTopic::Shell);
    }

    #[test]
    fn parses_sessions_show_help_flag() {
        let args = Args::parse([
            "sessions".to_string(),
            "show".to_string(),
            "--help".to_string(),
        ])
        .unwrap();

        assert_eq!(args.command, Command::Help);
        assert_eq!(args.help_topic, HelpTopic::SessionsShow);
    }

    #[test]
    fn parses_propose_apply_and_check_flags() {
        let args = Args::parse([
            "propose".to_string(),
            "--apply".to_string(),
            "--check".to_string(),
            "fix typo".to_string(),
            "README.md".to_string(),
        ])
        .unwrap();

        assert_eq!(args.command, Command::Propose);
        assert!(args.apply);
        assert!(args.check);
        assert_eq!(args.prompt.as_deref(), Some("fix typo"));
        assert_eq!(args.paths, ["README.md"]);
    }

    #[test]
    fn parses_status_command() {
        let args = Args::parse(["status".to_string()]).unwrap();

        assert_eq!(args.command, Command::Status);
    }

    #[test]
    fn parses_provider_command() {
        let args = Args::parse(["provider".to_string()]).unwrap();

        assert_eq!(args.command, Command::Provider);
        assert_eq!(args.provider_action, ProviderAction::Show);
    }

    #[test]
    fn parses_provider_list_command() {
        let args = Args::parse(["provider".to_string(), "list".to_string()]).unwrap();

        assert_eq!(args.command, Command::Provider);
        assert_eq!(args.provider_action, ProviderAction::List);
    }

    #[test]
    fn parses_model_command() {
        let args = Args::parse(["model".to_string()]).unwrap();

        assert_eq!(args.command, Command::Model);
        assert_eq!(args.model_action, ModelAction::Show);
    }

    #[test]
    fn parses_model_list_command() {
        let args = Args::parse(["model".to_string(), "list".to_string()]).unwrap();

        assert_eq!(args.command, Command::Model);
        assert_eq!(args.model_action, ModelAction::List);
    }

    #[test]
    fn parses_retry_invalid_for_propose() {
        let args = Args::parse([
            "propose".to_string(),
            "--retry-invalid".to_string(),
            "fix typo".to_string(),
            "README.md".to_string(),
        ])
        .unwrap();

        assert_eq!(args.command, Command::Propose);
        assert!(args.retry_invalid);
        assert_eq!(args.prompt.as_deref(), Some("fix typo"));
        assert_eq!(args.paths, ["README.md"]);
    }

    #[test]
    fn parses_retry_apply_check_for_propose() {
        let args = Args::parse([
            "propose".to_string(),
            "--retry-apply-check".to_string(),
            "fix typo".to_string(),
            "README.md".to_string(),
        ])
        .unwrap();

        assert_eq!(args.command, Command::Propose);
        assert!(args.retry_apply_check);
        assert_eq!(args.prompt.as_deref(), Some("fix typo"));
        assert_eq!(args.paths, ["README.md"]);
    }

    #[test]
    fn parses_provider_flag_for_chat() {
        let args = Args::parse([
            "chat".to_string(),
            "--provider".to_string(),
            "openai".to_string(),
            "--model".to_string(),
            "openai-explicit".to_string(),
            "hello".to_string(),
        ])
        .unwrap();

        assert_eq!(args.command, Command::Chat);
        assert_eq!(args.provider.as_deref(), Some("openai"));
        assert_eq!(args.model.as_deref(), Some("openai-explicit"));
        assert_eq!(args.prompt.as_deref(), Some("hello"));
    }

    #[test]
    fn parses_resume_flag_for_chat() {
        let args = Args::parse([
            "chat".to_string(),
            "--resume".to_string(),
            "latest".to_string(),
            "continue".to_string(),
        ])
        .unwrap();

        assert_eq!(args.command, Command::Chat);
        assert_eq!(args.resume.as_deref(), Some("latest"));
        assert_eq!(args.prompt.as_deref(), Some("continue"));
    }

    #[test]
    fn parses_resume_flag_for_propose() {
        let args = Args::parse([
            "propose".to_string(),
            "--resume".to_string(),
            "20260505-120346-af7653".to_string(),
            "fix typo".to_string(),
            "README.md".to_string(),
        ])
        .unwrap();

        assert_eq!(args.command, Command::Propose);
        assert_eq!(args.resume.as_deref(), Some("20260505-120346-af7653"));
        assert_eq!(args.prompt.as_deref(), Some("fix typo"));
        assert_eq!(args.paths, ["README.md"]);
    }

    #[test]
    fn rejects_resume_for_sessions() {
        let err = Args::parse([
            "sessions".to_string(),
            "--resume".to_string(),
            "latest".to_string(),
        ])
        .unwrap_err();

        assert_eq!(err, "--resume is only supported for chat and propose.");
    }

    #[test]
    fn rejects_retry_invalid_for_chat() {
        let err = Args::parse([
            "chat".to_string(),
            "--retry-invalid".to_string(),
            "hello".to_string(),
        ])
        .unwrap_err();

        assert_eq!(err, "--retry-invalid is only supported for propose.");
    }

    #[test]
    fn rejects_retry_apply_check_for_chat() {
        let err = Args::parse([
            "chat".to_string(),
            "--retry-apply-check".to_string(),
            "hello".to_string(),
        ])
        .unwrap_err();

        assert_eq!(err, "--retry-apply-check is only supported for propose.");
    }

    #[test]
    fn provider_defaults_to_unspecified() {
        let args = Args::parse(["models".to_string()]).unwrap();

        assert_eq!(args.command, Command::Models);
        assert_eq!(args.provider, None);
    }

    #[test]
    fn parses_sessions_list() {
        let args = Args::parse(["sessions".to_string()]).unwrap();

        assert_eq!(args.command, Command::Sessions);
        assert_eq!(args.sessions_action, super::SessionsAction::List);
    }

    #[test]
    fn parses_sessions_show_latest() {
        let args = Args::parse([
            "sessions".to_string(),
            "show".to_string(),
            "--latest".to_string(),
        ])
        .unwrap();

        assert_eq!(args.command, Command::Sessions);
        assert_eq!(args.sessions_action, super::SessionsAction::ShowLatest);
    }

    #[test]
    fn parses_sessions_show_by_id() {
        let args = Args::parse([
            "sessions".to_string(),
            "show".to_string(),
            "20260504-180818-02dfa3".to_string(),
        ])
        .unwrap();

        assert_eq!(args.command, Command::Sessions);
        assert_eq!(
            args.sessions_action,
            super::SessionsAction::Show {
                session_id: "20260504-180818-02dfa3".to_string()
            }
        );
    }

    #[test]
    fn rejects_unknown_command_instead_of_defaulting_to_providers() {
        let err = Args::parse(["unknown-command".to_string()]).unwrap_err();

        assert_eq!(
            err,
            "Unknown command `unknown-command`.\nTry `origin --help`."
        );
    }
}
