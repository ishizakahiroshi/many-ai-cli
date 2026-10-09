import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import * as tracking from '../src/app/session-spawn-state.js';
import * as tabs from '../src/app/session-list-tab-state.js';
import { buildDeriveBody } from '../src/app/derive-dialog-store.js';
const source=(name:string)=>readFileSync(new URL('../src/app/'+name,import.meta.url),'utf8');
const compile=(text:string)=>new Bun.Transpiler({loader:'ts'}).transformSync(text.replace(/^import[\s\S]*?;\r?$/gm,'').replace(/^export /gm,''));
const tick=async()=>{for(let i=0;i<12;i++)await Promise.resolve();};
const session=(id:number,request:string,extra:any={})=>({id,started_at:'start-'+id,cwd:'/repos/example',client_request_id:request,...extra});
function fixture(saved:string|null=null, deniedStorage=false) {
 const tabState=tabs.normalizeSessionListTabs(null);tabs.addSessionListTab(tabState,'Other','other');
 let storage=saved, clock=100000, counter=0;
 const assignment:any={...tabs,state:()=>tabState,persist:()=>{}};
 const text=source('session-list-tabs.ts');const a=text.indexOf('export function assignCorrelatedSessionListSpawn'),b=text.indexOf('export function isSessionInSelectedListTab',a);
 runInNewContext(compile(text.slice(a,b))+'\nglobalThis.assign=assignCorrelatedSessionListSpawn;',assignment);
 const context:any={...tracking,t:(key:string)=>key,showToast:()=>{},assignCorrelatedSessionListSpawn:assignment.assign,captureSessionListTab:()=>tabState.selectedId,
  sessionStorage:{getItem:()=>{if(deniedStorage)throw Error("denied");return storage;},setItem:(_key:string,value:string)=>{if(deniedStorage)throw Error("quota");storage=value;}},crypto:{getRandomValues:(v:Uint32Array)=>v.fill(++counter)},
  Date:{now:()=>clock},setTimeout,clearTimeout};
 runInNewContext(compile(source('session-spawn-tracker.ts'))+'\nglobalThis.ui={beginTrackedSpawn,noteSpawnDisconnected,noteSpawnSnapshot,noteSpawnSessions,noteSpawnCorrelation,waitForTrackedSpawn};',context);
 return {ui:context.ui,tabs:tabState,get saved(){return storage!;},advance(ms:number){clock+=ms;}};
}
test('actual tracker handles WS before HTTP/map, captured tab switching, and manual movement on reconnect',async()=>{
 const f=fixture();f.ui.noteSpawnSnapshot('hub-A',[]);const id=f.ui.beginTrackedSpawn(1);f.tabs.selectedId='tab-1';
 f.ui.noteSpawnCorrelation({client_request_id:id,session_id:1,started_at:'start-1',hub_instance:'hub-A'},[]);
 const root=session(1,id);f.ui.noteSpawnSessions([root]);
 assert.equal(tabs.sessionListOwners(f.tabs,[root]).get(1),'other');assert.deepEqual([...await f.ui.waitForTrackedSpawn(id,5)],[1]);
 tabs.moveSessionListFamily(f.tabs,[root],1,'tab-1');f.ui.noteSpawnSnapshot('hub-A',[root]);assert.equal(tabs.sessionListOwners(f.tabs,[root]).get(1),'tab-1');
 assert.ok(!f.saved.includes('cwd'));assert.ok(!f.saved.includes('state'));
});
test('actual tracker handles all grid siblings, partial failure, and unrelated external creation',async()=>{
 const f=fixture();f.ui.noteSpawnSnapshot('hub-A',[]);const id=f.ui.beginTrackedSpawn(3);const first=session(1,id),second=session(2,id),external=session(99,'other-browser');
 f.ui.noteSpawnSnapshot('hub-A',[first,external]);assert.equal(tabs.sessionListOwners(f.tabs,[first,external]).get(99),'tab-1');
 f.ui.noteSpawnSnapshot('hub-A',[first,second,external]);assert.equal(tabs.sessionListOwners(f.tabs,[first,second]).get(2),'other');
 assert.deepEqual([...await f.ui.waitForTrackedSpawn(id,1)],[1,2]);
});
test('actual tracker reload compares FIRST Hub snapshot and ignores old/reused lifecycles',async()=>{
 const f=fixture();f.ui.noteSpawnSnapshot('hub-A',[]);const id=f.ui.beginTrackedSpawn(1);
 const reloaded=fixture(f.saved);reloaded.ui.noteSpawnSnapshot('hub-B',[session(1,id)]);
 assert.equal(tabs.sessionListOwners(reloaded.tabs,[session(1,id)]).get(1),'tab-1');assert.deepEqual([...await reloaded.ui.waitForTrackedSpawn(id,1)],[]);
});
test('actual assignment keeps a deleted tab discoverable and a child in its real root family',()=>{
 const f=fixture();f.ui.noteSpawnSnapshot('hub-A',[]);const id=f.ui.beginTrackedSpawn(2);tabs.deleteSessionListTab(f.tabs,'other','tab-1');
 const parent=session(1,''),child=session(2,id,{parent_session_id:1});f.ui.noteSpawnSnapshot('hub-A',[child,parent]);
 assert.equal(tabs.sessionListOwners(f.tabs,[parent,child]).get(2),'tab-1');
});
test('actual assignment preserves a manual move made before the correlation arrives',async()=>{
 const f=fixture();f.ui.noteSpawnSnapshot('hub-A',[]);const id=f.ui.beginTrackedSpawn(1),root=session(1,id);
 f.ui.noteSpawnSessions([root]);tabs.moveSessionListFamily(f.tabs,[root],1,'tab-1');
 f.ui.noteSpawnCorrelation({client_request_id:id,session_id:1,started_at:'start-1',hub_instance:'hub-A'},[root]);
 assert.equal(tabs.sessionListOwners(f.tabs,[root]).get(1),'tab-1');assert.deepEqual([...await f.ui.waitForTrackedSpawn(id,1)],[1]);
});
test('actual root assignment falls back after its captured tab is deleted',async()=>{
 const f=fixture();f.ui.noteSpawnSnapshot('hub-A',[]);const id=f.ui.beginTrackedSpawn(1);tabs.deleteSessionListTab(f.tabs,'other','tab-1');
 const root=session(1,id);f.ui.noteSpawnSnapshot('hub-A',[root]);
 assert.equal(tabs.sessionListOwners(f.tabs,[root]).get(1),'tab-1');assert.deepEqual([...await f.ui.waitForTrackedSpawn(id,1)],[1]);
});
test('actual child correlation defers missing ancestry until the full family reconnects',async()=>{
 const f=fixture();f.ui.noteSpawnSnapshot('hub-A',[]);const id=f.ui.beginTrackedSpawn(1),child=session(2,id,{parent_session_id:1});
 f.ui.noteSpawnSnapshot('hub-A',[child]);assert.deepEqual([...await f.ui.waitForTrackedSpawn(id,1)],[]);
 const root=session(1,'external');tabs.moveSessionListFamily(f.tabs,[root,child],1,'other');
 f.ui.noteSpawnSnapshot('hub-A',[child,root]);
 assert.equal(tabs.sessionListOwners(f.tabs,[root,child]).get(2),'other');assert.deepEqual([...await f.ui.waitForTrackedSpawn(id,1)],[2]);
});
test('actual tracker does not let storage errors discard current-page correlation',async()=>{
 const f=fixture(null,true);f.ui.noteSpawnSnapshot('hub-A',[]);const id=f.ui.beginTrackedSpawn(1);f.ui.noteSpawnSnapshot('hub-A',[session(1,id)]);
 assert.deepEqual([...await f.ui.waitForTrackedSpawn(id,1)],[1]);
});

