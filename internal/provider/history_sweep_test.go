package provider

import (
	"encoding/json"
	"fmt"
	"strings"
	"testing"
)

type overrideFieldEdit struct {
	path  []string
	value any
	label string
}

// collectOverrideFieldEdits lists one edit per scalar/array leaf of a
// provider definition, the way the settings form changes a single field.
func collectOverrideFieldEdits(prefix []string, value any, out *[]overrideFieldEdit) {
	switch v := value.(type) {
	case map[string]any:
		for key, child := range v {
			collectOverrideFieldEdits(append(append([]string(nil), prefix...), key), child, out)
		}
	case bool:
		*out = append(*out, overrideFieldEdit{prefix, !v, "flip"})
	case string:
		*out = append(*out, overrideFieldEdit{prefix, "changed-value", "set"})
	case float64:
		*out = append(*out, overrideFieldEdit{prefix, v + 1, "inc"})
	case []any:
		if len(v) > 0 {
			if _, isObject := v[0].(map[string]any); isObject {
				return
			}
		}
		*out = append(*out, overrideFieldEdit{prefix, []any{"changed"}, "set"})
	}
}

func setOverrideField(root map[string]any, path []string, value any) {
	current := root
	for _, key := range path[:len(path)-1] {
		next, ok := current[key].(map[string]any)
		if !ok {
			next = map[string]any{}
			current[key] = next
		}
		current = next
	}
	current[path[len(path)-1]] = value
}

// TestSaveEffectiveOverrideAcceptsEverySingleFieldEdit edits each field of
// every built-in manifest one at a time and saves it as the settings form
// does. An edit that is valid as a whole definition must save and load back.
// It exists because the stored override is only the delta from the manifest,
// and readRevision re-validates that delta without the manifest: rules that
// need a sibling field (update.enabled needs update.args, launch.headless
// needs format) rejected legitimate one-field edits with 422 and left the
// revision written but HEAD unmoved.
func TestSaveEffectiveOverrideAcceptsEverySingleFieldEdit(t *testing.T) {
	definitions, _, err := EmbeddedDefinitions()
	if err != nil {
		t.Fatal(err)
	}
	checked := 0
	for _, baseline := range definitions {
		baseline.Source = SourceRef{}
		raw, err := json.Marshal(baseline)
		if err != nil {
			t.Fatal(err)
		}
		var baseMap map[string]any
		if err := json.Unmarshal(raw, &baseMap); err != nil {
			t.Fatal(err)
		}
		var edits []overrideFieldEdit
		collectOverrideFieldEdits(nil, baseMap, &edits)
		// update.* keys a manifest may omit but the form can still set.
		edits = append(edits,
			overrideFieldEdit{[]string{"update", "enabled"}, true, "set-true"},
			overrideFieldEdit{[]string{"update", "enabled"}, false, "set-false"},
			overrideFieldEdit{[]string{"update", "args"}, []any{"upgrade"}, "set"},
			overrideFieldEdit{[]string{"update", "version_args"}, []any{"-v"}, "set"},
			overrideFieldEdit{[]string{"update", "executable"}, "changed-value", "set"},
			overrideFieldEdit{[]string{"update", "timeout_seconds"}, float64(60), "set"},
		)
		for _, edit := range edits {
			switch edit.path[0] {
			case "id", "schema_version", "source", "adapters":
				continue
			}
			name := fmt.Sprintf("%s %s=%s", baseline.ID, strings.Join(edit.path, "."), edit.label)
			desiredMap := map[string]any{}
			if err := json.Unmarshal(raw, &desiredMap); err != nil {
				t.Fatal(err)
			}
			setOverrideField(desiredMap, edit.path, edit.value)
			desiredRaw, err := json.Marshal(desiredMap)
			if err != nil {
				t.Fatal(err)
			}
			var desired Definition
			if err := json.Unmarshal(desiredRaw, &desired); err != nil {
				continue
			}
			normalized, _ := json.Marshal(desired)
			if diagnostics, _ := ValidateDefinition(normalized, DefaultAdapterCatalog()); hasDiagnosticError(diagnostics) {
				continue
			}
			checked++
			store, err := NewHistoryStore(t.TempDir(), t.TempDir())
			if err != nil {
				t.Fatal(err)
			}
			if _, err := store.SaveEffectiveOverride(baseline.ID, desired, baseline, "", "edit"); err != nil {
				t.Errorf("%s: save failed: %v", name, err)
				continue
			}
			overrides, diagnostics, err := store.LoadOverrides()
			if err != nil || len(overrides) != 1 || hasDiagnosticError(diagnostics) {
				t.Errorf("%s: load failed: err=%v overrides=%d diagnostics=%v", name, err, len(overrides), diagnostics)
			}
		}
	}
	if checked < 100 {
		t.Fatalf("sweep checked only %d edits; the manifests or the edit generator changed shape", checked)
	}
}
