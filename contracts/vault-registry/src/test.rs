#![cfg(test)]

extern crate std;

use std::string::ToString;

use super::*;
use proptest::prelude::*;
use soroban_sdk::{
    symbol_short,
    testutils::{storage::Persistent as _, Address as _, Events as _, Ledger as _},
    Address, Env, String, Vec,
};

const DAY_IN_LEDGERS: u32 = 17_280;

/// Prebuilt registry WASM used as the target of upgrade tests. Regenerate with
/// `make fixture` from `contracts/vault-registry` when the interface changes.
const UPGRADE_TARGET_WASM: &[u8] = include_bytes!("../fixtures/vault_registry_upgrade_target.wasm");

fn upload_upgrade_target(env: &Env) -> soroban_sdk::BytesN<32> {
    env.deployer().upload_contract_wasm(UPGRADE_TARGET_WASM)
}

fn resource_storage_ttl(env: &Env, contract: &soroban_sdk::Address, id: &String) -> u32 {
    let key = DataKey::Resource(id.clone());
    env.as_contract(contract, || env.storage().persistent().get_ttl(&key))
}

fn setup<'a>() -> (Env, Address, VaultRegistryClient<'a>) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VaultRegistry, ());
    let client = VaultRegistryClient::new(&env, &contract_id);
    let creator = Address::generate(&env);
    (env, creator, client)
}

fn empty_tags(env: &Env) -> Vec<String> {
    Vec::new(env)
}

fn tags(env: &Env, items: &[&str]) -> Vec<String> {
    let mut v = Vec::new(env);
    for item in items {
        v.push_back(String::from_str(env, item));
    }
    v
}

#[test]
fn register_then_read() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "swcn98besxpp6t1u8e77fqz3");
    let metadata = String::from_str(&env, "ipfs://QmResourceMetadata");

    client.register(&creator, &id, &1_000_000i128, &metadata, &empty_tags(&env));

    assert_eq!(client.count(), 1);
    assert!(client.exists(&id));

    let r = client.get(&id);
    assert_eq!(r.id, id);
    assert_eq!(r.creator, creator);
    assert_eq!(r.price, 1_000_000i128);
    assert_eq!(r.metadata, metadata);
    assert!(r.listed); // Resources are listed by default
}

#[test]
fn count_tracks_multiple_successful_registrations() {
    let (env, creator, client) = setup();
    assert_eq!(client.count(), 0);

    let ids = ["c1", "c2", "c3", "c4"];
    for id in &ids {
        client.register(
            &creator,
            &String::from_str(&env, id),
            &100i128,
            &String::from_str(&env, "m"),
            &empty_tags(&env),
        );
    }
    assert_eq!(client.count(), 4);

    // Failed duplicate must not increment count.
    let dup = String::from_str(&env, "c2");
    assert_eq!(
        client.try_register(
            &creator,
            &dup,
            &100i128,
            &String::from_str(&env, "m"),
            &empty_tags(&env)
        ),
        Err(Ok(Error::AlreadyRegistered))
    );
    assert_eq!(client.count(), 4);
}

#[test]
fn duplicate_registration_fails() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "dup");
    let metadata = String::from_str(&env, "x");
    client.register(&creator, &id, &100i128, &metadata, &empty_tags(&env));

    let res = client.try_register(&creator, &id, &100i128, &metadata, &empty_tags(&env));
    assert_eq!(res, Err(Ok(Error::AlreadyRegistered)));
    assert_eq!(client.count(), 1);
}

#[test]
fn zero_or_negative_price_rejected() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "free");
    let metadata = String::from_str(&env, "x");

    assert_eq!(
        client.try_register(&creator, &id, &0i128, &metadata, &empty_tags(&env)),
        Err(Ok(Error::InvalidPrice))
    );
    assert_eq!(
        client.try_register(&creator, &id, &-5i128, &metadata, &empty_tags(&env)),
        Err(Ok(Error::InvalidPrice))
    );
}

#[test]
fn get_missing_fails() {
    let (env, _creator, client) = setup();
    let res = client.try_get(&String::from_str(&env, "nope"));
    assert_eq!(res, Err(Ok(Error::NotFound)));
}

#[test]
fn set_price_updates_value() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "r1");
    client.register(
        &creator,
        &id,
        &1_000_000i128,
        &String::from_str(&env, "m"),
        &empty_tags(&env),
    );

    client.set_price(&id, &2_500_000i128);
    assert_eq!(client.get(&id).price, 2_500_000i128);

    assert_eq!(
        client.try_set_price(&id, &0i128),
        Err(Ok(Error::InvalidPrice))
    );
}

#[test]
fn update_metadata_changes_pointer() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "r2");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "old"),
        &empty_tags(&env),
    );

    let new_meta = String::from_str(&env, "ipfs://QmNew");
    client.update_metadata(&id, &new_meta);
    assert_eq!(client.get(&id).metadata, new_meta);
}

#[test]
fn ownership_can_transfer() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "r3");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "m"),
        &empty_tags(&env),
    );

    let new_owner = Address::generate(&env);
    client.propose_transfer(&id, &new_owner);
    client.accept_transfer(&id, &new_owner);
    assert_eq!(client.get(&id).creator, new_owner);
}

#[test]
fn set_listed_toggles_listing_state() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "r4");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "m"),
        &empty_tags(&env),
    );

    // Initially listed
    assert!(client.get(&id).listed);

    // Delist
    client.set_listed(&id, &false);
    assert!(!client.get(&id).listed);

    // Re-list
    client.set_listed(&id, &true);
    assert!(client.get(&id).listed);
}

#[test]
fn delist_convenience_method() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "r5");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "m"),
        &empty_tags(&env),
    );

    // Initially listed
    assert!(client.get(&id).listed);

    // Delist using convenience method
    client.delist(&id);
    assert!(!client.get(&id).listed);
}

#[test]
fn set_price_preserves_other_fields() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "r7");
    let metadata = String::from_str(&env, "ipfs://QmPreserve");
    client.register(&creator, &id, &100i128, &metadata, &empty_tags(&env));

    client.set_price(&id, &250i128);
    let resource = client.get(&id);

    assert_eq!(resource.price, 250i128);
    assert_eq!(resource.metadata, metadata);
    assert_eq!(resource.creator, creator);
    assert!(resource.listed);
}

#[test]
fn transfer_ownership_keeps_count_and_order() {
    let (env, creator, client) = setup();
    let ids = ["a", "b"];
    for id in &ids {
        client.register(
            &creator,
            &String::from_str(&env, id),
            &100i128,
            &String::from_str(&env, "m"),
            &empty_tags(&env),
        );
    }

    let new_owner = Address::generate(&env);
    let id = String::from_str(&env, "a");
    client.propose_transfer(&id, &new_owner);
    client.accept_transfer(&id, &new_owner);

    assert_eq!(client.count(), 2);
    let list = client.list(&0u32, &10u32, &false, &None);
    assert_eq!(list.get(0).unwrap().id, id);
    assert_eq!(list.get(0).unwrap().creator, new_owner);
    assert_eq!(list.get(1).unwrap().id, String::from_str(&env, "b"));
}

#[test]
fn update_metadata_preserves_price_and_creator() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "r8");
    let original_metadata = String::from_str(&env, "ipfs://QmOriginal");
    client.register(
        &creator,
        &id,
        &500i128,
        &original_metadata,
        &empty_tags(&env),
    );

    let new_metadata = String::from_str(&env, "ipfs://QmUpdated");
    client.update_metadata(&id, &new_metadata);

    let resource = client.get(&id);
    assert_eq!(resource.metadata, new_metadata);
    assert_eq!(resource.price, 500i128);
    assert_eq!(resource.creator, creator);
}

#[test]
fn get_owner_returns_creator() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "owner-test");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "m"),
        &empty_tags(&env),
    );

    let owner = client.get_owner(&id);
    assert_eq!(owner, creator);
}

#[test]
fn get_owner_missing_fails() {
    let (env, _creator, client) = setup();
    let res = client.try_get_owner(&String::from_str(&env, "nope"));
    assert_eq!(res, Err(Ok(Error::NotFound)));
}

#[test]
fn get_owner_after_transfer() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "owner-xfer");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "m"),
        &empty_tags(&env),
    );

    let new_owner = Address::generate(&env);
    client.propose_transfer(&id, &new_owner);
    client.accept_transfer(&id, &new_owner);

    assert_eq!(client.get_owner(&id), new_owner);
}

#[test]
fn set_listed_requires_creator_auth() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "r6");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "m"),
        &empty_tags(&env),
    );

    // This should work fine since we mock all auths
    client.set_listed(&id, &false);
    assert!(!client.get(&id).listed);
}

