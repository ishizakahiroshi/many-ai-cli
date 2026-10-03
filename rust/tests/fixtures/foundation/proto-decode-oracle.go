package main

import (
	"encoding/base64"
	"encoding/json"
	"many-ai-cli/internal/proto"
	"os"
	"strings"
)

func main() {
	inputs := []string{
		`{"TYPE":"register","PID":123}`,
		`{"type":"first","type":"last"}`,
		`{"type":"first","TYPE":"last"}`,
		`{"TYPE":"first","type":"last"}`,
		`{"type":"kept","type":null,"pid":7,"PID":null}`,
		`{"activity":{"output_idle":true},"ACTIVITY":{"workflow_active":true}}`,
		`{"activity":{"output_idle":true},"activity":null}`,
		`{"data":"AB=="}`, `{"data":[1,null,255]}`,
		`{"messages":[null,{"role":null,"tools":[null]}]}`,
		`{"spawn_child_approval":{"claude":null}}`,
		`{"spawn_child_approval":{"claude":{"x":{"tier":"full"}}},"SPAWN_CHILD_APPROVAL":{"codex":{}}}`,
		`{"pid":1e0}`, `{"pid":9223372036854775808}`, `{"data":"bad!"}`,
		`{"rows":"12"}`, `null`, `{"unknown":{"future":[1,2,3]},"token_statusbar":false}`,
		`{"ſeſſion_id":7,"type":"synthetic"}`,
		`{"pid":"bad","pid":1}`,
		`{"pid":1.0,"pid":2}`,
		`{"activity":7,"activity":{"output_idle":true}}`,
		`{"future":1e1000,"type":"register"}`,
		`{"future":{"deep":[-1e9999,{"x":1e1000}]},"type":"register"}`,
		`{"type":"\ud800"}`,
		`{"type":"\udfff"}`,
		`{"type":"\ud800\ud800\udc00"}`,
		`{"type":"\ud83d\ude80"}`,
		`{"type":"\\ud800"}`,
		`{"\ud800":1e999,"type":"register"}`,
		`{"type":"\ud800x\udc00"}`,
		`{"type":"\ud800\u0041"}`,
		`{"type":"\ud800\x41"}`,
		`{"type":"\uZZZZ"}`,
		`{"pid":-0}`,
		`{"pid":-0.0}`,
		`{"pid":1e1000,"pid":1}`,
		`{"data":"bad!","data":"AA=="}`,
		`{"data":[256],"data":[1]}`,
		`{"data":[1.0],"data":[1]}`,
		`{"activity":{"output_idle":"bad"},"activity":null}`,
		`{"activity":{"output_idle":true},"activity":{"output_idle":null}}`,
		`{"messages":[{"tools":[7]}],"messages":[]}`,
		`{"spawn_child_approval":{"claude":7},"spawn_child_approval":{"claude":{}}}`,
		`{"spawn_child_approval":{"claude":{"x":{"tier":"full"}}},"spawn_child_approval":{"claude":{"y":{"tier":"readonly"}}}}`,
		`{"future":01,"type":"register"}`,
		`{"future":1e,"type":"register"}`,
		`{"future":[1,],"type":"register"}`,
		`{"future":"\ud800","type":"register"}`,
		"{\"type\":\"a\xffb\xc0\xaf\xe2\x82\"}",
		"{\"type\":\"ok\",\"\xff\":true}",
		"{\"type\":\"ok\"}\xff",

		`{"messages":[{"role":"assistant","text":"old"}],"messages":[{"text":"new"}]}`,
		`{"messages":[{"role":"assistant"},{"role":"user"}],"messages":[null]}`,
		`{"messages":[{"role":"assistant"},{"role":"user"}],"messages":[{}],"messages":[{},{}]}`,
		`{"messages":[{"role":"assistant"}],"messages":[],"messages":[{}]}`,
		`{"messages":[{"role":"assistant"}],"messages":null,"messages":[{}]}`,
		`{"data":[1,2],"data":[null]}`,
		`{"data":"AQI=","data":[null]}`,
		`{"replay_epoch":-0}`, `{"cost_usd":1e1000,"cost_usd":1}`,
	}
	inputs = append(inputs, `{"future":`+strings.Repeat("[", 9999)+"0"+strings.Repeat("]", 9999)+"}", `{"future":`+strings.Repeat("[", 10000)+"0"+strings.Repeat("]", 10000)+"}")
	out := []map[string]any{}
	for _, input := range inputs {
		var m proto.Message
		err := json.Unmarshal([]byte(input), &m)
		row := map[string]any{"input": input, "input_base64": base64.StdEncoding.EncodeToString([]byte(input)), "error": err != nil}
		if err == nil {
			row["value"] = m
		}
		out = append(out, row)
	}
	e := json.NewEncoder(os.Stdout)
	e.SetIndent("", "  ")
	if e.Encode(out) != nil {
		os.Exit(1)
	}
}
