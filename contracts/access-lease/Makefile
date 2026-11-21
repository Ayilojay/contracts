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

# Refresh the prebuilt WASM used as the upgrade target in lease unit tests.
#
# Provenance record for fixtures/access_lease_upgrade_target.wasm:
#   Source target:  this Makefile's `fixture` target (package access-lease)
#   Source commit:  the commit that last modified this Makefile / fixture
#                   (see `git log -1 --format=%H -- fixtures/access_lease_upgrade_target.wasm`)
#   Build command:  make fixture
#   Tool versions:  stellar-cli and Rust toolchain pinned by the repo
#                   (record with `stellar --version` and `rustc --version`)
#   Checksum:       recorded in fixtures/access_lease_upgrade_target.wasm.sha256
#
# Regeneration procedure (reproducible):
#   1. Check out the source commit above.
#   2. Run `make fixture` from this directory to rebuild and copy the WASM.
#   3. Run `make fixture-verify` to confirm the checksum matches.
fixture:
	stellar contract build --package access-lease
	cp ../../target/wasm32v1-none/release/access_lease.wasm fixtures/access_lease_upgrade_target.wasm
	cd fixtures && sha256sum access_lease_upgrade_target.wasm > access_lease_upgrade_target.wasm.sha256

# Verify the checked-in upgrade fixture matches its recorded checksum.
fixture-verify:
	cd fixtures && sha256sum -c access_lease_upgrade_target.wasm.sha256