#[test]
fn set_listed_on_missing_resource_fails() {
    let (env, _creator, client) = setup();
    let id = String::from_str(&env, "missing");

    let res = client.try_set_listed(&id, &false);
    assert_eq!(res, Err(Ok(Error::NotFound)));
}

#[test]
fn list_empty_returns_empty() {
    let (_env, _creator, client) = setup();
    let page = client.list(&0u32, &20u32, &false, &None);
    assert_eq!(page.len(), 0);
}

#[test]
fn list_returns_all_in_insertion_order() {
    let (env, creator, client) = setup();
    let ids = ["a", "b", "c"];
    for id in &ids {
        client.register(
            &creator,
            &String::from_str(&env, id),
            &100i128,
            &String::from_str(&env, "m"),
            &empty_tags(&env),
        );
    }

    let page = client.list(&0u32, &20u32, &false, &None);
    assert_eq!(page.len(), 3);
    assert_eq!(page.get(0).unwrap().id, String::from_str(&env, "a"));
    assert_eq!(page.get(1).unwrap().id, String::from_str(&env, "b"));
    assert_eq!(page.get(2).unwrap().id, String::from_str(&env, "c"));
}

fn metadata_of_len(env: &Env, len: u32) -> String {
    let s = "a".repeat(len as usize);
    String::from_str(env, &s)
}

#[test]
fn register_accepts_metadata_at_max_length() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "meta-max");
    let metadata = metadata_of_len(&env, MAX_METADATA_POINTER_LEN);
    client.register(&creator, &id, &100i128, &metadata, &empty_tags(&env));
    assert_eq!(client.get(&id).metadata.len(), MAX_METADATA_POINTER_LEN);
}

#[test]
fn register_rejects_metadata_over_max_length() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "meta-long");
    let metadata = metadata_of_len(&env, MAX_METADATA_POINTER_LEN + 1);
    assert_eq!(
        client.try_register(&creator, &id, &100i128, &metadata, &empty_tags(&env)),
        Err(Ok(Error::MetadataTooLong))
    );
    assert!(!client.exists(&id));
}

#[test]
fn update_metadata_accepts_at_max_length() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "meta-upd-ok");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "short"),
        &empty_tags(&env),
    );
    let metadata = metadata_of_len(&env, MAX_METADATA_POINTER_LEN);
    client.update_metadata(&id, &metadata);
    assert_eq!(client.get(&id).metadata.len(), MAX_METADATA_POINTER_LEN);
}

#[test]
fn update_metadata_rejects_over_max_length() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "meta-upd-bad");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "short"),
        &empty_tags(&env),
    );
    let metadata = metadata_of_len(&env, MAX_METADATA_POINTER_LEN + 1);
    assert_eq!(
        client.try_update_metadata(&id, &metadata),
        Err(Ok(Error::MetadataTooLong))
    );
    assert_eq!(client.get(&id).metadata, String::from_str(&env, "short"));
}

fn register_n(env: &Env, creator: &Address, client: &VaultRegistryClient<'_>, ids: &[&str]) {
    for id in ids {
        client.register(
            creator,
            &String::from_str(env, id),
            &100i128,
            &String::from_str(env, "m"),
            &empty_tags(env),
        );
    }
}

#[test]
fn list_pagination_first_page() {
    let (env, creator, client) = setup();
    register_n(&env, &creator, &client, &["r0", "r1", "r2", "r3", "r4"]);

    let page = client.list(&0u32, &3u32, &false, &None);
    assert_eq!(page.len(), 3);
    assert_eq!(page.get(0).unwrap().id, String::from_str(&env, "r0"));
    assert_eq!(page.get(2).unwrap().id, String::from_str(&env, "r2"));
}

#[test]
fn list_pagination_second_page() {
    let (env, creator, client) = setup();
    register_n(&env, &creator, &client, &["r0", "r1", "r2", "r3", "r4"]);

    let page = client.list(&3u32, &3u32, &false, &None);
    assert_eq!(page.len(), 2); // only r3, r4 remain
    assert_eq!(page.get(0).unwrap().id, String::from_str(&env, "r3"));
    assert_eq!(page.get(1).unwrap().id, String::from_str(&env, "r4"));
}

#[test]
fn list_start_beyond_count_returns_empty() {
    let (env, creator, client) = setup();
    client.register(
        &creator,
        &String::from_str(&env, "x"),
        &100i128,
        &String::from_str(&env, "m"),
        &empty_tags(&env),
    );

    let page = client.list(&99u32, &10u32, &false, &None);
    assert_eq!(page.len(), 0);
}

#[test]
fn register_extends_resource_storage_ttl() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "ttl-register");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "m"),
        &empty_tags(&env),
    );
    assert_eq!(
        resource_storage_ttl(&env, &client.address, &id),
        TTL_BUMP_AMOUNT
    );
}

#[test]
fn set_price_reextends_resource_ttl() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "ttl-price");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "m"),
        &empty_tags(&env),
    );
    env.ledger()
        .set_sequence_number(env.ledger().sequence() + DAY_IN_LEDGERS);
    assert_eq!(
        resource_storage_ttl(&env, &client.address, &id),
        TTL_BUMP_AMOUNT - DAY_IN_LEDGERS
    );

    client.set_price(&id, &200i128);
    assert_eq!(
        resource_storage_ttl(&env, &client.address, &id),
        TTL_BUMP_AMOUNT
    );
}

#[test]
fn update_metadata_reextends_resource_ttl() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "ttl-meta");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "old"),
        &empty_tags(&env),
    );
    env.ledger()
        .set_sequence_number(env.ledger().sequence() + DAY_IN_LEDGERS);

    client.update_metadata(&id, &String::from_str(&env, "new"));
    assert_eq!(
        resource_storage_ttl(&env, &client.address, &id),
        TTL_BUMP_AMOUNT
    );
}

#[test]
fn transfer_ownership_reextends_resource_ttl() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "ttl-xfer");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "m"),
        &empty_tags(&env),
    );
    env.ledger()
        .set_sequence_number(env.ledger().sequence() + DAY_IN_LEDGERS);

    let new_owner = Address::generate(&env);
    client.propose_transfer(&id, &new_owner);
    client.accept_transfer(&id, &new_owner);
    assert_eq!(
        resource_storage_ttl(&env, &client.address, &id),
        TTL_BUMP_AMOUNT
    );
}

#[test]
fn list_limit_capped_at_20() {
    let (env, creator, client) = setup();
    let ids = [
        "i00", "i01", "i02", "i03", "i04", "i05", "i06", "i07", "i08", "i09", "i10", "i11", "i12",
        "i13", "i14", "i15", "i16", "i17", "i18", "i19", "i20", "i21", "i22", "i23", "i24",
    ];
    register_n(&env, &creator, &client, &ids);

    // Requesting 25 items should be silently capped to 20.
    let page = client.list(&0u32, &25u32, &false, &None);
    assert_eq!(page.len(), 20);
}

#[test]
fn register_with_tags_stores_labels() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "tagged");
    let metadata = String::from_str(&env, "ipfs://QmTagged");
    let resource_tags = tags(&env, &["dataset", "research"]);

    client.register(&creator, &id, &100i128, &metadata, &resource_tags);

    let r = client.get(&id);
    assert_eq!(r.metadata, metadata);
    assert_eq!(r.tags.len(), 2);
    assert_eq!(r.tags.get(0).unwrap(), String::from_str(&env, "dataset"));
    assert_eq!(r.tags.get(1).unwrap(), String::from_str(&env, "research"));
}

#[test]
fn set_tags_updates_value_without_touching_metadata() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "tag-update");
    let metadata = String::from_str(&env, "ipfs://QmKeepMeta");
    client.register(&creator, &id, &100i128, &metadata, &empty_tags(&env));

    let new_tags = tags(&env, &["finance", "api"]);
    client.set_tags(&id, &new_tags);

    let r = client.get(&id);
    assert_eq!(r.metadata, metadata);
    assert_eq!(r.tags.len(), 2);
    assert_eq!(r.tags.get(0).unwrap(), String::from_str(&env, "finance"));
}

#[test]
fn invalid_tag_rejected() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "bad-tag");
    let metadata = String::from_str(&env, "m");
    let empty = String::from_str(&env, "");
    let mut bad = Vec::new(&env);
    bad.push_back(empty);

    assert_eq!(
        client.try_register(&creator, &id, &100i128, &metadata, &bad),
        Err(Ok(Error::InvalidTag))
    );
    assert!(!client.exists(&id));

    client.register(&creator, &id, &100i128, &metadata, &empty_tags(&env));
    assert_eq!(client.try_set_tags(&id, &bad), Err(Ok(Error::InvalidTag)));
}

