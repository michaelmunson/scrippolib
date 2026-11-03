.PHONY: build build-release test install

build:
	cargo build

build-release:
	cargo build --release

test:
	cargo test

install: build-release
	install -m 755 target/release/scrippolib /usr/local/bin/scrippolib

sudo-install: build-release
	sudo install -m 755 target/release/scrippolib /usr/local/bin/scrippolib
