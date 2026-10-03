// Offline yaml.v3 regression oracle, Go baseline 21d0bc7.
// Run at repository root: go run ./rust/tests/fixtures/foundation/yaml/oracle.go
// Reads no user files and starts no providers or services.
package main

import (
	"encoding/json"
	"fmt"
	"gopkg.in/yaml.v3"
	"many-ai-cli/internal/config"
	"os"
	"strings"
)

type fixture struct {
	Name                         string         `json:"name"`
	YAML                         string         `json:"yaml"`
	Paths                        []string       `json:"paths"`
	Expected                     map[string]any `json:"expected"`
	Rejected                     bool           `json:"rejected,omitempty"`
	SavedHasHallucinationPhrases bool           `json:"saved_has_hallucination_phrases"`
}

func ptr(root any, path string) any {
	for _, part := range strings.Split(strings.TrimPrefix(path, "/"), "/") {
		object, ok := root.(map[string]any)
		if !ok {
			return nil
		}
		root = object[part]
	}
	return root
}
func main() {
	base := func(extra string) string { return "token: synthetic-stable-token\n" + extra }
	var fixtures []fixture
	add := func(name, body string, paths ...string) {
		fixtures = append(fixtures, fixture{Name: name, YAML: body, Paths: paths})
	}
	add("string-lexemes-and-octal-int", "token: 00123\nauth_cookie_secret: 0xFF\nremote_pin_hash: TRUE\nhub: {port: 0123}\n", "/token", "/auth_cookie_secret", "/remote_pin_hash", "/Hub/Port")
	add("exponent-lexemes-and-map-keys", "token: 1e3\nauth_cookie_secret: 1_234\nremote_pin_hash: .125\nuser_prefs:\n  spawn:\n    last_model: {00123: 0xFF, TRUE: 1e3, 1e3: TRUE, nullish: null}\n", "/token", "/auth_cookie_secret", "/remote_pin_hash", "/user_prefs/spawn/last_model")
	add("quoted-lexemes", "token: '00123'\nauth_cookie_secret: \"0xFF\"\nremote_pin_hash: 'TRUE'\n", "/token", "/auth_cookie_secret", "/remote_pin_hash")
	add("string-tags", "token: !!str 00123\nauth_cookie_secret: !future 0xFF\nremote_pin_hash: !!str null\n", "/token", "/auth_cookie_secret", "/remote_pin_hash")
	add("explicit-numeric-tags-keep-string-lexemes", "token: !!int '00123'\nauth_cookie_secret: !!float 1e3\nremote_pin_hash: !!bool TRUE\nhub: {port: !!int '0123'}\n", "/token", "/auth_cookie_secret", "/remote_pin_hash", "/Hub/Port")
	add("binary-string", "token: !!binary c3ludGhldGljLWJpbmFyeQ==\n", "/token")
	add("block-strings", "token: |\n  synthetic line one\n  line two\nauth_cookie_secret: >-\n  folded synthetic\n  text\n", "/token", "/auth_cookie_secret")
	add("unknown-yaml-only-values", base("future_nan: .nan\nfuture_inf: -.inf\nfuture_complex: {[a, b]: x}\nhub: {port: 48888, future: {[x, y]: .nan}}\n"), "/token", "/Hub/Port")
	add("unknown-invalid-tag-is-not-decoded", base("future_tag: !!int not-a-number\nfuture_binary: !!binary not-base64!\n"), "/token")
	add("unused-alias-cycle", base("future: &unused {self: *unused}\n"), "/token")
	add("unknown-duplicate-subtree", base("future: {a: 1, a: 2}\n"), "/token")
	add("null-slice-elements", base("voice:\n  whisper:\n    hallucination_phrases: [null, 00123, ~, TRUE, 'null', '', 0xFF]\nsubscriptions:\n  claude:\n    - id: work\n      profile_owned_keys: [null, 00123, TRUE, ~, 'null']\n      default_wins_keys: [null, theme]\n"), "/token", "/voice/whisper/hallucination_phrases", "/subscriptions")
	add("all-null-string-list", base("voice: {whisper: {hallucination_phrases: [null, ~]}}\n"), "/voice/whisper/hallucination_phrases")
	add("legacy-session-order-scalars", base("user_prefs: {session_order: [0123, 08, '0123', 1e3, 1.5, null, .nan, 0xFF, ' 7 ', TRUE, {}, -2]}\n"), "/user_prefs/session_order")
	add("integer-numeric-forms", base("hub: {port: 0xFF}\nlog: {max_size_mb: 0123, max_backups: 0b11, session_retention_days: 1e3, session_max_size_mb: 7.9}\n"), "/Hub/Port", "/Log/max_size_mb", "/Log/max_backups", "/Log/session_retention_days", "/Log/session_max_size_mb")
	add("integer-legacy-invalid-octal-falls-back-float", base("hub: {port: 08}\n"), "/Hub/Port")
	add("boolean-legacy-and-explicit", base("hub: {open_browser: 'off'}\nlog: {enabled: Yes, compress: !!bool 'TRUE'}\n"), "/Hub/OpenBrowser", "/Log/enabled", "/Log/compress")
	add("boolean-null-leaves-existing-default", base("hub: {open_browser: null}\n"), "/Hub/OpenBrowser")
	add("anchor-scalars-use-destination-type", "future: &value 0123\ntoken: *value\nauth_cookie_secret: *value\nhub: {port: *value}\n", "/token", "/auth_cookie_secret", "/Hub/Port")
	add("anchor-strings-with-nan", "future: &value .nan\ntoken: *value\n", "/token")
	add("mapping-merge-explicit-wins", base("defaults: &defaults {port: 0123, open_browser: false, future: .nan}\nhub: {<<: *defaults, port: 48888}\n"), "/Hub/Port", "/Hub/OpenBrowser")
	add("mapping-merge-sequence-first-wins", base("first: &first {port: 0123, open_browser: false}\nsecond: &second {port: 49999, open_browser: true}\nhub: {<<: [*first, *second]}\n"), "/Hub/Port", "/Hub/OpenBrowser")
	add("mapping-merge-later-position", base("defaults: &defaults {port: 0123, open_browser: false}\nhub: {port: 48888, <<: *defaults}\n"), "/Hub/Port", "/Hub/OpenBrowser")
	add("anchor-sequence-and-tolerant-profile", base("keys: &keys [00123, null, TRUE, {}, theme]\nsubscriptions:\n  claude:\n    - id: 00123\n      profile_owned_keys: *keys\n      default_wins_keys: *keys\n      enabled: invalid\n      future: {[x,y]: .nan}\n"), "/subscriptions")
	add("custom-providers-historical-scalars", base("custom_providers:\n  - {id: 00123, command: 0xFF, label: TRUE, headless: {args: [null, 00123], format: text}}\n  - {id: bad, command: fake, headless: {args: [.nan, null], format: text}}\n  - {id: wrong, command: [fake]}\n"), "/custom_providers")
	add("tolerant-malformed-optional-sections", base("custom_providers: {[a,b]: .nan}\nsubscriptions: {[x,y]: .nan}\nuser_prefs: {session_order: .nan}\n"), "/token", "/custom_providers", "/subscriptions", "/user_prefs/session_order")
	add("duplicate-top-level-known", base("hub: {port: 48888}\nhub: {port: 48889}\n"), "/token")
	add("duplicate-top-level-unknown", base("future: 1\nfuture: 2\n"), "/token")
	add("malformed-known-int-quoted", base("hub: {port: '0123'}\n"), "/token")
	add("malformed-known-int-tag", base("hub: {port: !!int invalid}\n"), "/token")
	add("malformed-known-string-tag", "token: !!int invalid\n", "/token")
	add("malformed-known-string-map", "token: {[a,b]: x}\n", "/token")
	add("malformed-known-bool-quoted-true", base("hub: {open_browser: 'true'}\n"), "/token")
	add("malformed-known-nan-int", base("hub: {port: .nan}\n"), "/token")
	add("malformed-known-map-key", base("user_prefs: {spawn: {last_model: {[a,b]: x}}}\n"), "/token")
	add("malformed-merge-target", base("hub: {<<: [scalar]}\n"), "/token")
	add("malformed-syntax", base("hub: {port: [}\n"), "/token")
	add("first-document-only", base("hub: {port: 48888}\n---\ntoken: later-document\n"), "/token", "/Hub/Port")
	add("timestamp-tags-valid", "token: !!timestamp 2026-2-3\nauth_cookie_secret: !!timestamp 2026-10-03T04:50:00Z\n", "/token", "/auth_cookie_secret")
	add("timestamp-tag-malformed", "token: !!timestamp not-a-date\n", "/token")
	add("session-order-complex-key-discards-list", base("user_prefs: {session_order: [1, {[a,b]: x}, 2]}\n"), "/user_prefs/session_order")
	add("custom-null-pointer-elements", base("custom_providers: [null, {id: fake, command: fake-cli}]\nsubscriptions: {claude: [null, {id: work, enabled: null}]}\n"), "/custom_providers", "/subscriptions")
	for i := range fixtures {
		cfg := config.Config{}
		cfg.Hub.OpenBrowser = true
		cfg.Hub.Port = 47777
		err := yaml.Unmarshal([]byte(fixtures[i].YAML), &cfg)
		fixtures[i].Rejected = err != nil
		if err != nil {
			continue
		}
		saved, err := yaml.Marshal(cfg)
		if err != nil {
			panic(err)
		}
		fixtures[i].SavedHasHallucinationPhrases = strings.Contains(string(saved), "hallucination_phrases:")
		bytes, err := json.Marshal(cfg)
		if err != nil {
			panic(err)
		}
		var projection map[string]any
		if err = json.Unmarshal(bytes, &projection); err != nil {
			panic(err)
		}
		projection["token"] = cfg.Token
		projection["auth_cookie_secret"] = cfg.AuthCookieSecret
		projection["remote_pin_hash"] = cfg.RemotePINHash
		fixtures[i].Expected = map[string]any{}
		for _, path := range fixtures[i].Paths {
			fixtures[i].Expected[path] = ptr(projection, path)
		}
	}
	bytes, err := json.MarshalIndent(fixtures, "", "  ")
	if err != nil {
		panic(err)
	}
	if err = os.WriteFile("rust/tests/fixtures/foundation/yaml/compat.json", append(bytes, '\n'), 0600); err != nil {
		panic(err)
	}
	fmt.Printf("wrote %d schema-aware YAML cases\n", len(fixtures))
}