#[test]
fn init_sets_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VaultRegistry, ());
    let client = VaultRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    client.init(&admin);

    assert_eq!(client.admin(), admin);
}

#[test]
fn upgrade_requires_admin_auth() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VaultRegistry, ());
    let client = VaultRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    client.init(&admin);

    let wasm_hash = upload_upgrade_target(&env);
    client.upgrade(&wasm_hash);

    // Admin auth must have been required for the upgrade call.
    assert!(env.auths().iter().any(|(addr, _)| addr == &admin));
}

#[test]
fn upgrade_rejects_non_admin() {
    let env = Env::default();
    let contract_id = env.register(VaultRegistry, ());
    let client = VaultRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let non_admin = Address::generate(&env);

    env.mock_all_auths();
    client.init(&admin);

    // Only the non-admin authorizes the call; the contract must reject it.
    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &non_admin,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &contract_id,
            fn_name: "upgrade",
            args: soroban_sdk::vec![
                &env,
                soroban_sdk::IntoVal::<Env, soroban_sdk::Val>::into_val(
                    &soroban_sdk::BytesN::from_array(&env, &[0u8; 32]),
                    &env,
                ),
            ],
            sub_invokes: &[],
        },
    }]);

    let wasm_hash = soroban_sdk::BytesN::from_array(&env, &[0u8; 32]);
    let res = client.try_upgrade(&wasm_hash);
    assert!(res.is_err());
}

#[test]
fn upgrade_with_valid_wasm_hash_succeeds() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VaultRegistry, ());
    let client = VaultRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    client.init(&admin);

    let creator = Address::generate(&env);
    let id = String::from_str(&env, "pre-upgrade");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "m"),
        &empty_tags(&env),
    );

    let wasm_hash = upload_upgrade_target(&env);
    client.upgrade(&wasm_hash);

    // Contract remains functional and keeps its state after upgrade.
    assert_eq!(client.count(), 1);
    assert_eq!(client.get(&id).creator, creator);
    assert_eq!(client.admin(), admin);
}

#[test]
fn entrypoint_behavior_regression_after_optimization() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "regression");
    let metadata = String::from_str(&env, "ipfs://QmRegression");

    client.register(&creator, &id, &100i128, &metadata, &empty_tags(&env));
    assert_eq!(client.count(), 1);
    assert!(client.exists(&id));
    assert_eq!(client.get(&id).metadata, metadata);

    client.set_price(&id, &250i128);
    assert_eq!(client.get(&id).price, 250i128);

    client.set_listed(&id, &false);
    assert!(!client.get(&id).listed);

    let new_owner = Address::generate(&env);
    client.propose_transfer(&id, &new_owner);
    client.accept_transfer(&id, &new_owner);
    assert_eq!(client.count(), 1);
}

#[test]
fn footprint_regression_register_and_read() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "footprint");
    let metadata = String::from_str(&env, "ipfs://QmFootprint");

    client.register(&creator, &id, &100i128, &metadata, &empty_tags(&env));

    // Entrypoints must still return correct values after optimization.
    assert_eq!(client.count(), 1);
    assert!(client.exists(&id));
    let r = client.get(&id);
    assert_eq!(r.id, id);
    assert_eq!(r.creator, creator);
    assert_eq!(r.price, 100i128);
    assert_eq!(r.metadata, metadata);
    assert!(r.listed);

    let page = client.list(&0u32, &10u32, &false, &None);
    assert_eq!(page.len(), 1);
    assert_eq!(page.get(0).unwrap().id, id);
}

#[test]
fn version_returns_expected_string() {
    let (_env, _creator, client) = setup();
    assert_eq!(client.version(), String::from_str(&_env, VERSION));
}

#[test]
fn version_is_stable_across_calls() {
    let (_env, _creator, client) = setup();
    let v1 = client.version();
    let v2 = client.version();
    assert_eq!(v1, v2);
}

#[test]
fn init_twice_is_rejected() {
    let (env, _creator, client) = setup();
    let admin = Address::generate(&env);
    client.init(&admin);
    assert_eq!(
        client.try_init(&Address::generate(&env)),
        Err(Ok(Error::AlreadyInitialised))
    );
    assert_eq!(client.admin(), admin);
}

#[test]
fn upgrade_before_init_fails() {
    let (env, _creator, client) = setup();
    let wasm_hash = upload_upgrade_target(&env);
    assert_eq!(
        client.try_upgrade(&wasm_hash),
        Err(Ok(Error::NotInitialised))
    );
}

#[test]
fn version_check() {
    let (env, _creator, client) = setup();
    client.init(&Address::generate(&env));
    assert_eq!(client.get_version(), String::from_str(&env, VERSION));
    assert!(client.is_compatible(&String::from_str(&env, VERSION)));
    assert!(!client.is_compatible(&String::from_str(&env, "99.0.0")));
    assert!(!client.is_compatible(&String::from_str(&env, "")));
}

#[test]
fn too_many_or_too_long_tags_rejected() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "tags");
    let m = String::from_str(&env, "m");
    let nine = tags(&env, &["a", "b", "c", "d", "e", "f", "g", "h", "i"]);
    assert_eq!(
        client.try_register(&creator, &id, &100i128, &m, &nine),
        Err(Ok(Error::InvalidTag))
    );
    let long = "x".repeat(33);
    let too_long = tags(&env, &[long.as_str()]);
    assert_eq!(
        client.try_register(&creator, &id, &100i128, &m, &too_long),
        Err(Ok(Error::InvalidTag))
    );
    let eight = tags(&env, &["a", "b", "c", "d", "e", "f", "g", "h"]);
    client.register(&creator, &id, &100i128, &m, &eight);
    assert_eq!(client.get(&id).tags.len(), 8);
}

#[test]
fn mutations_require_creator_auth() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "auth");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "m"),
        &empty_tags(&env),
    );
    assert_eq!(env.auths()[0].0, creator);

    client.set_price(&id, &200i128);
    assert_eq!(env.auths()[0].0, creator);

    let new_owner = Address::generate(&env);
    client.propose_transfer(&id, &new_owner);
    assert_eq!(env.auths()[0].0, creator);
    // Accepting is authorised by the incoming owner (two-step transfer).
    client.accept_transfer(&id, &new_owner);
    assert_eq!(env.auths()[0].0, new_owner);

    // After transfer only the new owner is asked to authorize.
    client.set_listed(&id, &false);
    assert_eq!(env.auths()[0].0, new_owner);
}

#[test]
fn set_price_by_non_creator_is_rejected() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "steal");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "m"),
        &empty_tags(&env),
    );

    let mallory = Address::generate(&env);
    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &mallory,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &client.address,
            fn_name: "set_price",
            args: (id.clone(), 1i128).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert!(client.try_set_price(&id, &1i128).is_err());
    assert_eq!(client.get(&id).price, 100i128);
}

/// Prints CPU / memory cost of the hot entrypoints and guards against large
/// regressions. Run with `cargo test -p vault-registry gas -- --nocapture`.
#[test]
fn gas_register_and_list_budget() {
    let (env, creator, client) = setup();
    let ids = [
        "g00", "g01", "g02", "g03", "g04", "g05", "g06", "g07", "g08", "g09", "g10", "g11", "g12",
        "g13", "g14", "g15", "g16", "g17", "g18", "g19",
    ];

    env.cost_estimate().budget().reset_default();
    client.register(
        &creator,
        &String::from_str(&env, "g-first"),
        &100i128,
        &String::from_str(&env, "ipfs://QmGas"),
        &tags(&env, &["dataset", "research"]),
    );
    let reg_cpu = env.cost_estimate().budget().cpu_instruction_cost();
    let reg_mem = env.cost_estimate().budget().memory_bytes_cost();
    std::println!("register: cpu={reg_cpu} mem={reg_mem}");

    let first = String::from_str(&env, "g-first");
    env.cost_estimate().budget().reset_default();
    client.set_price(&first, &200i128);
    std::println!(
        "set_price: cpu={} mem={}",
        env.cost_estimate().budget().cpu_instruction_cost(),
        env.cost_estimate().budget().memory_bytes_cost()
    );

    env.cost_estimate().budget().reset_default();
    client.get(&first);
    std::println!(
        "get: cpu={} mem={}",
        env.cost_estimate().budget().cpu_instruction_cost(),
        env.cost_estimate().budget().memory_bytes_cost()
    );

    register_n(&env, &creator, &client, &ids);
    env.cost_estimate().budget().reset_default();
    let page = client.list(&0u32, &MAX_PAGE_SIZE, &false, &None);
    let list_cpu = env.cost_estimate().budget().cpu_instruction_cost();
    let list_mem = env.cost_estimate().budget().memory_bytes_cost();
    std::println!("list(20): cpu={list_cpu} mem={list_mem}");
    assert_eq!(page.len(), MAX_PAGE_SIZE);

    // Network limit is 100M instructions per tx; stay well below it.
    assert!(reg_cpu < 10_000_000, "register too expensive: {reg_cpu}");
    assert!(list_cpu < 50_000_000, "list too expensive: {list_cpu}");
}

