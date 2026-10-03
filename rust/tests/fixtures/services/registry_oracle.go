//go:build ignore

// Offline synthetic service oracle. Does not access vendor homes or run CLIs.
package main

import (
	"encoding/json"
	"many-ai-cli/internal/provider"
	"os"
)

type registryCase struct {
	Name        string                         `json:"name"`
	Layers      provider.Layers                `json:"layers"`
	Revision    string                         `json:"revision"`
	List        []provider.Summary             `json:"list"`
	Definitions []provider.EffectiveDefinition `json:"definitions"`
	Diagnostics []provider.Diagnostic          `json:"diagnostics"`
}
type validationCase struct {
	Name        string                `json:"name"`
	Raw         string                `json:"raw"`
	Diagnostics []provider.Diagnostic `json:"diagnostics"`
	Rejected    bool                  `json:"rejected"`
}

func main() {
	embedded, _, err := provider.EmbeddedDefinitions()
	if err != nil {
		panic(err)
	}
	custom := provider.Definition{SchemaVersion: 1, ID: "synthetic", DisplayName: "日本語 & <Test>", Launch: &provider.LaunchDefinition{Executable: "synthetic-cli", Args: []string{"--safe"}}, Adapters: provider.AdapterRefs{Launch: "launch:generic-v1"}}
	off := false
	cases := []registryCase{
		{Name: "empty"},
		{Name: "embedded", Layers: provider.Layers{Embedded: embedded}},
		{Name: "custom", Layers: provider.Layers{User: []provider.Definition{custom}}},
		{Name: "builtin_collision", Layers: provider.Layers{Embedded: embedded, User: []provider.Definition{{ID: "codex", DisplayName: "blocked"}}}},
		{Name: "override", Layers: provider.Layers{Embedded: embedded, Overrides: []provider.Definition{{ID: "codex", Enabled: &off, Launch: &provider.LaunchDefinition{Args: []string{"--custom"}}}}}},
		{Name: "legacy_replaced", Layers: provider.Layers{Legacy: []provider.Definition{custom}, User: []provider.Definition{{ID: "synthetic", DisplayName: "explicit"}}}},
		{Name: "duplicate_missing", Layers: provider.Layers{User: []provider.Definition{custom, custom, {DisplayName: "missing"}}}},
		{Name: "accepted_preferred", Layers: provider.Layers{Embedded: embedded, Distribution: []provider.Definition{{ID: "codex", DisplayName: "ignored"}}, AcceptedDistribution: []provider.Definition{{ID: "codex", DisplayName: "accepted"}}}},
	}
	for i := range cases {
		c := &cases[i]
		r, d := provider.Build(c.Layers, provider.DefaultAdapterCatalog())
		c.Revision = r.Revision()
		c.List = r.List()
		c.Diagnostics = d
		c.Definitions = []provider.EffectiveDefinition{}
		for _, s := range c.List {
			v, _ := r.Lookup(s.ID)
			c.Definitions = append(c.Definitions, v)
		}
	}
	raws := []string{
		`{"schema_version":1,"id":"synthetic","display_name":"Synthetic","launch":{"executable":"fake"}}`,
		`{"SCHEMA_VERSION":1,"ID":"synthetic","display_name":"Synthetic","launch":{"executable":"fake"},"unknown":1e1000}`,
		`{"schema_version":1,"id":"shell","display_name":" ","launch":{"executable":"bad;run","args":["{bad}"],"model_args":["{model}"],"allowed_env":["A=bad"]},"presentation":{"icon_text":"abc","color":"red"},"update":{"enabled":true,"timeout_seconds":3601}}`,
		`{"schema_version":1,"id":"synthetic","display_name":"Synthetic","launch":{"executable":"fake"},"presentation":{"icon_text":"👩‍💻","color":"#12aBcD"}}`,
		`{"schema_version":1,"id":"synthetic","display_name":"Synthetic","launch":{"executable":"fake"},"presentation":{"icon_text":"🇯🇵","color":""}}`,
		`{"schema_version":1,"id":"synthetic","display_name":"Synthetic","launch":{"executable":"fake"},"models":{"items":[{"id":"bad/id","label":""},{"id":"bad/id","label":"Again"}]},"adapters":{"usage":"unknown"},"capabilities":{"arbitrary":true}}`,
		`null`,
		`{"id":12,"id":"synthetic"}`,
		`{"id":"synthetic"} {"extra":true}`,
	}
	validators := []validationCase{}
	for i, raw := range raws {
		d, e := provider.ValidateDefinition([]byte(raw), provider.DefaultAdapterCatalog())
		validators = append(validators, validationCase{Name: string(rune('a' + i)), Raw: raw, Diagnostics: d, Rejected: e != nil})
	}
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", "  ")
	if err := enc.Encode(map[string]any{"registry": cases, "validation": validators}); err != nil {
		panic(err)
	}
}
