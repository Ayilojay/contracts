#![no_std]
//! SynapsVault — Subscription Manager (Soroban / Stellar)
//!
//! Manages recurring 30-day subscription plans for SynapsVault publishers.
//! Publishers create plans; the backend subscribes buyers after payment confirmation
//! and renews each billing cycle. Subscribers can self-cancel with access until period end.

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, Address, BytesN,
    ContractExecutable, Env, String,
};

/// Ledgers per day (~5 s per ledger).
const DAY: u32 = 17_280;

/// Length of one billing cycle in ledgers (30 days).
pub const CYCLE: u32 = 30 * DAY;

/// Maximum length of a plan identifier in bytes.
pub const MAX_PLAN_ID_LEN: u32 = 64;

/// Shared persistent-storage TTL policy (see `docs/CONTRACTS.md`).
const BUMP: u32 = 365 * DAY; // 1-year TTL
const BUMP_THRESH: u32 = BUMP - DAY;

/// Contract version, sourced from Cargo.toml at compile time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A recurring subscription plan offered by a publisher.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub plan_id: String,
    pub publisher: Address,
    pub price_per_cycle: i128,
    pub active: bool,
}

/// A subscriber's enrolment in a [`Plan`].
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Subscription {
    pub plan_id: String,
    pub subscriber: Address,
    pub started_at: u32,
    pub current_period_end: u32,
    pub cancelled: bool,
    pub total_renewals: u32,
}

#[contracttype]
pub enum DataKey {
    Admin,
    Plan(String),
    Sub(String, Address),
    Version,
}

/// Subscription-manager error codes.
///
/// Numbering is **per-contract** and intentionally not aligned with the
/// `vault-registry` or `access-lease` enums: the same numeric value can mean
/// different things in different contracts (e.g. `AlreadyInitialised` is 9
/// here, 8 in the registry and 7 in the lease). The cross-contract mapping is
/// documented in `docs/CONTRACTS.md`; the discriminants are pinned by tests so
/// any renumbering is deliberate.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Error {
    // 1 was NotAdmin, removed as it is never returned (admin auth traps on failure)
    PlanNotFound = 2,
    PlanInactive = 3,
    AlreadySubbed = 4,
    SubNotFound = 5,
    Cancelled = 6,
    InvalidPrice = 7,
    NotInitialised = 8,
    AlreadyInitialised = 9,
    PlanExists = 10,
    Overflow = 11,
    /// `plan_id` is empty or exceeds the maximum allowed length.
    InvalidPlanId = 12,
    /// The new admin address is the same as the current admin address.
    SameAdmin = 13,
}

#[contractevent(topics = ["init"], data_format = "single-value")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitEvent {
    pub admin: Address,
}

#[contractevent(topics = ["setadmin"], data_format = "single-value")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetAdminEvent {
    pub new_admin: Address,
}

#[contractevent(topics = ["plan"], data_format = "single-value")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanCreatedEvent {
    #[topic]
    pub plan_id: String,
    #[topic]
    pub publisher: Address,
    pub price_per_cycle: i128,
}

#[contractevent(topics = ["setactive"], data_format = "single-value")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanActiveEvent {
    #[topic]
    pub plan_id: String,
    pub active: bool,
}

#[contractevent(topics = ["subscribe"], data_format = "vec")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscribedEvent {
    #[topic]
    pub plan_id: String,
    #[topic]
    pub subscriber: Address,
    pub current_period_end: u32,
    pub price: i128,
    pub payment_reference: String,
}

#[contractevent(topics = ["renew"], data_format = "vec")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenewedEvent {
    #[topic]
    pub plan_id: String,
    #[topic]
    pub subscriber: Address,
    pub current_period_end: u32,
    pub price: i128,
    pub payment_reference: String,
}

#[contractevent(topics = ["cancel"], data_format = "single-value")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CancelledEvent {
    #[topic]
    pub plan_id: String,
    #[topic]
    pub subscriber: Address,
}

