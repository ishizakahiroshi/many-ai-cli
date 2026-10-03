//go:build ignore

// Synthetic Go Decoder.Decode contracts for the shared HTTP body boundary.
package main

import (
	"encoding/json"
	"os"
	"strings"
)

type nested struct {
	Enabled bool   `json:"enabled"`
	Text    string `json:"text"`
}
type body struct {
	Count  int      `json:"count"`
	Nested *nested  `json:"nested"`
	Names  []string `json:"names"`
}

func main() {
	inputs := []string{
		`{}`, `null`, `{"COUNT":3,"unknown":1e9999}`, `{"count":1} {"count":2}`, `{"count":1} malformed`,
		`nullgarbage`, `{"count":1,"count":null}`, `{"count":"bad","count":1}`,
		`{"nested":{"enabled":true},"NESTED":{"text":"preserved"}}`,
		`{"nested":{"enabled":true},"nested":null}`, `{"names":["first","second"],"names":[null],"names":[null,null]}`,
		`{"names":["first"],"names":[],"names":[null]}`, `{"nested":{"text":"\ud800"}}`,
		`{"nested":7,"nested":{}}`, ``, ` `, `[1]`, `{"count":1`, `{"count":1.0}`, `{"count":-0}`,
	}
	out := []map[string]any{}
	for _, input := range inputs {
		var b body
		err := json.NewDecoder(strings.NewReader(input)).Decode(&b)
		row := map[string]any{"input": input, "error": err != nil}
		if err == nil {
			row["value"] = b
		}
		out = append(out, row)
	}
	e := json.NewEncoder(os.Stdout)
	e.SetIndent("", "  ")
	if e.Encode(out) != nil {
		os.Exit(1)
	}
}
