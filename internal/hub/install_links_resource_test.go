package hub

import (
	"encoding/json"
	"net/url"
	"os"
	"testing"
)

// このテストは resources/install-links/defaults.json の形式だけを検査する。
// C2（internal/hub/install_link_fetch.go 等）が無くても単独で通ること
// （C1 と C2 は並列作業のため、依存させない）。

func TestInstallLinksResourceJSONIsWellFormed(t *testing.T) {
	data, err := os.ReadFile("../../resources/install-links/defaults.json")
	if err != nil {
		t.Fatalf("failed to read defaults.json: %v", err)
	}

	var links map[string]string
	if err := json.Unmarshal(data, &links); err != nil {
		t.Fatalf("defaults.json is not valid JSON: %v", err)
	}

	if len(links) == 0 {
		t.Fatal("defaults.json has no entries")
	}

	for key, rawURL := range links {
		if key == "" {
			t.Error("found empty provider key")
		}
		if key == "shell" {
			t.Error("\"shell\" must not be a key in defaults.json")
		}

		if len(rawURL) < len("https://") || rawURL[:len("https://")] != "https://" {
			t.Errorf("provider %q: URL must start with https://, got %q", key, rawURL)
			continue
		}

		if _, err := url.Parse(rawURL); err != nil {
			t.Errorf("provider %q: URL failed to parse: %v", key, err)
		}
	}
}
