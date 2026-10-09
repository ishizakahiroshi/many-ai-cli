import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import * as model from '../src/app/session-list-tab-state.js';

// Like routines-dom.test.ts: execute actual source/listeners with a minimal synthetic DOM.
// This is caller evidence, not browser layout/native-input/Windows acceptance. No new dependency.
class El {
  children: El[] = []; parentElement: El | null = null; attrs = new Map<string, string>();
  listeners = new Map<string, Array<{ fn: Function; capture: boolean }>>();
  id=''; className=''; hidden=false; disabled=false; open=false; value=''; type=''; tabIndex=0;
  scrollTop=0; scrollLeft=0; scrollHeight=1000; clientHeight=300; dataset: Record<string,string>={}; style: Record<string,string>={};
  onclick: Function | null=null; onsubmit: Function | null=null; text='';
  constructor(public tagName: string, public doc: any) {}
  get isConnected(): boolean { return this === this.doc.body || !!this.parentElement?.isConnected; }
  get textContent(): string { return this.text + this.children.map(child=>child.textContent).join(''); }
  set textContent(value: string) { this.text=String(value); this.replaceChildren(); }
  get classList() { const self=this;return { contains:(name:string)=>self.className.split(' ').includes(name),
    add:(...names:string[])=>{self.className=[...new Set([...self.className.split(' '),...names])].join(' ');},
    remove:(...names:string[])=>{self.className=self.className.split(' ').filter(name=>!names.includes(name)).join(' ');},
    toggle:(name:string,on:boolean)=>{if(on)self.classList.add(name);else self.classList.remove(name);}}; }
  append(...nodes:El[]) { for(const node of nodes){node.remove();node.parentElement=this;this.children.push(node);} }
  appendChild(node:El){this.append(node);return node;}
  replaceChildren(...nodes:El[]){for(const child of this.children)child.parentElement=null;this.children=[];this.append(...nodes);}
  remove(){if(this.parentElement)this.parentElement.children=this.parentElement.children.filter(child=>child!==this);this.parentElement=null;}
  setAttribute(key:string,value:string){this.attrs.set(key,String(value));}
  getAttribute(key:string){return this.attrs.get(key)??null;}
  contains(node:El|null):boolean{return node===this||this.children.some(child=>child.contains(node));}
  matches(selector:string):boolean {
    if(selector.includes(','))return selector.split(',').some(part=>this.matches(part.trim()));
    const enabled=selector.includes(':not(:disabled)'); selector=selector.replace(':not(:disabled)','');if(enabled&&this.disabled)return false;
    for(const [,name,value] of selector.matchAll(/\[([\w-]+)(?:="([^"]*)")?\]/g)){
      const actual=name.startsWith('data-')?this.dataset[name.slice(5).replace(/-([a-z])/g,(_,c)=>c.toUpperCase())]:this.getAttribute(name);
      if(actual==null||value!==undefined&&actual!==value)return false;
    }
    const rest=selector.replace(/\[[^\]]*\]/g,'');
    for(const [,name] of rest.matchAll(/\.([\w-]+)/g))if(!this.classList.contains(name))return false;
    const id=/#([\w-]+)/.exec(rest)?.[1];if(id&&id!==this.id)return false;
    const tag=/^[a-z]+/.exec(rest)?.[0];return !tag||this.tagName===tag;
  }
  closest(selector:string):El|null{return this.matches(selector)?this:this.parentElement?.closest(selector)||null;}
  querySelectorAll(selector:string):El[]{return this.children.flatMap(child=>[...(child.matches(selector)?[child]:[]),...child.querySelectorAll(selector)]);}
  querySelector(selector:string):El|null{return this.querySelectorAll(selector)[0]||null;}
  getBoundingClientRect(){return {left:0,right:240,top:0,bottom:34,width:240,height:34};}
  addEventListener(type:string,fn:Function,options:any=false){this.listeners.set(type,[...(this.listeners.get(type)||[]),{fn,capture:options===true||!!options?.capture}]);}
  focus(){this.doc.activeElement=this;} select(){} showModal(){this.open=true;} close(){this.open=false;this.emit('close');}
  emit(type:string,extra:Record<string,any>={}) {
    const event:any={target:this,button:0,pointerType:'mouse',clientX:0,clientY:0,pointerId:1,defaultPrevented:false,
      preventDefault(){this.defaultPrevented=true;},stopPropagation(){this.stopped=true;},stopImmediatePropagation(){this.stopped=true;this.immediate=true;},...extra};
    const chain:El[]=[];for(let p:El|null=this;p;p=p.parentElement)chain.push(p);
    const call=(node:El,capture:boolean)=>{for(const item of node.listeners.get(type)||[]){if(item.capture===capture)item.fn(event);if(event.immediate)break;}};
    for(const node of chain.slice().reverse()){call(node,true);if(event.stopped)return event;}
    for(const node of chain){call(node,false);if(!event.immediate&&node===this)(type==='click'?node.onclick:type==='submit'?node.onsubmit:null)?.(event);if(event.stopped)break;}
    return event;
  }
  click(){if(!this.disabled)this.emit('click');}
}
function fixture(saved?:unknown) {
  const timers=new Map<number,Function>(), frames=new Map<number,Function>();let seq=0;const sessions:any[]=[{id:1,started_at:'start-1',cwd:'/repos/example'},{id:2,started_at:'start-2',cwd:'/repos/example',parent_session_id:1},{id:3,started_at:'start-3',cwd:'/repos/example'}];
  const doc:any={activeElement:null};doc.body=new El('body',doc);doc.createElement=(tag:string)=>new El(tag,doc);
  doc.querySelector=(selector:string)=>doc.body.querySelector(selector);doc.getElementById=(id:string)=>all().find(el=>el.id===id)||null;
  doc.addEventListener=doc.body.addEventListener.bind(doc.body);
  const all=(root:El=doc.body):El[]=>[root,...root.children.flatMap(child=>all(child))];
  const el=(tag:string,id:string,parent:El=doc.body)=>{const node=new El(tag,doc);node.id=id;parent.append(node);return node;};
  const sidebar=el('aside','session-list'), strip=el('div','session-list-tabs',sidebar), add=el('button','session-list-tab-add',sidebar), cards=el('div','sessions',sidebar);el('p','session-list-tabs-storage',sidebar);
  let storage=JSON.stringify(saved??null), writes=0, active=1, menus=0;
  const window=new El('window',doc);
  const ctx:any={...model,document:doc,window,Element:El,HTMLElement:El,HTMLInputElement:El,HTMLSelectElement:El,
    innerWidth:1280,innerHeight:900,CSS:{escape:(value:string)=>value},crypto:{getRandomValues:(array:Uint32Array)=>array.fill(++seq)},
    t:(key:string)=>key,localStorage:{getItem:()=>storage,setItem:(_key:string,value:string)=>{storage=value;writes++;}},
    setTimeout:(fn:Function)=>{timers.set(++seq,fn);return seq;},clearTimeout:(id:number)=>timers.delete(id),
    requestAnimationFrame:(fn:Function)=>{frames.set(++seq,fn);return seq;},cancelAnimationFrame:(id:number)=>frames.delete(id),
  };
  const source=readFileSync(new URL('../src/app/session-list-tabs.ts',import.meta.url),'utf8').replace(/^import[\s\S]*?;\r?$/gm,'').replace(/^export /gm,'');
  runInNewContext(new Bun.Transpiler({loader:'ts'}).transformSync(source)+'\nglobalThis.ui={initializeSessionListTabs,renderSessionListTabs,visibleSessionListTree,revealSessionListOwner,preserveSessionListSelection,sessionListScrollBeforeRender,sessionListScrollAfterRender,restoreSessionListDrawerScroll,openSessionListMoveDialog,sessionListMoveLabel,sessionListCanMove,sessionListTapSuppressed,restoreSessionListFocus};',ctx);
  const ui=ctx.ui;
  const render=()=>{const top=ui.sessionListScrollBeforeRender(cards);cards.replaceChildren();for(const session of sessions){if(!ui.visibleSessionListTree([{key:'example',label:'example',favorite:false,children:[{id:session.id,depth:0,children:[]}]}],sessions).length)continue;const row=el('div','',cards);row.className='card';row.dataset.sessionId=String(session.id);}ui.sessionListScrollAfterRender(cards,top);};
  ui.initializeSessionListTabs({sessions:()=>sessions,render,cardMenu:()=>{menus++;},resetDrag:()=>{},pending:()=>false});ui.renderSessionListTabs();render();
  // Execute the existing product card pointerup/click delegates, rather than a hand-written substitute.
  const listSource=readFileSync(new URL('../src/app/session-list.ts',import.meta.url),'utf8');
  const listeners=listSource.slice(listSource.indexOf('  if (!_sessionListClickDelegated) {'),listSource.indexOf("  root.innerHTML = '';",listSource.indexOf('export function renderSessionList()')));
  Object.assign(ctx,{root:cards,_sessionListClickDelegated:false,_sessionCardPointerDown:null,isSessionCardInteractiveTarget:()=>false,sessionCardTapMoveLimit:()=>18,markSessionCardPointerActivated:()=>{},onSessionCardActivate:(id:number)=>{active=id;},consumeDuplicateSessionCardClick:()=>false,openCardCtxMenu:()=>{menus++;}});
  runInNewContext(new Bun.Transpiler({loader:'ts'}).transformSync(listeners),ctx);
  return {ui,ctx,doc,sidebar,strip,add,cards,sessions,all,el,render,timers,frames,get storage(){return JSON.parse(storage);},get writes(){return writes;},get active(){return active;},get menus(){return menus;},flush(){for(const [id,fn]of [...frames]){frames.delete(id);fn();}},hold(){for(const[id,fn]of[...timers]){timers.delete(id);fn();}}};
}

