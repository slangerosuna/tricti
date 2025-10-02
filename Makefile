.PHONY: build run test fmt lint clean setup

SHELL := /usr/bin/env bash
ARGS := $(filter-out $@,$(MAKECMDGOALS))

build:
	@./scripts/build.sh

run:
	@./scripts/run.sh

test:
	@./scripts/test.sh $(ARGS)

fmt lint:
	@./scripts/fmt.sh

clean:
	@cargo clean

setup:
	@zchmod +x scripts/*.sh
	@echo "Marked scripts executable. Create and edit .env if needed (see .env.example)."

%:
	@: