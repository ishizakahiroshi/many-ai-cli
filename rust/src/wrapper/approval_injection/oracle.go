package main

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"path/filepath"

	"many-ai-cli/internal/wrapper"
)

type Case struct {
	Name     string
	Provider string
	Body     string
}

type Result struct {
	Name     string
	Injected string
	Removed  string
	Stripped string
	Error    bool
}

func observe(c Case) (result Result, rules []byte, err error) {
	root, err := os.MkdirTemp("", "synthetic-instructions-")
	if err != nil {
		return result, nil, err
	}
	defer func() {
		if cleanupErr := os.RemoveAll(root); err == nil {
			err = cleanupErr
		}
	}()
	if err = os.Setenv("HOME", root); err != nil {
		return result, nil, err
	}
	if err = os.Setenv("USERPROFILE", root); err != nil {
		return result, nil, err
	}
	path := filepath.Join(root, "AGENTS.md")
	body, err := base64.StdEncoding.DecodeString(c.Body)
	if err != nil {
		return result, nil, err
	}
	if err = os.WriteFile(path, body, 0o600); err != nil {
		return result, nil, err
	}
	injectErr := wrapper.InjectRules(c.Provider, path)
	result = Result{Name: c.Name, Error: injectErr != nil}
	// InjectRules calls SyncRulesFile before checking the provider. Capture the
	// actual central file, including its final newline, without reconstructing it.
	rules, err = os.ReadFile(filepath.Join(root, ".many-ai-cli", "approval-rules.md"))
	if err != nil {
		return result, nil, err
	}
	if injectErr == nil {
		var data []byte
		data, err = os.ReadFile(path)
		if err != nil {
			return result, nil, err
		}
		result.Injected = base64.StdEncoding.EncodeToString(data)
		if err = wrapper.RemoveRules(c.Provider, path); err != nil {
			return result, nil, err
		}
		data, err = os.ReadFile(path)
		if err != nil {
			return result, nil, err
		}
		result.Removed = base64.StdEncoding.EncodeToString(data)
	}
	result.Stripped = base64.StdEncoding.EncodeToString(wrapper.StripInjectedBlocks(body))
	return result, rules, nil
}

func run() error {
	rulesOutput := flag.String("rules-output", "", "absolute output path for exact central rules bytes")
	flag.Parse()
	if !filepath.IsAbs(*rulesOutput) || flag.NArg() != 0 {
		return fmt.Errorf("--rules-output must be an absolute output path")
	}
	var cases []Case
	if err := json.NewDecoder(os.Stdin).Decode(&cases); err != nil {
		return err
	}
	if len(cases) == 0 {
		return fmt.Errorf("at least one synthetic case is required")
	}
	results := make([]Result, 0, len(cases))
	var central []byte
	for _, c := range cases {
		result, rules, err := observe(c)
		if err != nil {
			return fmt.Errorf("observe %s: %w", c.Name, err)
		}
		if central == nil {
			central = rules
		} else if !bytes.Equal(central, rules) {
			return fmt.Errorf("central rules differed between synthetic cases")
		}
		results = append(results, result)
	}
	if err := os.WriteFile(*rulesOutput, central, 0o600); err != nil {
		return err
	}
	return json.NewEncoder(os.Stdout).Encode(results)
}

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
