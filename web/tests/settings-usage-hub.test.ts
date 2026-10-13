import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { createRequire } from 'node:module';
const ts = createRequire(import.meta.url)('typescript');

// Execute actual Settings, Usage and exposure listeners in synthetic DOM/HTTP.
// No Hub, product build, browser, network or new dependency is used.
const read = (path: string) => readFileSync(new URL(path, import.meta.url), 'utf8');
const settings = read('../src/app/settings.ts');
const html = read('../src/index.html');
const clean = (source: string) => source.replace(/^import[\s\S]*?;\r?$/gm, '').replace(/^export /gm, '');
const run = (source: string, ctx: any) => runInNewContext(ts.transpileModule(clean(source), { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText, ctx);
const tick = async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); };
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
  removeAttribute(key:string){this.attrs.delete(key);}
  scrollIntoView(){} scrollTo(options:any){this.scrollTop=options.top||0;}
  cloneNode(){const el=new El(this.tagName,this.doc);el.className=this.className;return el;}
  dispatchEvent(event:any){return this.emit(event.type,event);}
  contains(node:El|null):boolean{return node===this||this.children.some(child=>child.contains(node));}
  matches(selector:string):boolean {
    if(selector==='[hidden]')return this.hidden;
    if(selector==='summary > span:first-child')return this.tagName==='span'&&this.parentElement?.tagName==='summary'&&this.parentElement.children[0]===this;
    if(selector.includes(','))return selector.split(',').some(part=>this.matches(part.trim()));
    if(selector.includes(' ')){const cut=selector.lastIndexOf(' ');return this.matches(selector.slice(cut+1))&&!!this.parentElement?.closest(selector.slice(0,cut));}
    const visible=selector.includes(':not([hidden])'); selector=selector.replace(':not([hidden])','');if(visible&&this.hidden)return false;
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
function fixture(mobile = false) {
  const doc: any = { activeElement: null }; doc.body = new El('body', doc);
  const all = (root: El = doc.body): El[] => [root, ...root.children.flatMap(child => all(child))];
  doc.createElement = (tag: string) => new El(tag, doc);
  doc.getElementById = (id: string) => all().find(el => el.id === id) || null;
  doc.querySelector = (selector: string) => doc.body.querySelector(selector);
  doc.querySelectorAll = (selector: string) => doc.body.querySelectorAll(selector);
  doc.addEventListener = doc.body.addEventListener.bind(doc.body);
  doc.removeEventListener = (type: string, fn: Function) => doc.body.listeners.set(type, (doc.body.listeners.get(type) || []).filter(item => item.fn !== fn));
  doc.dispatchEvent = (event: any) => doc.body.emit(event.type, event);
  const el = (tag: string, id: string, parent: El = doc.body) => { const node = new El(tag, doc); node.id = id; parent.append(node); return node; };
  const panel = el('div', 'settings-panel'); panel.hidden = true; panel.className = 'aac-wheel-overlay';
  const body = el('div', '', panel); body.className = 'settings-body';
  const nav = el('nav', 'settings-nav', panel); el('div', 'settings-nav-list', nav);
  const search = el('input', 'settings-search-input', nav); el('p', 'settings-search-status', nav);
  for (const id of ['settings-back-btn', 'settings-close-btn', 'settings-save-btn', 'settings-reset-btn']) el('button', id, panel);
  el('button', 'settings-btn'); el('span', 'settings-expose-dot');
  const sections = new Map<string, El>();
  // Section identities/grouping and destination labels come from the actual HTML.
  for (const match of html.matchAll(/<details class="settings-section[^"]*" data-section="([^"]+)" data-group="([^"]+)">[\s\S]*?<summary[^>]*><span[^>]*>([^<]*)<\/span>/g)) {
    const section = el('details', '', body); section.className = 'settings-section';
    section.dataset.section = match[1]; section.dataset.group = match[2]; sections.set(match[1], section);
    const summary = el('summary', '', section); el('span', '', summary).textContent = match[3];
    const current = el('span', match[1] === 'hub' ? 'settings-hub-state' : '', summary); current.className = 'settings-section-current';
    if (['usage', 'hub'].includes(match[1])) el('svg', '', summary).className = 'settings-destination-icon';
  }
  el('div', 'settings-usage-panel', sections.get('usage'));
  const hub = el('div', 'settings-hub-actions', sections.get('hub'));
  for (const id of ['expose-btn', 'mobile-connect-btn', 'shutdown-btn', 'bug-report-btn']) el('button', id, hub);
  el('span', 'expose-dot', doc.getElementById('expose-btn')); el('span', 'expose-state', doc.getElementById('expose-btn'));
  for (const id of ['theme-select', 'fontsize-select', 'lang-select', 'usage-links-reset-btn', 'provider-order-reset-btn', 'settings-readme-btn', 'settings-licenses-btn']) el('input', id, sections.get('general'));
  el('div', 'about-panel').hidden = true; el('button', 'about-close-btn'); el('div', 'session-inline-usage');
  let seq = 0, saves = 0, resets = 0, confirmed = true;
  const timers = new Map<number, Function>(), frames: Function[] = [];
  const media = { matches: mobile, addEventListener: (_type: string, fn: Function) => { media.change = fn; }, change: (_event?: any) => {} };
  const window: any = new El('window', doc); window.matchMedia = () => media; window.location = { hash: '' };
  window.__settingsSaveAll = async () => { saves++; }; window.__settingsResetAll = async () => { resets++; };
  const ctx: any = { document: doc, window, Element: El, HTMLElement: El, HTMLInputElement: El, HTMLSelectElement: El,
    CustomEvent: class { constructor(public type: string) {} },
    localStorage: { getItem: () => null, setItem: () => {}, removeItem: () => {} },
    setTimeout: (fn: Function) => { timers.set(++seq, fn); return seq; }, clearTimeout: (id: number) => timers.delete(id),
    requestAnimationFrame: (fn: Function) => frames.push(fn), getComputedStyle: (el: El) => ({ display: el.hidden ? 'none' : 'block' }),
    t: (key: string) => key, showToast: () => {}, pathPopupEl: null,
    appConfirm: async () => confirmed, renderSettingsSummary: () => {}, attachSummaryToggleListeners: () => {},
    applyTheme: () => {}, applyFontSize: () => {}, applyLang: () => {}, applyUsageLinks: () => {}, initUsageSettings: () => {},
    maybeAutoSwitchToNextApproval: () => {}, bindCustomThemeControls: () => {}, setUserPref: () => {},
    STORAGE_THEME_KEY: '', STORAGE_FONTSIZE_KEY: '', STORAGE_LANG_KEY: '', queueMicrotask,
  };
  window.setTimeout = ctx.setTimeout;
  run(clean(read('../src/app/settings-visibility.ts')), ctx);
  const first = settings.indexOf('const SETTINGS_DETAIL_CLASS');
  const last = settings.indexOf('// =============================================================================', first);
  const architecture = settings.slice(settings.indexOf('let _openSettingsSectionImpl'), settings.indexOf('// データ削除は確認ダイアログ'));
  run(settings.slice(first, last) + architecture + '\ninitSettingsNav(); initSettingsInformationArchitecture(); globalThis.ui={setSettingsPanelOpen,openSettingsSection};', ctx);
  const flush = () => { for (const fn of frames.splice(0)) fn(); };
  flush();
  return { ctx, doc, panel, body, nav, search, sections, timers, media, el, all, flush, ui: ctx.ui,
    button: (id: string) => doc.getElementById(id) as El,
    select: (id: string) => doc.getElementById('settings-nav-' + id).click(),
    get saves() { return saves; }, get resets() { return resets; }, set confirmed(value: boolean) { confirmed = value; },
  };
}

test('footer has only Settings; destinations are embedded exactly once and retain static content', () => {
  const footer = html.slice(html.indexOf('<div id="session-footer"'), html.indexOf('</aside>', html.indexOf('<div id="session-footer"')));
  assert.equal((footer.match(/<button\b/g) || []).length, 1);
  assert.match(footer, /id="settings-btn"/);
  assert.doesNotMatch(html, /id="(?:usage-menu-btn|web-menu-btn|usage-dropdown|web-menu-dropdown|usage-dropdown-close)"/);
  for (const id of ['usage', 'hub']) assert.equal((html.match(new RegExp('data-section="' + id + '" data-group=', 'g')) || []).length, 1);
  assert.doesNotMatch(html, /data-section="settings"/);
  const usage = html.slice(html.indexOf('data-section="usage" data-group='), html.indexOf('</details>', html.indexOf('data-section="usage" data-group=')));
  for (const provider of ['claude', 'codex', 'grok', 'copilot', 'cursor-agent', 'opencode', 'command-code', 'ollama', 'lm-studio']) {
    assert.match(usage, new RegExp('id="usage-link-' + provider + '"[^>]+target="_blank" rel="noopener noreferrer"'));
  }
  assert.equal((usage.match(/data-i18n="usage_unavailable_tooltip"/g) || []).length, 5);
  assert.match(usage, /data-i18n="usage_local_group"/); assert.match(usage, /href="http:\/\/localhost:1234"/);
  const hub = html.slice(html.indexOf('id="settings-hub-actions"'), html.indexOf('</details>', html.indexOf('id="settings-hub-actions"')));
  for (const id of ['expose-btn', 'mobile-connect-btn', 'shutdown-btn', 'bug-report-btn']) assert.match(hub, new RegExp('id="' + id + '"'));
  assert.doesNotMatch(hub, /role="menu/);
  assert.equal((html.match(/id="session-inline-usage"/g) || []).length, 1);
  assert.ok(html.indexOf('id="session-inline-usage"') > html.indexOf('id="session-footer"'));
});

test('General is initial content only; every other section retains exactly one navigation destination', () => {
  const f = fixture(); f.button('settings-btn').click();
  const items = f.nav.querySelectorAll('.settings-nav-item');
  assert.equal(f.doc.getElementById('settings-nav-general'), null);
  assert.equal(items.length, f.sections.size - 1);
  assert.equal(f.sections.get('general')!.hidden, false);
  assert.ok([...f.sections].filter(([id]) => id !== 'general').every(([, section]) => section.hidden));
  assert.ok(items.every(item => !item.classList.contains('active') && item.getAttribute('aria-current') === null));
  for (const [id, section] of f.sections) {
    if (id === 'general') continue;
    assert.equal(items.filter(item => item.dataset.section === id).length, 1);
    f.select(id);
    assert.deepEqual([...f.sections].filter(([, el]) => !el.hidden).map(([key]) => key), [id]);
    assert.equal(f.button('settings-nav-' + id).getAttribute('aria-current'), 'true');
    assert.equal(f.button('settings-nav-' + id).getAttribute('aria-controls'), section.id);
  }
});

test('desktop and mobile reopen at General content without a nav item or stale search', () => {
  for (const mobile of [false, true]) {
    const f = fixture(mobile);
    for (const destination of ['usage', 'hub', 'appearance']) {
      f.button('settings-btn').click(); f.select(destination);
      assert.equal(f.sections.get(destination)!.hidden, false);
      f.search.value = 'no-such-synthetic-match'; f.search.emit('input');
      f.button('settings-close-btn').click(); f.button('settings-btn').click();
      assert.deepEqual([...f.sections].filter(([, el]) => !el.hidden).map(([id]) => id), ['general']);
      assert.equal(f.doc.getElementById('settings-nav-general'), null);
      assert.equal(f.search.value, ''); assert.equal(f.button('settings-search-status').textContent, '');
      assert.ok(f.nav.querySelectorAll('.settings-nav-item').every(el => !el.hidden && !el.classList.contains('active')));
      assert.equal(f.panel.classList.contains('settings-detail-open'), true);
      f.button('settings-close-btn').click(); assert.equal(f.doc.activeElement, f.button('settings-btn'));
    }
  }
});

test('actual search, section transfer and hash route remain available inside Settings', () => {
  const f = fixture(); f.button('settings-btn').click(); f.search.value = 'Usage'; f.search.emit('input');
  assert.equal(f.sections.get('usage')!.hidden, false);
  f.ui.openSettingsSection('subscriptions'); f.flush();
  assert.equal(f.sections.get('subscriptions')!.hidden, false); assert.equal(f.search.value, '');
  f.button('settings-close-btn').click(); f.ctx.window.location.hash = '#settings/hub'; f.ctx.window.emit('hashchange'); f.flush();
  assert.equal(f.panel.hidden, false); assert.equal(f.sections.get('hub')!.hidden, false);
  assert.equal(f.button('settings-nav-hub').getAttribute('aria-current'), 'true');
  assert.equal(f.button('settings-nav-usage').getAttribute('aria-current'), null);
});

test('General stays reachable by search, explicit section open and hash without a navigation item', () => {
  for (const mobile of [false, true]) {
    const f = fixture(mobile); f.button('settings-btn').click(); f.select('hub');
    f.search.value = 'General'; f.search.emit('input');
    assert.equal(f.sections.get('general')!.hidden, false);
    assert.equal(f.sections.get('hub')!.hidden, true);
    assert.equal(f.button('settings-search-status').textContent, 'settings_search_matched');
    assert.equal(f.nav.querySelector('.settings-nav-item.active'), null);
    f.ui.openSettingsSection('general'); f.flush();
    assert.equal(f.search.value, '');
    assert.ok(f.sections.get('general')!.contains(f.doc.activeElement));
    f.button('settings-close-btn').click(); f.ctx.window.location.hash = '#settings/general'; f.ctx.window.emit('hashchange'); f.flush();
    assert.equal(f.panel.hidden, false); assert.equal(f.sections.get('general')!.hidden, false);
    assert.equal(f.panel.classList.contains('settings-detail-open'), true);
    assert.ok(f.sections.get('general')!.contains(f.doc.activeElement));
    assert.equal(f.doc.getElementById('settings-nav-general'), null);
  }
});

test('actual mobile Back and Escape return to navigation; Close and footer toggle close', () => {
  const f = fixture(true); f.button('settings-btn').click();
  assert.equal(f.sections.get('general')!.hidden, false); assert.equal(f.panel.classList.contains('settings-detail-open'), true);
  f.button('settings-back-btn').focus(); f.button('settings-back-btn').click(); assert.equal(f.panel.hidden, false);
  assert.equal(f.panel.classList.contains('settings-detail-open'), false); assert.equal(f.doc.activeElement, f.search);
  f.ui.openSettingsSection('general'); f.flush(); f.doc.body.emit('keydown', { key: 'Escape' });
  assert.equal(f.panel.classList.contains('settings-detail-open'), false); assert.equal(f.doc.activeElement, f.search);
  f.select('usage'); f.doc.body.emit('keydown', { key: 'Escape' });
  assert.equal(f.panel.hidden, false); assert.equal(f.panel.classList.contains('settings-detail-open'), false);
  assert.equal(f.doc.activeElement, f.button('settings-nav-usage'));
  f.doc.body.emit('keydown', { key: 'Escape' }); assert.equal(f.panel.hidden, true);
  f.button('settings-btn').click(); f.button('settings-btn').click(); assert.equal(f.panel.hidden, true);
});

test('mobile search keeps input visible and Enter reaches General and ordinary results without IME submission', () => {
  const f = fixture(true); f.button('settings-btn').click(); f.button('settings-back-btn').click();
  for (const query of ['General', 'Hub']) {
    f.search.value = query; f.search.emit('input');
    assert.equal(f.panel.classList.contains('settings-detail-open'), false);
    assert.equal(f.doc.activeElement, f.search);
    assert.equal(f.button('settings-search-status').textContent, 'settings_search_matched');
    if (query === 'General') assert.ok(f.nav.querySelectorAll('.settings-nav-item').every(item => item.hidden));
    for (const blocked of [{ isComposing: true }, { keyCode: 229 }, { defaultPrevented: true }]) {
      f.search.emit('keydown', { key: 'Enter', ...blocked });
      assert.equal(f.panel.classList.contains('settings-detail-open'), false);
    }
    assert.equal(f.search.emit('keydown', { key: 'Enter' }).defaultPrevented, true);
    const section = f.sections.get(query.toLowerCase())!;
    assert.equal(section.hidden, false); assert.equal(f.panel.classList.contains('settings-detail-open'), true);
    assert.ok(section.contains(f.doc.activeElement));
    f.button('settings-back-btn').click();
    if (query === 'General') assert.equal(f.doc.activeElement, f.search);
    else { assert.equal(f.doc.activeElement, f.button('settings-nav-hub')); f.search.focus(); }
  }
  for (const query of ['', 'no-such-synthetic-match']) {
    f.search.value = query; f.search.emit('input');
    f.search.emit('keydown', { key: 'Enter' });
    assert.equal(f.panel.classList.contains('settings-detail-open'), false);
  }
  // A filtered-out active destination must not receive focus when returning from detail.
  f.ui.openSettingsSection('hub'); f.search.value = 'no-such-synthetic-match'; f.search.emit('input');
  f.doc.body.emit('keydown', { key: 'Escape' }); assert.equal(f.doc.activeElement, f.search);
});

test('Save and Reset keep existing hooks, cancellation and Settings state', async () => {
  const f = fixture(); f.button('settings-btn').click(); f.select('hub');
  f.button('settings-save-btn').click(); await tick(); assert.equal(f.saves, 1); assert.equal(f.panel.hidden, false);
  f.confirmed = false; f.button('settings-reset-btn').click(); await tick(); assert.equal(f.resets, 0);
  f.confirmed = true; f.button('settings-reset-btn').click(); await tick(); assert.equal(f.resets, 1);
});

function usageFixture(mobile = false) {
  const f = fixture(mobile); let requests = 0, resolve: ((value: unknown) => void) | null = null;
  const ctx = { ...f.ctx, activeSessionId: null, sessions: new Map(), PROVIDER_ORDER_CHANGED_EVENT: 'provider-order-changed',
    fillProviderIconSlots: () => {}, sortByProviderOrder: (items: any[]) => items,
    apiFetch: async () => { requests++; return { ok: true, json: () => new Promise(done => { resolve = done; }) }; },
  };
  run(clean(read('../src/app/usage-panel.ts')) + '\nrenderProfiles=()=>{}; renderInlineUsage=()=>{}; globalThis.usage={initUsagePanel};', ctx);
  ctx.usage.initUsagePanel(f.button('settings-usage-panel')); ctx.usage.initUsagePanel(f.button('settings-usage-panel'));
  return { ...f, requests: () => requests, respond: async () => { await tick(); resolve?.({ providers: [{ provider: 'claude', profiles: [{ id: 'synthetic', auth_checking: true }] }] }); await tick(); } };
}

test('Usage initializes once, refreshes on entry, and stops polling after navigation or Close', async () => {
  const f = usageFixture(); await tick();
  assert.equal(f.doc.body.listeners.get('session-usage-target-changed').length, 1);
  f.button('settings-btn').click(); assert.equal(f.requests(), 0);
  f.select('usage'); await f.respond(); assert.equal(f.requests(), 1); assert.equal(f.timers.size, 1);
  f.select('hub'); assert.equal(f.timers.size, 0);
  f.select('usage'); await f.respond(); assert.equal(f.requests(), 2);
  f.ui.openSettingsSection('general'); assert.equal(f.timers.size, 0);
  f.select('usage'); await f.respond(); assert.equal(f.requests(), 3);
  f.search.value = 'General'; f.search.emit('input'); assert.equal(f.timers.size, 0);
  f.ui.openSettingsSection('usage'); await f.respond(); assert.equal(f.requests(), 4);
  f.button('settings-close-btn').click(); assert.equal(f.timers.size, 0);
});

test('late Usage response cannot restart polling after Close, mobile Back or General selection', async () => {
  for (const mobile of [false, true]) {
    const f = usageFixture(mobile); f.button('settings-btn').click(); f.select('usage');
    f.button(mobile ? 'settings-back-btn' : 'settings-close-btn').click(); await f.respond();
    assert.equal(f.timers.size, 0); assert.equal(f.requests(), 1);
    if (mobile) { f.select('usage'); await f.respond(); assert.equal(f.requests(), 2); }
  }
  for (const route of ['search', 'deep-link']) {
    const f = usageFixture(); f.button('settings-btn').click(); f.select('usage');
    if (route === 'search') { f.search.value = 'General'; f.search.emit('input'); }
    else { f.ctx.window.location.hash = '#settings/general'; f.ctx.window.emit('hashchange'); }
    await f.respond();
    assert.equal(f.sections.get('general')!.hidden, false);
    assert.equal(f.timers.size, 0); assert.equal(f.requests(), 1);
  }
});

test('Hub exposure state stays synchronized and nested Escape dismisses only its help', async () => {
  const f = fixture(); let state = 'not_installed'; const calls: string[] = [];
  const ctx: any = { ...f.ctx, apiFetch: async (path: string) => { calls.push(path); return { ok: true, json: async () => ({ state }) }; },
    el: (tag: string, opts: any) => { const node = new El(tag, f.doc); node.className = opts.class || ''; node.textContent = opts.text || ''; for (const [key, value] of Object.entries(opts.attrs || {})) node.setAttribute(key, String(value)); return node; },
  };
  run(clean(read('../src/app/host-expose.ts')) + '\nglobalThis.expose={initHostExpose,fetchExposeStatus};', ctx);
  ctx.expose.initHostExpose(); await tick(); f.button('settings-btn').click(); f.select('hub');
  f.button('expose-btn').click(); await tick(); assert.equal(f.all().filter(el => el.className === 'expose-pop').length, 1);
  const event = f.doc.body.emit('keydown', { key: 'Escape' }); assert.equal(event.defaultPrevented, true);
  assert.equal(f.panel.hidden, false); assert.equal(f.all().filter(el => el.className === 'expose-pop').length, 0);
  f.button('expose-btn').click(); await tick(); f.select('usage');
  assert.equal(f.all().filter(el => el.className === 'expose-pop').length, 0);
  state = 'ready'; await ctx.expose.fetchExposeStatus(true);
  assert.match(f.button('settings-expose-dot').className, /expose-dot-ready/);
  assert.equal(f.button('settings-hub-state').textContent, 'expose_state_ready');
  assert.equal(f.button('expose-btn').getAttribute('aria-pressed'), 'true');
  assert.ok(calls.every(path => path === '/api/mobile-connect/tailscale'));
});

test('source keeps Hub confirmations/actions and responsive labels without standalone menu initialization', () => {
  const app = read('../src/app.ts');
  assert.match(app, /const result = await appConfirmShutdown\(\{ exposeActive \}\)/);
  assert.match(app, /if \(!result\) return/); assert.match(app, /result.action === 'sessions'/);
  assert.match(app, /apiFetch\('\/api\/kill-all'/); assert.match(app, /apiFetch\('\/api\/shutdown'/);
  assert.match(read('../src/app/mobile-connect.ts'), /getElementById\('mobile-connect-btn'\)/);
  assert.match(read('../src/app/bug-report-modal.ts'), /bug-report-btn/);
  assert.doesNotMatch(read('../src/app-entry.ts'), /initWebMenu|web-menu\.js/);
  assert.doesNotMatch(settings, /appendChild\(dropdown\)|initUsageDropdown/);
  const css = read('../src/styles.css'); const navcss = read('../src/styles/settings.css');
  assert.match(css, /minmax\(min\(100%, 340px\), 1fr\)/);
  assert.doesNotMatch(css, /\.usage-dropdown \{|#web-menu-dropdown|\.usage-menu-btn/);
  assert.match(navcss, /\.settings-destination-icon \{[^}]*flex-shrink: 0;[^}]*overflow: visible;/);
  assert.match(css, /#settings-hub-actions #expose-btn span\[data-i18n="expose_btn"\]/);
  for (const lang of ['en', 'ja', 'vi']) {
    const dict = JSON.parse(read('../src/i18n/' + lang + '.json'));
    for (const key of ['usage_menu_btn', 'web_menu_btn', 'settings_general', 'expose_btn', 'shutdown_tooltip', 'bug_report_button']) assert.ok(dict[key]);
  }
});


test('actual shutdown listener preserves cancellation and both affirmative payloads', async () => {
  const app = read('../src/app.ts');
  const start = app.indexOf("  const shutdownBtn = document.getElementById('shutdown-btn');");
  const listener = app.slice(start, app.indexOf('})();', start));
  for (const result of [null, { action: 'shutdown', stopExpose: false }, { action: 'sessions', stopExpose: true }]) {
    const f = fixture(); const calls: string[] = []; let closed = 0;
    const ctx = { ...f.ctx, getExposeStatus: () => ({ state: 'ready' }), fetchExposeStatus: async () => ({ ok: true, status: { state: 'ready' } }),
      appConfirmShutdown: async (opts: any) => { assert.equal(opts.exposeActive, true); return result; },
      disableExpose: async () => { calls.push('disable-expose'); }, apiFetch: async (path: string, init: any) => { assert.equal(init.method, 'POST'); calls.push(path); },
    };
    ctx.window.close = () => { closed++; }; run('(function(){' + listener + '})();', ctx);
    f.button('settings-btn').click(); f.select('hub'); f.button('shutdown-btn').click(); await tick();
    assert.deepEqual(calls, !result ? [] : result.action === 'sessions' ? ['disable-expose', '/api/kill-all', '/api/shutdown'] : ['/api/shutdown']);
    assert.equal(closed, result ? 1 : 0);
  }
});


test('Settings exposure control overrides legacy nowrap with a scoped ID selector', () => {
  const css = read('../src/styles.css');
  const legacy = read('../src/styles/host-expose.css');
  const scoped = /#settings-hub-actions\s+#expose-btn\s*\{([^}]+)\}/.exec(css);
  assert.ok(scoped, 'the wrapping override must be scoped to Settings');
  assert.match(scoped[1], /white-space:\s*normal\s*;/);
  assert.match(scoped[1], /overflow-wrap:\s*anywhere\s*;/);
  assert.match(legacy, /#expose-btn\s*\{[^}]*white-space:\s*nowrap\s*;/);
  const idCount = (selector: string) => (selector.match(/#[\w-]+/g) || []).length;
  assert.ok(idCount('#settings-hub-actions #expose-btn') > idCount('#expose-btn'));
});
