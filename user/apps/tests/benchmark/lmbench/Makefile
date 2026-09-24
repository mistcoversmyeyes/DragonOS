# LMbench benchmark test suite Makefile.
# Build the corrected FIFO helper and install the guest-side files.
# Invoked by DADK (build-from-source) with DADK_CURRENT_BUILD_DIR set.

ifdef DADK_CURRENT_BUILD_DIR
INSTALL_DIR = $(DADK_CURRENT_BUILD_DIR)
else
INSTALL_DIR = ./install
endif

# Files needed inside the guest at runtime. Sources live under runner/;
# `install` flattens them into INSTALL_DIR (guest runtime layout is flat).
RUNNER_DIR    = runner
GUEST_SCRIPTS = $(RUNNER_DIR)/run.sh $(RUNNER_DIR)/init.sh $(RUNNER_DIR)/env.sh $(RUNNER_DIR)/clean_up.sh $(RUNNER_DIR)/result.sh
GUEST_DATA    = whitelist.txt config

.PHONY: all build install clean help

all: install

build:
	sh upstream/build-fifo.sh ./build/fifo

install: build
	@echo "[lmbench] installing to $(INSTALL_DIR)"
	@mkdir -p $(INSTALL_DIR)
	@rm -f $(INSTALL_DIR)/run_tests.sh $(INSTALL_DIR)/run_test_case.sh
	@install -m755 $(GUEST_SCRIPTS) $(INSTALL_DIR)/
	@install -m644 $(GUEST_DATA) $(INSTALL_DIR)/
	@cp -r $(RUNNER_DIR)/test_cases $(INSTALL_DIR)/
	@chmod +x $(INSTALL_DIR)/test_cases/*.sh 2>/dev/null || true
	@mkdir -p $(INSTALL_DIR)/helpers
	@install -m755 build/fifo/lat_fifo $(INSTALL_DIR)/helpers/
	@install -m644 build/fifo/COPYING.lmbench build/fifo/lat_fifo-cleanup.patch build/fifo/build-info.txt $(INSTALL_DIR)/helpers/
	@echo "[lmbench] install done"

clean:
	@rm -rf ./install ./build

help:
	@echo "lmbench benchmark Makefile targets:"
	@echo "  install  - build FIFO helper and copy guest files into INSTALL_DIR"
	@echo "  clean    - remove local ./install and ./build"