test('actual UI: add/switch preserves active conversation; waiting badge, literal rename and blank rejection',()=>{
  const f=fixture();f.flush();f.add.click();assert.equal(f.strip.children.length,2);assert.equal(f.active,1);assert.equal(f.cards.children.length,0);
  const first=f.strip.children[0];first.click();assert.equal(f.cards.children.length,3);
  f.strip.children[0].emit('contextmenu',{clientX:10,clientY:10});const rename=f.all().find(el=>el.textContent==='Rename tab')!;rename.click();
  const input=f.all().find(el=>el.id==='session-list-tab-dialog-value')!;input.value='  ';input.parentElement!.emit('submit');assert.equal(f.strip.children[0].textContent,'Tab 1');
  input.value='<img src=x>';input.parentElement!.emit('submit');assert.equal(f.strip.children[0].textContent,'<img src=x>');
  f.sessions[2].awaiting_approval=true;f.add.click();assert.ok(f.strip.children[0].textContent.includes('1'));
});
test('actual card pointerup: long press survives live rerender without activating; scrolling/cancel leaves taps alone',()=>{
  const f=fixture();f.flush();const original=f.cards.children[2];original.emit('pointerdown',{pointerType:'touch'});f.render();f.hold();
  assert.equal(f.menus,1);const replacement=f.cards.children[2];replacement.emit('pointermove',{pointerType:'touch',clientY:12});replacement.emit('pointerup',{pointerType:'touch'});replacement.click();assert.equal(f.active,1);
  const fresh=fixture();fresh.flush();const row=fresh.cards.children[2];row.emit('pointerdown',{pointerType:'touch'});row.emit('pointermove',{pointerType:'touch',clientY:30});fresh.hold();assert.equal(fresh.menus,0);
  row.emit('pointercancel',{pointerType:'touch'});assert.equal(fresh.timers.size,0);
});
test('actual move dialog rejects ID reuse and restores focus to selected tab after a successful move',()=>{
  const f=fixture();f.flush();f.add.click();f.strip.children[0].click();const origin=f.cards.children[2];
  f.ui.openSessionListMoveDialog(3,origin);const select=f.all().find(el=>el.id==='session-list-tab-dialog-value')!;select.value=f.storage.tabs[1].id;
  f.sessions[2].started_at='replacement';select.parentElement!.emit('submit');assert.equal(f.cards.children.length,3);assert.ok(f.all().some(el=>el.tagName==='dialog'));
  f.all().find(el=>el.tagName==='dialog')!.close();f.ui.openSessionListMoveDialog(3,origin);
  const next=f.all().find(el=>el.id==='session-list-tab-dialog-value')!;next.value=f.storage.tabs[1].id;next.parentElement!.emit('submit');assert.equal(f.cards.children.length,2);assert.equal(f.doc.activeElement,f.strip.children[0]);
});
test('actual selection boundary preserves startup tab but reveals explicit session selections',()=>{
  const saved=model.normalizeSessionListTabs(null);model.addSessionListTab(saved,'Other','other');
  const f=fixture(saved);assert.equal(f.strip.children[1].getAttribute('aria-selected'),'true');
  f.ui.preserveSessionListSelection(()=>f.ui.revealSessionListOwner(1));assert.equal(f.strip.children[1].getAttribute('aria-selected'),'true');
  f.ui.revealSessionListOwner(1);assert.equal(f.strip.children[0].getAttribute('aria-selected'),'true');
});
test('mobile restore survives hidden desktop rendering and first-open pre-class rendering',()=>{
  const saved=model.normalizeSessionListTabs(null);model.addSessionListTab(saved,'Other','other');saved.scroll.other=180;
  const f=fixture(saved);const drawer=f.el('div','mobile-drawer-content',f.sidebar),body=f.el('div','',drawer);body.className='mobile-drawer-body';body.scrollTop=8;
  assert.equal(f.ui.sessionListScrollBeforeRender(body,true),180);
  f.doc.body.classList.add('mobile-drawer-open');
  f.ctx.window.renderMobileSessionDrawer=()=>{const top=f.ui.sessionListScrollBeforeRender(body,true);f.ui.sessionListScrollAfterRender(body,top,true);};
  f.strip.children[0].click();body.scrollTop=37;f.flush();body.emit('scroll');f.strip.children[1].click();assert.equal(body.scrollTop,180);
});
test('drag payloads cannot consume foreign pane/project data; drop/cancel stop edge frames',()=>{
  const f=fixture();f.flush();f.add.click();f.strip.children[0].click();
  const foreign={types:['application/x-many-ai-cli-pane'],setData(){},getData(){return '{"kind":"tab"}';}};
  const before=JSON.stringify(f.storage);const rejected=f.strip.children[1].emit('dragover',{dataTransfer:foreign});assert.equal(rejected.defaultPrevented,false);assert.equal(JSON.stringify(f.storage),before);
  const data=new Map<string,string>();const transfer={get types(){return [...data.keys()];},setData:(k:string,v:string)=>data.set(k,v)};
  f.cards.children[2].emit('dragstart',{dataTransfer:transfer});assert.ok(data.has(model.SESSION_LIST_CARD_DRAG));
  f.strip.children[1].emit('dragover',{dataTransfer:transfer,clientX:239});assert.ok(f.frames.size>0);
  f.strip.children[1].emit('drop',{dataTransfer:transfer,clientX:239});assert.equal(f.cards.children.length,2);f.flush();assert.equal(f.frames.size,0);
});

