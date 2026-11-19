# Gas Optimization

This document describes the storage layout, TTL policy, and read-cost
characteristics of the SynapsVault contracts, and reconciles the guide with the
current code after the TTL-on-read changes.

## Storage layout

Each contract keeps its admin/version metadata in **instance** storage and its
per-record data (leases, plans, subscriptions) in **persistent** storage.

## TTL policy

Persistent entries use a 90-day TTL with a refresh threshold of 89 days
(`PERSISTENT_TTL` / `PERSISTENT_TTL_THRESHOLD`). Instance storage shares the
same policy (`BUMP` / `BUMP_THRESH`).

### TTL is extended on reads

Earlier revisions of this guide claimed that read paths never bump TTL and that
access checks perform no TTL bumps. **That is no longer accurate.** The current
code extends TTL on read paths as well as write paths:

- `is_valid` / access-check paths load the lease and refresh its persistent TTL
  when the entry is below the threshold.
- Subscription read paths (`get_plan`, subscription lookups) also load the plan
  entry and refresh its TTL.

This keeps hot entries alive without requiring a separate write, at the cost of
additional persistent-entry touches per call (see footprint below).

### Simulated reads vs. submitted TTL extensions

Soroban distinguishes two things that both look like "reads" from the caller's
perspective:

1. **Simulated reads** — during transaction simulation the host reads storage to
   estimate resource usage. These reads do not persist any state change.
2. **Submitted TTL extensions** — when the transaction is actually submitted,
   any `extend_ttl` performed by the contract (including on read paths) is
   committed to the ledger and charged as a write footprint entry.

So a read-only call that touches a below-threshold entry will, on submission,
include that entry in the **write** footprint because of the TTL extension, even
though the contract logic only "read" the value. Simulation reflects this, but
callers should budget for the write entry rather than assuming a pure read.

## Footprint examples

Each hot call now touches **more than one** persistent entry, because the read
path loads the record *and* refreshes its TTL (and subscription paths also load
the plan):

| Call | Persistent entries touched |
| --- | --- |
| `is_valid` (lease below threshold) | lease entry (read + TTL write) |
| `is_valid` (lease above threshold) | lease entry (read only) |
| subscription read | subscription entry + plan entry (read + TTL writes) |
| `grant` / `extend` | lease entry (read + write + TTL) |

When an entry is above the refresh threshold, the read is a pure read and no
write footprint entry is added. When it is below the threshold, the TTL
extension adds a write footprint entry for that key.

## Guidance

- Budget for write footprint entries on read paths when entries may be near
  their TTL threshold.
- Keep hot entries refreshed so reads stay on the pure-read path.
- Do not assume a single persistent entry per hot call; subscription reads load
  both the subscription and its plan.
