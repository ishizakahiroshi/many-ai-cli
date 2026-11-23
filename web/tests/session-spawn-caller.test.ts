import test from 'node:test';
import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { receiveHeadlessProgress, setHeadlessConnected } from '../src/app/headless-progress.js';
import * as tracking from '../src/app/session-spawn-state.js';
import * as tabs from '../src/app/session-list-tab-state.js';
import { buildDeriveBody } from '../src/app/derive-dialog-store.js';
import { SessionStartupStore } from '../src/app/session-startup.js';
import { REMOTE_ATTACH_LAUNCH_EVENT, normalizeLaunchChoice, remoteAttachLaunchDetails } from '../src/app/remote-attach-profiles.js';
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
 const context:any={...tracking,sessionStartups:new SessionStartupStore(),t:(key:string)=>key,showToast:()=>{},assignCorrelatedSessionListSpawn:assignment.assign,captureSessionListTab:()=>tabState.selectedId,
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
 const noop=()=>{};const ctx:any={receiveHeadlessProgress,setHeadlessConnected,isHeadlessSession:(s:any)=>s?.execution_mode==='headless',scheduleDeadSessionDismiss:noop,
  sessions,_hubInstance:'',_pendingOpenSessionId:0,pendingOpenFromReconnect:false,
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
 const ctx:any={spawnLaunchSequence:0,captureSpawnTab:()=>tab,document:{getElementById:(id:string)=>inputs[id]||{value:''}},spawnCwdInput:{value:'/repos/example'},
  spawnLaunchBlockReason:()=>'',isPathMissing:()=>false,commandMissingProviderIds:new Set(),spawnLaunchBtn:{disabled:false},spawnModelInput:{value:''},
  updateSpawnLaunchButton:no,getCachedSpawnModelGroups:()=>[],isModelCompatibleWithProvider:()=>true,resolveRoute:()=>'',isCustomProviderValue:()=>false,
 providerHasPermissionSelect:()=>true,
  readSpawnDefaults:()=>({provider_claude:'existing'}),hasProviderBooleanSetting:()=>false,spawnInitialPrompt:{value:''},selectedSubscriptionID:()=>'',
  effortForSpawnBody:()=>'',selectedEffort:()=>'',claudeModelSelection:null,codexModelSelection:null,isClaudeHighRisk:()=>true,
  appConfirm:async()=>{tab='during-risk';return true;},spawnOrchestrationMode:false,boardHandoff:null,setBoardHandoff:no,
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

// 複数のシェルを一度に開く別窓プリセットでも、起動フォームの「添付の送り先」を最初の 1 本だけでなく全部に効かせる。
// 呼び出しの順（起動フォーム → プリセット → spawn-grid → 始まった番号）のとおりに並べる。
class FakeCustomEvent { type:string;detail:any;constructor(type:string,init:any){this.type=type;this.detail=init?.detail;} }
// 起動フォームのファイルは、spawnSession を持つ 1 本として探す（ファイル名を書かない）。
const launchFormSource=()=>{const hits=readdirSync(new URL('../src/app/',import.meta.url)).filter(n=>n.endsWith('.ts')&&source(n).includes('  async function spawnSession()'));assert.equal(hits.length,1);return source(hits[0]);};
test('actual spawn click hands the shell remote-attach choice to its own shells AND to the multi-shell preset',async()=>{
 const text=launchFormSource(),start=text.indexOf('  async function spawnSession()'),end=text.indexOf('\n  }',start)+4;
 const no=()=>{};let nested:any;const events:any[]=[];
 const inputs:any={'spawn-provider':{value:'shell'},'spawn-label':{value:''}};
 const ctx:any={spawnLaunchSequence:0,captureSpawnTab:()=> 'click-tab',spawnCwdInput:{value:'/repos/example'},
  document:{getElementById:(id:string)=>inputs[id]||{value:''},dispatchEvent:(e:any)=>{events.push(e);return true;}},CustomEvent:FakeCustomEvent,
  spawnLaunchBlockReason:()=>'',isPathMissing:()=>false,commandMissingProviderIds:new Set(),spawnLaunchBtn:{disabled:false},spawnModelInput:{value:''},
  updateSpawnLaunchButton:no,getCachedSpawnModelGroups:()=>[],isModelCompatibleWithProvider:()=>true,resolveRoute:()=>'',isCustomProviderValue:()=>false,
  providerHasPermissionSelect:()=>false,readSpawnDefaults:()=>({}),hasProviderBooleanSetting:()=>false,spawnInitialPrompt:{value:''},selectedSubscriptionID:()=>'',
  effortForSpawnBody:()=>'',selectedEffort:()=>'',claudeModelSelection:null,codexModelSelection:null,spawnOrchestrationMode:false,boardHandoff:null,setBoardHandoff:no,
  beginTrackedSpawn:()=> 'ordinary-id',apiFetch:async()=>({ok:true}),saveCwdHistory:no,waitForTrackedSpawn:async()=>[21],
  getSpawnOpenTarget:()=> 'detached',getSpawnGridLayout:()=> '2x2',spawnDetachedPreset:{value:'shell-2x2'},
  spawnRemoteAttachSelect:{value:'last'},normalizeLaunchChoice,remoteAttachLaunchDetails,REMOTE_ATTACH_LAUNCH_EVENT,REMOTE_LAUNCH_KEY:'remote_attach_launch',
  spawnSubscriptionRow:{hidden:true},spawnIsolateWorktree:{checked:false},spawnDelegation:{checked:false},spawnRememberApprovalSettings:{checked:true},
  mergeProviderBooleanSetting:(base:any)=>base,mergeApprovalSettings:(base:any)=>base,saveSpawnSettings:async()=>{},newSessionPanel:{hidden:false},setSpawnOrchestrationMode:no,
  window:{launchDetachedPreset:(opts:any)=>{nested=opts;return Promise.resolve();}},alert:(msg:string)=>{throw Error(msg);},t:(key:string)=>key,showToast:no};
 runInNewContext(compile(text.slice(start,end))+'\nglobalThis.spawn=spawnSession;',ctx);await ctx.spawn();await tick();
 assert.equal(nested.presetId,'shell-2x2');assert.equal(nested.remoteAttachChoice,'last');
 assert.deepEqual(events.map(e=>[e.type,e.detail.sessionId,e.detail.choice]),[[REMOTE_ATTACH_LAUNCH_EVENT,21,'last']]);
});
test('actual preset launcher passes the remote-attach choice to every shell-spawning preset',async()=>{
 const text=source('detached-grid-launcher.ts'),start=text.indexOf('export async function launchDetachedPreset'),end=text.indexOf('// ─── Launcher ダイアログ',start);
 assert.ok(start>=0&&end>start);const seen:any[]=[];
 const ctx:any={DETACHED_PRESETS:[{id:'shell-2x2',defaultLayout:'2x2'},{id:'shell-3x3',defaultLayout:'3x3'},{id:'claude+shell-2x2',defaultLayout:'2x2'},{id:'custom',defaultLayout:'2x2'}],
  spawnGridAndOpen:async(opts:any)=>{seen.push(opts);},getDetachedGridPrefs:()=>({}),setDetachedGridPrefs:()=>{},sessions:new Map(),openDetachedGridForSessions:()=>{}};
 runInNewContext(compile(text.slice(start,end))+'\nglobalThis.launch=launchDetachedPreset;',ctx);
 for(const presetId of ['shell-2x2','shell-3x3','claude+shell-2x2','custom'])await ctx.launch({presetId,cwd:'/repos/example',remoteAttachChoice:'local'});
 assert.deepEqual(seen.map(o=>o.remoteAttachChoice),['local','local','local','local']);
});
test('actual grid caller hands the remote-attach choice to EVERY correlated session, not only the first',async()=>{
 const text=source('detached-grid-launcher.ts'),start=text.indexOf('export async function spawnGridAndOpen'),end=text.indexOf('// ─── プリセット実行',start);
 const run=async(choice:string|undefined,ids:number[])=>{
  let resolve!:(ids:number[])=>void;const events:any[]=[];const opened:string[]=[];
  const ctx:any={beginTrackedSpawn:()=> 'grid-request',captureSpawnTab:()=> 'tab',waitForTrackedSpawn:()=>new Promise(r=>{resolve=r;}),
   window:{open:(url:string)=>opened.push(url),showToast:()=>{},t:()=> 'partial'},buildDetachedGridUrl:(list:number[])=>'grid:'+list.join(','),apiFetch:async()=>({ok:true}),
   document:{dispatchEvent:(e:any)=>{events.push(e);return true;}},CustomEvent:FakeCustomEvent,remoteAttachLaunchDetails,REMOTE_ATTACH_LAUNCH_EVENT};
  runInNewContext(compile(text.slice(start,end))+'\nglobalThis.spawn=spawnGridAndOpen;',ctx);
  await ctx.spawn({preset:'shell',count:ids.length,cwd:'/repos/example',layout:'2x2',sourceTabId:'tab',remoteAttachChoice:choice});
  resolve(ids);await tick();return {events,opened};
 };
 const chosen=await run('profile:dev',[7,8,9,10]);
 assert.deepEqual(chosen.events.map(e=>[e.type,e.detail.sessionId,e.detail.choice]),[7,8,9,10].map(id=>[REMOTE_ATTACH_LAUNCH_EVENT,id,'profile:dev']));
 assert.deepEqual(chosen.opened,['grid:7,8,9,10']);
 // 「決めない」・選択なし（ランチャーの窓から開いた場合）は何も知らせない
 assert.deepEqual((await run('',[7,8])).events,[]);assert.deepEqual((await run(undefined,[7,8])).events,[]);
});

// 画面なし実行の進み: ws-client の実際の分岐を合成の値で回す。
const headlessProgress=(extra:any={})=>({version:1,run_id:'17:1',sequence:1,observed_turns:1,result_turns:null,active_tools:[],started_at_ms:1,elapsed_ms:2,ended_at_ms:null,
 cost:{amount_usd:null,currency:'USD',kind:'unavailable',coverage:'unavailable'},last_assistant:'safe',state:'running',data_incomplete:false,final_cost_missing:false,...extra});
const isHeadless=(s:any)=>s?.execution_mode==='headless';

test('actual headless progress caller replaces only a newer snapshot of a headless session',()=>{
 const text=source('ws-client.ts'),start=text.indexOf("  if (m.type === 'headless_progress') {"),end=text.indexOf("  if (m.type === 'session_spawn_correlated')",start);
 const headless:any={id:17,execution_mode:'headless',state:'running',headless_progress:receiveHeadlessProgress(headlessProgress({sequence:0}))};
 const interactive:any={id:18,state:'running'};let refreshes=0;
 const ctx:any={sessions:new Map([[17,headless],[18,interactive]]),receiveHeadlessProgress,isHeadlessSession:isHeadless,refreshHeadlessProgress:()=>{refreshes++;}};
 runInNewContext(compile('function apply(m:any) {\n'+text.slice(start,end)+'\n}\nglobalThis.apply=apply;'),ctx);
 ctx.apply({type:'headless_progress',session_id:17,headless_progress:headlessProgress({sequence:2,observed_turns:4})});
 assert.equal(headless.headless_progress.observed_turns,4);assert.equal(refreshes,1);
 // An older or repeated snapshot never adds to or rolls back the shown values.
 ctx.apply({type:'headless_progress',session_id:17,headless_progress:headlessProgress({sequence:2,observed_turns:9})});
 ctx.apply({type:'headless_progress',session_id:17,headless_progress:headlessProgress({sequence:1,observed_turns:1})});
 assert.equal(headless.headless_progress.observed_turns,4);assert.equal(refreshes,1);
 ctx.apply({type:'headless_progress',session_id:18,headless_progress:headlessProgress({sequence:3})});
 assert.equal(interactive.headless_progress,undefined);assert.equal(refreshes,1);
});

test('actual snapshot caller keeps a completed headless run with its final progress and dismisses others',()=>{
 const text=source('ws-client.ts'),start=text.indexOf("  if (m.type === 'snapshot') {"),end=text.indexOf("  } else if (m.type === 'session_update')",start);
 const sessions=new Map();const dismissed:number[]=[],scheduled:number[]=[];const noop=()=>{};
 const ctx:any={receiveHeadlessProgress,setHeadlessConnected,isHeadlessSession:isHeadless,scheduleDeadSessionDismiss:(id:number)=>scheduled.push(id),
  sessions,_hubInstance:'',_pendingOpenSessionId:0,pendingOpenFromReconnect:false,purgeLocalStateForHubRestart:noop,setApprovalFoldScope:noop,
  requestSessionDismiss:(id:number)=>dismissed.push(id),deriveProjectKeyFromCwd:()=> 'example',addToSessionOrder:noop,noteSpawnSnapshot:noop,
  document:{dispatchEvent:noop,getElementById:()=>({textContent:''})},Event:class{},window:{},t:(key:string)=>key,
  migratePinnedSessionsOnce:noop,renderSessionList:noop,checkApprovalOnStartup:noop,syncElapsedTimer:noop};
 runInNewContext(compile('function apply(m:any) {\n'+text.slice(start,end)+'\n}\n}\nglobalThis.apply=apply;'),ctx);
 const final=headlessProgress({sequence:5,state:'completed',ended_at_ms:3,cost:{amount_usd:0.25,currency:'USD',kind:'result',coverage:'cli-result'}});
 ctx.apply({type:'snapshot',hub_instance:'hub-A',sessions:[{id:1,state:'completed',cwd:'/repos/example'},
  {id:2,state:'completed',execution_mode:'headless',cwd:'/repos/example',headless_progress:final},
  {id:3,state:'running',execution_mode:'headless',cwd:'/repos/example',headless_progress:{...final,elapsed_ms:-1}}]});
 assert.deepEqual(dismissed,[1]);assert.deepEqual(scheduled,[2]);
 assert.equal(sessions.get(2).headless_progress.cost.amount_usd,0.25);
 // A snapshot that fails the allowlist is dropped rather than shown.
 assert.equal(sessions.get(3).headless_progress,null);
});

test('actual session_end caller keeps a completed headless card and its pane placement',()=>{
 const text=source('ws-client.ts'),start=text.indexOf("  } else if (m.type === 'session_end') {"),end=text.indexOf("  } else if (m.type === 'session_removed')",start);
 const removed:number[]=[],dismissed:number[]=[],ended:number[]=[],scheduled:number[]=[];const noop=()=>{};
 const headless:any={id:17,execution_mode:'headless',state:'running'};
 const ctx:any={sessions:new Map([[17,headless],[18,{id:18,state:'running'}]]),activeSessionId:0,isHeadlessSession:isHeadless,
  window:{multiPaneManager:{onSessionEnded:(id:number)=>ended.push(id)}},scheduleGitLockCheck:noop,
  requestSessionDismiss:(id:number)=>dismissed.push(id),removeLocalSession:(id:number)=>removed.push(id),
  updateInputAffordance:noop,syncLiveStatusDomForActive:noop,clearApprovalMarkerSuppressed:noop,removeApprovalAutoSwitchTarget:noop,
  maybeAutoSwitchToNextApproval:noop,scheduleDeadSessionDismiss:(id:number)=>scheduled.push(id)};
 runInNewContext(compile('function apply(m:any) {\n  if (false) {\n'+text.slice(start,end)+'\n  }\n}\nglobalThis.apply=apply;'),ctx);
 ctx.apply({type:'session_end',session_id:17,state:'completed'});
 ctx.apply({type:'session_end',session_id:18,state:'completed'});
 assert.equal(headless.state,'completed');assert.deepEqual(scheduled,[17]);
 assert.deepEqual(removed,[18]);assert.deepEqual(dismissed,[18]);assert.deepEqual(ended,[18]);
});
