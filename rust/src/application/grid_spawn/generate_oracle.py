"""Extract the fixed Go grid layout/spec construction; no provider execution."""
from pathlib import Path
import os
import subprocess
import tempfile

here = Path(__file__).resolve().parent
root = here.parents[3]
source = (root / "internal/hub/spawn_handler.go").read_text(encoding="utf-8")
start = source.index("func calcGridLayout(")
opening = source.index("{", start)
level = 1
end = opening + 1
while level:
    level += (source[end] == "{") - (source[end] == "}")
    end += 1
layout = source[start:end]
start = source.index("\ttype sessionSpec struct", source.index("func (s *Server) handleSpawnGrid"))
end = source.index("\n\t// セッションを順次 spawn", start)
specs = source[start:end]
program = '''package main
import("encoding/json";"fmt";"os")
''' + layout + '''
type Input struct {Preset string;Count int;LabelPrefix string;Provider string}
func main(){
out:=[]map[string]any{}
for _,preset:=range []string{"shell","ai+shell"}{
for count:=1;count<=18;count++{
for _,prefix:=range []string{"","日本語"}{
body:=Input{preset,count,prefix,"custom-ai"}
aiProvider:=body.Provider
''' + specs + '''
rows:=[][2]string{}
for _,spec:=range specs {rows=append(rows,[2]string{spec.provider,spec.label})}
out=append(out,map[string]any{"preset":preset,"count":count,"label_prefix":prefix,"provider":body.Provider,"layout":calcGridLayout(count),"specs":rows})
}}}
if err:=json.NewEncoder(os.Stdout).Encode(out);err!=nil{panic(err)}
}
'''
(here / "oracle.go").write_text(program, encoding="utf-8")
with tempfile.TemporaryDirectory(prefix="many-grid-oracle-") as temp:
    env = {key: os.environ[key] for key in ["SYSTEMROOT", "WINDIR", "TEMP", "TMP", "PATH"] if key in os.environ}
    env.update(HOME=temp,USERPROFILE=temp,GOCACHE=r"D:\.gocache",GOMODCACHE=r"D:\.gomodcache",GOTOOLCHAIN="local",GOWORK="off")
    go = root.parent / ".toolchains/go1268/go/bin/go.exe"
    result=subprocess.run([str(go),"run",str(here / "oracle.go")],cwd=temp,env=env,check=True,capture_output=True,timeout=60)
    (here / "go-specs.json").write_bytes(result.stdout)
print("72 fixed-Go grid layout/spec cases generated")
