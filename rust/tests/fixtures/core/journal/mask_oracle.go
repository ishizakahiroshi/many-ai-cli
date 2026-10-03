//go:build ignore

// Run from the repository with the fixed Go source oracle. Synthetic bytes only.
package main

import (
    "encoding/base64"
    "encoding/json"
    "os"
    "many-ai-cli/internal/sessionlog"
)
func main() {
    inputs := [][]byte{
        []byte("ordinary\xff\xfe tail"),
        []byte("split emoji \xf0\x9f"),
        []byte("bad scalar \xed\xa0\x80 end"),
        []byte("API_KEY=ab\xffcd \xfe unmatched"),
        []byte("TOKEN=abcde\xff untouched"),
        []byte("PASSWORD=界界界界界"),
        []byte("PASSWORD=界界界界界界"),
        []byte("PASSWORD=界界界界界\xff"),
        []byte("Bearer 界界界界界界界"),
        []byte("Bearer 界界界界界界界\xfe"),
        []byte("https://name:pa\xffss@host/path \xff"),
        []byte("API_KEY=a\xffb\nBearer 1234567\xff tail\xf0\x9f"),
        []byte("\x1b[31mAPI_KEY=value\xff\x1b[0m\r\n"),
        []byte("\xee\x80\x80 \xf3\xb0\x80\x80 \xff"),
        []byte("-----BEGIN PRIVATE KEY-----\nabc\xff\n-----END PRIVATE KEY----- tail\xff"),
        []byte("PASSWORD=ab\xe3\x81cd"),
    }
    type Case struct { Input string `json:"input_b64"`; Masked string `json:"masked_b64"`; Text string `json:"text"` }
    result:=make([]Case,0,len(inputs))
    for _,input:=range inputs { masked:=sessionlog.MaskSecrets(string(input)); result=append(result,Case{base64.StdEncoding.EncodeToString(input),base64.StdEncoding.EncodeToString([]byte(masked)),sessionlog.StripANSI(masked)}) }
    encoder:=json.NewEncoder(os.Stdout);encoder.SetIndent("","  ");if err:=encoder.Encode(result);err!=nil{panic(err)}
}
