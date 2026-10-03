#!/usr/bin/env python3
"""Source inventory: dispatch/flags/environment reads, not implementation coverage."""
import json,re,hashlib,pathlib
root=pathlib.Path(__file__).resolve().parents[2]
entries=[]
for file in ['cmd/many-ai-cli/main.go','cmd/many-ai-cli/provider.go','cmd/many-ai-cli/issue.go','cmd/many-ai-cli-launcher/main.go']:
 p=root/file
 if not p.exists():continue
 text=p.read_text();cases=[];flags=[]
 for n,line in enumerate(text.splitlines(),1):
  if re.match(r'\s*case ',line):cases.append({'line':n,'case':line.strip()})
  if 'fs.' in line and any(k in line for k in ['.Bool(','.String(','.Int(','.Duration(','.Var(']):flags.append({'line':n,'declaration':line.strip()})
 entries.append({'source':file,'sha256':hashlib.sha256(p.read_bytes()).hexdigest(),'cases':cases,'flags':flags})
env=[]
for p in sorted((root/'internal').rglob('*.go')):
 if p.name.endswith('_test.go'):continue
 for n,l in enumerate(p.read_text().splitlines(),1):
  for m in re.finditer(r'os\.(?:Getenv|LookupEnv)\("([A-Z_][A-Z0-9_]*)"\)',l):env.append({'name':m[1],'source':str(p.relative_to(root)),'line':n})
commands=['<no arguments>','version','--version','-v','serve','connect','profile-export','status','tray','doctor','stop','log-clean','uninstall','shell-init','setup','issue','provider','wrap','claude','codex','copilot','cursor-agent','opencode','grok','command-code','usage-relay','orchestrate','-h','--help','help','<configured custom provider ID>']
d={'baseline':'21d0bc7935a2c4696fb89ccff2e324157a528c2d','commands':[{ 'name':c,'dispatch':'rust/src/cli.rs::parse','caller_status':'pending' if c not in ['version','--version','-v','-h','--help','help'] else 'candidate-only; baseline configuration/version lookup side effects pending'} for c in commands],'entry_sources':entries,'environment_reads':env,'limitations':['Case/flag scans are discovery records; delegated command function flags and env precedence require source-caller fixtures.','Existing Go config loads before all commands, including version/help; current Rust diagnostic entry has not ported that side effect.','Launcher execution remains explicitly unavailable until integration, not a success stub.']}
(root/'rust/inventory/cli.json').write_text(json.dumps(d,ensure_ascii=False,indent=2)+'\n');print(len(commands),'dispatch forms;',len(env),'literal environment reads')
