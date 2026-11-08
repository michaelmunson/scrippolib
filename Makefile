.PHONY: build build-release test install

# `scrippolib` is a scrippo script (cli/scrippolib.scrippo) compiled to bash, wrapping the
# `scrippolib-core` Rust binary that sits next to it. Compiling it needs `scrippo-core`.
SCRIPPO_CORE ?= ../scriptopotamus/target/debug/scrippo-core

define wrapper
$(SCRIPPO_CORE) -c cli/scrippolib.scrippo > target/$(1)/scrippolib.tmp
chmod 755 target/$(1)/scrippolib.tmp
mv target/$(1)/scrippolib.tmp target/$(1)/scrippolib
endef

build:
	cargo build
	$(call wrapper,debug)

build-release:
	cargo build --release
	$(call wrapper,release)

test:
	cargo test

install: build-release
	install -m 755 target/release/scrippolib-core /usr/local/bin/scrippolib-core
	install -m 755 target/release/scrippolib /usr/local/bin/scrippolib

sudo-install: build-release
	sudo install -m 755 target/release/scrippolib-core /usr/local/bin/scrippolib-core
	sudo install -m 755 target/release/scrippolib /usr/local/bin/scrippolib
