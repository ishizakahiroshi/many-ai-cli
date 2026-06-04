BINARY            := dist/any-ai-cli.exe
LAUNCHER_BINARY   := dist/any-ai-cli-launcher.exe
LINUX_BINARY      := dist/linux/any-ai-cli
MAIN              := ./cmd/any-ai-cli
LAUNCHER_MAIN     := ./cmd/any-ai-cli-launcher

.PHONY: build build-web build-windows build-launcher build-linux deploy-wsl clean run

build: build-windows build-launcher build-linux deploy-wsl

build-web:
	cd web && npm ci && npm run build

build-windows: build-web
	go-winres make --out cmd/any-ai-cli/rsrc
	go build -o $(BINARY) $(MAIN)

build-launcher:
	go-winres make --in winres/winres-launcher.json --out cmd/any-ai-cli-launcher/rsrc
	go build -o $(LAUNCHER_BINARY) $(LAUNCHER_MAIN)

build-linux: build-web
	cmd /C "if not exist dist\linux mkdir dist\linux"
	cmd /C "set CGO_ENABLED=0&& set GOOS=linux&& set GOARCH=amd64&& go build -o $(LINUX_BINARY) $(MAIN)"

deploy-wsl:
	powershell -NoProfile -ExecutionPolicy Bypass -File scripts/deploy-wsl.ps1

run: build-windows
	$(BINARY) serve

clean:
	rm -f $(BINARY) $(LAUNCHER_BINARY) $(LINUX_BINARY) cmd/any-ai-cli/rsrc_windows_*.syso cmd/any-ai-cli-launcher/rsrc_windows_*.syso
