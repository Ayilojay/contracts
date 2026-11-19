.PHONY: default all test build fmt clean fixture fixture-verify

default: build

all: test

test: build
	cargo test

build:
	stellar contract build
	@ls -l ../../target/wasm32v1-none/release/*.wasm

fmt:
	cargo fmt --all

clean:
	cargo clean

# Refresh the prebuilt WASM used as the upgrade target in unit tests.
#
# Provenance record for fixtures/vault_registry_upgrade_target.wasm:
#   Source target:  this Makefile's `fixture` target (package vault-registry)
#   Source commit:  the commit that last modified this Makefile / fixture
#                   (see `git log -1 --format=%H -- fixtures/vault_registry_upgrade_target.wasm`)
#   Build command:  make fixture
#   Tool versions:  stellar-cli and Rust toolchain pinned by the repo
#                   (record with `stellar --version` and `rustc --version`)
#   Checksum:       recorded in fixtures/vault_registry_upgrade_target.wasm.sha256
#
# Regeneration procedure (reproducible):
#   1. Check out the source commit above.
#   2. Run `make fixture` to rebuild and copy the WASM.
#   3. Run `make fixture-verify` to confirm the checksum matches.
fixture:
	stellar contract build --package vault-registry
	cp ../../target/wasm32v1-none/release/vault_registry.wasm fixtures/vault_registry_upgrade_target.wasm

# Verify the checked-in upgrade fixture matches its recorded checksum.
fixture-verify:
	cd fixtures && sha256sum -c vault_registry_upgrade_target.wasm.sha256
