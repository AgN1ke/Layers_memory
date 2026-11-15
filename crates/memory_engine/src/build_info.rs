use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BuildInfo {
    pub version: &'static str,
    pub commit: Option<&'static str>,
    pub dirty: Option<bool>,
}

/// Source identity captured at build time and available without a checkout.
pub fn build_info() -> BuildInfo {
    let commit = env!("MEMORY_ENGINE_SOURCE_COMMIT");
    BuildInfo {
        version: env!("CARGO_PKG_VERSION"),
        commit: (!commit.is_empty()).then_some(commit),
        dirty: env!("MEMORY_ENGINE_SOURCE_DIRTY").parse().ok(),
    }
}
