BINARY            := dist/many-ai-cli.exe
LAUNCHER_BINARY   := dist/many-ai-cli-launcher.exe
LINUX_BINARY      := dist/linux/many-ai-cli
MAIN              := ./cmd/many-ai-cli
LAUNCHER_MAIN     := ./cmd/many-ai-cli-launcher

# ローカル（make）ビルドでも稼働中 Hub の素性を判別できるよう、git commit と
# その日時を ldflags で注入する。引用符を含むコマンド（powershell 等）は GnuWin32
# make の $(shell) で CreateProcess に失敗するため、引用符不要の git のみで構成する。
# buildTime には HEAD のコミット日時（ISO 8601）を入れる（=どの版から建てたか判別用。
# リリースは .goreleaser.yaml が実ビルド日時 {{ .Date }} を入れる）。未注入でも main
# 側は空文字で正常動作する（致命的依存はない）。
GIT_COMMIT        := $(shell git rev-parse --short HEAD)
BUILD_TIME        := $(shell git show -s --format=%cI HEAD)
GO_LDFLAGS        := -X main.gitCommit=$(GIT_COMMIT) -X main.buildTime=$(BUILD_TIME)

.PHONY: build build-web build-windows build-launcher build-linux deploy-wsl clean run

build: build-windows build-launcher build-linux deploy-wsl

build-web:
	cd web && bun install && bun run build

build-windows: build-web
	go-winres make --out cmd/many-ai-cli/rsrc
	go build -ldflags "$(GO_LDFLAGS)" -o $(BINARY) $(MAIN)

build-launcher:
	go-winres make --in winres/winres-launcher.json --out cmd/many-ai-cli-launcher/rsrc
	go build -o $(LAUNCHER_BINARY) $(LAUNCHER_MAIN)

build-linux: build-web
	cmd /C "if not exist dist\linux mkdir dist\linux"
	cmd /C "set CGO_ENABLED=0&& set GOOS=linux&& set GOARCH=amd64&& go build -o $(LINUX_BINARY) $(MAIN)"

deploy-wsl:
	powershell -NoProfile -ExecutionPolicy Bypass -File scripts/deploy-wsl.ps1

run: build-windows
	$(BINARY) serve

clean:
	rm -f $(BINARY) $(LAUNCHER_BINARY) $(LINUX_BINARY) cmd/many-ai-cli/rsrc_windows_*.syso cmd/many-ai-cli-launcher/rsrc_windows_*.syso
