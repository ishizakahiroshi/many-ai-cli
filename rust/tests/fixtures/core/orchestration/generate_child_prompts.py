#!/usr/bin/env python3
"""Fixed-Go prompt-route observations only; never launch a provider or Hub."""
import argparse, json, re, subprocess, tempfile
from pathlib import Path
ORACLE='21d0bc7935a2c4696fb89ccff2e324157a528c2d'
def function(source,name):
    start=re.search(r'^func '+re.escape(name)+r'\(',source,re.M).start()
    line=source[start:source.index('\n',start)]
    if line.rstrip().endswith('}'):return line
    return source[start:source.index('\n}',start)+2]
def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--go',required=True);parser.add_argument('--work-root',required=True,type=Path)
    args=parser.parse_args()
    if not Path(args.go).is_absolute() or not args.work_root.is_absolute():parser.error('absolute paths required')
    repo=Path(__file__).resolve().parents[5]
    def source(path):return subprocess.check_output(['git','show',f'{ORACLE}:{path}'],cwd=repo,text=True)
    hub=source('internal/hub/orchestration.go');headless=source('internal/headless/prompt.go');config=source('internal/config/headless.go');effort=source('internal/config/effort.go')
    placeholder=re.search(r'^const SessionIDPlaceholder = (".*")$',headless,re.M).group(1)
    mode=re.search(r'^\s*ExecutionModeHeadless\s*=\s*(".*")$',effort,re.M).group(1)
    code='package main\nimport("strings";"strconv";"path/filepath";"encoding/json";"os")\n'
    code+=f'const SessionIDPlaceholder={placeholder}\nconst ExecutionModeHeadless={mode}\n'
    for name in ['childProgressPathFor','buildChildInitialPrompt','buildHeadlessChildInitialPrompt','buildChildInitialPromptFor','sanitizeInjectText','childLaunchPrompt']:
        code+=function(hub,name).replace('headless.SessionIDPlaceholder','SessionIDPlaceholder').replace('config.IsHeadlessExecutionMode','IsHeadlessExecutionMode')+'\n'
    code+=function(effort,'NormalizeExecutionMode')+'\n'+function(config,'IsHeadlessExecutionMode')+'\n'+function(headless,'ExpandPrompt')+'\n'
    code+='''type input struct{Name string `json:"name"`; Base string `json:"base"`; Mode string `json:"mode"`; ViaArg bool `json:"via_arg"`; Board string `json:"board"`; Role string `json:"role"`; Branch string `json:"branch"`; ID int `json:"id"`};func main(){var inputs []input;if e:=json.NewDecoder(os.Stdin).Decode(&inputs);e!=nil{panic(e)};var outputs []any;for _,in:=range inputs{launch,inject:=childLaunchPrompt(in.Mode,in.ViaArg,in.Base,in.Board,in.Role,in.Branch);outputs=append(outputs,map[string]any{"input":in,"launch_prompt":launch,"inject_after_start":inject,"typed_prompt":buildChildInitialPrompt(in.Base,in.Board,in.Role,in.Branch,in.ID),"expanded_launch":ExpandPrompt(launch,in.ID)})};e:=json.NewEncoder(os.Stdout);e.SetIndent("","  ");if err:=e.Encode(outputs);err!=nil{panic(err)}}'''
    cases=[]
    for mode in ['', 'interactive','headless',' headless ']:
        for via in [False,True]:
            for name,base in [('plain','Synthetic child task.\n'),('controls','one\r\ntwo\r\t\x07\x1b[31m\x7f世界\n'),('placeholder','Caller literal {{many-ai-cli:session-id}}.')]:
                cases.append(dict(name=f'{mode}-{via}-{name}',base=base,mode=mode,via_arg=via,board='board.md',role='review',branch='synthetic-branch' if via else '',id=42))
    args.work_root.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='child-prompts-',dir=args.work_root) as directory:
        path=Path(directory)/'oracle.go';path.write_text(code)
        output=subprocess.check_output([args.go,'run',str(path)],cwd=directory,input=json.dumps(cases).encode())
    Path(__file__).with_name('child-prompts-go-21d0bc7.json').write_bytes(output)
    print(f'{len(cases)} source-derived prompt-route cases generated')
if __name__=='__main__':main()
