# EyeSaver build tasks. Everything here is a thin wrapper around cargo plus the two
# packaging scripts, so nothing is hidden: `make -n <target>` shows the exact commands.

CARGO ?= cargo
# Files excluded from the coverage gate: the OS adapters and the event loop, which cannot
# be exercised without a logged-in desktop session. See "Testing" in the README.
COVERAGE_EXCLUDE := (crates/app/|crates/platform/src/os/|xtask/)
COVERAGE_MIN := 95

.DEFAULT_GOAL := help
.PHONY: help build release test coverage coverage-html lint fmt icons run dev-bundle app dmg msi check clean

help: ## Show this help
	@grep -hE '^[a-zA-Z-]+:.*?## ' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-14s %s\n", $$1, $$2}'

build: ## Debug build
	$(CARGO) build --workspace

release: ## Optimised build
	$(CARGO) build --release --workspace

test: ## Run every test
	$(CARGO) test --workspace

coverage: ## Run tests and enforce the coverage gate
	$(CARGO) llvm-cov --workspace --summary-only \
		--ignore-filename-regex '$(COVERAGE_EXCLUDE)' \
		--fail-under-lines $(COVERAGE_MIN)

coverage-html: ## Write an HTML coverage report to target/llvm-cov/html
	$(CARGO) llvm-cov --workspace --html --ignore-filename-regex '$(COVERAGE_EXCLUDE)'
	@echo "open target/llvm-cov/html/index.html"

lint: ## Formatting and clippy, as CI runs them
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy --workspace --all-targets -- -D warnings

fmt: ## Reformat
	$(CARGO) fmt --all

icons: ## Regenerate the installer icons from the renderer in eye-saver-core
	$(CARGO) run -p xtask --quiet -- icons

run: ## Run from the source tree (no notifications: see dev-bundle)
	$(CARGO) run -p eye-saver -- --verbose

dev-bundle: ## macOS: build and run a debug .app, so notifications work
	./packaging/macos/build-app.sh --dev
	./dist/EyeSaver.app/Contents/MacOS/eye-saver --verbose

app: ## macOS: build the universal EyeSaver.app
	./packaging/macos/build-app.sh

dmg: ## macOS: build the installer disk image
	./packaging/macos/make-dmg.sh

msi: ## Windows: build the installer (run this on Windows)
	powershell -ExecutionPolicy Bypass -File packaging/windows/build-msi.ps1

check: lint test coverage ## Everything CI checks

clean: ## Remove build output
	$(CARGO) clean
	rm -rf dist packaging/generated
