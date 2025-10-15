ca.PHONY: build-f,release

target_dir=$(shell cargo metadata --format-version=1 | jq -r '.target_directory')
name=rig-rag

build-f:
	# npm install
	npm run build

release:
	cargo build --release --target x86_64-unknown-linux-musl
	mkdir -p ~/Downloads/release/
	cp $(target_dir)/x86_64-unknown-linux-musl/release/$(name) $(target_dir)

release-win:
	cargo build --release --target x86_64-pc-windows-gnu
	mkdir -p ~/Downloads/release/
	cp $(target_dir)/x86_64-pc-windows-gnu/release/$(name) $(target_dir)