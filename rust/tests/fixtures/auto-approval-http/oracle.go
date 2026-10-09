//go:build ignore
package main
import("encoding/json";"fmt";"os")
func main(){values:=[]string{"","bogus","1tail"," +2 trailing","-1","0","999999999999999999999999","0x10","-01","\u3000+3end"};rows:=[]any{};for _,value:=range values{n:=100;if value!=""{_,_=fmt.Sscanf(value,"%d",&n)};rows=append(rows,map[string]any{"input":value,"n":n})};json.NewEncoder(os.Stdout).Encode(rows)}
