package provider

import (
	"reflect"
	"testing"
)

func TestResolveLaunchBuildsArgvWithoutShellExpansion(t *testing.T) {
	enabled := true
	definition := EffectiveDefinition{
		Definition: Definition{
			ID:      "example",
			Enabled: &enabled,
			Launch: &LaunchDefinition{
				Executable: "example",
				Args:       []string{"--fixed", "$(not-shell)"},
				ModelArgs:  []string{"--model", "{model}"},
				Headless:   &HeadlessDefinition{Args: []string{"--print"}, PromptVia: "arg"},
			},
		},
		Revision: "rev-1",
	}
	got, err := ResolveLaunch(definition, LaunchRequest{Model: "fast", Prompt: "hello", Headless: true})
	if err != nil {
		t.Fatal(err)
	}
	want := []string{"--fixed", "$(not-shell)", "--print", "hello", "--model", "fast"}
	if !reflect.DeepEqual(got.Args, want) || got.Revision != "rev-1" {
		t.Fatalf("ResolvedLaunch = %#v, want args %#v and revision", got, want)
	}
}

func TestResolveLaunchRejectsUnresolvedPlaceholders(t *testing.T) {
	definition := EffectiveDefinition{Definition: Definition{ID: "example", Launch: &LaunchDefinition{Executable: "example", ModelArgs: []string{"--model", "{unknown}"}}}}
	if _, err := ResolveLaunch(definition, LaunchRequest{Model: "fast"}); err == nil {
		t.Fatal("ResolveLaunch accepted an unresolved placeholder")
	}
}
