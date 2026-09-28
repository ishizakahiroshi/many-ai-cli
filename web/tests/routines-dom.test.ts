import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { isRoutineRunActive, routineRunFromURL, routineSessionMatches } from '../src/app/routines-model';

// Executes the actual module in a synthetic DOM/HTTP environment; no Hub, browser,
// service worker registration, or web/dist build is involved.
class Element {
  children: Element[] = []; parentElement: Element | null = null;
  listeners = new Map<string, Function[]>(); attrs = new Map<string, string>();
  id='';className='';hidden=false;disabled=false;open=false;value='';name='';checked=false;scrollTop=0;selectedIndex=0;
  dataset: Record<string,string> = {}; private text='';
  constructor(public tagName: string, private document: any) {}
  get textContent(): string { return this.text + this.children.map(child => child.textContent).join(''); }
  set textContent(value: string) { this.text = value; this.children = []; }
  get lastElementChild(): Element | undefined { return this.children.at(-1); }
  classList={add:(name:string)=>{this.className+=' '+name;}};
  append(...nodes:Element[]):void { for(const node of nodes){node.parentElement=this;this.children.push(node);} }
  replaceChildren(...nodes:Element[]):void{this.children=[];this.text='';this.append(...nodes);}
  setAttribute(key:string,value:string):void{this.attrs.set(key,value);}
  addEventListener(name:string,listener:Function):void{this.listeners.set(name,[...(this.listeners.get(name)||[]),listener]);}
  async emit(name:string,event:object={}):Promise<void>{for(const listener of this.listeners.get(name)||[])await listener(event);}
  click():void{if(!this.disabled)void this.emit('click');}
  remove():void{if(this.parentElement)this.parentElement.children=this.parentElement.children.filter(node=>node!==this);}
  focus():void{this.document.activeElement=this;}
  showModal():void{this.open=true;}
  close():void{this.open=false;void this.emit('close');}
}
const tick=async()=>{for(let i=0;i<12;i++)await Promise.resolve();};
function fixture(url='https://hub.example/', ready=true){
  const listeners=new Map<string,Function[]>();const timers=new Map<number,Function>();let timerID=0;
  const document:any={documentElement:{lang:'en'},readyState:'complete',activeElement:null,addEventListener:(type:string,fn:Function)=>listeners.set(type,[...(listeners.get(type)||[]),fn])};
  document.createElement=(tag:string)=>new Element(tag,document);document.body=new Element('body',document);
  const calls:Array<{path:string,init:any}>=[];let respond:(path:string,init:any)=>any=()=>({run:run()});
  const sessionMap=new Map();const ctx:any={document,HTMLElement:Element,Element,URL,Intl,Date,Promise,Error,crypto:{randomUUID:()=> 'synthetic-request'},location:{href:url},
    history:{state:null,replaceState(_state:any,_title:string,target:URL){ctx.location.href=String(target);}},
    window:{t:ready?((key:string)=>key):undefined,addEventListener:(type:string,fn:Function)=>listeners.set(type,[...(listeners.get(type)||[]),fn])},
    t:(key:string)=>key,activeSessionId:null,sessions:sessionMap,activateSession:()=>{},getPromptTemplates:()=>[],ORCHESTRATION_CLI_OPTIONS:[{value:'codex',label:'Codex'}],
    isRoutineRunActive,routineRunFromURL,routineSessionMatches,
    apiFetch:async(path:string,init:any)=>{calls.push({path,init});const value=await respond(path,init);return value?.httpError?{ok:false,status:value.httpError,text:async()=>value.message||'conflict'}:{ok:true,status:200,json:async()=>value};},
    setTimeout:(fn:Function)=>{timers.set(++timerID,fn);return timerID;},clearTimeout:(id:number)=>timers.delete(id),queueMicrotask,
  };
  const source=readFileSync(new URL('../src/app/routines.ts',import.meta.url),'utf8').replace(/^import .*;\r?$/gm,'').replace(/^export /gm,'');
  runInNewContext(new Bun.Transpiler({loader:'ts'}).transformSync(source)+'\nglobalThis.ui={openRoutines,editRoutine,showRoutine};',ctx);
  const all=(root:Element=document.body):Element[]=>[root,...root.children.flatMap(child=>all(child))];
  return {ctx,calls,timers,sessions:sessionMap,all,document,ui:ctx.ui,setResponse(fn:typeof respond){respond=fn;},find(predicate:(el:Element)=>boolean){const el=all().find(predicate);if(!el)throw new Error('element missing');return el;},async poll(){const [id,fn]=timers.entries().next().value||[];if(fn){timers.delete(id);fn();await tick();}},async event(name:string){for(const fn of listeners.get(name)||[])fn();await tick();}};
}
function run(){return {id:'run-one',routine_id:'routine-one',routine_name:'Review',status:'finished',session_id:7,session_label:'unique-run-one',provider:'codex',cwd:'sample',prompt:'Review synthetic changes',started_at:'2026-09-28T01:00:00Z',updated_at:'2026-09-28T01:00:00Z',result:'Result',summary:'',result_available:true};}
function routine(){return {id:'routine-one',name:'Review',provider:'codex',cwd:'sample',prompt:'Review',enabled:true,schedule:{kind:'manual',time:'09:00',timezone:'UTC'},updated_at:'old-version'};}

