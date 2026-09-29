package provider

import (
	"encoding/json"
	"path/filepath"
	"strings"
	"testing"
)

func presentationDefinitionJSON(presentation string) []byte {
	return []byte(`{"schema_version":1,"id":"example-cli","display_name":"Example CLI","launch":{"executable":"example-cli"},"presentation":` + presentation + `}`)
}

func presentationDiagnostics(t *testing.T, presentation string) []Diagnostic {
	t.Helper()
	diagnostics, err := ValidateDefinition(presentationDefinitionJSON(presentation), DefaultAdapterCatalog())
	if err != nil {
		t.Fatalf("ValidateDefinition: %v", err)
	}
	var out []Diagnostic
	for _, diagnostic := range diagnostics {
		if strings.HasPrefix(diagnostic.Field, "presentation.") {
			out = append(out, diagnostic)
		}
	}
	return out
}

func TestValidatePresentationAcceptsPlainColorsAndShortLetters(t *testing.T) {
	for _, presentation := range []string{
		`{}`,
		`{"color":"#1E90FF"}`,
		`{"color":"#1e90ff","icon_text":"A"}`,
		`{"icon_text":"AB"}`,
		`{"icon_text":"あ"}`,
		`{"icon_text":"Gé"}`,
		`{"icon_text":"é"}`,
		`{"icon_text":"🇯🇵"}`,
		`{"icon_text":"👨‍👩‍👧"}`,
	} {
		if got := presentationDiagnostics(t, presentation); len(got) != 0 {
			t.Errorf("presentation %s produced diagnostics %#v, want none", presentation, got)
		}
	}
}

func TestValidatePresentationRejectsCSSInjectionAndBadLetters(t *testing.T) {
	cases := map[string]string{
		`{"color":"red;}"}`:                                "presentation.color",
		`{"color":"red"}`:                                  "presentation.color",
		`{"color":"#fff"}`:                                 "presentation.color",
		`{"color":"#12345g"}`:                              "presentation.color",
		`{"color":"#123456;background:url(x)"}`:            "presentation.color",
		`{"color":"rgb(0,0,0)"}`:                           "presentation.color",
		`{"color":"#1E90FF "}`:                             "presentation.color",
		`{"icon_text":"ABC"}`:                              "presentation.icon_text",
		`{"icon_text":" A"}`:                               "presentation.icon_text",
		`{"icon_text":"A\nB"}`:                             "presentation.icon_text",
		`{"icon_text":"🇯🇵🇺🇸🇫🇷"}`:                           "presentation.icon_text",
		`{"icon_text":"́"}`:                                "presentation.icon_text",
		`{"icon_text":"` + strings.Repeat("A", 600) + `"}`: "presentation.icon_text",
	}
	for presentation, field := range cases {
		got := presentationDiagnostics(t, presentation)
		if len(got) == 0 {
			t.Errorf("presentation %.40s produced no diagnostics, want an error on %s", presentation, field)
			continue
		}
		for _, diagnostic := range got {
			if diagnostic.Field != field || diagnostic.Code != "invalid_presentation" || !diagnostic.IsError() {
				t.Errorf("presentation %.40s diagnostic = %#v, want error invalid_presentation on %s", presentation, diagnostic, field)
			}
		}
	}
}

