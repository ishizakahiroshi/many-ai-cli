RUST_TOOLCHAIN    := 1.90.0
RUST_MANIFEST     := rust/Cargo.toml
BINARY            := rust/target/debug/many-ai-cli.exe

# 本線は Rust 版（2026-10-19 から）。Go 版のソースは凍結して置いてあり（go/ へ移す予定）、
# この Makefile からはビルドしない。毎日の試用ビルドと起動は scripts/rust-local.ps1
# が受け持ち、ここは同じ手順（画面資材 → cargo）を make の 1 語で呼べるようにするだけ。
#
# 版数・commit・日時は rust/build.rs が MANY_AI_BUILD_VERSION / _COMMIT / _TIME から
# 埋め込む。git describe の "祖先タグ + 距離 + sha" 形式（例: v0.9.0-160-g7da5a51-dirty）を
# 渡すので、タグから離れていることと uncommitted のままビルドしたことが画面で分かる。
# 引用符を含むコマンド（powershell 等）は GnuWin32 make の $(shell) で CreateProcess に
# 失敗するため、引用符不要の git のみで構成する。
export MANY_AI_BUILD_COMMIT  := $(shell git rev-parse --short HEAD)
export MANY_AI_BUILD_TIME    := $(shell git show -s --format=%cI HEAD)
export MANY_AI_BUILD_VERSION := $(shell git describe --tags --always --dirty)

.PHONY: build build-web build-rust build-release clean run fmt fmt-check debug-purge debug-restore

build: build-rust

build-web:
	cd web && bun install --frozen-lockfile && bun run build

# rust/build.rs は web/dist を検査して同梱するので、画面資材を先に作る。
build-rust: build-web
	cargo +$(RUST_TOOLCHAIN) build --locked --manifest-path $(RUST_MANIFEST) --bins

build-release: build-web
	cargo +$(RUST_TOOLCHAIN) build --locked --manifest-path $(RUST_MANIFEST) --release --bins

run: build-rust
	$(BINARY) serve

clean:
	cargo +$(RUST_TOOLCHAIN) clean --manifest-path $(RUST_MANIFEST)

fmt-check:
	cargo +$(RUST_TOOLCHAIN) fmt --manifest-path $(RUST_MANIFEST) -- --check

fmt:
	cargo +$(RUST_TOOLCHAIN) fmt --manifest-path $(RUST_MANIFEST)

debug-purge:
	node scripts/check-instrumentation.mjs --purge --id=$(id)

debug-restore:
	node scripts/debug-restore.mjs --id=$(id)
