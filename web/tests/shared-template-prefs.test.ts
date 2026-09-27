import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { SharedTemplateSync } from '../src/app/shared-template-sync';
import { createUserPrefsPutQueue } from '../src/app/user-prefs-put-queue';
import { sanitizeProjectViews } from '../src/app/project-view-memory';

const LIST='ai_cli_hub_prompt_templates';
const DIRTY='ai_cli_hub_templates_unsaved';
const VERSION='ai_cli_hub_templates_version';
const BASE='ai_cli_hub_templates_edit_base';
const rows=(body:string)=>[{body,providers:['codex'],tags:['synthetic']}];
const clone=<T>(value:T):T=>JSON.parse(JSON.stringify(value));
const settle=async()=>{for(let i=0;i<15;i++)await Promise.resolve();};
const response=(prefs:any,version:string,status=200)=>({ok:status>=200&&status<300,status,headers:{get:(name:string)=>name.toLowerCase()==='x-template-version'?version:null},json:async()=>clone(prefs)});

// Run the actual integration module, including startup mirroring, its debounce
// queue, HTTP headers, and localStorage writes. Only browser/HTTP boundaries and
// the unrelated theme sanitizer are replaced; SharedTemplateSync is the real one.
function fixture(storage=new Map<string,string>(), migrated=true){
  let server:any={migrated_from_localstorage:migrated,templates:rows('server-original'),display:{theme:'light',font_size:'medium',lang:'en'}};
  let version='version-1';const calls:Array<{method:string,headers:Record<string,string>,body:any}>=[];
  const timers=new Map<number,Function>();let timerID=0;
  const events:string[]=[];const warnings:unknown[]=[];
  let nextGet:Promise<any>|null=null;let beforePut:(()=>void)|null=null;let nextPutError:Error|null=null;
  const ctx:any={SharedTemplateSync,createUserPrefsPutQueue,sanitizeProjectViews,sanitizeCustomThemes:(value:any)=>value,
    token:'synthetic-token',showToast:(value:any)=>warnings.push(value),Error,Promise,JSON,Event,
    CustomEvent:class {constructor(public type:string,public options:any){}},
    console:{warn:(...args:unknown[])=>warnings.push(args),info(){}},
    window:{t:(key:string)=>key},document:{dispatchEvent:(event:any)=>{events.push(event.type);return true;}},
    localStorage:{getItem:(key:string)=>storage.get(key)??null,setItem:(key:string,value:string)=>storage.set(key,value),removeItem:(key:string)=>storage.delete(key)},
    setTimeout:(fn:Function)=>{timers.set(++timerID,fn);return timerID;},clearTimeout:(id:number)=>timers.delete(id),
    fetch:async(_url:string,init:any={})=>{
      const method=init.method||'GET';const body=init.body?JSON.parse(init.body):null;const headers=init.headers||{};
      calls.push({method,headers,body});
      if(method==='GET'){if(nextGet){const pending=nextGet;nextGet=null;return pending;}return response(server,version);}
      if(nextPutError){const error=nextPutError;nextPutError=null;throw error;}
      beforePut?.();beforePut=null;
      if(headers['X-Template-Write']==='compare'&&headers['X-Template-Version']!==version)return response({error:'conflict'},version,409);
      const preserved=server.templates;server=clone(body);
      if(headers['X-Template-Write']==='preserve'){
        if(preserved===undefined)delete server.templates;else server.templates=preserved;
      }else version=version+'-saved';
      return response(server,version);
    },
  };
  const source=readFileSync(new URL('../src/app/user-prefs.ts',import.meta.url),'utf8').replace(/^import .*;\r?$/gm,'').replace(/^export /gm,'');
  runInNewContext(new Bun.Transpiler({loader:'ts'}).transformSync(source)+'\nglobalThis.api={setUserPref,_putUserPrefsNow,flushUserPrefsPut,refreshSharedPromptTemplates,setSharedTemplateEditing,sharedTemplatesHaveConflict,resolveSharedTemplateConflict};',ctx);
  return {storage,calls,timers,events,warnings,api:ctx.api,get server(){return server;},get version(){return version;},remote(templates:any,newVersion:string){server={...server};if(templates===undefined)delete server.templates;else server.templates=clone(templates);version=newVersion;},holdGet(){let release!:(value:any)=>void;nextGet=new Promise(resolve=>{release=resolve;});return release;},beforePut(fn:()=>void){beforePut=fn;},failNextPut(){nextPutError=new Error('synthetic offline');},async fireTimer(){const entry=timers.entries().next().value;if(!entry)throw new Error('No pending debounce');const [id,fn]=entry;timers.delete(id);fn();await settle();}};
}