test('actual derive caller retains the FIRST request/tab through a risk-confirmation retry',async()=>{
 const text=source('derive-dialog.ts'),start=text.indexOf('  async function submit('),end=text.indexOf('  async function saveManualMemo',start);
 assert.ok(start>=0&&end>start);let tab='first-tab',starts=0,focused=0;const bodies:any[]=[];
 const ctx:any={captureSpawnTab:()=>tab,beginTrackedSpawn:(_n:number,destination:string)=>{starts++;assert.equal(destination,'first-tab');return 'same-request';},
  currentSelection:()=>({kind:'handoff',provider:'codex',cwd:'/repos/example',prompt:'Synthetic',sourceSessionID:1}),
  apiFetch:async(_path:string,init:any)=>{bodies.push(JSON.parse(init.body));return {ok:bodies.length===2,json:async()=>bodies.length===1?{error:'risk_confirmation_required'}:{ok:true}};},
  deriveRequestPath:()=>'/api/spawn',buildDeriveBody,getCachedSpawnModelGroups:()=>[],appConfirm:async()=>{tab='switched-during-confirmation';return true;},
  close:()=>{},showToast:()=>{},tx:(k:string)=>k,t:(k:string)=>k,setError:()=>{},focusNewSession:(id:number)=>{focused=id;},waitForTrackedSpawn:async()=>[44]};
 runInNewContext(compile(text.slice(start,end))+'\nglobalThis.submit=submit;',ctx);await ctx.submit(false);await tick();
 assert.equal(starts,1);assert.equal(bodies[0].client_request_id,'same-request');assert.equal(bodies[1].client_request_id,'same-request');assert.equal(focused,44);
});
test('derive body never adds correlation to a child request',()=>{
 const body=buildDeriveBody({kind:'child',provider:'codex',role:'worker',sourceSessionID:1,clientRequestId:'must-not-use'} as any,[]);
 assert.equal(body.client_request_id,undefined);
});
test('actual grid caller opens only its correlated identities, including after HTTP failure',async()=>{
 const text=source('detached-grid-launcher.ts'),start=text.indexOf('export async function spawnGridAndOpen'),end=text.indexOf('// ─── プリセット実行',start);
 let resolve!:(ids:number[])=>void;const opened:string[]=[];let body:any;
 const ctx:any={beginTrackedSpawn:(count:number,tab:string)=>{assert.equal(count,3);assert.equal(tab,'captured');return 'grid-request';},captureSpawnTab:()=> 'wrong-tab',
  waitForTrackedSpawn:()=>new Promise(r=>{resolve=r;}),window:{open:(url:string)=>opened.push(url),showToast:()=>{},t:()=> 'partial'},
  buildDetachedGridUrl:(ids:number[])=>'grid:'+ids.join(','),apiFetch:async(_p:string,init:any)=>{body=JSON.parse(init.body);return {ok:false,text:async()=> 'synthetic partial failure'};}};
 runInNewContext(compile(text.slice(start,end))+'\nglobalThis.spawn=spawnGridAndOpen;',ctx);
 await assert.rejects(ctx.spawn({preset:'shell',count:3,cwd:'/repos/example',layout:'2x2',sourceTabId:'captured'}));
 resolve([7,9]);await tick();assert.equal(body.client_request_id,'grid-request');assert.deepEqual(opened,['grid:7,9']);
});
test('actual snapshot caller builds the FULL session map before family correlation',()=>{
 const text=source('ws-client.ts'),start=text.indexOf("  if (m.type === 'snapshot') {"),end=text.indexOf("  } else if (m.type === 'session_update')",start);
 const sessions=new Map();let seen:number[]=[];
 const noop=()=>{};const ctx:any={sessions,_hubInstance:'',_pendingOpenSessionId:0,pendingOpenFromReconnect:false,
  purgeLocalStateForHubRestart:noop,setApprovalFoldScope:noop,requestSessionDismiss:noop,deriveProjectKeyFromCwd:()=> 'example',addToSessionOrder:noop,
  noteSpawnSnapshot:(_instance:string,all:any[])=>{seen=all.map(s=>s.id);assert.equal(all.find(s=>s.id===2).parent_session_id,1);},
  document:{dispatchEvent:noop,getElementById:()=>({textContent:''})},Event:class{},window:{},t:(key:string)=>key,
  migratePinnedSessionsOnce:noop,renderSessionList:noop,checkApprovalOnStartup:noop,syncElapsedTimer:noop};
 runInNewContext(compile('function apply(m:any) {\n'+text.slice(start,end)+'\n}\n}\nglobalThis.apply=apply;'),ctx);
 ctx.apply({type:'snapshot',hub_instance:'hub-A',sessions:[session(2,'r',{parent_session_id:1}),session(1,'r')]});assert.deepEqual([...seen],[2,1]);
});

