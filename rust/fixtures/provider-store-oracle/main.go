// Generate byte/hash compatibility evidence with the frozen Go provider types.
// Run only in the isolated migration checkout; no user roots or provider calls.
package main

import (
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"many-ai-cli/internal/provider"
	"os"
)

func main() {
	enabled := false
	definitions := []provider.Definition{
		{SchemaVersion: 1, ID: "fixture-cli", DisplayName: "CLI <one> & two\u2028三", Description: "ordered fields", Enabled: &enabled, Launch: &provider.LaunchDefinition{Executable: "fixture-cli", Args: []string{"--agent"}}, Capabilities: map[string]bool{"usage": false, "launch": true}, Source: provider.SourceRef{Origin: provider.OriginUser, Version: "v1", Digest: "example", Revision: "parent"}},
		{ID: "claude", Enabled: &enabled},
	}
	rows := make([]map[string]any, 0, len(definitions))
	for _, definition := range definitions {
		raw, err := json.Marshal(definition)
		if err != nil {
			panic(err)
		}
		sum := sha256.Sum256(raw)
		digest := hex.EncodeToString(sum[:])
		at := "2026-10-03T00:01:02.123456789Z"
		revision := sha256.Sum256([]byte(definition.ID + "\nparent\n" + digest + "\n" + at))
		rows = append(rows, map[string]any{"definition": definition, "canonical": string(raw), "digest": digest, "parent": "parent", "created_at": at, "revision": hex.EncodeToString(revision[:])[:24]})
	}
	embedded, _, err := provider.EmbeddedDefinitions()
	if err != nil {
		panic(err)
	}
	registry, diagnostics := provider.Build(provider.Layers{Embedded: embedded, User: definitions[:1]}, provider.DefaultAdapterCatalog())
	distributed := provider.Definition{SchemaVersion: 1, ID: "distributed-fixture", DisplayName: "Distribution <fixture>", Launch: &provider.LaunchDefinition{Executable: "distribution-fixture"}}
	payload := provider.BuildDistributionPayload([]provider.Definition{distributed}, "fixture-v1", "2026-10-03T00:01:02Z", "")
	key := ed25519.NewKeyFromSeed(make([]byte, ed25519.SeedSize))
	bundle, err := provider.SignDistributionPayload(payload, "synthetic-fixture-key", key)
	if err != nil {
		panic(err)
	}
	bundleRaw, err := json.Marshal(bundle)
	if err != nil {
		panic(err)
	}
	bundleHash := sha256.Sum256(bundleRaw)
	out := map[string]any{"rows": rows, "registry_revision": registry.Revision(), "registry_diagnostics": diagnostics, "accepted_bundle": string(bundleRaw), "accepted_digest": hex.EncodeToString(bundleHash[:])}
	raw, err := json.MarshalIndent(out, "", "  ")
	if err != nil {
		panic(err)
	}
	fmt.Fprintln(os.Stdout, string(raw))
}