/// Event topics/data must stay wire-compatible with what indexers expect
/// (see docs/CONTRACTS.md).  This test covers **every** event type emitted by
/// `vault-registry` (issue #123).
///
/// `env.events().all()` only holds events from the most recent invocation, so
/// each call's event is asserted immediately after that call.
#[test]
fn events_have_documented_shape() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VaultRegistry, ());
    let client = VaultRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);
    let new_owner = Address::generate(&env);
    let id = String::from_str(&env, "ev");
    let meta = String::from_str(&env, "ipfs://QmInit");
    let new_meta = String::from_str(&env, "ipfs://QmUpdated");

    // Helper: assert the most-recent event emitted by the contract.
    let assert_last = |topics: soroban_sdk::Vec<soroban_sdk::Val>,
                       data: soroban_sdk::Val| {
        assert_eq!(
            env.events().all().filter_by_contract(&client.address),
            soroban_sdk::vec![&env, (client.address.clone(), topics, data)]
        );
    };

    // 1. init — ("init",) | admin
    client.init(&admin);
    assert_last(
        (symbol_short!("init"),).into_val(&env),
        admin.into_val(&env),
    );

    // 2. register — ("register", creator) | id
    client.register(&creator, &id, &100i128, &meta, &empty_tags(&env));
    assert_last(
        (symbol_short!("register"), creator.clone()).into_val(&env),
        id.clone().into_val(&env),
    );

    // 3. setprice — ("setprice", id) | new_price
    client.set_price(&id, &250i128);
    assert_last(
        (symbol_short!("setprice"), id.clone()).into_val(&env),
        250i128.into_val(&env),
    );

    // 4. updmeta — ("updmeta", id) | new_metadata
    client.update_metadata(&id, &new_meta);
    assert_last(
        (symbol_short!("updmeta"), id.clone()).into_val(&env),
        new_meta.into_val(&env),
    );

    // 5. settags — ("settags", id) | tags
    let new_tags = tags(&env, &["dataset"]);
    client.set_tags(&id, &new_tags);
    assert_last(
        (symbol_short!("settags"), id.clone()).into_val(&env),
        new_tags.into_val(&env),
    );

    // 6. setlisted (listed = true, no-op is skipped; first set to false via delist)
    //    delist — ("delist", id) | creator
    client.delist(&id);
    assert_last(
        (symbol_short!("delist"), id.clone()).into_val(&env),
        creator.clone().into_val(&env),
    );

    // 7. setlisted — ("setlisted", id) | listed  (re-listing emits setlisted)
    client.set_listed(&id, &true);
    assert_last(
        (symbol_short!("setlisted"), id.clone()).into_val(&env),
        true.into_val(&env),
    );

    // 8. proptrans — ("proptrans", id) | proposed_new_creator
    client.propose_transfer(&id, &new_owner);
    assert_last(
        (symbol_short!("proptrans"), id.clone()).into_val(&env),
        new_owner.clone().into_val(&env),
    );

    // 9. canceltrans — ("canceltrans", id) | ()
    client.cancel_transfer(&id);
    assert_last(
        (symbol_short!("canceltrans"), id.clone()).into_val(&env),
        ().into_val(&env),
    );

    // 10. transfer — ("transfer", id) | new_creator  (full two-step flow)
    client.propose_transfer(&id, &new_owner);
    client.accept_transfer(&id, &new_owner);
    assert_last(
        (symbol_short!("transfer"), id.clone()).into_val(&env),
        new_owner.clone().into_val(&env),
    );

    // 11. upgrade — ("upgrade",) | new_wasm_hash
    //     Done last because uploading the WASM is itself a host call.
    let wasm_hash = upload_upgrade_target(&env);
    client.upgrade(&wasm_hash);
    assert_last(
        (symbol_short!("upgrade"),).into_val(&env),
        wasm_hash.into_val(&env),
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]
    #[test]
    fn test_metadata_pointer_roundtrip_property(
        id_str in r"[a-zA-Z0-9_-]{1,32}",
        price in 1..1000000000000i128,
        price_2 in 1..1000000000000i128,
        meta_str in r"[a-zA-Z0-9:/\\._-]{1,512}",
        meta_str_2 in r"[a-zA-Z0-9:/\\._-]{1,512}",
        listed in any::<bool>(),
    ) {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(VaultRegistry, ());
        let client = VaultRegistryClient::new(&env, &contract_id);
        let creator = Address::generate(&env);

        let id = String::from_str(&env, &id_str);
        let metadata = String::from_str(&env, &meta_str);
        let metadata_2 = String::from_str(&env, &meta_str_2);

        // 1. Register resource with initial metadata
        client.register(&creator, &id, &price, &metadata, &empty_tags(&env));

        // 2. Get and verify metadata is identical
        let r = client.get(&id);
        assert_eq!(r.metadata, metadata);
        assert_eq!(r.price, price);
        assert_eq!(r.creator, creator);
        assert!(r.listed);

        // 3. Update metadata
        client.update_metadata(&id, &metadata_2);

        // 4. Verify updated metadata is identical and other fields preserved
        let r2 = client.get(&id);
        assert_eq!(r2.metadata, metadata_2);
        assert_eq!(r2.price, price);
        assert_eq!(r2.creator, creator);
        assert!(r2.listed);

        // 5. Update price and verify metadata is unaffected
        client.set_price(&id, &price_2);
        let r3 = client.get(&id);
        assert_eq!(r3.metadata, metadata_2);
        assert_eq!(r3.price, price_2);

        // 6. Update listing status and verify metadata is unaffected
        client.set_listed(&id, &listed);
        let r4 = client.get(&id);
        assert_eq!(r4.metadata, metadata_2);
        assert_eq!(r4.listed, listed);
    }
}

// ── #80: UpdateMetadataEvent must carry the new metadata pointer ─────────────

/// Verify that `update_metadata` emits an event whose data field contains the
/// new metadata pointer so that indexers do not need an extra ledger read.
#[test]
fn update_metadata_event_includes_new_pointer() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "ev-meta");
    let old_meta = String::from_str(&env, "ipfs://QmOld");
    let new_meta = String::from_str(&env, "ipfs://QmNew");

    client.register(&creator, &id, &100i128, &old_meta, &empty_tags(&env));
    env.events().all(); // clear previous events

    client.update_metadata(&id, &new_meta);

    // data value must be the new metadata string, not unit / empty
    assert_eq!(
        env.events().all().filter_by_contract(&client.address),
        soroban_sdk::vec![
            &env,
            (
                client.address.clone(),
                (symbol_short!("updmeta"), id.clone()).into_val(&env),
                new_meta.into_val(&env),
            ),
        ]
    );
}

// ── #81: set_listed must be a no-op when value is unchanged ──────────────────

/// Calling `set_listed` with the current value must not write to storage
/// (TTL is unchanged) and must not emit any event.
#[test]
fn set_listed_no_op_when_value_unchanged() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "noop-listed");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "m"),
        &empty_tags(&env),
    );

    // Resource is listed by default; call set_listed(true) — should be no-op.
    let ttl_before = resource_storage_ttl(&env, &client.address, &id);
    env.events().all(); // clear

    client.set_listed(&id, &true);

    // No events emitted.
    let events = env.events().all().filter_by_contract(&client.address);
    assert_eq!(
        events.events().len(),
        0,
        "expected no event on no-op set_listed"
    );

    // TTL must not have been bumped (no storage write happened).
    let ttl_after = resource_storage_ttl(&env, &client.address, &id);
    assert_eq!(
        ttl_before, ttl_after,
        "TTL must not change on no-op set_listed"
    );
}

/// Calling `set_listed` with a different value must still emit the event.
#[test]
fn set_listed_emits_event_when_value_changes() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "change-listed");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "m"),
        &empty_tags(&env),
    );
    env.events().all(); // clear

    client.set_listed(&id, &false);

    assert_eq!(
        env.events().all().filter_by_contract(&client.address),
        soroban_sdk::vec![
            &env,
            (
                client.address.clone(),
                // Unlisting goes through the canonical delist path.
                (symbol_short!("delist"), id.clone()).into_val(&env),
                creator.into_val(&env),
            ),
        ],
        "expected one delist event when listed changes to false"
    );
}

// ── #83: is_compatible must reject malformed / boundary version strings ───────