test('actual card-menu close does not steal focus on subsequent scroll or outside dismissal',()=>{
  const f=fixture();f.flush();const origin=f.cards.children[0],input=f.el('input','synthetic-input');
  const source=readFileSync(new URL('../src/app/settings.ts',import.meta.url),'utf8');
  const start=source.indexOf('export function closeCardCtxMenu('),end=source.indexOf('// ─── Ctrl+Shift+G',start);
  Object.assign(f.ctx,{_cardCtxMenuEl:f.el('div','card-ctx-menu'),_cardCtxSid:1,cardCtxReturnFocus:origin,restoreSessionListFocus:f.ui.restoreSessionListFocus});
  runInNewContext(new Bun.Transpiler({loader:'ts'}).transformSync(source.slice(start,end).replace(/^export /gm,'')),f.ctx);
  input.focus();const menu=f.ctx._cardCtxMenuEl;menu.emit('scroll');assert.equal(f.ctx._cardCtxMenuEl,menu);
  f.ctx.closeCardCtxMenu();assert.equal(f.doc.activeElement,input);
  f.cards.emit('scroll');assert.equal(f.doc.activeElement,input);
  f.ctx._cardCtxMenuEl=f.el('div','card-ctx-menu');f.ctx.cardCtxReturnFocus=origin;f.ctx.closeCardCtxMenu(true);assert.equal(f.doc.activeElement,origin);
  input.focus();f.cards.emit('scroll');assert.equal(f.doc.activeElement,input);
});
test('actual delete menu preserves sessions and cancel leaves storage unchanged; final tab stays protected',()=>{
  const f=fixture();f.flush();f.add.click();f.strip.children[0].click();
  const original=JSON.stringify(f.sessions),before=JSON.stringify(f.storage);
  f.strip.children[0].emit('contextmenu');f.all().find(el=>el.tagName==='button'&&el.textContent==='Delete tab')!.click();
  const current=f.all().find(el=>el.tagName==='dialog')!;assert.ok(current);
  current.close();assert.equal(JSON.stringify(f.storage),before);
  f.strip.children[0].emit('contextmenu');f.all().find(el=>el.tagName==='button'&&el.textContent==='Delete tab')!.click();
  const select=f.all().find(el=>el.id==='session-list-tab-dialog-value')!;select.value=f.storage.tabs[1].id;select.parentElement!.emit('submit');
  assert.equal(f.strip.children.length,1);assert.equal(JSON.stringify(f.sessions),original);assert.equal(f.cards.children.length,3);
  f.strip.children[0].emit('contextmenu');assert.equal(f.all().find(el=>el.tagName==='button'&&el.textContent==='Delete tab')!.disabled,true);
});

