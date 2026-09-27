package provider

import (
	"encoding/json"
	"strings"
	"testing"
)

func validDefinitionJSON() []byte {
	return []byte(`{"schema_version":1,"id":"example-cli","display_name":"Example CLI","launch":{"executable":"example-cli","args":["--agent"],"model_args":["--model","{model}"]},"models":{"allow_custom":true,"items":[{"id":"fast","label":"Fast"}]},"adapters":{"approval":"none"}}`)
}

func TestValidateDefinitionReportsIndependentErrors(t *testing.T) {
	raw := []byte(`{"schema_version":2,"id":"Bad ID","display_name":"","launch":{"args":["$(bad)"],"allowed_env":["TOKEN=value"]},"adapters":{"approval":"approval:missing"},"unknown":true}`)
	diagnostics, err := ValidateDefinition(raw, DefaultAdapterCatalog())
	if err != nil {
		t.Fatalf("ValidateDefinition returned decode error: %v", err)
	}
	if len(diagnostics) < 5 {
		t.Fatalf("ValidateDefinition returned %d diagnostics, want independent errors: %#v", len(diagnostics), diagnostics)
	}
	joined := make([]string, 0, len(diagnostics))
	for _, diagnostic := range diagnostics {
		joined = append(joined, diagnostic.Code)
	}
	for _, code := range []string{"unsupported_schema_version", "invalid_id", "invalid_display_name", "invalid_arg", "invalid_env_name", "unknown_adapter", "unknown_field"} {
		if !contains(joined, code) {
			t.Errorf("diagnostics do not contain %q: %#v", code, diagnostics)
		}
	}
}

func TestValidateDefinitionRejectsMalformedJSON(t *testing.T) {
	if _, err := ValidateDefinition([]byte(`{"schema_version":1}`), DefaultAdapterCatalog()); err != nil {
		t.Fatalf("valid JSON returned error: %v", err)
	}
	if _, err := ValidateDefinition([]byte(`{"schema_version":1} {}`), DefaultAdapterCatalog()); err == nil {
		t.Fatal("multiple JSON values returned nil error")
	}
}

func TestValidateDefinitionRejectsShellMetacharacters(t *testing.T) {
	for _, char := range []string{"&", "|", "<", ">", "^", "%", "$(", "${", ";", "`"} {
		raw := []byte(`{"schema_version":1,"id":"test-cli","display_name":"Test","launch":{"executable":"test","args":["arg` + char + `test"]}}`)
		diagnostics, err := ValidateDefinition(raw, DefaultAdapterCatalog())
		if err != nil {
			t.Fatalf("ValidateDefinition error: %v", err)
		}
		found := false
		for _, d := range diagnostics {
			if d.Code == "invalid_arg" {
				found = true
				break
			}
		}
		if !found {
			t.Errorf("expected invalid_arg for metacharacter %q, got %#v", char, diagnostics)
		}
	}
}

func TestBuildUsesDeterministicOrderAndRevision(t *testing.T) {
	var definition Definition
	if err := json.Unmarshal(validDefinitionJSON(), &definition); err != nil {
		t.Fatal(err)
	}
	claude := definition
	claude.ID = "claude"
	claude.DisplayName = "Claude"
	claude.Launch = &LaunchDefinition{Executable: "claude"}
	claude.Source = SourceRef{Version: "test"}
	custom := definition
	custom.ID = "zeta"
	custom.DisplayName = "Zeta"
	first, firstDiagnostics := Build(Layers{Embedded: []Definition{claude}, User: []Definition{custom}}, DefaultAdapterCatalog())
	second, secondDiagnostics := Build(Layers{Embedded: []Definition{claude}, User: []Definition{custom}}, DefaultAdapterCatalog())
	if len(firstDiagnostics) != 0 || len(secondDiagnostics) != 0 {
		t.Fatalf("Build diagnostics = %#v / %#v", firstDiagnostics, secondDiagnostics)
	}
	if first.Revision() == "" || first.Revision() != second.Revision() {
		t.Fatalf("revisions = %q / %q, want same non-empty revision", first.Revision(), second.Revision())
	}
	list := first.List()
	if len(list) != 2 || list[0].ID != "claude" || list[1].ID != "zeta" {
		t.Fatalf("List = %#v, want built-in before user id", list)
	}
	got, ok := first.Lookup("zeta")
	if !ok || got.DisplayName != "Zeta" {
		t.Fatalf("Lookup(zeta) = %#v, %v", got, ok)
	}
	got.Launch.Args[0] = "mutated"
	again, _ := first.Lookup("zeta")
	if again.Launch.Args[0] == "mutated" {
		t.Fatal("Lookup returned mutable internal slices")
	}
}

func TestBuildMergesLayersAndReplacesArgvArrays(t *testing.T) {
	base := Definition{SchemaVersion: 1, ID: "example", DisplayName: "Base", Launch: &LaunchDefinition{Executable: "example", Args: []string{"--base"}, ModelArgs: []string{"--model", "{model}"}}}
	distribution := Definition{SchemaVersion: 1, ID: "example", Launch: &LaunchDefinition{Args: []string{"--distributed"}}}
	user := Definition{SchemaVersion: 1, ID: "example", DisplayName: "User"}
	registry, diagnostics := Build(Layers{Embedded: []Definition{base}, AcceptedDistribution: []Definition{distribution}, User: []Definition{user}}, DefaultAdapterCatalog())
	if len(diagnostics) != 0 {
		t.Fatalf("Build diagnostics = %#v", diagnostics)
	}
	definition, ok := registry.Lookup("example")
	if !ok {
		t.Fatal("example was not registered")
	}
	if definition.DisplayName != "User" || strings.Join(definition.Launch.Args, ",") != "--distributed" {
		t.Fatalf("merged definition = %#v, want user label and distribution argv replacement", definition)
	}
	if strings.Join(definition.Launch.ModelArgs, ",") != "--model,{model}" {
		t.Fatalf("field merge lost model args: %#v", definition.Launch)
	}
}

func contains(values []string, want string) bool {
	for _, value := range values {
		if value == want {
			return true
		}
	}
	return false
}
