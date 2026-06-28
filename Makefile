.PHONY: run test lint fmt check clean

run:
	cargo run

test:
	cargo test

lint:
	cargo clippy -- -D warnings

fmt:
	cargo fmt --check

check: fmt lint test

clean:
	cargo clean
