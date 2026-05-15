BINARY       := dist/any-ai-cli.exe
WSL_BINARY   := dist/any-ai-cli-wsl.exe
LINUX_BINARY := dist/linux/any-ai-cli
MAIN         := ./cmd/any-ai-cli
WSL_MAIN     := ./cmd/any-ai-cli-wsl

.PHONY: build build-windows build-wsl-launcher build-linux clean run

build: build-windows build-wsl-launcher build-linux

build-windows:
	go-winres make --out cmd/any-ai-cli/rsrc
	go build -o $(BINARY) $(MAIN)

build-wsl-launcher:
	go build -o $(WSL_BINARY) $(WSL_MAIN)

build-linux:
	cmd /C "if not exist dist\linux mkdir dist\linux"
	cmd /C "set CGO_ENABLED=0&& set GOOS=linux&& set GOARCH=amd64&& go build -o $(LINUX_BINARY) $(MAIN)"

run: build-windows
	$(BINARY) serve

clean:
	rm -f $(BINARY) $(WSL_BINARY) $(LINUX_BINARY) cmd/any-ai-cli/rsrc_windows_*.syso
