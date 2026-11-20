//! Integration-level auth tests for `access-lease`.
//!
//! These tests use `mock_auths` rather than `mock_all_auths` so that every
//! call declares exactly which signer it expects. The intent is to verify that
//! the contract correctly enforces its auth requirements and does not accept
//! calls from unexpected signers.

use access_lease::{AccessLease, AccessLeaseClient, Error};
use soroban_sdk::{
    testutils::{Address as _, MockAuth, MockAuthInvoke},
    Address, Env, IntoVal, String,
};

fn setup<'a>() -> (Env, AccessLeaseClient<'a>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(AccessLease, ());
    let client = AccessLeaseClient::new(&env, &id);
    let admin = Address::generate(&env);
    client.init(&admin);
    (env, client, admin)
}

// ── grant_lease auth ─────────────────────────────────────────────────────────

#[test]
fn grant_lease_requires_admin() {
    let (env, client, admin) = setup();
    let buyer = Address::generate(&env);
    let rid = String::from_str(&env, "r");
    client.grant_lease(&rid, &buyer, &100u32);
    assert_eq!(env.auths()[0].0, admin);
}

#[test]
fn grant_lease_non_admin_rejected() {
    let (env, client, _admin) = setup();
    let mallory = Address::generate(&env);
    let buyer = Address::generate(&env);
    let rid = String::from_str(&env, "r");
    env.mock_auths(&[MockAuth {
        address: &mallory,
        invoke: &MockAuthInvoke {
            contract: &client.address,
            fn_name: "grant_lease",
            args: (rid.clone(), buyer.clone(), 100u32).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert!(client.try_grant_lease(&rid, &buyer, &100u32).is_err());
}

// ── extend_lease auth ────────────────────────────────────────────────────────

#[test]
fn extend_lease_requires_admin() {
    let (env, client, admin) = setup();
    let buyer = Address::generate(&env);
    let rid = String::from_str(&env, "r");
    client.grant_lease(&rid, &buyer, &100u32);
    client.extend_lease(&rid, &buyer, &50u32);
    assert_eq!(env.auths()[0].0, admin);
}

#[test]
fn extend_lease_non_admin_rejected() {
    let (env, client, _admin) = setup();
    let mallory = Address::generate(&env);
    let buyer = Address::generate(&env);
    let rid = String::from_str(&env, "r");
    client.grant_lease(&rid, &buyer, &100u32);
    env.mock_auths(&[MockAuth {
        address: &mallory,
        invoke: &MockAuthInvoke {
            contract: &client.address,
            fn_name: "extend_lease",
            args: (rid.clone(), buyer.clone(), 50u32).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert!(client.try_extend_lease(&rid, &buyer, &50u32).is_err());
}

// ── revoke_lease auth ────────────────────────────────────────────────────────

#[test]
fn revoke_lease_requires_admin() {
    let (env, client, admin) = setup();
    let buyer = Address::generate(&env);
    let rid = String::from_str(&env, "r");
    client.grant_lease(&rid, &buyer, &100u32);
    client.revoke_lease(&rid, &buyer);
    assert_eq!(env.auths()[0].0, admin);
}

#[test]
fn revoke_lease_non_admin_rejected() {
    let (env, client, _admin) = setup();
    let mallory = Address::generate(&env);
    let buyer = Address::generate(&env);
    let rid = String::from_str(&env, "r");
    client.grant_lease(&rid, &buyer, &100u32);
    env.mock_auths(&[MockAuth {
        address: &mallory,
        invoke: &MockAuthInvoke {
            contract: &client.address,
            fn_name: "revoke_lease",
            args: (rid.clone(), buyer.clone()).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert!(client.try_revoke_lease(&rid, &buyer).is_err());
}

// ── propose_admin auth ───────────────────────────────────────────────────────

#[test]
fn propose_admin_requires_admin() {
    let (env, client, admin) = setup();
    let candidate = Address::generate(&env);
    client.propose_admin(&candidate);
    assert_eq!(env.auths()[0].0, admin);
}

#[test]
fn propose_admin_non_admin_rejected() {
    let (env, client, _admin) = setup();
    let mallory = Address::generate(&env);
    let candidate = Address::generate(&env);
    env.mock_auths(&[MockAuth {
        address: &mallory,
        invoke: &MockAuthInvoke {
            contract: &client.address,
            fn_name: "propose_admin",
            args: (candidate.clone(),).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert!(client.try_propose_admin(&candidate).is_err());
}

// ── accept_admin auth ────────────────────────────────────────────────────────

#[test]
fn accept_admin_requires_pending_admin_signature() {
    let (env, client, _admin) = setup();
    let b = Address::generate(&env);
    client.propose_admin(&b);
    client.accept_admin(&b);
    // The auth record for accept_admin must come from B.
    assert_eq!(env.auths()[0].0, b);
}

#[test]
fn accept_admin_wrong_signer_rejected() {
    let (env, client, _admin) = setup();
    let b = Address::generate(&env);
    let impostor = Address::generate(&env);
    client.propose_admin(&b);
    // Impostor signing for accept_admin with B's address still fails because
    // the contract will call b.require_auth() which won't match impostor.
    env.mock_auths(&[MockAuth {
        address: &impostor,
        invoke: &MockAuthInvoke {
            contract: &client.address,
            fn_name: "accept_admin",
            args: (b.clone(),).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert!(client.try_accept_admin(&b).is_err());
}

// ── cancel_admin_proposal auth ───────────────────────────────────────────────

#[test]
fn cancel_admin_proposal_requires_admin() {
    let (env, client, admin) = setup();
    let candidate = Address::generate(&env);
    client.propose_admin(&candidate);
    client.cancel_admin_proposal();
    assert_eq!(env.auths()[0].0, admin);
}

#[test]
fn cancel_admin_proposal_non_admin_rejected() {
    let (env, client, _admin) = setup();
    let mallory = Address::generate(&env);
    let candidate = Address::generate(&env);
    client.propose_admin(&candidate);
    env.mock_auths(&[MockAuth {
        address: &mallory,
        invoke: &MockAuthInvoke {
            contract: &client.address,
            fn_name: "cancel_admin_proposal",
            args: ().into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert!(client.try_cancel_admin_proposal().is_err());
}

// ── set_admin auth ───────────────────────────────────────────────────────────

#[test]
fn set_admin_requires_admin() {
    let (env, client, admin) = setup();
    let new_admin = Address::generate(&env);
    client.set_admin(&new_admin);
    assert_eq!(env.auths()[0].0, admin);
}

#[test]
fn set_admin_non_admin_rejected() {
    let (env, client, _admin) = setup();
    let mallory = Address::generate(&env);
    let new_admin = Address::generate(&env);
    env.mock_auths(&[MockAuth {
        address: &mallory,
        invoke: &MockAuthInvoke {
            contract: &client.address,
            fn_name: "set_admin",
            args: (new_admin.clone(),).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert!(client.try_set_admin(&new_admin).is_err());
}

// ── upgrade auth ─────────────────────────────────────────────────────────────

#[test]
fn upgrade_non_admin_rejected() {
    let (env, client, _admin) = setup();
    let mallory = Address::generate(&env);
    let hash = soroban_sdk::BytesN::from_array(&env, &[1; 32]);
    env.mock_auths(&[MockAuth {
        address: &mallory,
        invoke: &MockAuthInvoke {
            contract: &client.address,
            fn_name: "upgrade",
            args: (hash.clone(),).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert!(client.try_upgrade(&hash).is_err());
}

// ── Deadline boundary tests (issue #211) ────────────────────────────────────

#[test]
fn accept_admin_on_last_valid_ledger_succeeds() {
    use access_lease::ADMIN_PROPOSAL_TTL_LEDGERS;
    let (env, client, _admin) = setup();
    let b = Address::generate(&env);
    let now = env.ledger().sequence();
    client.propose_admin(&b);
    // Advance to exactly the deadline (inclusive boundary).
    env.ledger()
        .set_sequence_number(now + ADMIN_PROPOSAL_TTL_LEDGERS);
    client.accept_admin(&b);
    assert_eq!(client.admin(), b);
}

#[test]
fn accept_admin_one_ledger_after_deadline_fails() {
    use access_lease::ADMIN_PROPOSAL_TTL_LEDGERS;
    let (env, client, _admin) = setup();
    let b = Address::generate(&env);
    let now = env.ledger().sequence();
    client.propose_admin(&b);
    env.ledger()
        .set_sequence_number(now + ADMIN_PROPOSAL_TTL_LEDGERS + 1);
    assert_eq!(
        client.try_accept_admin(&b),
        Err(Ok(Error::AdminProposalExpired)),
    );
}
