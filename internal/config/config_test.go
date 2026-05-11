package config

import "testing"

func TestDefaultConfigOpensBrowser(t *testing.T) {
	cfg := defaultConfig(t.TempDir())
	if !cfg.Hub.OpenBrowser {
		t.Fatal("defaultConfig().Hub.OpenBrowser = false, want true")
	}
}
