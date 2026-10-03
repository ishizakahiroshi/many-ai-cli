// Synthetic Go encoding/json oracle. Run from the repository root.
package main
import ("encoding/json"; "fmt"; "os"; "reflect"; "many-ai-cli/internal/proto")
type Case struct { Name string `json:"name"`; Type string `json:"type"`; Value any `json:"value"` }
func fill(v reflect.Value) {
 switch v.Kind() {
 case reflect.String: v.SetString("synthetic-日本語-<&>\u2028")
 case reflect.Bool: v.SetBool(true)
 case reflect.Int,reflect.Int64: v.SetInt(17)
 case reflect.Uint64: v.SetUint(23)
 case reflect.Float64: v.SetFloat(1.25)
 case reflect.Pointer: v.Set(reflect.New(v.Type().Elem()));fill(v.Elem())
 case reflect.Slice: v.Set(reflect.MakeSlice(v.Type(),1,1));fill(v.Index(0))
 case reflect.Uint8: v.SetUint(255)
 case reflect.Map: v.Set(reflect.MakeMap(v.Type()));k:=reflect.New(v.Type().Key()).Elem();k.SetString("synthetic");x:=reflect.New(v.Type().Elem()).Elem();fill(x);v.SetMapIndex(k,x)
 case reflect.Struct:for i:=0;i<v.NumField();i++ {fill(v.Field(i))}
 default: panic(v.Kind())
 }
}
func main(){
 values:=[]any{proto.Message{},proto.CrossSessionMessage{},proto.AgentChatMessage{},proto.AgentChatTool{},proto.SessionActivity{},proto.WorkflowProgress{},proto.WfPhase{},proto.WfAgent{},proto.WfAgentDetail{},proto.SubagentTree{},proto.SubagentNode{},proto.SessionMeta{},proto.ApprovalSummary{},proto.DoneSummary{},proto.RelayStatus{},proto.RelayEvent{},proto.ApprovalOption{},proto.ApprovalRecord{},proto.ApprovalRecordClose{},proto.ApprovalState{},proto.ApprovalSessionState{},proto.ChildApproval{}}
 cases:=[]Case{}
 for _,zero:=range values {name:=reflect.TypeOf(zero).Name();cases=append(cases,Case{name+"/zero",name,zero}); v:=reflect.New(reflect.TypeOf(zero)).Elem();fill(v);cases=append(cases,Case{name+"/filled",name,v.Interface()})}
 enc:=json.NewEncoder(os.Stdout);enc.SetIndent("","  ");if err:=enc.Encode(cases);err!=nil{fmt.Fprintln(os.Stderr,err);os.Exit(1)}
}
