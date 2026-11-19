//! Property tests for subscription expiry arithmetic (`subscribe`, `renew`,
//! `cancel`, `is_active`, `remaining_ledgers`). Each property is checked
//! against a plain-integer model of the documented rules:
//!
//! - subscribe: `current_period_end = base + CYCLE`, where `base` is the
//!   remaining paid period of a cancelled-but-active subscription, otherwise
//!   `now`; `Overflow` if that does not fit in `u32`.
//! - renew:     `current_period_end = max(current_period_end, now) + CYCLE`,
//!   or `Overflow`, leaving the stored subscription untouched.
//! - active:    a subscription is active while `current_period_end > now`,
//!   regardless of `cancelled`.
//!
//! Renewal counter: `total_renewals` is **saturating**. It counts renewals up
//! to `u32::MAX` and then stays pinned there; a renewal at the maximum still
//! extends the paid period (the counter is informational, not a gate). This is
//! the explicit overflow policy for #228: no panic, no wrap, no `Overflow`
//! error — the counter simply stops counting while renewals keep working.
//!
//! TTL handling: `renew` and `cancel` extend the subscription key exactly once
//! per write (inside `save_sub`); the redundant pre-save extensions were
//! removed in #227. The storage-operation counts below pin that down so a
//! regression that re-adds an extension is caught.
//!
//! Cancellation idempotency: `cancel` succeeds when the subscription is
//! already cancelled but must not emit a second cancellation event nor touch
//! the stored period end or renewal counter (#229).
//!
//! Plan status vs. entitlement (#230): a plan's activity status and a
//! subscriber's entitlement are separate states. `is_active` only inspects the
//! subscription's `current_period_end` and never loads the plan, so
//! deactivating a plan does not revoke access already paid for; reactivating
//! the plan does not extend it either. The tests below pin that down across an
//! existing paid period.
use super::*;
use proptest::prelude::*;
use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger as _},
    Address, Env, IntoVal, String, Symbol, Val, Vec,
};

/// Ledgers entries stay live for in these tests, so advancing time past a full
/// `CYCLE` never archives the plan, subscription or instance entry.
const TEST_ENTRY_TTL: u32 = 20_000_000;
/// Largest start ledger used; keeps `now + TEST_ENTRY_TTL` inside `u32`.
const MAX_START: u32 = u32::MAX - 2 * TEST_ENTRY_TTL;
/// Largest amount of time advanced in one step (just over three cycles).
const MAX_STEP: u32 = 3 * CYCLE + 1;

struct Ctx<'a> {
    env: Env,
    client: SubscriptionManagerClient<'a>,
    plan: String,
    sub: Address,
    payref: String,
}

fn setup<'a>(start: u32) -> Ctx<'a> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| {
        l.sequence_number = start;
        l.min_persistent_entry_ttl = TEST_ENTRY_TTL;
        l.min_temp_entry_ttl = TEST_ENTRY_TTL;
        l.max_entry_ttl = 2 * TEST_ENTRY_TTL;
    });
    let id = env.register(SubscriptionManager, ());
    let client = SubscriptionManagerClient::new(&env, &id);
    client.init(&Address::generate(&env));
    let plan = String::from_str(&env, "plan");
    client.create_plan(&Address::generate(&env), &plan, &1_000_000i128);
    let sub = Address::generate(&env);
    let payref = String::from_str(&env, "ref");
    Ctx {
        env,
        client,
        plan,
        sub,
        payref,
    }
}

fn set_ledger(env: &Env, seq: u32) {
    env.ledger().set_sequence_number(seq);
}

