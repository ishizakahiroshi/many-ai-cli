from pathlib import Path
import json
root=Path.cwd();out=root/'rust/tests/fixtures/subscription-probe'
s=(root/'internal/hub/usage_probe.go').read_text();other=(root/'internal/hub/orchestration.go').read_text()
def fn(src,start):
 a=src.index(start);return src[a:src.index('\n}\n',a)+3]
pieces=[]
for name in ['usageProbeConfirmNeedles','usageProbeDialogTargets']:
 a=s.index('var '+name+' =');pieces.append(s[a:s.index('\n}',a)+2])
for start in ['func usageProbeDialogKey(','func usageProbeIsCursorLine(','func absInt(','func usageProbeConfirmDialog(']:pieces.append(fn(s,start))
pieces.append(fn(other,'func collapseWhitespace('))
program='package main\nimport("strings";"unicode";"encoding/json";"os")\n'+'\n'.join(pieces)+r''' 
func main(){data,e:=os.ReadFile(os.Args[1]);if e!=nil{panic(e)};var cases [][]string;if e=json.Unmarshal(data,&cases);e!=nil{panic(e)};out:=[]any{};for _,lines:=range cases{key,ok:=usageProbeDialogKey(lines);out=append(out,map[string]any{"key":key,"ok":ok,"confirm":usageProbeConfirmDialog(strings.Join(lines,""))})};data,e=json.MarshalIndent(out,"","  ");if e!=nil{panic(e)};if e=os.WriteFile(os.Args[2],data,0600);e!=nil{panic(e)}}
'''
(out/'oracle.go').write_text(program)
print(len(json.loads((out/'cases.json').read_text())), 'synthetic probe oracle cases')
