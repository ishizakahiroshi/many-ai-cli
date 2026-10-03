//go:build ignore

// Fixed Go source handshake constructors; no runtime Hub is started.
package main

import (
    "encoding/json"
    "os"
    "many-ai-cli/internal/proto"
)
func main() {
    frames:=map[string]proto.Message{
        "reattach_ack":{Type:"reattach_ack",SessionID:1},
        "invalid_id":{Type:"reattach_reject",Reason:"invalid session_id"},
        "dismissed":{Type:"reattach_reject",SessionID:1,Reason:"session dismissed"},
        "invalid_replay":{Type:"reattach_reject",SessionID:1,Reason:"invalid replay_b64"},
    }
    e:=json.NewEncoder(os.Stdout);e.SetIndent("","  ");if err:=e.Encode(frames);err!=nil{panic(err)}
}