test('actual spawn click keeps its tab through risk wait, preference save, and nested grid; nonce never enters defaults',async()=>{
 const text=source('spawn-panel.ts'),start=text.indexOf('  async function spawnSession()'),end=text.indexOf('\n  }',start)+4;
 let tab='click-tab';let requestBody:any,defaults:any,nested:any;const no=()=>{};
 const inputs:any={'spawn-provider':{value:'claude'},'spawn-label':{value:'Synthetic'},'spawn-permission-mode':{value:'default'}};
 const ctx:any={captureSpawnTab:()=>tab,document:{getElementById:(id:string)=>inputs[id]||{value:''}},spawnCwdInput:{value:'/repos/example'},
  spawnLaunchBlockReason:()=>'',isPathMissing:()=>false,commandMissingProviderIds:new Set(),spawnLaunchBtn:{disabled:false},spawnModelInput:{value:''},
  updateSpawnLaunchButton:no,getCachedSpawnModelGroups:()=>[],isModelCompatibleWithProvider:()=>true,resolveRoute:()=>'',isCustomProviderValue:()=>false,
  readSpawnDefaults:()=>({provider_claude:'existing'}),hasProviderBooleanSetting:()=>false,spawnInitialPrompt:{value:''},selectedSubscriptionID:()=>'',
  effortForSpawnBody:()=>'',selectedEffort:()=>'',claudeModelSelection:null,codexModelSelection:null,isClaudeHighRisk:()=>true,
  appConfirm:async()=>{tab='during-risk';return true;},spawnOrchestrationMode:false,
  beginTrackedSpawn:(_count:number,captured:string)=>{assert.equal(captured,'click-tab');return 'ordinary-id';},
  apiFetch:async(_path:string,init:any)=>{requestBody=JSON.parse(init.body);return {ok:true};},saveCwdHistory:no,
  getSpawnOpenTarget:()=> 'detached',getSpawnGridLayout:()=> '2x2',spawnDetachedPreset:{value:'shell-2x2'},
  spawnSubscriptionRow:{hidden:true},spawnIsolateWorktree:{checked:false},spawnDelegation:{checked:false},spawnRememberApprovalSettings:{checked:true},
  mergeProviderBooleanSetting:(base:any)=>base,mergeApprovalSettings:(base:any,_provider:string,body:any)=>{assert.equal(body.client_request_id,undefined);return base;},
  saveSpawnSettings:async(value:any)=>{defaults=value;tab='during-save';},newSessionPanel:{hidden:false},setSpawnOrchestrationMode:no,
  window:{launchDetachedPreset:(opts:any)=>{nested=opts;return Promise.resolve();}},alert:(msg:string)=>{throw Error(msg);},t:(key:string)=>key};
 runInNewContext(compile(text.slice(start,end))+'\nglobalThis.spawn=spawnSession;',ctx);await ctx.spawn();
 assert.equal(requestBody.client_request_id,'ordinary-id');assert.equal(nested.sourceTabId,'click-tab');
 assert.equal(defaults.client_request_id,undefined);assert.equal(defaults.sourceTabId,undefined);
});