describe('shared templates through actual user-prefs module',()=>{
  test('stale unrelated preference save uses preserve and cannot replace remote templates',async()=>{
    const f=fixture();await settle();
    f.remote(rows('edited-on-other-device'),'version-2');
    f.api.setUserPref('display.theme','dark');await f.api.flushUserPrefsPut();
    const put=f.calls.findLast(call=>call.method==='PUT')!;
    expect(put.headers['X-Template-Write']).toBe('preserve');
    expect(put.body.templates).toEqual(rows('edited-on-other-device'));
    expect(f.server.templates).toEqual(rows('edited-on-other-device'));expect(f.server.display.theme).toBe('dark');
  });
  test('preserve also survives a remote template edit between GET and PUT',async()=>{
    const f=fixture();await settle();f.api.setUserPref('display.theme','dark');
    f.beforePut(()=>f.remote(rows('changed-after-read'),'version-2'));await f.api.flushUserPrefsPut();
    expect(f.server.templates).toEqual(rows('changed-after-read'));
    expect(f.calls.findLast(call=>call.method==='PUT')!.headers['X-Template-Write']).toBe('preserve');
  });
  test('an actual local edit compares its mirrored version and clears dirty only after acceptance',async()=>{
    const f=fixture();await settle();f.api.setUserPref('templates',rows('local-edit'));
    expect(f.storage.get(DIRTY)).toBe('1');expect(JSON.parse(f.storage.get(BASE)!).version).toBe('version-1');
    await f.api.flushUserPrefsPut();const put=f.calls.findLast(call=>call.method==='PUT')!;
    expect(put.headers['X-Template-Write']).toBe('compare');expect(put.headers['X-Template-Version']).toBe('version-1');
    expect(put.body.templates).toEqual(rows('local-edit'));expect(f.storage.has(DIRTY)).toBe(false);expect(f.storage.has(BASE)).toBe(false);expect(f.storage.get(VERSION)).toBe(f.version);
  });
  test('409 preserves the dirty local list and baseline while unrelated saves preserve server',async()=>{
    const f=fixture();await settle();f.api.setUserPref('templates',rows('unsaved-local'));f.remote(rows('remote-edit'),'version-2');
    await expect(f.api._putUserPrefsNow()).rejects.toMatchObject({status:409});
    expect(JSON.parse(f.storage.get(LIST)!)).toEqual(rows('unsaved-local'));expect(f.storage.get(DIRTY)).toBe('1');expect(JSON.parse(f.storage.get(BASE)!).version).toBe('version-1');
    expect(f.api.sharedTemplatesHaveConflict()).toBe(true);expect(f.events).toContain('shared-templates-conflict');
    expect(await f.api.refreshSharedPromptTemplates()).toBe(false);
    f.api.setUserPref('display.theme','dark');await f.api.flushUserPrefsPut();
    expect(f.calls.findLast(call=>call.method==='PUT')!.headers['X-Template-Write']).toBe('preserve');expect(f.server.templates).toEqual(rows('remote-edit'));expect(f.storage.get(DIRTY)).toBe('1');
  });
  test('a late refresh cannot overwrite a newer local edit or comparison version',async()=>{
    const f=fixture();await settle();const release=f.holdGet();const refreshing=f.api.refreshSharedPromptTemplates();
    f.api.setUserPref('templates',rows('new-local'));release(response({templates:rows('old-response')},'version-2'));
    expect(await refreshing).toBe(false);expect(JSON.parse(f.storage.get(LIST)!)).toEqual(rows('new-local'));expect(f.storage.get(VERSION)).toBe('version-1');expect(f.storage.get(DIRTY)).toBe('1');
  });
  test('omitted and explicit empty remote lists both clear the last saved template',async()=>{
    for(const empty of [undefined,[]]){const f=fixture();await settle();f.remote(empty,'version-2');expect(await f.api.refreshSharedPromptTemplates()).toBe(true);expect(JSON.parse(f.storage.get(LIST)!)).toEqual([]);expect(f.storage.get(VERSION)).toBe('version-2');}
  });
  test('editing blocks both new refreshes and an already in-flight response',async()=>{
    const f=fixture();await settle();f.api.setSharedTemplateEditing(true);const before=f.calls.length;
    expect(await f.api.refreshSharedPromptTemplates()).toBe(false);expect(f.calls.length).toBe(before);
    f.api.setSharedTemplateEditing(false);const release=f.holdGet();const pending=f.api.refreshSharedPromptTemplates();f.api.setSharedTemplateEditing(true);
    release(response({templates:rows('remote-during-form')},'version-2'));expect(await pending).toBe(false);expect(JSON.parse(f.storage.get(LIST)!)).toEqual(rows('server-original'));
    f.api.setSharedTemplateEditing(false);f.remote(rows('remote-after-form'),'version-3');expect(await f.api.refreshSharedPromptTemplates()).toBe(true);expect(JSON.parse(f.storage.get(LIST)!)).toEqual(rows('remote-after-form'));
  });
  test('persisted dirty reload retains edits, blocks mirroring, and can explicitly retry with original baseline',async()=>{
    const storage=new Map([[LIST,JSON.stringify(rows('unsaved-before-reload'))],[DIRTY,'1'],[VERSION,'version-1'],[BASE,JSON.stringify({version:'version-1'})]]);
    const f=fixture(storage);await settle();expect(JSON.parse(storage.get(LIST)!)).toEqual(rows('unsaved-before-reload'));expect(await f.api.refreshSharedPromptTemplates()).toBe(false);
    await f.api.flushUserPrefsPut();expect(f.server.templates).toEqual(rows('unsaved-before-reload'));expect(storage.has(DIRTY)).toBe(false);
    expect(f.calls.findLast(call=>call.method==='PUT')!.headers['X-Template-Version']).toBe('version-1');
  });
  test('startup automatically debounces a persisted dirty edit using its original baseline',async()=>{
    for(const migrated of [true,false]){
      const storage=new Map([[LIST,JSON.stringify(rows('unsaved-before-reload'))],[DIRTY,'1'],[VERSION,'version-1'],[BASE,JSON.stringify({version:'version-1'})]]);
      const f=fixture(storage,migrated);await settle();
      expect(f.calls.filter(call=>call.method==='PUT')).toHaveLength(0);
      expect(f.timers.size).toBe(1);expect(JSON.parse(storage.get(LIST)!)).toEqual(rows('unsaved-before-reload'));
      await f.fireTimer();
      const puts=f.calls.filter(call=>call.method==='PUT');expect(puts).toHaveLength(1);
      expect(puts[0].headers['X-Template-Write']).toBe('compare');expect(puts[0].headers['X-Template-Version']).toBe('version-1');
      expect(f.server.templates).toEqual(rows('unsaved-before-reload'));expect(storage.has(DIRTY)).toBe(false);expect(f.timers.size).toBe(0);
    }
  });
  test('focus refresh after a failed save retries the dirty edit without rebasing it',async()=>{
    const f=fixture();await settle();f.api.setUserPref('templates',rows('offline-edit'));f.failNextPut();await f.fireTimer();
    expect(f.storage.get(DIRTY)).toBe('1');expect(f.api.sharedTemplatesHaveConflict()).toBe(false);expect(f.timers.size).toBe(0);
    expect(await f.api.refreshSharedPromptTemplates()).toBe(false);expect(await f.api.refreshSharedPromptTemplates()).toBe(false);
    expect(f.timers.size).toBe(1);expect(JSON.parse(f.storage.get(BASE)!).version).toBe('version-1');
    await f.fireTimer();expect(f.server.templates).toEqual(rows('offline-edit'));expect(f.storage.has(DIRTY)).toBe(false);
    expect(f.calls.filter(call=>call.method==='PUT').map(call=>call.headers['X-Template-Version'])).toEqual(['version-1','version-1']);
  });
  test('a scheduled 409 stops automatic focus retries and preserves the conflicting edit',async()=>{
    const f=fixture();await settle();f.api.setUserPref('templates',rows('conflicting-local'));f.remote(rows('new-remote'),'version-2');
    await f.fireTimer();expect(f.api.sharedTemplatesHaveConflict()).toBe(true);expect(f.timers.size).toBe(0);
    const callsBefore=f.calls.length;
    expect(await f.api.refreshSharedPromptTemplates()).toBe(false);expect(await f.api.refreshSharedPromptTemplates()).toBe(false);await settle();
    expect(f.timers.size).toBe(0);expect(f.calls.length).toBe(callsBefore);
    expect(f.storage.get(DIRTY)).toBe('1');expect(JSON.parse(f.storage.get(BASE)!).version).toBe('version-1');
    expect(JSON.parse(f.storage.get(LIST)!)).toEqual(rows('conflicting-local'));expect(f.server.templates).toEqual(rows('new-remote'));
  });
  test('explicit conflict resolution keeps local against a fresh version or discards it for remote',async()=>{
    for(const keepLocal of [true,false]){const f=fixture();await settle();f.api.setUserPref('templates',rows('local'));f.remote(rows('remote'),'version-2');await expect(f.api._putUserPrefsNow()).rejects.toMatchObject({status:409});
      expect(await f.api.resolveSharedTemplateConflict(keepLocal)).toBe(true);expect(f.api.sharedTemplatesHaveConflict()).toBe(false);
      if(keepLocal){await f.api.flushUserPrefsPut();expect(f.server.templates).toEqual(rows('local'));expect(f.calls.findLast(call=>call.method==='PUT')!.headers['X-Template-Version']).toBe('version-2');}
      else{expect(JSON.parse(f.storage.get(LIST)!)).toEqual(rows('remote'));expect(f.storage.has(DIRTY)).toBe(false);}
    }
  });
});
