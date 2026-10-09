//go:build ignore
package main
import("encoding/json";"os";"unicode")
func main(){ pairs:=[][2]int32{}; for r:=rune(0);r<=unicode.MaxRune;r++ {if r>=0xD800&&r<=0xDFFF{continue}; if lower:=unicode.ToLower(r);lower!=r {pairs=append(pairs,[2]int32{r,lower})}}; json.NewEncoder(os.Stdout).Encode(struct{Version string `json:"version"`;Pairs [][2]int32 `json:"pairs"`}{unicode.Version,pairs}) }
