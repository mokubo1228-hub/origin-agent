pub mod builtin;
pub mod file;
pub mod precedence;

pub use file::{
    load_user_config, resolve_config_path, LoadedConfig, ProviderConfig, UserConfig,
    ORIGIN_CONFIG_ENV,
};
pub use precedence::{resolve_config, CliOverrides, ConfigError, ResolvedConfig};
