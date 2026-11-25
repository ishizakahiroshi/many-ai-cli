import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { SharedTemplateSync } from '../src/app/shared-template-sync';
import { createUserPrefsPutQueue } from '../src/app/user-prefs-put-queue';
import { sanitizeProjectViews } from '../src/app/project-view-memory';

const LIST='ai_cli_hub_prompt_templates';
const SEND_IMMEDIATE='ai_cli_hub_template_send_immediate';
const clone=<T>(value:T):T=>JSON.parse(JSON.stringify(value));
const settle=async()=>{for(let i=0;i<15;i++)await Promise.resolve();};

// Load the real user-prefs and prompt-templates modules into one context so the
// list reader, its normalization, and the persistence write path are all actual
// code. Only browser/HTTP boundaries and unrelated app state are stubbed.
function fixture(storage=new Map<string,string>(),serverTemplates:unknown[]=[],intl: Partial<typeof Intl> = Intl){
  const serverPrefs={migrated_from_localstorage:true,templates:serverTemplates,display:{theme:'light',font_size:'medium',lang:'en'}};
  const timers=new Map<number,Function>();let timerID=0;
  const windowEvents:Array<{type:string,options:any}>=[];
  const makeEl=()=>({tag:'',children:[] as any[],className:'',textContent:'',value:'',title:'',type:'',dataset:{} as Record<string,string>,
    setAttribute(){},addEventListener(){},append(...c:any[]){this.children.push(...c);},replaceChildren(){this.children=[];},querySelectorAll(){return [];}});
  const dom:Record<string,any>={'prompt-template-palette':makeEl(),'prompt-template-list':makeEl(),'prompt-template-search':{...makeEl(),value:''}};
  const ctx:any={SharedTemplateSync,createUserPrefsPutQueue,sanitizeProjectViews,sanitizeCustomThemes:(value:any)=>value,
    token:'synthetic-token',showToast(){},Error,Promise,JSON,Event,Intl:intl,
    CustomEvent:class {constructor(public type:string,public options:any){}},
    console:{warn(){},info(){}},requestAnimationFrame(){},
    t:(key:string)=>key,activeSessionId:null,sessions:new Map(),
    window:{t:(key:string)=>key,addEventListener(){},dispatchEvent:(event:any)=>{windowEvents.push({type:event.type,options:event.options});return true;}},
    document:{getElementById:(id:string)=>dom[id]??null,addEventListener(){},dispatchEvent:()=>true,createElement:(tag:string)=>({...makeEl(),tag})},
    localStorage:{getItem:(key:string)=>storage.get(key)??null,setItem:(key:string,value:string)=>storage.set(key,value),removeItem:(key:string)=>storage.delete(key)},
    setTimeout:(fn:Function)=>{timers.set(++timerID,fn);return timerID;},clearTimeout:(id:number)=>timers.delete(id),
    apiFetch:async()=>({ok:true,status:200,headers:{get:()=> 'version-1'},json:async()=>clone(serverPrefs)}),
  };
  ctx.fetch=ctx.apiFetch;
  const load=(path:string)=>{const source=readFileSync(new URL(path,import.meta.url),'utf8').replace(/^import .*;\r?$/gm,'').replace(/^export /gm,'');return new Bun.Transpiler({loader:'ts'}).transformSync(source);};
  // const bindings do not cross runInNewContext scripts; functions do.
  runInNewContext(load('../src/app/user-prefs.ts')+'\nglobalThis.STORAGE_TEMPLATES_KEY=STORAGE_TEMPLATES_KEY;globalThis.STORAGE_TEMPLATE_SEND_IMMEDIATE_KEY=STORAGE_TEMPLATE_SEND_IMMEDIATE_KEY;',ctx);
  runInNewContext(load('../src/app/prompt-templates.ts')+'\nglobalThis.api={getPromptTemplates,addPromptTemplate,promptTemplateLabel,templatesForProvider,insertPromptTemplate,isTemplateSendImmediate,setTemplateSendImmediate,renderPalette};',ctx);
  return {storage,windowEvents,dom,api:ctx.api};
}

