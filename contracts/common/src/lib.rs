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
//!
//! # TTL policy model
//!
//! Shared TTL policies pair a *threshold* (the remaining-live-ledgers point at
//! which a bump is triggered) with a *target* (the live-ledgers value restored
//! by the bump). Valid policies satisfy:
//!
//! * `threshold > 0` and `target > 0`,
//! * `threshold < target` — a bump must extend beyond the point that triggers
//!   it, otherwise the entry would be bumped on every access.
//!
//! This mirrors the threshold-below-target relationship used by the
//! contract-local policies.

#![no_std]

use soroban_sdk::{contracterror, contracttype, Env};

/// Current on-chain state schema version.
///
/// Bump this only when the persisted storage layout changes in a way that
/// requires migration. This is unrelated to the crate/package version.
pub const VERSION: u32 = 1;

/// Minimum state schema version this code can read without migration.
pub const MIN_COMPATIBLE_VERSION: u32 = 1;

/// Shared TTL bump target: live-ledgers value restored when a bump fires.
pub const BUMP: u32 = 1_000_000;

/// Shared TTL bump threshold: remaining-live-ledgers point that triggers a
/// bump. Must be strictly below [`BUMP`].
///
/// Retired: the previous value (`1_000_000`) was not below the target and is
/// no longer used; the corrected value keeps the threshold-below-target
/// invariant.
pub const BUMP_THRESH: u32 = 100_000;

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

/// Returns `true` when `(threshold, target)` is a valid TTL policy pair:
/// both are non-zero and the threshold is strictly below the target.
pub fn is_valid_ttl_policy(threshold: u32, target: u32) -> bool {
    threshold > 0 && target > 0 && threshold < target
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

    #[test]
    fn shared_ttl_policy_is_valid() {
        assert!(is_valid_ttl_policy(BUMP_THRESH, BUMP));
        assert!(BUMP_THRESH < BUMP);
        assert!(BUMP_THRESH > 0);
        assert!(BUMP > 0);
    }

    #[test]
    fn ttl_policy_rejects_threshold_at_or_above_target() {
        assert!(!is_valid_ttl_policy(BUMP, BUMP));
        assert!(!is_valid_ttl_policy(BUMP, BUMP_THRESH));
    }

    #[test]
    fn ttl_policy_rejects_zero_values() {
        assert!(!is_valid_ttl_policy(0, BUMP));
        assert!(!is_valid_ttl_policy(BUMP_THRESH, 0));
        assert!(!is_valid_ttl_policy(0, 0));
    }
}
