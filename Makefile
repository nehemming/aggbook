# ============================================================================
# Makefile for building and running aggserver and aggclient
# ============================================================================

# Change port or server address if needed
PORT ?= 8099
SERVER_ADDR ?= http://localhost:8099

# ─── Local Targets ─────────────────────────────────────────────
.PHONY: all lint test local release network-tests
all: lint test local

lint:
	cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
	cargo test --workspace	

network-tests:
	cargo test --workspace --features network-tests

local:
	cargo build 

release: test
	cargo build --release

	
#./target/debug/aggserver --port $(PORT)
#./target/debug/aggclient --server-addr $(SERVER_ADDR)

# ─── Docker Targets ─────────────────────────────────────────────

.PHONY: docker-client docker-server docker-up docker-down

docker-client:
	docker build -f Dockerfile.client -t aggclient .

docker-server:
	docker build -f Dockerfile.server -t aggserver .

docker-up:
	docker compose up

docker-down:
	docker compose down
