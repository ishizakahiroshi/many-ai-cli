BINARY            := dist/any-ai-cli.exe
WSL_BINARY        := dist/any-ai-cli-wsl.exe
LAUNCHER_BINARY   := dist/any-ai-cli-launcher.exe
LINUX_BINARY      := dist/linux/any-ai-cli
MAIN              := ./cmd/any-ai-cli
WSL_MAIN          := ./cmd/any-ai-cli-wsl
LAUNCHER_MAIN     := ./cmd/any-ai-cli-launcher

.PHONY: build build-windows build-wsl-launcher build-launcher build-linux deploy-wsl clean run

build: build-windows build-wsl-launcher build-launcher build-linux deploy-wsl

build-windows:
	go-winres make --out cmd/any-ai-cli/rsrc
	go build -o $(BINARY) $(MAIN)

build-wsl-launcher:
	go-winres make --in winres/winres-wsl.json --out cmd/any-ai-cli-wsl/rsrc
	go build -o $(WSL_BINARY) $(WSL_MAIN)

build-launcher:
	go-winres make --in winres/winres-launcher.json --out cmd/any-ai-cli-launcher/rsrc
	go build -o $(LAUNCHER_BINARY) $(LAUNCHER_MAIN)

build-linux:
	cmd /C "if not exist dist\linux mkdir dist\linux"
	cmd /C "set CGO_ENABLED=0&& set GOOS=linux&& set GOARCH=amd64&& go build -o $(LINUX_BINARY) $(MAIN)"

deploy-wsl:
	powershell -NoProfile -ExecutionPolicy Bypass -File scripts/deploy-wsl.ps1

run: build-windows
	$(BINARY) serve

clean:
	rm -f $(BINARY) $(WSL_BINARY) $(LAUNCHER_BINARY) $(LINUX_BINARY) cmd/any-ai-cli/rsrc_windows_*.syso cmd/any-ai-cli-wsl/rsrc_windows_*.syso cmd/any-ai-cli-launcher/rsrc_windows_*.syso
