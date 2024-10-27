# Configuration
INSTALL_PATH ?= /usr/local/bin

.PHONY: release install

release: ## Build release binary
	cargo build --release

install: release ## Install to INSTALL_PATH (Default: /usr/local/bin)
	install -m 755 target/release/flatten-repo $(INSTALL_PATH)/

.DEFAULT_GOAL := help
.PHONY: help
help:
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'