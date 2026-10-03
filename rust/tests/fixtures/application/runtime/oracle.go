//go:build ignore

// Emits synthetic decoder cases using the unchanged internal/hubruntime Data.
package main

import (
    "encoding/json"
    "os"
    "many-ai-cli/internal/hubruntime"
)
func main() {
    inputs := []string{
        `{}`, `null`, `{"pid":42,"port":48888}`, `{"PID":42,"PORT":48889,"version":1,"started_at":"2024-02-03T04:05:06.123+09:00"}`,
        `{"pid":42,"port":70000}`, `{"pid":0,"port":48888}`, `{"pid":-1,"port":48888}`, `{"pid":42,"port":0}`,
        `{"pid":42,"port":48888,"started_at":null}`, `{"pid":"bad","pid":42,"port":48888}`,
        `{"pid":42,"port":48888,"started_at":"bad"}`, `{"pid":42,"port":48888,"future":1e1000}`,
        `{"pid":42,"port":48888} {}`, `{"pid":42,"port":48888,"started_at":"2024-02-03T04:05:06Z","started_at":null}`,
    }
    output := []map[string]any{}
    for _, input := range inputs {
        var data hubruntime.Data
        err := json.Unmarshal([]byte(input), &data)
        row := map[string]any{"input":input,"error":err!=nil}
        if err==nil { row["value"]=data; row["valid"]=data.PID>0&&data.Port>0 }
        output=append(output,row)
    }
    encoder:=json.NewEncoder(os.Stdout); encoder.SetIndent("","  "); if err:=encoder.Encode(output);err!=nil { panic(err) }
}