func TestSummaryCarriesOnlyValidPresentation(t *testing.T) {
	base := Definition{SchemaVersion: 1, ID: "alpha", DisplayName: "Alpha", Launch: &LaunchDefinition{Executable: "alpha"}}
	blue := Definition{SchemaVersion: 1, ID: "alpha", Presentation: &PresentationDefinition{IconText: "A", Color: "#1E90FF"}}
	registry, _ := Build(Layers{Embedded: []Definition{base}, Overrides: []Definition{blue}}, DefaultAdapterCatalog())
	list := registry.List()
	if len(list) != 1 || list[0].Presentation == nil || list[0].Presentation.IconText != "A" || list[0].Presentation.Color != "#1E90FF" {
		t.Fatalf("List = %#v, want presentation A / #1E90FF", list)
	}
	raw, err := json.Marshal(list[0])
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(raw), `"presentation":{"icon_text":"A","color":"#1E90FF"}`) {
		t.Fatalf("summary JSON = %s, want the presentation object", raw)
	}

	// Nothing set: the key is left out of the JSON entirely.
	plain, _ := Build(Layers{Embedded: []Definition{base}}, DefaultAdapterCatalog())
	rawPlain, _ := json.Marshal(plain.List()[0])
	if strings.Contains(string(rawPlain), "presentation") {
		t.Fatalf("summary JSON = %s, want no presentation key", rawPlain)
	}

	// A bad value that is already stored must not fail the build and must not
	// reach the list: the bad half is dropped, the good half stays.
	bad := Definition{SchemaVersion: 1, ID: "alpha", Presentation: &PresentationDefinition{IconText: "ABC", Color: "red;}"}}
	registry, diagnostics := Build(Layers{Embedded: []Definition{base}, Overrides: []Definition{bad}}, DefaultAdapterCatalog())
	if len(registry.List()) != 1 {
		t.Fatalf("List = %#v, want alpha to stay registered", registry.List())
	}
	if registry.List()[0].Presentation != nil {
		t.Fatalf("summary presentation = %#v, want it dropped", registry.List()[0].Presentation)
	}
	if !hasDiagnosticField(diagnostics, "alpha.presentation.color") || !hasDiagnosticField(diagnostics, "alpha.presentation.icon_text") {
		t.Fatalf("diagnostics = %#v, want both bad presentation fields reported", diagnostics)
	}
	half := Definition{SchemaVersion: 1, ID: "alpha", Presentation: &PresentationDefinition{IconText: "ABC", Color: "#00FF00"}}
	registry, _ = Build(Layers{Embedded: []Definition{base}, Overrides: []Definition{half}}, DefaultAdapterCatalog())
	if got := registry.List()[0].Presentation; got == nil || got.IconText != "" || got.Color != "#00FF00" {
		t.Fatalf("summary presentation = %#v, want only the valid color", got)
	}
}

func hasDiagnosticField(diagnostics []Diagnostic, field string) bool {
	for _, diagnostic := range diagnostics {
		if diagnostic.Field == field {
			return true
		}
	}
	return false
}

func TestSaveEffectiveOverrideRefusesBadPresentationAndKeepsSparseDelta(t *testing.T) {
	store, err := NewHistoryStore(filepath.Join(t.TempDir(), "overrides"), filepath.Join(t.TempDir(), "backups"))
	if err != nil {
		t.Fatal(err)
	}
	base := Definition{
		SchemaVersion: CurrentSchemaVersion,
		ID:            "codex", DisplayName: "Codex",
		Launch: &LaunchDefinition{Executable: "codex-v1"},
	}

	bad := base
	bad.Presentation = &PresentationDefinition{Color: "red;}"}
	if _, err := store.SaveEffectiveOverride("codex", bad, base, "", "edit"); err == nil {
		t.Fatal("SaveEffectiveOverride accepted color \"red;}\"")
	}

	good := base
	good.Presentation = &PresentationDefinition{IconText: "A", Color: "#1E90FF"}
	record, err := store.SaveEffectiveOverride("codex", good, base, "", "edit")
	if err != nil {
		t.Fatal(err)
	}
	if record.Payload.Presentation == nil || record.Payload.Presentation.Color != "#1E90FF" {
		t.Fatalf("stored presentation = %#v", record.Payload.Presentation)
	}
	if record.Payload.Launch != nil {
		t.Fatalf("unchanged launch was frozen into the override: %#v", record.Payload.Launch)
	}

	// Saving the form again with both fields blank leaves no presentation in
	// the override: the icon goes back to the shipped default.
	cleared, err := store.SaveEffectiveOverride("codex", base, base, record.Revision, "edit")
	if err != nil {
		t.Fatalf("clearing the presentation was refused: %v", err)
	}
	if cleared.Payload.Presentation != nil {
		t.Fatalf("cleared override still has presentation %#v", cleared.Payload.Presentation)
	}
}
