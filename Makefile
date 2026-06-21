# Makefile for bmo_cli
# Recipe lines must start with a TAB, not spaces.

BIN    := bmo
IMAGE  := bmo-cli
PREFIX ?= $(HOME)/.local/bin

# Native musl target for the current machine (x86_64 or aarch64).
ARCH        := $(shell uname -m)
MUSL_TARGET := $(ARCH)-unknown-linux-musl

.PHONY: help run build release check fmt lint clean install docker-build docker-run docker-install docker-extract

# Default target: list what is available.
help:
	@echo "bmo_cli — targets:"
	@echo "  make run             Run interactively (output is copied to the clipboard)"
	@echo "  make build           Debug build"
	@echo "  make release         Optimized build (./target/release/$(BIN))"
	@echo "  make check           Fast type-check, no binary"
	@echo "  make fmt             Format the code with rustfmt"
	@echo "  make lint            Run clippy"
	@echo "  make install         Install $(BIN) onto your PATH (needs Rust)"
	@echo "  make clean           Remove build artifacts"
	@echo "  make docker-build    Build the Docker image ($(IMAGE))"
	@echo "  make docker-run      Run the image against the current directory"
	@echo "  make docker-install  Install a '$(BIN)' wrapper that runs the image (no Rust; no clipboard)"
	@echo "  make docker-extract  Build a native binary in Docker and install it (no Rust; Linux only)"

# The binary copies its output to the clipboard automatically.
run:
	cargo run

build:
	cargo build

release:
	cargo build --release --locked

check:
	cargo check

fmt:
	cargo fmt

lint:
	cargo clippy

install:
	cargo install --path . --locked --force

clean:
	cargo clean

docker-build:
	docker build -t $(IMAGE) .

# Mounts the directory you run this from into the container as /work, so bmo
# captures THIS project. To capture another project, run the same docker
# command from that project's directory. Inside the container there is no
# host clipboard, so output falls back to stdout.
docker-run:
	docker run --rm -it -v "$(CURDIR)":/work $(IMAGE)

# Builds the image, then installs a small wrapper named '$(BIN)' that runs the
# container against whatever directory you call it from. Needs Docker at
# runtime, but NOT Rust. NOTE: clipboard does NOT work this way (the container
# can't reach your host clipboard) — use docker-extract on Linux instead.
docker-install: docker-build
	@mkdir -p "$(PREFIX)"
	@printf '#!/bin/sh\nexec docker run --rm -it -v "$$PWD":/work $(IMAGE) "$$@"\n' > "$(PREFIX)/$(BIN)"
	@chmod +x "$(PREFIX)/$(BIN)"
	@echo "Installed '$(BIN)' -> $(PREFIX)/$(BIN) (runs the Docker image; needs Docker, not Rust)."
	@echo "In Docker mode, output prints to the terminal (the container can't reach your host clipboard)."
	@case ":$$PATH:" in *":$(PREFIX):"*) echo "$(PREFIX) is already on your PATH." ;; *) echo "Add $(PREFIX) to your PATH (e.g. in ~/.zshrc), then restart your shell." ;; esac

# Builds a fully static (musl) Linux binary inside Docker and installs the real
# binary onto your machine. No Rust needed, runs on any Linux regardless of
# glibc, and the clipboard works (with xclip/wl-clipboard installed). This is
# the right choice for Linux. The binary is Linux-native — it won't run on macOS.
docker-extract:
	docker build -f Dockerfile.musl --build-arg TARGET=$(MUSL_TARGET) -t $(IMAGE)-musl .
	@mkdir -p "$(PREFIX)"
	@docker create --name $(IMAGE)-extract $(IMAGE)-musl >/dev/null
	docker cp $(IMAGE)-extract:/bmo "$(PREFIX)/$(BIN)"
	@docker rm $(IMAGE)-extract >/dev/null
	@chmod +x "$(PREFIX)/$(BIN)"
	@echo "Installed native '$(BIN)' -> $(PREFIX)/$(BIN) (no Docker or Rust needed to run it)."
	@echo "If an older Docker-wrapper '$(BIN)' is elsewhere on your PATH, remove it (check: command -v $(BIN))."
	@case ":$$PATH:" in *":$(PREFIX):"*) echo "$(PREFIX) is already on your PATH." ;; *) echo "Add $(PREFIX) to your PATH (e.g. in ~/.zshrc), then restart your shell." ;; esac
