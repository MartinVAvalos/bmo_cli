# Makefile for bmo_cli
# Recipe lines must start with a TAB, not spaces.

BIN   := bmo
IMAGE := bmo-cli

.PHONY: help run build release check fmt lint clean install docker-build docker-run

# Default target: list what is available.
help:
	@echo "bmo_cli — targets:"
	@echo "  make run           Run interactively (output is copied to the clipboard)"
	@echo "  make build         Debug build"
	@echo "  make release       Optimized build (./target/release/$(BIN))"
	@echo "  make check         Fast type-check, no binary"
	@echo "  make fmt           Format the code with rustfmt"
	@echo "  make lint          Run clippy"
	@echo "  make install       Install $(BIN) onto your PATH"
	@echo "  make clean         Remove build artifacts"
	@echo "  make docker-build  Build the Docker image ($(IMAGE))"
	@echo "  make docker-run    Run the image against the current directory"

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
	cargo install --path . --locked

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