#[contractevent(topics = ["upgrade"], data_format = "single-value")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpgradeEvent {
    pub new_wasm_hash: BytesN<32>,
}

#[contract]
pub struct SubscriptionManager;

#[contractimpl]
impl SubscriptionManager {
    pub fn init(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialised);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::Version, &String::from_str(&env, VERSION));
        env.storage().instance().extend_ttl(BUMP_THRESH, BUMP);
        InitEvent { admin }.publish(&env);
        Ok(())
    }

    pub fn admin(env: Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialised)
    }

    pub fn set_admin(env: Env, new_admin: Address) -> Result<(), Error> {
        Self::require_admin(&env)?;
        let current_admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        if current_admin == new_admin {
            return Err(Error::SameAdmin);
        }
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        SetAdminEvent { new_admin }.publish(&env);
        Ok(())
    }

    pub fn get_version(env: Env) -> String {
        env.storage()
            .instance()
            .get(&DataKey::Version)
            .unwrap_or_else(|| String::from_str(&env, VERSION))
    }

    pub fn is_compatible(env: Env) -> bool {
        match env.storage().instance().get::<_, String>(&DataKey::Version) {
            Some(stored) => stored == String::from_str(&env, VERSION),
            None => false,
        }
    }

    pub fn create_plan(
        env: Env,
        publisher: Address,
        plan_id: String,
        price_per_cycle: i128,
    ) -> Result<Plan, Error> {
        publisher.require_auth();
        if plan_id.is_empty() || plan_id.len() > MAX_PLAN_ID_LEN {
            return Err(Error::InvalidPlanId);
        }
        if price_per_cycle <= 0 {
            return Err(Error::InvalidPrice);
        }
        let key = DataKey::Plan(plan_id.clone());
        if env.storage().persistent().has(&key) {
            return Err(Error::PlanExists);
        }
        let plan = Plan {
            plan_id: plan_id.clone(),
            publisher: publisher.clone(),
            price_per_cycle,
            active: true,
        };
        env.storage().persistent().set(&key, &plan);
        env.storage()
            .persistent()
            .extend_ttl(&key, BUMP_THRESH, BUMP);
        PlanCreatedEvent {
            plan_id,
            publisher,
            price_per_cycle,
        }
        .publish(&env);
        Ok(plan)
    }

    pub fn get_plan(env: Env, plan_id: String) -> Result<Plan, Error> {
        let key = DataKey::Plan(plan_id);
        let plan: Plan = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::PlanNotFound)?;
        env.storage()
            .persistent()
            .extend_ttl(&key, BUMP_THRESH, BUMP);
        Ok(plan)
    }

    pub fn set_plan_active(env: Env, plan_id: String, active: bool) -> Result<(), Error> {
        let key = DataKey::Plan(plan_id.clone());
        let mut plan: Plan = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::PlanNotFound)?;
        env.storage()
            .persistent()
            .extend_ttl(&key, BUMP_THRESH, BUMP);

        plan.publisher.require_auth();
        plan.active = active;
        env.storage().persistent().set(&key, &plan);
        env.storage()
            .persistent()
            .extend_ttl(&key, BUMP_THRESH, BUMP);
        PlanActiveEvent { plan_id, active }.publish(&env);
        Ok(())
    }

    pub fn subscribe(
        env: Env,
        subscriber: Address,
        plan_id: String,
        payment_reference: String,
    ) -> Result<Subscription, Error> {
        subscriber.require_auth();
        let plan = Self::get_plan(env.clone(), plan_id.clone())?;
        if !plan.active {
            return Err(Error::PlanInactive);
        }
        let sub_key = DataKey::Sub(plan_id.clone(), subscriber.clone());
        if env.storage().persistent().has(&sub_key) {
            return Err(Error::AlreadySubbed);
        }
        let now = env.ledger().sequence();
        let sub = Subscription {
            plan_id: plan_id.clone(),
            subscriber: subscriber.clone(),
            started_at: now,
            current_period_end: now.saturating_add(CYCLE),
            cancelled: false,
            total_renewals: 0,
        };
        env.storage().persistent().set(&sub_key, &sub);
        env.storage()
            .persistent()
            .extend_ttl(&sub_key, BUMP_THRESH, BUMP);
        SubscribedEvent {
            plan_id,
            subscriber,
            current_period_end: sub.current_period_end,
            price: plan.price_per_cycle,
            payment_reference,
        }
        .publish(&env);
        Ok(sub)
    }

    pub fn get_subscription(
        env: Env,
        plan_id: String,
        subscriber: Address,
    ) -> Result<Subscription, Error> {
        let key = DataKey::Sub(plan_id, subscriber);
        let sub: Subscription = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::SubNotFound)?;
        env.storage()
            .persistent()
            .extend_ttl(&key, BUMP_THRESH, BUMP);
        Ok(sub)
    }

    pub fn renew(
        env: Env,
        subscriber: Address,
        plan_id: String,
        payment_reference: String,
    ) -> Result<Subscription, Error> {
        subscriber.require_auth();
        let plan = Self::get_plan(env.clone(), plan_id.clone())?;
        if !plan.active {
            return Err(Error::PlanInactive);
        }
        let key = DataKey::Sub(plan_id.clone(), subscriber.clone());
        let mut sub: Subscription = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::SubNotFound)?;
        if sub.cancelled {
            return Err(Error::Cancelled);
        }
        let now = env.ledger().sequence();
        let base = if sub.current_period_end > now {
            sub.current_period_end
        } else {
            now
        };
        sub.current_period_end = base.saturating_add(CYCLE);
        sub.total_renewals = sub.total_renewals.saturating_add(1);
        env.storage().persistent().set(&key, &sub);
        env.storage()
            .persistent()
            .extend_ttl(&key, BUMP_THRESH, BUMP);
        RenewedEvent {
            plan_id,
            subscriber,
            current_period_end: sub.current_period_end,
            price: plan.price_per_cycle,
            payment_reference,
        }
        .publish(&env);
        Ok(sub)
    }

    pub fn cancel(env: Env, subscriber: Address, plan_id: String) -> Result<(), Error> {
        subscriber.require_auth();
        let key = DataKey::Sub(plan_id.clone(), subscriber.clone());
        let mut sub: Subscription = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::SubNotFound)?;
        if sub.cancelled {
            return Err(Error::Cancelled);
        }
        sub.cancelled = true;
        env.storage().persistent().set(&key, &sub);
        env.storage()
            .persistent()
            .extend_ttl(&key, BUMP_THRESH, BUMP);
        CancelledEvent {
            plan_id,
            subscriber,
        }
        .publish(&env);
        Ok(())
    }

    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) -> Result<(), Error> {
        Self::require_admin(&env)?;
        env.deployer()
            .update_current_contract_wasm(new_wasm_hash.clone());
        UpgradeEvent { new_wasm_hash }.publish(&env);
        Ok(())
    }

    fn require_admin(env: &Env) -> Result<(), Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialised)?;
        admin.require_auth();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger as _},
        Env,
    };

    fn setup() -> (Env, SubscriptionManagerClient<'static>) {
        let env = Env::default();
        let id = env.register(SubscriptionManager, ());
        let client = SubscriptionManagerClient::new(&env, &id);
        (env, client)
    }

    fn plan_id(env: &Env) -> String {
        String::from_str(env, "plan-basic")
    }

    #[test]
    fn create_plan_stores_publisher_identity() {
        let (env, client) = setup();
        let publisher = Address::generate(&env);
        let id = plan_id(&env);

        // Scoped auth: only the publisher is authorised for this call.
        env.mock_auths(&[soroban_sdk::testutils::MockAuth {
            address: &publisher,
            invoke: &soroban_sdk::testutils::MockAuthInvoke {
                contract: &client.address,
                fn_name: "create_plan",
                args: (publisher.clone(), id.clone(), 100i128).into_val(&env),
                sub_invokes: &[],
            },
        }]);

        let plan = client.create_plan(&publisher, &id, &100i128);
        assert_eq!(plan.publisher, publisher);
        assert!(plan.active);

        let stored = client.get_plan(&id);
        assert_eq!(stored.publisher, publisher);
        assert_eq!(stored.price_per_cycle, 100);
    }

    #[test]
    fn create_plan_rejects_unauthorized_caller() {
        let (env, client) = setup();
        let publisher = Address::generate(&env);
        let attacker = Address::generate(&env);
        let id = plan_id(&env);

        // Scoped auth: only the attacker is authorised, not the publisher.
        env.mock_auths(&[soroban_sdk::testutils::MockAuth {
            address: &attacker,
            invoke: &soroban_sdk::testutils::MockAuthInvoke {
                contract: &client.address,
                fn_name: "create_plan",
                args: (publisher.clone(), id.clone(), 100i128).into_val(&env),
                sub_invokes: &[],
            },
        }]);

        let result = client.try_create_plan(&publisher, &id, &100i128);
        assert!(result.is_err());

        // Rollback: no plan was persisted.
        assert!(client.try_get_plan(&id).is_err());
    }

    #[test]
    fn set_plan_active_authorized_publisher_succeeds() {
        let (env, client) = setup();
        let publisher = Address::generate(&env);
        let id = plan_id(&env);

        env.mock_auths(&[soroban_sdk::testutils::MockAuth {
            address: &publisher,
            invoke: &soroban_sdk::testutils::MockAuthInvoke {
                contract: &client.address,
                fn_name: "create_plan",
                args: (publisher.clone(), id.clone(), 100i128).into_val(&env),
                sub_invokes: &[],
            },
        }]);
        client.create_plan(&publisher, &id, &100i128);

        env.mock_auths(&[soroban_sdk::testutils::MockAuth {
            address: &publisher,
            invoke: &soroban_sdk::testutils::MockAuthInvoke {
                contract: &client.address,
                fn_name: "set_plan_active",
                args: (id.clone(), false).into_val(&env),
                sub_invokes: &[],
            },
        }]);
        client.set_plan_active(&id, &false);

        let stored = client.get_plan(&id);
        assert_eq!(stored.publisher, publisher);
        assert!(!stored.active);
    }

    #[test]
    fn set_plan_active_rejects_unauthorized_caller() {
        let (env, client) = setup();
        let publisher = Address::generate(&env);
        let attacker = Address::generate(&env);
        let id = plan_id(&env);

        env.mock_auths(&[soroban_sdk::testutils::MockAuth {
            address: &publisher,
            invoke: &soroban_sdk::testutils::MockAuthInvoke {
                contract: &client.address,
                fn_name: "create_plan",
                args: (publisher.clone(), id.clone(), 100i128).into_val(&env),
                sub_invokes: &[],
            },
        }]);
        client.create_plan(&publisher, &id, &100i128);

        // Scoped auth: only the attacker is authorised for activation.
        env.mock_auths(&[soroban_sdk::testutils::MockAuth {
            address: &attacker,
            invoke: &soroban_sdk::testutils::MockAuthInvoke {
                contract: &client.address,
                fn_name: "set_plan_active",
                args: (id.clone(), false).into_val(&env),
                sub_invokes: &[],
            },
        }]);

        let result = client.try_set_plan_active(&id, &false);
        assert!(result.is_err());

        // Rollback: plan remains active and publisher unchanged.
        let stored = client.get_plan(&id);
        assert!(stored.active);
        assert_eq!(stored.publisher, publisher);
    }
}