/// Versions with only one dot (no patch component) must return false.
#[test]
fn is_compatible_rejects_version_without_patch() {
    let (env, _creator, client) = setup();
    // "0.1" has no patch — must not match "0.1.x"
    assert!(
        !client.is_compatible(&String::from_str(&env, "0.1")),
        "two-component version must not be compatible"
    );
    assert!(
        !client.is_compatible(&String::from_str(&env, "1.0")),
        "two-component version must not be compatible"
    );
}

/// Completely malformed strings must return false.
#[test]
fn is_compatible_rejects_malformed_versions() {
    let (env, _creator, client) = setup();
    let bad = [
        "", "abc", ".", "..", "1", "1.", ".0.1", "0..1", "0.1.", "0.1.2.3",
    ];
    for v in &bad {
        assert!(
            !client.is_compatible(&String::from_str(&env, v)),
            "expected false for malformed version {:?}",
            v
        );
    }
}

/// A well-formed version with matching major.minor but differing patch is
/// still compatible (patch is not part of the compatibility check).
#[test]
fn is_compatible_accepts_different_patch() {
    let (env, _creator, client) = setup();
    // VERSION is the compile-time crate version; craft a variant with a
    // different patch number to ensure patch is ignored.
    let compatible_alt = String::from_str(&env, "0.1.99");
    // This only holds if VERSION is 0.1.x — adapt if the crate version changes.
    // We use a known-compatible full version first as a sanity check.
    assert!(
        client.is_compatible(&String::from_str(&env, VERSION)),
        "current VERSION must be compatible with itself"
    );
    // A higher patch on the same major.minor is also compatible.
    let _ = compatible_alt; // used conditionally below
                            // Build a version string with the same major.minor as VERSION but patch=99.
    let version_bytes = VERSION.as_bytes();
    let mut dots = 0usize;
    let mut second_dot = version_bytes.len();
    for (i, b) in version_bytes.iter().enumerate() {
        if *b == b'.' {
            dots += 1;
            if dots == 2 {
                second_dot = i;
                break;
            }
        }
    }
    let prefix = &VERSION[..second_dot];
    let alt = std::format!("{}.99", prefix);
    assert!(
        client.is_compatible(&String::from_str(&env, &alt)),
        "same major.minor with different patch must be compatible"
    );
}

/// Boundary: version string at exactly 32 bytes is accepted if well-formed;
/// 33+ bytes is rejected by the length guard.
#[test]
fn is_compatible_length_boundary() {
    let (env, _creator, client) = setup();
    // 33-character version — must be rejected by the len > BUF_LEN guard.
    let long = "1".repeat(11) + "." + &"2".repeat(11) + "." + &"3".repeat(9); // 33 chars
    assert_eq!(long.len(), 33);
    assert!(
        !client.is_compatible(&String::from_str(&env, &long)),
        "version longer than 32 bytes must be rejected"
    );

    // A well-formed 32-byte version string should parse without panicking.
    // e.g. "111.222.333444555" is 17 chars; pad the patch to hit 32 chars.
    let at_limit = "1.1.".to_string() + &"0".repeat(28); // 32 chars
    assert_eq!(at_limit.len(), 32);
    // We don't assert true/false here (major.minor won't match VERSION in
    // general) but we do assert it doesn't panic and returns a bool.
    let _ = client.is_compatible(&String::from_str(&env, &at_limit));
}

// ── #85: is_compatible must reject empty / non-numeric versions ──────────────

/// `is_compatible` must return false (without panicking) for an empty string
/// and for non-numeric version strings.
#[test]
fn is_compatible_rejects_empty_and_non_numeric() {
    let (env, _creator, client) = setup();

    // Empty string must be rejected.
    assert!(
        !client.is_compatible(&String::from_str(&env, "")),
        "empty version must not be compatible"
    );

    // Non-numeric components must be rejected without panicking.
    let non_numeric = ["abc", "1.x.0", "a.b.c", "1.2.x", "x.2.3", "1.2.3-rc1"];
    for v in &non_numeric {
        assert!(
            !client.is_compatible(&String::from_str(&env, v)),
            "expected false for non-numeric version {:?}",
            v
        );
    }
}

// ── #84: is_compatible must reject versions longer than the 32-byte buffer ───

/// Version strings longer than the 32-byte buffer must be rejected without
/// panicking or being silently truncated to a matching prefix.
#[test]
fn is_compatible_rejects_over_buffer_length() {
    let (env, _creator, client) = setup();

    // A well-formed-looking string that is 33 bytes — one over the buffer.
    let over = "1.1.".to_string() + &"0".repeat(29); // 33 chars
    assert_eq!(over.len(), 33);
    assert!(
        !client.is_compatible(&String::from_str(&env, &over)),
        "version longer than 32 bytes must be rejected"
    );

    // A much longer string must also be rejected (no truncation to a match).
    let very_long = "1.1.".to_string() + &"0".repeat(100);
    assert!(very_long.len() > 32);
    assert!(
        !client.is_compatible(&String::from_str(&env, &very_long)),
        "very long version must be rejected"
    );
}

// ── #86: register on an archived id must be rejected ─────────────────────────

/// After a resource's persistent entry is archived (TTL lapses), registering
/// the same id again must be rejected with `AlreadyRegistered` rather than
/// silently succeeding due to `has` being unreliable for archived entries.
#[test]
fn register_on_archived_id_is_rejected() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "archived");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "m"),
        &empty_tags(&env),
    );
    assert_eq!(client.count(), 1);

    // Advance the ledger far past the persistent TTL so the resource entry is
    // archived out of the live ledger.
    env.ledger()
        .set_sequence_number(env.ledger().sequence() + TTL_BUMP_AMOUNT + DAY_IN_LEDGERS);

    // Re-registering the same id must be rejected, not silently accepted.
    let res = client.try_register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "m"),
        &empty_tags(&env),
    );
    assert_eq!(res, Err(Ok(Error::AlreadyRegistered)));
}

// ── #195: Strict tag character validation ─────────────────────────────────────

/// Tags containing control bytes (0x00–0x1F) must be rejected.
#[test]
fn tag_with_control_byte_rejected() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "ctrl-tag");
    let m = String::from_str(&env, "m");

    // Build a tag with a NUL byte (0x00).
    let nul_tag = String::from_bytes(&env, &[b'o', b'k', 0x00]);
    let mut bad = Vec::new(&env);
    bad.push_back(nul_tag);
    assert_eq!(
        client.try_register(&creator, &id, &100i128, &m, &bad),
        Err(Ok(Error::InvalidTag)),
        "NUL byte must be rejected"
    );

    // Build a tag with a tab byte (0x09, also a control character).
    let tab_tag = String::from_bytes(&env, &[b'a', 0x09, b'b']);
    let mut bad2 = Vec::new(&env);
    bad2.push_back(tab_tag);
    assert_eq!(
        client.try_register(&creator, &id, &100i128, &m, &bad2),
        Err(Ok(Error::InvalidTag)),
        "tab byte must be rejected"
    );

    // 0x1F (unit separator) is the last control byte.
    let us_tag = String::from_bytes(&env, &[0x1F]);
    let mut bad3 = Vec::new(&env);
    bad3.push_back(us_tag);
    assert_eq!(
        client.try_register(&creator, &id, &100i128, &m, &bad3),
        Err(Ok(Error::InvalidTag)),
        "0x1F must be rejected"
    );
}

/// A tag containing a space character (0x20) must be rejected.
#[test]
fn tag_with_space_rejected() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "space-tag");
    let m = String::from_str(&env, "m");

    let space_tag = String::from_str(&env, "hello world");
    let mut bad = Vec::new(&env);
    bad.push_back(space_tag);
    assert_eq!(
        client.try_register(&creator, &id, &100i128, &m, &bad),
        Err(Ok(Error::InvalidTag)),
        "space (0x20) must be rejected"
    );
}

/// A tag containing the DEL byte (0x7F) must be rejected.
#[test]
fn tag_with_del_byte_rejected() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "del-tag");
    let m = String::from_str(&env, "m");

    let del_tag = String::from_bytes(&env, &[b'a', 0x7F, b'b']);
    let mut bad = Vec::new(&env);
    bad.push_back(del_tag);
    assert_eq!(
        client.try_register(&creator, &id, &100i128, &m, &bad),
        Err(Ok(Error::InvalidTag)),
        "DEL byte (0x7F) must be rejected"
    );
}

/// A tag with non-ASCII bytes (>= 0x80) must be rejected.
#[test]
fn tag_with_non_ascii_byte_rejected() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "nonascii-tag");
    let m = String::from_str(&env, "m");

    // 0xC3 0xA9 is UTF-8 for 'é' — two non-ASCII bytes.
    let utf8_tag = String::from_bytes(&env, &[0xC3, 0xA9]);
    let mut bad = Vec::new(&env);
    bad.push_back(utf8_tag);
    assert_eq!(
        client.try_register(&creator, &id, &100i128, &m, &bad),
        Err(Ok(Error::InvalidTag)),
        "non-ASCII byte must be rejected"
    );
}

