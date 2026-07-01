pub mod error;
pub mod openai_compatible;
pub mod presets;
pub mod provider;
pub mod registry;

pub use error::{ProviderError, ProviderResult};
pub use provider::ModelProvider;
pub use registry::{builtin_provider_presets, builtin_registry, ProviderRegistry};
