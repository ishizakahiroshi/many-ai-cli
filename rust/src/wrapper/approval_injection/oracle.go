package main
import("encoding/json";"encoding/base64";"os";"path/filepath";"many-ai-cli/internal/wrapper")
type Case struct{Name string;Provider string;Body string}
type Result struct{Name string;Injected string;Removed string;Stripped string;Error bool}
func main(){var cases []Case;if err:=json.NewDecoder(os.Stdin).Decode(&cases);err!=nil{panic(err)};var results []Result
 for _,c:=range cases{root,err:=os.MkdirTemp("","synthetic-instructions-");if err!=nil{panic(err)};os.Setenv("HOME",root);os.Setenv("USERPROFILE",root)
 path:=filepath.Join(root,"AGENTS.md");body,_:=base64.StdEncoding.DecodeString(c.Body);os.WriteFile(path,body,0600)
 err=wrapper.InjectRules(c.Provider,path);r:=Result{Name:c.Name,Error:err!=nil};if err==nil{data,_:=os.ReadFile(path);r.Injected=base64.StdEncoding.EncodeToString(data);wrapper.RemoveRules(c.Provider,path);data,_=os.ReadFile(path);r.Removed=base64.StdEncoding.EncodeToString(data)};r.Stripped=base64.StdEncoding.EncodeToString(wrapper.StripInjectedBlocks(body));results=append(results,r);os.RemoveAll(root)
 };json.NewEncoder(os.Stdout).Encode(results)}