test('delayed actual correlation WS branch refreshes the open mobile drawer after its earlier update render',()=>{
 const text=source('ws-client.ts'),start=text.indexOf("  if (m.type === 'session_spawn_correlated') {"),end=text.indexOf("  if (m.type === 'snapshot')",start);
 const f=fixture();f.ui.noteSpawnSnapshot('hub-A',[]);const request=f.ui.beginTrackedSpawn(1),root=session(1,request);
 f.ui.noteSpawnSessions([root]);
 // The previous session_update's debounced drawer render has already run.
 let drawerIds:number[]=[];let calls=0;
 const ctx:any={sessions:new Map([[1,root]]),noteSpawnCorrelation:f.ui.noteSpawnCorrelation,renderSessionList:()=>{},
  window:{renderMobileSessionDrawer:(force?:boolean)=>{assert.notEqual(force,true);calls++;
   drawerIds=tabs.sessionListOwners(f.tabs,[root]).get(1)===f.tabs.selectedId?[1]:[];}}};
 runInNewContext(compile('function apply(m:any) {\n'+text.slice(start,end)+'\n}\nglobalThis.apply=apply;'),ctx);
 ctx.apply({type:'session_spawn_correlated',client_request_id:request,session_id:1,started_at:'start-1',hub_instance:'hub-A'});
 assert.equal(calls,1);assert.deepEqual(drawerIds,[1]);
});
