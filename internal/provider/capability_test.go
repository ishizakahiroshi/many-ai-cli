package provider

import "testing"

func TestEmbeddedEffortCapabilitiesMatchCurrentLevels(t *testing.T) {
	definitions, _, err := EmbeddedDefinitions()
	if err != nil {
		t.Fatal(err)
	}
	want := map[string][]string{
		"claude":   {"low", "medium", "high", "xhigh", "max"},
		"codex":    {"minimal", "low", "medium", "high", "xhigh", "max"},
		"opencode": {"low", "medium", "high"},
	}
	for _, definition := range definitions {
		levels := []string(nil)
		if definition.Launch != nil {
			levels = definition.Launch.EffortLevels
		}
		if expected, ok := want[definition.ID]; ok {
			if len(levels) != len(expected) {
				t.Fatalf("%s effort levels = %#v, want %#v", definition.ID, levels, expected)
			}
			continue
		}
		if len(levels) != 0 {
			t.Fatalf("%s declares unsupported effort levels: %#v", definition.ID, levels)
		}
	}
}