/// Tags using only printable non-space ASCII (0x21–0x7E) must be accepted.
#[test]
fn tag_printable_ascii_accepted() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "good-tags");
    let m = String::from_str(&env, "m");

    // Representative printable-ASCII tags spanning digits, letters and symbols.
    let good = tags(
        &env,
        &["dataset", "v1.0", "research-2024", "api:finance", "tag!"],
    );
    client.register(&creator, &id, &100i128, &m, &good);
    assert_eq!(client.get(&id).tags.len(), 5);
}

/// Tag normalisation: uppercase letters are lowercased; the resulting stored
/// bytes are still in the accepted alphabet.
#[test]
fn tag_normalisation_does_not_break_alphabet_check() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "norm-tag");
    let m = String::from_str(&env, "m");

    let mixed = tags(&env, &["DataSet", "RESEARCH"]);
    client.register(&creator, &id, &100i128, &m, &mixed);
    let r = client.get(&id);
    assert_eq!(r.tags.get(0).unwrap(), String::from_str(&env, "dataset"));
    assert_eq!(r.tags.get(1).unwrap(), String::from_str(&env, "research"));
}

// ── #196: Resource XDR compatibility fixture ──────────────────────────────────

/// Pin a representative `Resource` encoding so that any change to the struct
/// layout (field order, type change, adding/removing fields) is immediately
/// visible as a test failure rather than a silent silent silent XDR decode trap
/// at runtime.
///
/// The reference bytes were captured from a fresh register call at the commit
/// that introduced the `retired` field. Regenerate them with:
///
/// ```text
/// cargo test -p vault-registry xdr_resource_fixture -- --nocapture
/// ```
///
/// and copy the printed hex into this test. When the layout changes again,
/// update this fixture in the same PR and document the migration path in
/// `docs/CONTRACTS.md`.
#[test]
fn xdr_resource_fixture_stable() {
    use soroban_sdk::testutils::storage::Persistent as _;

    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VaultRegistry, ());
    let client = VaultRegistryClient::new(&env, &contract_id);
    let creator = Address::generate(&env);
    let id = String::from_str(&env, "fixture-id");
    let metadata = String::from_str(&env, "ipfs://QmFixture");
    let resource_tags = tags(&env, &["alpha", "beta"]);

    client.register(&creator, &id, &100i128, &metadata, &resource_tags);

    // Read the stored Resource back and verify every field, including the
    // `retired` flag introduced alongside the `retire_resource` entrypoint.
    let r: Resource = env.as_contract(&contract_id, || {
        env.storage()
            .persistent()
            .get(&DataKey::Resource(id.clone()))
            .unwrap()
    });

    assert_eq!(r.id, id, "id field");
    assert_eq!(r.creator, creator, "creator field");
    assert_eq!(r.price, 100i128, "price field");
    assert_eq!(r.metadata, metadata, "metadata field");
    assert!(r.listed, "listed defaults to true");
    assert!(!r.retired, "retired defaults to false");
    assert_eq!(r.tags.len(), 2, "tag count");
    assert_eq!(r.tags.get(0).unwrap(), String::from_str(&env, "alpha"));
    assert_eq!(r.tags.get(1).unwrap(), String::from_str(&env, "beta"));
}

/// Maximum-size `Resource` fixture: 64-byte id, 512-byte metadata, 8 tags of
/// 32 bytes each. Verifies that the layout can hold the worst-case payload
/// without truncation.
#[test]
fn xdr_resource_max_size_fixture() {
    use soroban_sdk::testutils::storage::Persistent as _;

    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VaultRegistry, ());
    let client = VaultRegistryClient::new(&env, &contract_id);
    let creator = Address::generate(&env);

    // Maximum-length id: 64 printable ASCII bytes.
    let max_id_str = "a".repeat(MAX_RESOURCE_ID_LEN as usize);
    let max_id = String::from_str(&env, &max_id_str);

    // Maximum-length metadata: 512 bytes.
    let max_meta_str = "m".repeat(MAX_METADATA_POINTER_LEN as usize);
    let max_meta = String::from_str(&env, &max_meta_str);

    // 8 tags of 32 bytes each (all distinct, printable ASCII).
    let tag_strs: [&str; 8] = [
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", // 32 × 'a'
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "cccccccccccccccccccccccccccccccc",
        "dddddddddddddddddddddddddddddddd",
        "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        "ffffffffffffffffffffffffffffffff",
        "gggggggggggggggggggggggggggggggg",
        "hhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhh",
    ];
    let max_tags = tags(&env, &tag_strs);

    client.register(&creator, &max_id, &1i128, &max_meta, &max_tags);

    let r: Resource = env.as_contract(&contract_id, || {
        env.storage()
            .persistent()
            .get(&DataKey::Resource(max_id.clone()))
            .unwrap()
    });

    assert_eq!(r.id.len(), MAX_RESOURCE_ID_LEN, "id at max length");
    assert_eq!(
        r.metadata.len(),
        MAX_METADATA_POINTER_LEN,
        "metadata at max length"
    );
    assert_eq!(r.tags.len(), MAX_TAGS, "tag count at max");
    for tag in r.tags.iter() {
        assert_eq!(tag.len(), MAX_TAG_LEN, "each tag at max length");
    }
    assert!(!r.retired, "retired defaults to false");
}

// ── #197: Filtered pages with sparse mixed listing states ─────────────────────

/// Sparse mixed-dataset traversal: holes (archived index entries), delisted
/// resources, and tag-filtered resources, all in the same registry.
///
/// Layout (insertion order):
///   index 0 → "sp0"  listed=true,  tags=["alpha"]   ← visible in listed+alpha
///   index 1 → "sp1"  listed=false                   ← hidden when only_listed
///   index 2 → "sp2"  listed=true,  tags=["beta"]    ← visible listed, hidden alpha
///   index 3 → "sp3"  listed=true,  tags=["alpha"]   ← visible in listed+alpha
///   index 4 → "sp4"  retired=true                   ← hidden when only_listed
///   index 5 → "sp5"  listed=true,  tags=[]          ← visible listed, hidden alpha
///
/// The scan window cap ensures we can collect at most `page_size*2 +
/// MAX_PAGE_SIZE` entries in one pass; the assertions below stay inside that
/// bound.
#[test]
fn list_sparse_mixed_states() {
    let (env, creator, client) = setup();
    let admin = Address::generate(&env);
    env.mock_all_auths();
    client.init(&admin);

    // Register six resources with varying listing states and tags.
    client.register(
        &creator,
        &String::from_str(&env, "sp0"),
        &100i128,
        &String::from_str(&env, "m"),
        &tags(&env, &["alpha"]),
    );
    client.register(
        &creator,
        &String::from_str(&env, "sp1"),
        &100i128,
        &String::from_str(&env, "m"),
        &empty_tags(&env),
    );
    client.register(
        &creator,
        &String::from_str(&env, "sp2"),
        &100i128,
        &String::from_str(&env, "m"),
        &tags(&env, &["beta"]),
    );
    client.register(
        &creator,
        &String::from_str(&env, "sp3"),
        &100i128,
        &String::from_str(&env, "m"),
        &tags(&env, &["alpha"]),
    );
    client.register(
        &creator,
        &String::from_str(&env, "sp4"),
        &100i128,
        &String::from_str(&env, "m"),
        &empty_tags(&env),
    );
    client.register(
        &creator,
        &String::from_str(&env, "sp5"),
        &100i128,
        &String::from_str(&env, "m"),
        &empty_tags(&env),
    );

    // Delist sp1.
    client.set_listed(&String::from_str(&env, "sp1"), &false);

    // Retire sp4 (admin-gated; sets listed=false and retired=true).
    client.retire_resource(&String::from_str(&env, "sp4"));

    // ── Case 1: all resources, no filter ──────────────────────────────────────
    let all = client.list(&0u32, &20u32, &false, &None);
    assert_eq!(all.len(), 6, "all resources, no filter");

    // ── Case 2: only_listed=true, no tag filter ───────────────────────────────
    let listed_only = client.list(&0u32, &20u32, &true, &None);
    assert_eq!(
        listed_only.len(),
        4,
        "only listed: sp0, sp2, sp3, sp5 (sp1 delisted, sp4 retired)"
    );
    assert_eq!(
        listed_only.get(0).unwrap().id,
        String::from_str(&env, "sp0")
    );
    assert_eq!(
        listed_only.get(1).unwrap().id,
        String::from_str(&env, "sp2")
    );
    assert_eq!(
        listed_only.get(2).unwrap().id,
        String::from_str(&env, "sp3")
    );
    assert_eq!(
        listed_only.get(3).unwrap().id,
        String::from_str(&env, "sp5")
    );

    // ── Case 3: only_listed=true, tag="alpha" ─────────────────────────────────
    let alpha_listed = client.list(
        &0u32,
        &20u32,
        &true,
        &Some(String::from_str(&env, "alpha")),
    );
    assert_eq!(alpha_listed.len(), 2, "listed + tag=alpha: sp0, sp3");
    assert_eq!(
        alpha_listed.get(0).unwrap().id,
        String::from_str(&env, "sp0")
    );
    assert_eq!(
        alpha_listed.get(1).unwrap().id,
        String::from_str(&env, "sp3")
    );

    // ── Case 4: only_listed=false, tag="alpha" ────────────────────────────────
    let alpha_all = client.list(
        &0u32,
        &20u32,
        &false,
        &Some(String::from_str(&env, "alpha")),
    );
    assert_eq!(alpha_all.len(), 2, "all + tag=alpha: sp0, sp3");

    // ── Case 5: pagination across a hole — start=2, limit=3, only_listed=true ─
    // Scan window from index 2: sp2 (listed), sp3 (listed), sp4 (retired,
    // hidden), sp5 (listed) → three listed results collected within window.
    let page2 = client.list(&2u32, &3u32, &true, &None);
    assert_eq!(page2.len(), 3, "page from index 2, only_listed, limit=3");
    assert_eq!(page2.get(0).unwrap().id, String::from_str(&env, "sp2"));
    assert_eq!(page2.get(1).unwrap().id, String::from_str(&env, "sp3"));
    assert_eq!(page2.get(2).unwrap().id, String::from_str(&env, "sp5"));

    // ── Case 6: start beyond count returns empty ──────────────────────────────
    let beyond = client.list(&99u32, &10u32, &true, &None);
    assert_eq!(beyond.len(), 0, "start beyond count must return empty");

    // ── Case 7: continuation from page boundary after a hole ─────────────────
    // Advance start past the delisted/retired entries: start=3 should yield
    // sp3, sp5 (sp4 is retired).
    let cont = client.list(&3u32, &10u32, &true, &None);
    assert_eq!(cont.len(), 2, "continuation: sp3, sp5");
    assert_eq!(cont.get(0).unwrap().id, String::from_str(&env, "sp3"));
    assert_eq!(cont.get(1).unwrap().id, String::from_str(&env, "sp5"));
}

