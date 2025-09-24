.PHONY: build run test fmt lint clean setup

SHELL := /usr/bin/env bash

build:
	./scripts/build.sh

run:
	./scripts/run.sh

test:
	./scripts/test.sh

fmt lint:
	./scripts/fmt.sh

clean:
	cargo clean

setup:
	chmod +x scripts/*.sh
	@echo "Marked scripts executable. Create and edit .env if needed (see .env.example)."
