//! Shared helpers for SynapsVault contracts.
//!
//! # Versioning model
//!
//! There are two distinct notions of "version" in this workspace and they
//! must not be conflated:
//!
//! * **Package / ABI version** — the crate version in `Cargo.toml` (e.g.
//!   `0.1.0`). This is a build-time, string-valued identifier used for
//!   release tracking and off-chain tooling. It is *not* stored on-chain and
//!   is *not* part of any deployed method signature.
//! * **State schema version** — the numeric `VERSION` constant exposed here.
//!   It describes the on-chain storage layout and is what contracts compare
//!   against via [`is_compatible`] when deciding whether persisted state can
//!   be read by the current code.
//!
//! The numeric helpers below are the shared, canonical state-schema API. They
//! are intentionally independent of the string package version so that
//! storage migrations can be reasoned about without touching crate releases.

#![no_std]

use soroban_sdk::{contracterror, contracttype, Env};

/// Current on-chain state schema version.
///
/// Bump this only when the persisted storage layout changes in a way that
/// requires migration. This is unrelated to the crate/package version.
pub const VERSION: u32 = 1;

/// Minimum state schema version this code can read without migration.
pub const MIN_COMPATIBLE_VERSION: u32 = 1;

/// Storage key used to persist the state schema version.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VersionKey {
    Schema,
}

/// Errors returned by the version helpers.
#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum VersionError {
    /// Persisted state schema is newer than this code understands.
    Incompatible = 1,
}

/// Returns the current state schema version compiled into this crate.
pub fn get_version(_env: &Env) -> u32 {
    VERSION
}

/// Returns `true` when `version` can be read by this code without migration.
pub fn is_compatible(version: u32) -> bool {
    version >= MIN_COMPATIBLE_VERSION && version <= VERSION
}

/// Reads the persisted state schema version, if any.
pub fn read_state_version(env: &Env) -> Option<u32> {
    env.storage().instance().get(&VersionKey::Schema)
}

/// Persists the current state schema version.
pub fn write_state_version(env: &Env) {
    env.storage().instance().set(&VersionKey::Schema, &VERSION);
}

/// Ensures persisted state is compatible with this code, returning an error
/// otherwise. Call during initialization or upgrade paths.
pub fn ensure_compatible(env: &Env) -> Result<(), VersionError> {
    match read_state_version(env) {
        Some(v) if is_compatible(v) => Ok(()),
        Some(_) => Err(VersionError::Incompatible),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    #[test]
    fn current_version_is_compatible() {
        assert!(is_compatible(VERSION));
    }

    #[test]
    fn older_than_min_is_incompatible() {
        assert!(!is_compatible(MIN_COMPATIBLE_VERSION - 1));
    }

    #[test]
    fn newer_than_current_is_incompatible() {
        assert!(!is_compatible(VERSION + 1));
    }

    #[test]
    fn state_version_roundtrip() {
        let env = Env::default();
        assert_eq!(read_state_version(&env), None);
        assert!(ensure_compatible(&env).is_ok());
        write_state_version(&env);
        assert_eq!(read_state_version(&env), Some(VERSION));
        assert!(ensure_compatible(&env).is_ok());
    }
}
