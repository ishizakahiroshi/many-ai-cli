import json
from pathlib import Path
out=[]
def add(name,body='',**kw):
 out.append(dict(name=name,method=kw.pop('method','PUT'),body=body if isinstance(body,str) else json.dumps(body,ensure_ascii=False),**kw))
for name,initial,backends,push in [('get-default',{},0,0),('get-backend',{},1,0),('get-push',{},0,1),('get-explicit-false',{'done_summary_notify':{'enabled':False}},2,2),('get-explicit-true',{'done_summary_notify':{'enabled':True}},0,0)]:
 add(name,method='GET',initial=initial,backends=backends,push=push)
full={'trigger':{'enabled':True,'phrase':'synthetic trigger'},'notify_sound':{'enabled':True,'type':'bell'},'desktop_notifications':{'enabled':True},'push_notifications':{'enabled':True},'approval':{'auto_switch':True,'auto_approval_enabled':True,'high_risk_confirmation_mode':'dialog'},'quick_cmds':{'cmd1':'one','cmd2':'two','cmd3':'three','cmd4':'four','cmd5':'five','show1':False,'show2':True,'show3':False,'show4':True,'show5':False},'templates':[{'label':'second','body':'synthetic B','providers':['codex','claude'],'tags':['work','CJK 日本語']},{'label':'first','body':'synthetic A'}],'template_send':{'immediate':False},'usage_links':{'claude':'https://example.invalid/claude','codex':'https://example.invalid/codex','copilot':'c','cursor-agent':'d','opencode':'e','grok':'f','command-code':'g'},'usage_probe_model':'  synthetic-model  ','voice':{'grace_seconds':7,'wake_word_enabled':True,'wake_word_phrase':'hello','input_disabled':True},'session_order':[9,2,5],'group_order':['b','a'],'project_favorites':['b'],'collapsed_nodes':['b','session:9'],'cwd_history':['/synthetic/alpha'],'cwd_favorites':['/synthetic/beta'],'spawn':{'defaults':{'codex':'synthetic'},'last_model':{'claude':'synthetic-model'},'worktree_auto':True,'worktree_cleanup':'always','delegation_auto':True,'role_provider':{'review':'codex'},'role_effort':{'review':'high'},'role_permission':{'review':'default'}},'display':{'theme':'u-ok','custom_themes':[{'id':'u-ok','name':'night','mode':'dark','hue':220,'contrast':80}],'font_size':'large','lang':'ja','locked_mode':'all','live_status_bg':'#000000','live_status_fg':'#ffffff'},'migrated_from_localstorage':True,'sidebar_pin_migrated':True,'project_views':{'/synthetic/alpha':{'session_id':9,'tab':'git'}},'open_project':'/synthetic/alpha','avatar':'https://example.invalid/synthetic.png','display_name':'Synthetic user','token_statusbar':{'enabled':False,'segments':{'cost':False,'tokens':True}},'done_summary_notify':{'enabled':False},'workflow_completion_notify':{'enabled':True}}
add('all-fields-full-snapshot',full)
add('empty-snapshot-clears',{},initial=full)
add('null-snapshot-clears','null',initial=full)
for model in ['', 'synthetic model', 'foo;bar','synthetic:model/1','  synthetic-model  ']:add('probe-model-'+str(len(out)),{'usage_probe_model':model})
templates=[{'label':'original','body':'synthetic initial','providers':['codex'],'tags':['old']}]
for policy,version,body in [('compare','current',{'templates':[{'body':'updated'}]}),('compare','stale',{'templates':[{'body':'updated'}]}),('compare','',{'templates':[{'body':'updated'}]}),('preserve','stale',{'templates':[{'body':'stale cache'}],'open_project':'/synthetic/new'}),('preserve','',{}),('Compare','stale',{'templates':[]}),('','stale',{}),('compare','current',{'templates':[]}),('compare','current',{'templates':None})]:
 add('template-policy-'+str(len(out)),body,initial={'templates':templates},policy=policy,version=version)
add('template-save-failure',{'templates':[{'body':'unsaved'}],'open_project':'/synthetic/unsaved'},initial={'templates':templates},policy='compare',version='current',fail_save=True)
add('non-template-save-failure',{'display_name':'unsaved'},initial={'display_name':'original'},fail_save=True)
for body in ['{"templates":[{"body":"<>&\\u2028\\u2029日本語","label":"quoted \\\" x","providers":["codex","claude"],"tags":["<tag>"]}]}','{"templates":[{"label":"","body":"","providers":[],"tags":[]}]}','{"templates":[null]}','{"templates":[{"label":"a","body":"b"},{"label":"c","body":"d"}],"templates":[{"body":"e"}],"templates":[{},{}]}','{"templates":[{"label":"a","body":"b"}],"templates":null,"templates":[{"body":"e"}]}','{"templates":[{"body":"a"}],"templates":[],"templates":[{"label":"b"}]}']:
 add('template-encoding-'+str(len(out)),body)
