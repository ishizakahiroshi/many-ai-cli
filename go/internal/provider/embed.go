package provider

import (
	"embed"
	"encoding/json"
	"fmt"
	"io/fs"
	"sort"
)

//go:embed schema.json
var schemaJSON []byte

//go:embed manifests/*.json
var embeddedFiles embed.FS

func SchemaJSON() []byte {
	return append([]byte(nil), schemaJSON...)
}

// EmbeddedDefinitions returns the shipped definitions in the explicit
// BuiltinProviderIDs order, not filesystem order.
func EmbeddedDefinitions() ([]Definition, []Diagnostic, error) {
	paths, err := fs.Glob(embeddedFiles, "manifests/*.json")
	if err != nil {
		return nil, nil, fmt.Errorf("list embedded provider manifests: %w", err)
	}
	byID := make(map[string]Definition, len(paths))
	for _, path := range paths {
		raw, readErr := embeddedFiles.ReadFile(path)
		if readErr != nil {
			return nil, nil, fmt.Errorf("read embedded provider manifest %q: %w", path, readErr)
		}
		var definition Definition
		if decodeErr := json.Unmarshal(raw, &definition); decodeErr != nil {
			return nil, nil, fmt.Errorf("decode embedded provider manifest %q: %w", path, decodeErr)
		}
		definition.Source.Origin = OriginEmbedded
		byID[definition.ID] = definition
	}
	ordered := make([]Definition, 0, len(byID))
	for _, id := range BuiltinProviderIDs {
		if definition, ok := byID[id]; ok {
			ordered = append(ordered, definition)
		}
	}
	unknown := make([]string, 0)
	for id := range byID {
		found := false
		for _, builtin := range BuiltinProviderIDs {
			if builtin == id {
				found = true
				break
			}
		}
		if !found {
			unknown = append(unknown, id)
		}
	}
	sort.Strings(unknown)
	for _, id := range unknown {
		ordered = append(ordered, byID[id])
	}
	var diagnostics []Diagnostic
	for _, definition := range ordered {
		raw, marshalErr := json.Marshal(definition)
		if marshalErr != nil {
			return nil, nil, fmt.Errorf("encode embedded provider manifest %q: %w", definition.ID, marshalErr)
		}
		definitionDiagnostics, validateErr := ValidateDefinition(raw, DefaultAdapterCatalog())
		if validateErr != nil {
			return nil, nil, fmt.Errorf("validate embedded provider manifest %q: %w", definition.ID, validateErr)
		}
		for i := range definitionDiagnostics {
			definitionDiagnostics[i].Field = definition.ID + "." + definitionDiagnostics[i].Field
		}
		diagnostics = append(diagnostics, definitionDiagnostics...)
	}
	return ordered, diagnostics, nil
}
