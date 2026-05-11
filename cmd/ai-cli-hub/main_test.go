package main

import "testing"

func TestDisplayVersionPrefersInjectedVersion(t *testing.T) {
	oldVersion := version
	t.Cleanup(func() { version = oldVersion })

	version = "0.1.1"
	if got := displayVersion(); got != "0.1.1" {
		t.Fatalf("displayVersion() = %q, want 0.1.1", got)
	}
}

func TestDisplayVersionTrimsVPrefix(t *testing.T) {
	oldVersion := version
	t.Cleanup(func() { version = oldVersion })

	version = "v0.1.1"
	if got := displayVersion(); got != "0.1.1" {
		t.Fatalf("displayVersion() = %q, want 0.1.1", got)
	}
}
