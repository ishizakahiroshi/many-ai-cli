package main
import("encoding/json";"os";"many-ai-cli/internal/proto")
func main(){
 inputs:=[]string{
 `{"TYPE":"register","PID":123}`,
 `{"type":"first","type":"last"}`,
 `{"type":"first","TYPE":"last"}`,
 `{"TYPE":"first","type":"last"}`,
 `{"type":"kept","type":null,"pid":7,"PID":null}`,
 `{"activity":{"output_idle":true},"ACTIVITY":{"workflow_active":true}}`,
 `{"activity":{"output_idle":true},"activity":null}`,
 `{"data":"AB=="}`,`{"data":[1,null,255]}`,
 `{"messages":[null,{"role":null,"tools":[null]}]}`,
 `{"spawn_child_approval":{"claude":null}}`,
 `{"spawn_child_approval":{"claude":{"x":{"tier":"full"}}},"SPAWN_CHILD_APPROVAL":{"codex":{}}}`,
 `{"pid":1e0}`,`{"pid":9223372036854775808}`,`{"data":"bad!"}`,
 `{"rows":"12"}`,`null`,`{"unknown":{"future":[1,2,3]},"token_statusbar":false}`,
 `{"ſeſſion_id":7,"type":"synthetic"}`,
 }
 out:=[]map[string]any{}
 for _,input:=range inputs{var m proto.Message;err:=json.Unmarshal([]byte(input),&m);row:=map[string]any{"input":input,"error":err!=nil};if err==nil{row["value"]=m};out=append(out,row)}
 e:=json.NewEncoder(os.Stdout);e.SetIndent("","  ");if e.Encode(out)!=nil{os.Exit(1)}
}