for avatar in ['', ' https://example.invalid/x.png ', 'http://', 'HTTPS://example.invalid/x.png','data:image/png;base64,synthetic','/synthetic/arbitrary.bin','<root>/user_avatar.bin','<root>/nested/../user_avatar.bin','  <root>/./user_avatar.bin  ','<root>/user_avatar.bin/','<root>/user_avatar.bin.other']:
 add('avatar-'+str(len(out)),{'avatar':avatar})
for path,mime in [('', ''),('<root>/notify_sound_custom.bin','audio/wav'),('<root>/a/../notify_sound_custom.bin',' AUDIO/WAV ; codec=x '),('<root>/notify_sound_custom.bin','audio/'),('<root>/notify_sound_custom.bin','text/html'),('/synthetic/elsewhere','audio/wav'),('','audio/wav'),('<root>/notify_sound_custom.bin',''),(' <root>/notify_sound_custom.bin ','audio/wav')]:
 add('sound-'+str(len(out)),{'notify_sound':{'enabled':True,'type':'custom','custom_file':path,'custom_mime':mime}})
add('theme-sanitize',{'display':{'theme':' u-ok ','custom_themes':[{'id':' u-ok ','name':'\u0001Synthetic 日本語abcdefghijklmnop\u007f','mode':'LIGHT','hue':500,'contrast':-2},{'id':'u-ok','name':'duplicate'},{'id':'dark','name':'invalid'},{'id':'u-99','name':'\u0001\u007f'}]}})
add('theme-missing-falls-light',{'display':{'theme':'u-missing'}})
add('theme-blank-stays-blank',{'display':{'theme':'  '}})
add('theme-limit',{'display':{'theme':'u-x21','custom_themes':[{'id':f'u-x{i:02}','name':f'theme{i}','mode':'light','hue':i,'contrast':i} for i in range(22)]}})
add('all-null-collections',{'templates':None,'session_order':None,'group_order':None,'project_favorites':None,'collapsed_nodes':None,'cwd_history':None,'cwd_favorites':None,'project_views':None,'spawn':{'defaults':None,'last_model':None,'role_provider':None,'role_effort':None,'role_permission':None},'display':{'custom_themes':None},'token_statusbar':{'segments':None},'template_send':{'immediate':None},'quick_cmds':{'show1':None},'done_summary_notify':{'enabled':None}})
add('nested-null-collections','{"templates":[{"providers":null,"tags":null}],"project_views":{"a":null},"spawn":{"defaults":{"codex":null}},"token_statusbar":{"segments":{"cost":null}}}')
add('null-scalar-retains','{"display_name":"kept","display_name":null,"trigger":{"enabled":true,"phrase":"kept"},"trigger":null,"trigger":{"enabled":null}}')
add('pointer-null-clears','{"quick_cmds":{"show1":true},"quick_cmds":{"show1":null},"done_summary_notify":{"enabled":true},"done_summary_notify":{"enabled":null}}')
add('map-merge-replace','{"project_views":{"a":{"session_id":7,"tab":"git"}},"project_views":{"a":{"tab":"terminal"},"b":{"session_id":2}},"spawn":{"defaults":{"a":"x"}},"spawn":{"defaults":{"b":"y"}}}')
add('map-null-reset','{"spawn":{"defaults":{"a":"x"}},"spawn":{"defaults":null},"spawn":{"defaults":{"b":"y"}}}')
add('casefold-known-fields','{"DISPLAY_NAME":"name","TRIGGER":{"ENABLED":true},"quicK_cmds":{"show1":false},"diſplay":{"theme":"dark"},"unknown":{"deep":1e1000}}')
for raw in ['[1," 2 ",2.0,3.2,null,true,{},"bad",-4]','{}','"not-array"','[1e1000]','[9007199254740993,9223372036854775808,-9223372036854775808,"9223372036854775807"]']:
 add('session-order-'+str(len(out)),'{"session_order":'+raw+'}')
add('session-order-replace','{"session_order":[1,2],"SESSION_ORDER":[null,"3"]}')
add('session-order-reset','{"session_order":[1,2],"session_order":{}}')
for body in ['', '{', '[]','{"templates":{}}','{"display_name":1,"display_name":"later"}','{"quick_cmds":{"show1":"bad"}}','{"voice":{"grace_seconds":1.5}}','{"templates":[{"providers":[1]}]}','{"project_views":{"a":{"session_id":"1"}}}']:
 add('invalid-json-'+str(len(out)),body)
add('first-value-trailing','{"display_name":"first"} garbage')
add('unknown-large-number','{"unknown":1e1000}')
add('unknown-depth','{"ignored":'+('['*150)+'0'+(']'*150)+'}')
add('wrong-method-before-body','{',method='POST')
add('session-order-deep-ignored-neighbors', '{"session_order":[7,'+('['*150)+'0'+(']'*150)+',"8"]}')
add('session-order-deep-overflow-clears', '{"session_order":[7,'+('['*150)+'1e1000'+(']'*150)+',"8"]}')
Path('rust/tests/fixtures/services/user-preferences/cases.json').write_text(json.dumps(out,ensure_ascii=True,indent=2)+'\n')
print(len(out),'cases')
