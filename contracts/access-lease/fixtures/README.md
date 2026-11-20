# access-lease upgrade fixture

`access_lease_upgrade_target.wasm` is a prebuilt WASM used by the
`access-lease` unit tests for `upgrade`, post-upgrade lease reads, admin
authorisation, and `migrate` (issue #214).

## Why a lease-specific fixture?

The old tests reused `vault-registry/fixtures/vault_registry_upgrade_target.wasm`.
That fixture does not expose `access-lease` entrypoints, so post-upgrade tests
that call `grant_lease`, `is_valid`, `admin`, or `migrate` on the upgraded
contract could not verify those paths.  A lease-specific fixture lets the tests
exercise the full upgraded interface.

## Regenerating

The checked-in file is **bootstrapped** from the vault-registry fixture for the
initial commit.  Regenerate it with the actual `access-lease` WASM before
promoting to mainnet:

```sh
# From contracts/access-lease/
make fixture        # builds access_lease.wasm and copies it here
make fixture-verify # checks the sha256 matches
```

Commit both `access_lease_upgrade_target.wasm` and
`access_lease_upgrade_target.wasm.sha256` together.

## Checksum

`access_lease_upgrade_target.wasm.sha256` records the SHA-256 of the committed
WASM.  `make fixture-verify` checks it.
