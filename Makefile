BINARY := dist/any-ai-cli.exe
MAIN   := ./cmd/any-ai-cli

.PHONY: build clean run

build:
	go-winres make --out cmd/any-ai-cli/rsrc
	go build -o $(BINARY) $(MAIN)

run: build
	$(BINARY) serve

clean:
	rm -f $(BINARY) cmd/any-ai-cli/rsrc_windows_*.syso
