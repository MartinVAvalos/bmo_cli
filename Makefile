# Makefile for bmo_cli
# Recipe lines must start with a TAB, not spaces.

BIN := bmo_cli

.PHONY: help run build release check fmt lint clean install

# Default target: list what is available.
help:
	@echo "bmo_cli — targets:"
	@echo "  make run      Run interactively (output is copied to the clipboard)"
	@echo "  make build    Debug build"
	@echo "  make release  Optimized build (./target/release/$(BIN))"
	@echo "  make check    Fast type-check, no binary"
	@echo "  make fmt      Format the code with rustfmt"
	@echo "  make lint     Run clippy"
	@echo "  make install  Install $(BIN) onto your PATH"
	@echo "  make clean    Remove build artifacts"

# The binary copies its output to the clipboard automatically.
run:
	cargo run

build:
	cargo build

release:
	cargo build --release

check:
	cargo check

fmt:
	cargo fmt

lint:
	cargo clippy

install:
	cargo install --path .

clean:
	cargo clean
