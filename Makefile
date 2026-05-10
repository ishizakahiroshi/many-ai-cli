BINARY := dist/ai-cli-hub.exe
MAIN   := ./cmd/ai-cli-hub

.PHONY: build clean run

build:
	go-winres make --out cmd/ai-cli-hub/rsrc
	go build -o $(BINARY) $(MAIN)

run: build
	$(BINARY) serve

clean:
	rm -f $(BINARY) cmd/ai-cli-hub/rsrc_windows_*.syso
