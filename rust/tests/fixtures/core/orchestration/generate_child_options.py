#!/usr/bin/env python3
"""Run fixed Go permission/execution functions on synthetic request values."""
import argparse, json, os, re, subprocess, tempfile
from pathlib import Path
ORACLE = '21d0bc7935a2c4696fb89ccff2e324157a528c2d'
def function(source, name):
    start = re.search(r'^func ' + re.escape(name) + r'\(', source, re.M).start()
    return source[start:source.index('\n}', start)+2]
def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--go', required=True)
    parser.add_argument('--work-root', required=True, type=Path)
    args=parser.parse_args()
    if not Path(args.go).is_absolute() or not args.work_root.is_absolute(): parser.error('absolute tool and work-root paths required')
    repo=Path(__file__).resolve().parents[5]
    def source(path): return subprocess.check_output(['git','show',f'{ORACLE}:{path}'],cwd=repo,text=True)
    # Imported package code is unchanged from the oracle; no Go file is edited.
    subprocess.run(['git','diff','--exit-code',ORACLE,'--','internal/config','internal/proto','go.mod','go.sum'],cwd=repo,check=True,stdout=subprocess.DEVNULL)
    original=source('internal/hub/child_permission.go')
    code='package main\nimport("strings";"encoding/json";"os";"many-ai-cli/internal/config";"many-ai-cli/internal/proto")\n'
    code+=original[original.index('type childApproval struct'):original.index('// headlessCapableProvider')]
    code+='''\ntype spawnChildRequest struct { Provider, PermissionPreset, ExecutionMode, Origin, PermissionMode, Sandbox, AskForApproval string; AllowedTools []string; RiskConfirmed bool }\n'''
    code+='\n'.join(function(original,name) for name in ['headlessCapableProvider','applyChildExecutionMode','applyChildPermission'])
    code+='''
type input struct { Name string `json:"name"`; Provider string `json:"provider"`; Preset string `json:"preset"`; Mode string `json:"mode"`; Origin string `json:"origin"`; Confirmed bool `json:"confirmed"`; FullBypass *bool `json:"full_bypass"`; DefaultTier string `json:"default_tier"`; DefaultMode string `json:"default_mode"`; Tools map[string][]string `json:"tools"`; ExistingTools []string `json:"existing_tools"`; PermissionMode string `json:"permission_mode"`; Sandbox string `json:"sandbox"`; AskForApproval string `json:"ask_for_approval"` }
func main(){var inputs []input;if e:=json.NewDecoder(os.Stdin).Decode(&inputs);e!=nil{panic(e)};var outputs []any;for _,in:=range inputs {cfg:=config.Config{Orchestration:config.OrchestrationConfig{ChildFullBypass:in.FullBypass,ChildPermissionDefault:in.DefaultTier,ChildExecutionMode:in.DefaultMode,BoundedAllowedTools:in.Tools}};body:=spawnChildRequest{Provider:in.Provider,PermissionPreset:in.Preset,ExecutionMode:in.Mode,Origin:in.Origin,AllowedTools:in.ExistingTools,PermissionMode:in.PermissionMode,Sandbox:in.Sandbox,AskForApproval:in.AskForApproval};out:=map[string]any{"input":in};if e:=applyChildExecutionMode(&body,&cfg);e!=nil{out["error"]=e.Error()}else{applyChildPermission(&body,cfg.Orchestration);r:=resolveChildPermission(body.Provider,body.PermissionPreset,body.ExecutionMode,body.Origin,cfg.Orchestration);out["mode"]=body.ExecutionMode;out["approval"]=proto.ChildApproval{PermissionMode:body.PermissionMode,Sandbox:body.Sandbox,AskForApproval:body.AskForApproval,AllowedTools:body.AllowedTools,RiskConfirmed:body.RiskConfirmed,Tier:r.Tier,FallbackFrom:r.FallbackFrom}};outputs=append(outputs,out)};e:=json.NewEncoder(os.Stdout);e.SetIndent("","  ");if err:=e.Encode(outputs);err!=nil{panic(err)}}
'''
    cases=[]
    for provider in ['claude','codex','copilot','opencode','grok','cursor-agent','command-code','shell','synthetic-custom']:
        for preset in ['attended','bounded','full']:
            cases.append(dict(name=f'{provider}-{preset}',provider=provider,preset=preset,mode='',origin='',confirmed=False))
    for mode in ['interactive','auto']:
        for origin,confirmed,name in [('',False,'autonomous'),('ui',False,'direct-ui'),('',True,'confirmed-autonomous')]:
            cases.append(dict(name=f'{name}-{mode}',provider='claude',preset='',mode='',origin=origin,confirmed=confirmed,default_mode=mode))
    cases += [
        dict(name='full-bypass-disabled',provider='codex',preset='full',full_bypass=False),
        dict(name='explicit-fields-survive-disabled',provider='codex',preset='bounded',full_bypass=False,sandbox='read-only',ask_for_approval='on-request'),
        dict(name='explicit-mode-survives-bounded',provider='claude',preset='bounded',permission_mode='default'),
        dict(name='empty-allowlist-override',provider='claude',preset='bounded',tools={'claude':[]}),
        dict(name='existing-internal-tools-preserved',provider='claude',preset='bounded',existing_tools=['Read']),
        dict(name='unsupported-explicit-headless',provider='codex',mode='headless'),
        dict(name='direct-ui-headless-unattended',provider='claude',mode='headless',origin='ui'),
    ]
    args.work_root.mkdir(parents=True,exist_ok=True)
    manifest=source('go.mod').replace('module many-ai-cli\n','module many-ai-cli/synthetic-child-options\n',1)
    with tempfile.TemporaryDirectory(prefix='child-options-',dir=args.work_root) as directory:
        root=Path(directory)
        (root/'go.mod').write_text(manifest+f'\nrequire many-ai-cli v0.0.0\nreplace many-ai-cli => {repo}\n')
        (root/'go.sum').write_text(source('go.sum'))
        (root/'main.go').write_text(code)
        output=subprocess.check_output([args.go,'run','-mod=readonly','.'],cwd=root,input=json.dumps(cases).encode(),env={**os.environ,'GOPROXY':'off','GOSUMDB':'off'})
    Path(__file__).with_name('child-options-go-21d0bc7.json').write_bytes(output)
    print(f'{len(cases)} source-derived child option cases generated')
if __name__=='__main__': main()