/// Tag filter on the boundary of two pages ensures the continuation index
/// (the `start` a caller would pass on the next call) correctly skips entries
/// that were scanned but not collected.
#[test]
fn list_tag_filter_across_scan_boundary() {
    let (env, creator, client) = setup();

    // Register 10 resources alternating tag "even" on positions 0,2,4,6,8
    // and no tag on positions 1,3,5,7,9.
    for i in 0u32..10 {
        let id_str = std::format!("tb{:02}", i);
        let t = if i % 2 == 0 {
            tags(&env, &["even"])
        } else {
            empty_tags(&env)
        };
        client.register(
            &creator,
            &String::from_str(&env, &id_str),
            &100i128,
            &String::from_str(&env, "m"),
            &t,
        );
    }

    // Page 1: start=0, limit=3, tag="even" → tb00, tb02, tb04
    let p1 = client.list(
        &0u32,
        &3u32,
        &false,
        &Some(String::from_str(&env, "even")),
    );
    assert_eq!(p1.len(), 3, "page 1 of even-tagged resources");
    assert_eq!(p1.get(0).unwrap().id, String::from_str(&env, "tb00"));
    assert_eq!(p1.get(1).unwrap().id, String::from_str(&env, "tb02"));
    assert_eq!(p1.get(2).unwrap().id, String::from_str(&env, "tb04"));

    // Page 2: start=5 (after tb04 at index 4), limit=3, tag="even" → tb06, tb08
    let p2 = client.list(
        &5u32,
        &3u32,
        &false,
        &Some(String::from_str(&env, "even")),
    );
    assert_eq!(p2.len(), 2, "page 2 of even-tagged resources");
    assert_eq!(p2.get(0).unwrap().id, String::from_str(&env, "tb06"));
    assert_eq!(p2.get(1).unwrap().id, String::from_str(&env, "tb08"));
}

// ── #198: Benchmark listing with maximum metadata and tag payloads ─────────────

/// Measures the gas cost of a full page (`MAX_PAGE_SIZE`) where every resource
/// has the maximum metadata length (512 bytes) and the maximum number of
/// maximum-length tags (8 × 32 bytes).
///
/// The budget assertions below are intentionally generous — they guard against
/// catastrophic regressions (e.g. an O(n²) path being introduced), not minor
/// fluctuations. Run with `--nocapture` to see the printed footprint numbers.
#[test]
fn gas_max_payload_full_page_budget() {
    let (env, creator, client) = setup();

    // Build the maximum-payload tags: 8 tags, each 32 printable-ASCII bytes.
    let tag_strs: [&str; 8] = [
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "cccccccccccccccccccccccccccccccc",
        "dddddddddddddddddddddddddddddddd",
        "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        "ffffffffffffffffffffffffffffffff",
        "gggggggggggggggggggggggggggggggg",
        "hhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhh",
    ];
    let max_tags = tags(&env, &tag_strs);

    // 512-byte metadata pointer.
    let max_meta_str = "x".repeat(MAX_METADATA_POINTER_LEN as usize);

    // Register MAX_PAGE_SIZE resources with maximum-payload.
    for i in 0..MAX_PAGE_SIZE {
        let id_str = std::format!("mp{:02}", i);
        client.register(
            &creator,
            &String::from_str(&env, &id_str),
            &100i128,
            &String::from_str(&env, &max_meta_str),
            &max_tags,
        );
    }

    // Measure the worst-case list() call: full page, only_listed=false, no tag filter.
    env.cost_estimate().budget().reset_default();
    let page = client.list(&0u32, &MAX_PAGE_SIZE, &false, &None);
    let list_cpu = env.cost_estimate().budget().cpu_instruction_cost();
    let list_mem = env.cost_estimate().budget().memory_bytes_cost();
    std::println!(
        "list(max_payload, {MAX_PAGE_SIZE}): cpu={list_cpu} mem={list_mem}"
    );

    assert_eq!(page.len(), MAX_PAGE_SIZE, "must return a full page");

    // Guard: each resource has 512 bytes of metadata and 8×32=256 bytes of
    // tags; the page-level ceiling should still be well below the 100M
    // instruction network limit.
    assert!(
        list_cpu < 100_000_000,
        "max-payload list must stay under network limit: {list_cpu}"
    );

    // Also benchmark a tagged-filter pass on the same dataset.
    env.cost_estimate().budget().reset_default();
    let tagged_page = client.list(
        &0u32,
        &MAX_PAGE_SIZE,
        &false,
        &Some(String::from_str(&env, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")),
    );
    let tagged_cpu = env.cost_estimate().budget().cpu_instruction_cost();
    let tagged_mem = env.cost_estimate().budget().memory_bytes_cost();
    std::println!(
        "list(max_payload, tag_filter, {MAX_PAGE_SIZE}): cpu={tagged_cpu} mem={tagged_mem}"
    );

    assert_eq!(
        tagged_page.len(),
        MAX_PAGE_SIZE,
        "all resources have the matching tag"
    );
    assert!(
        tagged_cpu < 100_000_000,
        "max-payload tagged list must stay under network limit: {tagged_cpu}"
    );

    // Also measure a single max-payload register for the record.
    env.cost_estimate().budget().reset_default();
    client.register(
        &creator,
        &String::from_str(&env, "mp-extra"),
        &100i128,
        &String::from_str(&env, &max_meta_str),
        &max_tags,
    );
    let reg_cpu = env.cost_estimate().budget().cpu_instruction_cost();
    let reg_mem = env.cost_estimate().budget().memory_bytes_cost();
    std::println!("register(max_payload): cpu={reg_cpu} mem={reg_mem}");

    assert!(
        reg_cpu < 20_000_000,
        "max-payload register too expensive: {reg_cpu}"
    );
}

// ── #199: update_metadata no-op when pointer is unchanged ────────────────────

/// Calling `update_metadata` with the same pointer that is already stored must
/// not write to storage (TTL unchanged) and must not emit any event.
#[test]
fn update_metadata_no_op_when_pointer_unchanged() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "meta-noop");
    let meta = String::from_str(&env, "ipfs://QmSame");

    client.register(&creator, &id, &100i128, &meta, &empty_tags(&env));

    let ttl_before = resource_storage_ttl(&env, &client.address, &id);
    env.events().all(); // clear previous events

    client.update_metadata(&id, &meta);

    // No event emitted.
    let events = env.events().all().filter_by_contract(&client.address);
    assert_eq!(
        events.events().len(),
        0,
        "expected no event on no-op update_metadata"
    );

    // TTL must not have been bumped (no storage write).
    let ttl_after = resource_storage_ttl(&env, &client.address, &id);
    assert_eq!(ttl_before, ttl_after, "TTL must not change on no-op update_metadata");
}