describe('prompt template list normalization',()=>{
  for (const [runtime, intl] of [['native', Intl], ['fallback', {}]] as const) {
    test(`preserves the complete twelfth emoji in ${runtime} mode`, async () => {
      const f = fixture(new Map(), [], intl);
      await settle();
      for (const emoji of ['👩‍💻', '👩🏽‍💻', '🇯🇵', '1️⃣', '☕️', 'か\u3099', '🏴\u{E0067}\u{E0062}\u{E007F}']) {
        const expected = 'a'.repeat(11) + emoji;
        expect(f.api.addPromptTemplate(emoji, expected + 'x')).toBe(true);
        expect(f.api.getPromptTemplates().at(-1).display_label).toBe(expected);
      }
    });
  }
  test('legacy label stays dropped, display_label is trimmed, empty bodies are removed',async()=>{
    const f=fixture(new Map(),[
      {label:'旧ラベル',body:'監査して',frequency:9},
      {body:'要約して',display_label:'  📝  '},
      {body:'   '},
      {body:42},
    ]);
    await settle();
    const templates=f.api.getPromptTemplates();
    expect(templates).toHaveLength(2);
    expect(templates[0]).toEqual({body:'監査して',providers:[],tags:[]});
    expect(templates[0].label).toBeUndefined();
    expect(templates[0].frequency).toBeUndefined();
    expect(templates[1]).toEqual({body:'要約して',providers:[],tags:[],display_label:'📝'});
  });
  test('addPromptTemplate stores display_label, dedups on body, and caps at 12 graphemes',async()=>{
    const f=fixture();await settle();
    expect(f.api.addPromptTemplate('本文',' 🔍 ')).toBe(true);
    expect(f.api.addPromptTemplate('本文')).toBe(false);
    expect(f.api.addPromptTemplate('   ')).toBe(false);
    expect(f.api.addPromptTemplate('別の本文')).toBe(true);
    expect(f.api.addPromptTemplate('長い','１２３４５６７８９０１２３４５')).toBe(true);
    const stored=JSON.parse(f.storage.get(LIST)!);
    expect(stored).toHaveLength(3);
    expect(stored[0]).toEqual({body:'本文',providers:[],tags:[],display_label:'🔍'});
    expect(stored[1]).toEqual({body:'別の本文',providers:[],tags:[]});
    expect(stored[1].display_label).toBeUndefined();
    expect(stored[2].display_label).toBe('１２３４５６７８９０１２');
  });
  test('the list row renders the label in front of the body',async()=>{
    const f=fixture(new Map(),[
      {body:'監査して',display_label:'🔍'},
      {body:'ラベルなし'},
    ]);
    await settle();
    f.api.renderPalette();
    const rows=f.dom['prompt-template-list'].children.filter((c:any)=>c.className==='prompt-template-row');
    expect(rows).toHaveLength(2);
    const labelled=rows[0].children.find((c:any)=>typeof c.className==='string'&&c.className.includes('prompt-template-item'));
    expect(labelled.children.map((c:any)=>c.className)).toEqual(['prompt-template-item-label','prompt-template-item-body']);
    expect(labelled.children[0].textContent).toBe('🔍');
    expect(labelled.children[1].textContent).toBe('監査して');
    expect(labelled.title).toBe('🔍 監査して');
    const plain=rows[1].children.find((c:any)=>typeof c.className==='string'&&c.className.includes('prompt-template-item'));
    expect(plain.children).toHaveLength(0);
    expect(plain.textContent).toBe('ラベルなし');
    expect(plain.title).toBe('ラベルなし');
  });
  for (const send of [false, true]) {
    test(`selection dispatches only the body with immediate send ${send}`, async () => {
      const storage = new Map([[SEND_IMMEDIATE, send ? '1' : '0']]);
      const f = fixture(storage);
      await settle();
      const body = '差分を確認\n結果をまとめる';
      f.api.insertPromptTemplate({ body, providers: [], tags: [], display_label: '🔍' });
      expect(f.windowEvents).toHaveLength(1);
      expect(f.windowEvents[0].type).toBe('many-ai-cli:insert-template');
      expect(f.windowEvents[0].options.detail).toEqual({ text: body, send });
    });
  }
});
