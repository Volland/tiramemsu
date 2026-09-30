//! Execution errors. `tm-exec` reports the shared [`tm_core::Error`]: `InvalidQuery`,
//! `Unsupported`, the host's `Sqlite` error, and `MissingCapability` at open.

use tm_core::Error;

/// `InvalidQuery { msg }`.
pub fn invalid(msg: impl Into<String>) -> Error {
    Error::InvalidQuery { msg: msg.into() }
}

/// `Unsupported { feature }`.
pub fn unsupported(feature: impl Into<String>) -> Error {
    Error::Unsupported {
        feature: feature.into(),
    }
}

/// `MissingCapability { capability }`.
pub fn missing_capability(capability: &str) -> Error {
    Error::MissingCapability {
        capability: capability.to_string(),
    }
}

/// The feature name reported for path patterns without a registered operator.
pub const NO_PATH_OPERATOR: &str =
    "path patterns: no path operator registered (tm_path, add-path-engine M3)";