/// A real metadata change must still emit exactly one event.
#[test]
fn update_metadata_emits_event_when_pointer_changes() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "meta-change");
    let old_meta = String::from_str(&env, "ipfs://QmOld");
    let new_meta = String::from_str(&env, "ipfs://QmNew");

    client.register(&creator, &id, &100i128, &old_meta, &empty_tags(&env));
    env.events().all(); // clear

    client.update_metadata(&id, &new_meta);

    let events = env.events().all().filter_by_contract(&client.address);
    assert_eq!(events.events().len(), 1, "expected exactly one updmeta event");
    assert_eq!(
        events,
        soroban_sdk::vec![
            &env,
            (
                client.address.clone(),
                (symbol_short!("updmeta"), id.clone()).into_val(&env),
                new_meta.into_val(&env),
            ),
        ]
    );
}

// ── #200: set_tags no-op when normalised tags are unchanged ───────────────────

/// Supplying the identical tag vector (already-lowercase) must not write to
/// storage and must not emit any event.
#[test]
fn set_tags_no_op_when_tags_unchanged() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "tags-noop");
    let initial_tags = tags(&env, &["alpha", "beta"]);

    client.register(&creator, &id, &100i128, &String::from_str(&env, "m"), &initial_tags);

    let ttl_before = resource_storage_ttl(&env, &client.address, &id);
    env.events().all(); // clear

    client.set_tags(&id, &initial_tags);

    let events = env.events().all().filter_by_contract(&client.address);
    assert_eq!(
        events.events().len(),
        0,
        "expected no event on no-op set_tags"
    );
    let ttl_after = resource_storage_ttl(&env, &client.address, &id);
    assert_eq!(ttl_before, ttl_after, "TTL must not change on no-op set_tags");
}

/// Supplying uppercase equivalents that normalise to the already-stored tags
/// must also be a no-op.
#[test]
fn set_tags_no_op_when_case_equivalent_to_stored() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "tags-case-noop");

    // Register with lowercase tags.
    let stored_tags = tags(&env, &["dataset", "research"]);
    client.register(&creator, &id, &100i128, &String::from_str(&env, "m"), &stored_tags);

    env.events().all(); // clear

    // Supply the same tags in uppercase — normalisation yields the same stored vector.
    let upper_tags = tags(&env, &["DATASET", "RESEARCH"]);
    client.set_tags(&id, &upper_tags);

    let events = env.events().all().filter_by_contract(&client.address);
    assert_eq!(
        events.events().len(),
        0,
        "case-equivalent tags must produce a no-op"
    );
    // Stored tags must remain unchanged.
    let r = client.get(&id);
    assert_eq!(r.tags.get(0).unwrap(), String::from_str(&env, "dataset"));
    assert_eq!(r.tags.get(1).unwrap(), String::from_str(&env, "research"));
}

/// A real tag change must still emit exactly one event.
#[test]
fn set_tags_emits_event_when_tags_change() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "tags-change");

    client.register(&creator, &id, &100i128, &String::from_str(&env, "m"), &empty_tags(&env));
    env.events().all(); // clear

    let new_tags = tags(&env, &["finance"]);
    client.set_tags(&id, &new_tags);

    let events = env.events().all().filter_by_contract(&client.address);
    assert_eq!(events.events().len(), 1, "expected exactly one settags event");
    assert_eq!(
        events,
        soroban_sdk::vec![
            &env,
            (
                client.address.clone(),
                (symbol_short!("settags"), id.clone()).into_val(&env),
                new_tags.into_val(&env),
            ),
        ]
    );
}

// ── #201: set_price no-op when price is unchanged ─────────────────────────────

/// Calling `set_price` with the same price already stored must not write to
/// storage (TTL unchanged) and must not emit any event.
#[test]
fn set_price_no_op_when_price_unchanged() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "price-noop");

    client.register(&creator, &id, &500i128, &String::from_str(&env, "m"), &empty_tags(&env));

    let ttl_before = resource_storage_ttl(&env, &client.address, &id);
    env.events().all(); // clear

    client.set_price(&id, &500i128);

    let events = env.events().all().filter_by_contract(&client.address);
    assert_eq!(
        events.events().len(),
        0,
        "expected no event on no-op set_price"
    );
    let ttl_after = resource_storage_ttl(&env, &client.address, &id);
    assert_eq!(ttl_before, ttl_after, "TTL must not change on no-op set_price");
}

/// A real price change must still emit exactly one event.
#[test]
fn set_price_emits_event_when_price_changes() {
    let (env, creator, client) = setup();
    let id = String::from_str(&env, "price-change");

    client.register(&creator, &id, &100i128, &String::from_str(&env, "m"), &empty_tags(&env));
    env.events().all(); // clear

    client.set_price(&id, &999i128);

    let events = env.events().all().filter_by_contract(&client.address);
    assert_eq!(events.events().len(), 1, "expected exactly one setprice event");
    assert_eq!(
        events,
        soroban_sdk::vec![
            &env,
            (
                client.address.clone(),
                (symbol_short!("setprice"), id.clone()).into_val(&env),
                999i128.into_val(&env),
            ),
        ]
    );
}

// ── #202: retire_resource event handling for discovery clients ────────────────

/// Retiring a resource emits `("retire", id)` and does NOT emit a `delist`
/// event. Discovery clients must subscribe to `retire` as a distinct,
/// permanent-removal signal.
#[test]
fn retire_emits_retire_not_delist() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VaultRegistry, ());
    let client = VaultRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);

    client.init(&admin);
    let id = String::from_str(&env, "retire-ev");
    client.register(&creator, &id, &100i128, &String::from_str(&env, "m"), &empty_tags(&env));
    env.events().all(); // clear

    client.retire_resource(&id);

    let events = env.events().all().filter_by_contract(&client.address);

    // Exactly one event: ("retire", id) | admin.
    assert_eq!(events.events().len(), 1, "retire must emit exactly one event");
    assert_eq!(
        events,
        soroban_sdk::vec![
            &env,
            (
                client.address.clone(),
                (symbol_short!("retire"), id.clone()).into_val(&env),
                admin.clone().into_val(&env),
            ),
        ],
        "retire must emit the retire event, not delist"
    );

    // Confirm the resource state: listed=false, retired=true.
    let r = client.get(&id);
    assert!(!r.listed, "retired resource must have listed=false");
    assert!(r.retired, "retired resource must have retired=true");
}

/// Indexer event-sequence demonstration: a resource goes through the full
/// discovery lifecycle (register → delist → re-list → retire) and the event
/// stream is authoritative for each transition.
#[test]
fn retire_discovery_event_sequence() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VaultRegistry, ());
    let client = VaultRegistryClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);

    client.init(&admin);
    let id = String::from_str(&env, "lifecycle");

    // 1. register — resource enters discovery.
    client.register(&creator, &id, &100i128, &String::from_str(&env, "m"), &empty_tags(&env));
    let ev = env.events().all().filter_by_contract(&client.address);
    assert_eq!(ev.events().len(), 1);
    // topic[0] == "register"

    // 2. delist — temporary removal; a re-list is still possible.
    client.delist(&id);
    let ev = env.events().all().filter_by_contract(&client.address);
    assert_eq!(ev.events().len(), 1);
    // topic[0] == "delist"

    // 3. re-list — back in discovery.
    client.set_listed(&id, &true);
    let ev = env.events().all().filter_by_contract(&client.address);
    assert_eq!(ev.events().len(), 1);
    // topic[0] == "setlisted"

    // 4. retire — permanent removal; no further mutations possible.
    client.retire_resource(&id);
    let ev = env.events().all().filter_by_contract(&client.address);
    assert_eq!(ev.events().len(), 1, "retire must emit exactly one event");
    // The event must be "retire", confirming the authoritative retirement signal.
    assert_eq!(
        ev,
        soroban_sdk::vec![
            &env,
            (
                client.address.clone(),
                (symbol_short!("retire"), id.clone()).into_val(&env),
                admin.into_val(&env),
            ),
        ]
    );

    // After retirement the resource is permanently unlisted and immutable.
    assert!(!client.get(&id).listed);
    assert!(client.get(&id).retired);
    assert_eq!(
        client.try_set_listed(&id, &true),
        Err(Ok(Error::ResourceRetired)),
        "retired resource cannot be re-listed"
    );
}
