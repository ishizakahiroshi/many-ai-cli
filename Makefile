BINARY            := dist/many-ai-cli.exe
LAUNCHER_BINARY   := dist/many-ai-cli-launcher.exe
LINUX_BINARY      := dist/linux/many-ai-cli
MAIN              := ./cmd/many-ai-cli
LAUNCHER_MAIN     := ./cmd/many-ai-cli-launcher

.PHONY: build build-web build-windows build-launcher build-linux deploy-wsl clean run

build: build-windows build-launcher build-linux deploy-wsl

build-web:
	cd web && bun install && bun run build

build-windows: build-web
	go-winres make --out cmd/many-ai-cli/rsrc
	go build -o $(BINARY) $(MAIN)

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