describe('routines async DOM paths',()=>{
  test('finished run reevaluates late session arrival while keeping details and focus',async()=>{
    const f=fixture();f.ui.openRoutines('run-one');await tick();
    const details=f.find(el=>el.tagName==='details');details.open=true;
    const summary=details.children[0];summary.focus();const content=f.find(el=>el.className==='routines-content');content.scrollTop=75;
    f.sessions.set(7,{id:7,label:'renamed from card',launch_label:'unique-run-one'});await f.poll();
    expect(f.find(el=>el.tagName==='details')).toBe(details);expect(details.open).toBe(true);expect(f.document.activeElement).toBe(summary);expect(content.scrollTop).toBe(75);
    expect(f.find(el=>el.tagName==='button'&&el.textContent==='routines_open_session').hidden).toBe(false);
  });
  test('late HTTP response cannot update a closed dialog or restart polling',async()=>{
    const f=fixture();let resolve!:(value:unknown)=>void;f.setResponse(()=>new Promise(done=>{resolve=done;}));
    f.ui.openRoutines('run-one');f.find(el=>el.tagName==='dialog').close();resolve({run:run()});await tick();
    expect(f.timers.size).toBe(0);expect(f.all().some(el=>el.tagName==='h3'&&el.textContent==='Review')).toBe(false);
  });
  test('cancel delete restores the entry button and permits another confirmation',async()=>{
    const f=fixture();f.ui.openRoutines();f.ui.editRoutine(routine());await tick();
    const remove=f.find(el=>el.tagName==='button'&&el.textContent==='routines_delete');remove.click();await tick();expect(remove.disabled).toBe(true);
    f.find(el=>el.tagName==='button'&&el.textContent==='routines_cancel').click();await tick();expect(remove.disabled).toBe(false);
    remove.click();await tick();expect(f.all().filter(el=>el.tagName==='button'&&el.textContent==='routines_cancel')).toHaveLength(1);
  });
  test('save conflict leaves changed form values available for retry',async()=>{
    const f=fixture();f.ui.openRoutines();f.ui.editRoutine(routine());f.setResponse(()=>({httpError:409,message:'version conflict'}));
    const prompt=f.find(el=>el.tagName==='textarea');prompt.value='unsaved edited instruction';
    await f.find(el=>el.tagName==='form').emit('submit',{preventDefault(){}});await tick();
    expect(prompt.value).toBe('unsaved edited instruction');expect(f.find(el=>el.tagName==='button'&&el.textContent==='routines_save').disabled).toBe(false);
    expect(f.find(el=>el.className==='routines-feedback').textContent).toContain('409');
  });
  test('startup notification opens once whether i18n already completed or arrives later',async()=>{
    for(const ready of [true,false]){const f=fixture('https://hub.example/?routine_run=run-one',ready);if(!ready)await f.event('i18n-ready');await tick();await f.event('i18n-ready');
      expect(f.calls.filter(call=>call.path==='/api/routine-runs/run-one')).toHaveLength(1);expect(f.ctx.location.href).not.toContain('routine_run');}
  });
  test('UUID failure is displayed and re-enables launch',async()=>{
    const f=fixture();f.ui.openRoutines();f.ui.showRoutine(routine());f.ctx.crypto.randomUUID=()=>{throw new Error('UUID unavailable');};
    f.find(el=>el.tagName==='button'&&el.textContent==='routines_run').click();await tick();
    expect(f.find(el=>el.className==='routines-feedback').textContent).toContain('UUID unavailable');expect(f.find(el=>el.tagName==='button'&&el.textContent==='routines_retry_run').disabled).toBe(false);
  });
});
