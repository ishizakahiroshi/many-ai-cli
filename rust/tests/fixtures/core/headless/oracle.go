//go:build ignore
// Synthetic exported-API oracle; Go source 21d0bc7935a2c4696fb89ccff2e324157a528c2d.
package main
import("encoding/json";"os";"many-ai-cli/internal/headless";"strings")
type test struct {Format string `json:"format"`; Input string `json:"input"`; Events []headless.Event `json:"events"`; Lines []string `json:"lines"`}
func main(){ inputs:=[]string{
`{"type":"system","subtype":"init","model":" synthetic "}`,
`{"type":"system","subtype":"other"}`,
`{"type":"assistant","message":{"content":[{"type":"text","text":" Hello \nworld "},{"type":"tool_use","name":"Read","input":{"file_path":"/synthetic/file","content":"BODY_NOT_RENDERED"}}]}}`,
`{"type":"assistant","message":{"content":" bare text "}}`,
`{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"HIDDEN"}]}}`,
`{"type":"user","message":{"content":[{"type":"tool_result","is_error":true,"content":[{"type":"text","text":"one"},{"type":"text","text":"two"}]}]}}`,
`{"type":"result","subtype":"success","result":" done \nother"}`,
`{"type":"result","subtype":"failure","is_error":false,"error":{"message":"failure details\nmore","stack":"NOT_RENDERED"}}`,
`{"type":"result","is_error":false}`,
`{"type":"result","error":"quota"}`,
`{"type":"result","error":{"message":""}}`,
`{"type":"assistant","message":{`,
`{"type":99}`,
`null`, `[]`, `   `,
`{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Shell","input":{"command":99,"path":"/fallback"}}]}}`,
`{"type":"result","subtype":null,"is_error":null,"result":null}`,
}; inputs=append(inputs,
`{"TYPE":"assistant","MESSAGE":{"CONTENT":"upper"}}`,
`{"type":"assistant","message":{"content":"first"},"message":{"role":"assistant"}}`,
`{"type":"assistant","message":{"content":"first"},"message":null,"message":{}}`,
`{"type":"assistant","message":{"content":[null,{"type":"text","text":"after null"}]}}`,
`{"type":"result","result":null,"RESULT":"later"}`,
`{"type":"result","result":99,"result":"later"}`,
`{"type":"assistant","message":{"content":"bad \ud800 unicode"}}`,
`{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Shell","input":{"command":"echo hi","irrelevant":1e1000}}]}}`,
`{"type":"assistant","ignored":1e1000,"message":{"content":"still accepted"}}`,
`{"type":"result","error":{"message":null}}`,
); var out []test
for _,format:=range []string{"text","claude-stream-json"}{p,_:=headless.ParserFor(format);for _,in:=range inputs{events:=p.Parse([]byte(in));var lines []string;for _,e:=range events{lines=append(lines,e.Line())};out=append(out,test{format,in,events,lines})}}
for _,in:=range []string{"first\r\nsecond\t\x1b[31mred\x7f\ufffd",strings.Repeat("日",3000),"", "\x00\x01\t"}{p,_:=headless.ParserFor("text");events:=p.Parse([]byte(in));out=append(out,test{"text",in,events,[]string{events[0].Line()}})}
enc:=json.NewEncoder(os.Stdout);enc.SetIndent("","  ");if err:=enc.Encode(out);err!=nil{panic(err)}}