test('actual mobile-open caller restores scroll after display:none stops clamping it',()=>{
  const saved=model.normalizeSessionListTabs(null);saved.scroll['tab-1']=180;
  const f=fixture(saved);const drawer=f.el('div','mobile-drawer-content',f.sidebar),body=f.el('div','',drawer);body.className='mobile-drawer-body';
  let y=0;Object.defineProperty(body,'scrollTop',{get:()=>y,set:value=>{y=f.doc.body.classList.contains('mobile-drawer-open')?value:0;}});
  f.ctx.isMobileViewport=()=>true;f.ctx.window.renderMobileSessionDrawer=()=>{f.ui.sessionListScrollAfterRender(body,f.ui.sessionListScrollBeforeRender(body,true),true);};
  f.ctx.restoreSessionListDrawerScroll=f.ui.restoreSessionListDrawerScroll;
  const source=readFileSync(new URL('../src/app.ts',import.meta.url),'utf8');
  const start=source.indexOf('export function openMobileSessionDrawer()'),end=source.indexOf('export function closeMobileSessionDrawer()',start);
  runInNewContext(new Bun.Transpiler({loader:'ts'}).transformSync(source.slice(start,end).replace(/^export /gm,'')),f.ctx);
  f.ctx.openMobileSessionDrawer();f.flush();assert.equal(body.scrollTop,180);
});