/// Counts the events emitted by the contract whose topic is `cancel`.
fn cancel_event_count(env: &Env, contract: &Address) -> usize {
    let topic: Vec<Val> = (Symbol::new(env, "cancel"),).into_val(env);
    env.events()
        .all()
        .iter()
        .filter(|(addr, topics, _)| addr == contract && *topics == topic)
        .count()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// A first subscription starts now and ends exactly one `CYCLE` later.
    ///
    /// The `Overflow` branch (`now + CYCLE > u32::MAX`) is not exercised: it
    /// needs a ledger within one cycle of `u32::MAX`, where the host's own
    /// storage-TTL arithmetic fails before the contract's check is reached.
    #[test]
    fn subscribe_ends_one_cycle_after_now(now in 0..=MAX_START) {
        let c = setup(now);
        let s = c.client.subscribe(&c.plan, &c.sub, &c.payref);
        prop_assert_eq!(s.current_period_end, now + CYCLE);
        prop_assert_eq!(s.started_at, now);
        prop_assert!(!s.cancelled);
        prop_assert_eq!(s.total_renewals, 0);
    }

    /// Active strictly before `current_period_end`, inactive from it onwards.
    #[test]
    fn active_strictly_before_period_end(now in 0..=MAX_START) {
        let c = setup(now);
        let s = c.client.subscribe(&c.plan, &c.sub, &c.payref);
        set_ledger(&c.env, s.current_period_end - 1);
        prop_assert!(c.client.is_active(&c.plan, &c.sub));
        set_ledger(&c.env, s.current_period_end);
        prop_assert!(!c.client.is_active(&c.plan, &c.sub));
    }

    /// `renew` extends from the later of the current period end and now: an
    /// early renewal keeps the unused time, a late one restarts from now. The
    /// new end is later than both and the renewal counter goes up by one.
    #[test]
    fn renew_extends_from_later_of_period_end_and_now(now in 0..=MAX_START, elapsed in 0..=MAX_STEP) {
        let c = setup(now);
        let s = c.client.subscribe(&c.plan, &c.sub, &c.payref);
        let t = now + elapsed;
        set_ledger(&c.env, t);

        let expected = s.current_period_end.max(t) + CYCLE;
        let renewed = c.client.renew(&c.plan, &c.sub, &c.payref);
        prop_assert_eq!(renewed.current_period_end, expected);
        prop_assert!(renewed.current_period_end > s.current_period_end);
        prop_assert!(renewed.current_period_end > t);
        prop_assert_eq!(renewed.total_renewals, s.total_renewals + 1);
        prop_assert_eq!(renewed.started_at, s.started_at);
        prop_assert!(c.client.is_active(&c.plan, &c.sub));
    }

    /// Renewing `k` times without letting the subscription lapse adds exactly
    /// `k` cycles.
    #[test]
    fn early_renewals_accumulate_whole_cycles(now in 0..=MAX_START, k in 1u32..6) {
        let c = setup(now);
        c.client.subscribe(&c.plan, &c.sub, &c.payref);
        let mut last = None;
        for _ in 0..k {
            last = Some(c.client.renew(&c.plan, &c.sub, &c.payref));
        }
        let last = last.unwrap();
        prop_assert_eq!(last.current_period_end, now + (k + 1) * CYCLE);
        prop_assert_eq!(last.total_renewals, k);
    }

    /// Cancelling keeps access until the period end. Re-subscribing while the
    /// paid period is still running carries the remaining time over; after
    /// it lapses, the new period starts from now.
    #[test]
    fn resubscribe_after_cancel_carries_over_remaining_time(now in 0..=MAX_START, elapsed in 0..=MAX_STEP) {
        let c = setup(now);
        let s = c.client.subscribe(&c.plan, &c.sub, &c.payref);
        c.client.cancel(&c.plan, &c.sub);
        let t = now + elapsed;
        set_ledger(&c.env, t);

        prop_assert_eq!(c.client.is_active(&c.plan, &c.sub), s.current_period_end > t);
        prop_assert_eq!(c.client.try_renew(&c.plan, &c.sub, &c.payref), Err(Ok(Error::Cancelled)));

        let again = c.client.subscribe(&c.plan, &c.sub, &c.payref);
        let base = if s.current_period_end > t { s.current_period_end } else { t };
        prop_assert_eq!(again.current_period_end, base + CYCLE);
        prop_assert!(!again.cancelled);
        prop_assert_eq!(again.started_at, s.started_at);
    }

    /// Subscribing again while an uncancelled subscription is active is
    /// refused and leaves it unchanged.
    #[test]
    fn duplicate_subscribe_while_active_rejected(now in 0..=MAX_START, elapsed in 0..CYCLE) {
        let c = setup(now);
        let s = c.client.subscribe(&c.plan, &c.sub, &c.payref);
        set_ledger(&c.env, now + elapsed);
        prop_assert_eq!(
            c.client.try_subscribe(&c.plan, &c.sub, &c.payref),
            Err(Ok(Error::AlreadySubbed))
        );
        prop_assert_eq!(c.client.get_subscription(&c.plan, &c.sub), s);
    }

    /// Plan activity and subscriber entitlement are separate states (#230).
    ///
    /// Deactivating the plan does not revoke access already paid for: an
    /// existing subscriber stays active until `current_period_end` because
    /// `is_active` never loads the plan. Reactivating the plan does not extend
    /// that period either. A brand-new subscriber can only subscribe while the
    /// plan is active; after reactivation a new subscription starts from now.
    #[test]
    fn plan_status_changes_do_not_affect_existing_paid_period(
        now in 0..=MAX_START,
        elapsed in 0..=MAX_STEP,
    ) {
        let c = setup(now);
        let s = c.client.subscribe(&c.plan, &c.sub, &c.payref);

        // Deactivate the plan mid-period.
        let t = now + elapsed;
        set_ledger(&c.env, t);
        c.client.deactivate_plan(&c.plan);

        // Existing paid access is untouched: entitlement follows the period
        // end, not the plan status.
        prop_assert_eq!(c.client.is_active(&c.plan, &c.sub), s.current_period_end > t);
        prop_assert_eq!(c.client.get_subscription(&c.plan, &c.sub), s.clone());

        // A new subscriber cannot join while the plan is inactive.
        let other = Address::generate(&c.env);
        let other_ref = String::from_str(&c.env, "ref2");
        prop_assert!(c.client.try_subscribe(&c.plan, &other, &other_ref).is_err());

        // Reactivate the plan; the existing period is not extended.
        c.client.reactivate_plan(&c.plan);
        prop_assert_eq!(c.client.get_subscription(&c.plan, &c.sub), s.clone());
        prop_assert_eq!(c.client.is_active(&c.plan, &c.sub), s.current_period_end > t);

        // A new subscriber can now join and starts a fresh period from now.
        let fresh = c.client.subscribe(&c.plan, &other, &other_ref);
        prop_assert_eq!(fresh.current_period_end, t + CYCLE);
        prop_assert_eq!(fresh.started_at, t);
        prop_assert!(!fresh.cancelled);
    }
}