test('tab menus use the established native-wheel marker',()=>{
  const f=fixture();f.strip.children[0].emit('contextmenu');
  assert.equal(f.all().find(el=>el.classList.contains('session-list-tab-menu'))!.getAttribute('data-wheel-native'),'');
});

test('C3-R1: a reused session ID during a hold cannot open the replacement menu or activate it',()=>{
  const f=fixture();f.flush();f.cards.children[2].emit('pointerdown',{pointerType:'touch'});
  f.sessions[2].started_at='replacement-during-hold';f.render();f.hold();
  assert.equal(f.menus,0);f.cards.children[2].emit('pointerup',{pointerType:'touch'});f.cards.children[2].click();assert.equal(f.active,1);
});

test('actual tab drag listeners reorder tabs with feedback without emitting a pane payload',()=>{
 const f=fixture();f.flush();f.add.click();f.add.click();const before=f.strip.children.map(el=>el.dataset.sessionListTab);
 const data=new Map<string,string>();const transfer={get types(){return [...data.keys()];},setData:(k:string,v:string)=>data.set(k,v)};
 f.strip.children[2].emit('dragstart',{dataTransfer:transfer});assert.equal(data.has('application/x-many-ai-cli-pane'),false);
 f.strip.children[0].emit('dragover',{dataTransfer:transfer,clientX:239});assert.equal(f.strip.children[0].classList.contains('drop-after'),true);
 f.strip.children[0].emit('drop',{dataTransfer:transfer,clientX:239});f.flush();
 assert.deepEqual(f.strip.children.map(el=>el.dataset.sessionListTab),[before[0],before[2],before[1]]);assert.equal(f.active,1);
});
test('actual existing card drag listener keeps its pane payload alongside membership movement',()=>{
 const f=fixture();f.flush();const c=f.cards.children[2];const s=f.sessions[2];const text=readFileSync(new URL('../src/app/session-list.ts',import.meta.url),'utf8');
 const start=text.indexOf("      c.addEventListener('dragstart'"),end=text.indexOf("      c.addEventListener('dragend'",start);
 Object.assign(f.ctx,{c,s,set_dragSrcId:()=>{},set_dragSrcGroupKey:()=>{}});
 runInNewContext(new Bun.Transpiler({loader:'ts'}).transformSync(text.slice(start,end)),f.ctx);
 const data=new Map<string,string>();const transfer={get types(){return [...data.keys()];},setData:(k:string,v:string)=>data.set(k,v)};
 c.emit('dragstart',{dataTransfer:transfer});assert.equal(data.has(model.SESSION_LIST_CARD_DRAG),true);
 assert.deepEqual(JSON.parse(data.get('application/x-many-ai-cli-pane')!),{kind:'session',sessionId:s.id});
});
