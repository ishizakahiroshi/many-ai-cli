import { t } from '../i18n.js';
import { apiFetch } from './util.js';
import { activeSessionId, sessions } from './state.js';
import { activateSession } from './session-list.js';
import { getPromptTemplates } from './prompt-templates.js';
import { ORCHESTRATION_CLI_OPTIONS } from './orchestration-roles.js';
import { isRoutineRunActive, routineRunFromURL, routineSessionMatches } from './routines-model.js';
import type { Routine, RoutineRun, RoutineSchedule } from './routines-model.js';

let dialog: HTMLDialogElement | null = null;
let content: HTMLElement;
let feedback: HTMLElement;
let opener: HTMLElement | null = null;
let generation = 0;
let refreshTimer: ReturnType<typeof setTimeout> | null = null;
const requests = new Map<string, string>();
const tr = (key: string) => t('routines_' + key);

function node<K extends keyof HTMLElementTagNameMap>(tag: K, text = '', className = ''): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag); el.textContent = text; el.className = className; return el;
}
function button(label: string, action: () => void, primary = false): HTMLButtonElement {
  const el = node('button', label, primary ? 'routine-button primary' : 'routine-button');
  el.type = 'button'; el.addEventListener('click', action); return el;
}
async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await apiFetch(path, init);
  if (!response.ok) {
    const raw = await response.text();
    let detail = '';
    try {
      const error = JSON.parse(raw);
      if (typeof error.detail === 'string') detail = error.detail.slice(0, 350);
    } catch (_) { /* A proxy error page is not useful in the product dialog. */ }
    throw new Error(`${tr('request_failed')} (${response.status})${detail ? ' ' + detail : ''}`);
  }
  return await response.json() as T;
}
const body = (method: string, data: unknown): RequestInit => ({method, headers: {'Content-Type': 'application/json'}, body: JSON.stringify(data)});
function errorMessage(error: unknown): void { feedback.textContent = error instanceof Error ? error.message : tr('request_failed'); }
function clearTimer(): void { if (refreshTimer !== null) clearTimeout(refreshTimer); refreshTimer = null; }
function page(): number {
  clearTimer(); generation++; content.replaceChildren(); feedback.textContent = ''; return generation;
}
function current(epoch: number): boolean { return !!dialog?.open && epoch === generation; }
function date(value?: string): string {
  if (!value || value.startsWith('0001-')) return '—';
  const parsed = new Date(value); if (!Number.isFinite(parsed.valueOf())) return '—';
  return new Intl.DateTimeFormat(document.documentElement.lang || 'en', {dateStyle:'short',timeStyle:'short'}).format(parsed);
}
function status(run: RoutineRun): string { return tr('status_' + run.status); }
function close(): void { dialog?.close(); }
function ensureDialog(): HTMLDialogElement {
  if (dialog) return dialog;
  dialog = node('dialog', '', 'routines-dialog aac-wheel-overlay'); dialog.id = 'routines-dialog';
  dialog.setAttribute('aria-labelledby', 'routines-title');
  const header = node('div', '', 'routines-header');
  const heading = node('h2', tr('title')); heading.id = 'routines-title';
  const dismiss = button('✕', close); dismiss.classList.add('routines-dialog-close'); dismiss.setAttribute('aria-label', t('settings_close'));
  header.append(heading, dismiss);
  feedback = node('p', '', 'routines-feedback'); feedback.setAttribute('role', 'status');
  content = node('div', '', 'routines-content');
  dialog.append(header, feedback, content); document.body.append(dialog);
  dialog.addEventListener('close', () => { generation++; clearTimer(); opener?.focus({preventScroll:true}); });
  return dialog;
}
export function openRoutines(runID?: string): void {
  const modal = ensureDialog();
  if (!modal.open) { opener = document.activeElement instanceof HTMLElement ? document.activeElement : null; modal.showModal(); }
  if (runID) void showRun(runID); else void showList();
}
async function showList(): Promise<void> {
  const epoch = page();
  const actions = node('div', '', 'routines-actions');
  actions.append(button(tr('new'), () => editRoutine(), true), button(tr('history'), () => void showHistory()));
  content.append(actions, node('p', tr('loading')));
  try {
    const {routines} = await request<{routines: Routine[]}>('/api/routines'); if (!current(epoch)) return;
    content.lastElementChild?.remove();
    if (!routines.length) content.append(node('p', tr('empty'), 'routines-note'));
    for (const routine of routines) {
      const card = node('article', '', 'routine-card');
      card.append(node('h3', routine.name), node('p', `${routine.provider} · ${routine.cwd}`, 'routines-note'));
      const scheduled = routine.schedule.kind !== 'manual';
      card.append(node('p', scheduled ? `${tr(routine.schedule.kind)} ${routine.schedule.time} · ${routine.schedule.timezone}${routine.enabled ? '' : ' · '+tr('paused')}` : tr('manual')));
      if (scheduled && routine.enabled) card.append(node('p', `${tr('next')}: ${date(routine.next_run_at)}`, 'routines-note'));
      const controls = node('div', '', 'routines-actions');
      controls.append(button(tr('view'), () => showRoutine(routine), true), button(tr('edit'), () => editRoutine(routine)));
      card.append(controls); content.append(card);
    }
  } catch(error) { if(current(epoch)) { content.lastElementChild?.remove(); errorMessage(error); content.append(button(tr('retry'), () => void showList())); } }
}
function showRoutine(routine: Routine): void {
  const epoch=page();
  content.append(button('← '+tr('title'),()=>void showList()), node('h3',routine.name));
  content.append(node('p',`${routine.provider} · ${routine.model || tr('default_model')}`),node('p',routine.cwd,'routines-note'),node('pre',routine.prompt,'routine-prompt'));
  content.append(node('p',tr('launch_note'),'routines-note'));
  if(routine.completion_mode==='marker')content.append(node('p',tr('completion_note'),'routines-note'));
  const run=button(tr('run'),()=>void launch(),true);
  content.append(run, button(tr('history'),()=>void showHistory(routine.id)));
  async function launch(): Promise<void> {
    if(run.disabled)return;run.disabled=true;feedback.textContent=tr('launching');
    try {
      let requestID=requests.get(routine.id);
      if(!requestID){requestID=crypto.randomUUID();requests.set(routine.id,requestID);}
      const {run: result}=await request<{run:RoutineRun;existing:boolean}>(`/api/routines/${encodeURIComponent(routine.id)}/runs`,body('POST',{request_id:requestID}));
      requests.delete(routine.id);if(current(epoch))void showRun(result.id);
    }catch(error){if(current(epoch)){errorMessage(error);run.disabled=false;run.textContent=tr('retry_run');}}
  }
}
function editRoutine(routine?: Routine): void {
  const epoch=page();const session=activeSessionId===null?undefined:sessions.get(activeSessionId);
  content.append(button('← '+tr('title'),()=>void showList()),node('h3',routine?tr('edit'):tr('new')));
  const form=node('form','','routine-form');
  function input(key:string,value:string,required=false,type='text'):HTMLInputElement {
    const label=node('label',tr(key));const field=node('input');field.name=key;field.value=value;field.required=required;field.type=type;label.append(field);form.append(label);return field;
  }
  const name=input('name',routine?.name||'',true);name.maxLength=120;
  const cwd=input('cwd',routine?.cwd||session?.cwd||'',true);
  const providerLabel=node('label',tr('provider'));const provider=node('select');provider.name='provider';
  for(const option of ORCHESTRATION_CLI_OPTIONS){if(!option.value || option.value==='shell')continue;const el=node('option',option.label);el.value=option.value;provider.append(el);}
  provider.value=routine?.provider||session?.provider||'codex';providerLabel.append(provider);form.append(providerLabel);
  const model=input('model',routine?.model||'');model.placeholder=tr('default_model');
  const templateLabel=node('label',tr('template'));const templates=node('select');templates.append(node('option',tr('choose_template')));
  const stored=getPromptTemplates();stored.forEach((template,index)=>{const option=node('option',template.body);option.value=String(index);templates.append(option);});
  templateLabel.append(templates);form.append(templateLabel);
  const promptLabel=node('label',tr('prompt'));const prompt=node('textarea');prompt.name='prompt';prompt.required=true;prompt.rows=6;prompt.value=routine?.prompt||'';promptLabel.append(prompt);form.append(promptLabel);
  templates.addEventListener('change',()=>{const selected=stored[Number(templates.value)];if(templates.selectedIndex>0&&selected){prompt.value+=prompt.value?'\n'+selected.body:selected.body;prompt.focus();}});
  const scheduleLabel=node('label',tr('schedule'));const schedule=node('select');
  for(const kind of ['manual','daily','weekdays']){const option=node('option',tr(kind));option.value=kind;schedule.append(option);}schedule.value=routine?.schedule.kind||'manual';scheduleLabel.append(schedule);form.append(scheduleLabel);
  const time=input('time',routine?.schedule.time||'09:00',false,'time');
  const timezone=input('timezone',routine?.schedule.timezone||Intl.DateTimeFormat().resolvedOptions().timeZone);
  const enabled=input('enabled','',false,'checkbox');enabled.checked=routine?.enabled??true;
  const updateSchedule=()=>{const scheduled=schedule.value!=='manual';for(const field of [time,timezone,enabled]){field.parentElement!.hidden=!scheduled;field.disabled=!scheduled;}time.required=scheduled;timezone.required=scheduled;};schedule.addEventListener('change',updateSchedule);updateSchedule();
  form.append(node('p',tr('schedule_note'),'routines-note'));
  const save=node('button',tr('save'),'routine-button primary');save.type='submit';form.append(save);content.append(form);
  form.addEventListener('submit',async event=>{
    event.preventDefault();if(save.disabled)return;save.disabled=true;
    const value={name:name.value.trim(),cwd:cwd.value.trim(),provider:provider.value,model:model.value.trim(),prompt:prompt.value.trim(),enabled:enabled.checked,updated_at:routine?.updated_at,schedule:{kind:schedule.value,time:time.value,timezone:timezone.value.trim()} as RoutineSchedule};
    try{await request(`/api/routines${routine?'/'+encodeURIComponent(routine.id):''}`,body(routine?'PUT':'POST',value));if(current(epoch))void showList();}
    catch(error){if(current(epoch)){errorMessage(error);save.disabled=false;}}
  });
  if(routine){const remove=button(tr('delete'),()=>{
    const confirm=node('div','', 'routines-actions');confirm.append(node('p',tr('delete_confirm')));
    const cancel=button(tr('cancel'),()=>{confirm.remove();remove.disabled=false;remove.focus();});
    const confirmDelete=button(tr('delete'),async()=>{
      if(confirmDelete.disabled)return;confirmDelete.disabled=true;cancel.disabled=true;
      try{await request(`/api/routines/${encodeURIComponent(routine.id)}`,{method:'DELETE'});if(current(epoch))void showList();}
      catch(error){if(current(epoch)){errorMessage(error);confirmDelete.disabled=false;cancel.disabled=false;}}
    });
    confirm.append(cancel,confirmDelete);content.append(confirm);remove.disabled=true;
  });content.append(remove);}
}
async function showHistory(routineID?:string):Promise<void>{
  const epoch=page();content.append(button('← '+tr('title'),()=>void showList()),node('h3',tr('history')));
  try{const {runs}=await request<{runs:RoutineRun[]}>(`/api/routine-runs${routineID?'?routine_id='+encodeURIComponent(routineID):''}`);if(!current(epoch))return;
    if(!runs.length)content.append(node('p',tr('no_runs'),'routines-note'));
    for(const run of runs){const card=button('',()=>void showRun(run.id));card.classList.add('routine-card','routine-history-row');card.append(node('strong',run.routine_name),node('span',status(run)),node('small',date(run.started_at)+' · '+run.id));content.append(card);}
  }catch(error){if(current(epoch)){errorMessage(error);content.append(button(tr('retry'),()=>void showHistory(routineID)));}}
}
async function showRun(id:string):Promise<void>{
  const epoch=page();content.append(button('← '+tr('history'),()=>void showHistory()));
  const result=node('section','','routine-result');content.append(result);
  // Keep interactive nodes mounted: polling must not close details, steal focus,
  // replace a selected answer, or reset the reader's scroll position.
  const title=node('h3'),state=node('p',tr('loading'),'routine-run-status');
  const identity=node('p','','routines-note'),target=node('p','','routines-note');
  const answer=node('pre','','routine-answer'),empty=node('p'),error=node('p','','routines-error');
  const truncated=node('p',tr('truncated'),'routines-note');
  const unavailable=node('p',tr('session_unavailable'),'routines-note');
  let latestRun:RoutineRun|undefined;
  const openSession=button(tr('open_session'),()=>{
    const run=latestRun;
    if(!run||!routineSessionMatches(run,sessions.get(run.session_id!))){feedback.textContent=tr('session_unavailable');return;}
    close();activateSession(run.session_id!);
  },true);
  const details=node('details');const savedPrompt=node('pre','','routine-prompt');
  details.append(node('summary',tr('saved_prompt')),savedPrompt);
  const retry=button(tr('retry'),()=>{retry.hidden=true;void load();});
  for(const el of [answer,empty,error,truncated,openSession,unavailable,details,retry])el.hidden=true;
  result.append(title,state,identity,target,node('p',tr('finish_note'),'routines-note'),answer,empty,error,truncated,openSession,unavailable,details,retry);
  function text(el:HTMLElement,value:string):void{if(el.textContent!==value)el.textContent=value;}
  let loading=false;
  async function load():Promise<void>{
    if(loading||!current(epoch))return;loading=true;clearTimer();
    try{
      const {run}=await request<{run:RoutineRun}>(`/api/routine-runs/${encodeURIComponent(id)}`);if(!current(epoch))return;
      latestRun=run;feedback.textContent='';retry.hidden=true;
      const scroll=content.scrollTop;
      text(title,run.routine_name);text(state,status(run));text(identity,`${date(run.started_at)} · ${run.id}`);
      text(target,`${run.provider} · ${run.cwd}`);text(answer,run.result||run.summary||'');answer.hidden=!answer.textContent;
      text(empty,tr(isRoutineRunActive(run)?'working':'result_unavailable'));empty.hidden=!answer.hidden;
      text(error,run.error||'');error.hidden=!run.error;truncated.hidden=!run.result_truncated;
      const available=routineSessionMatches(run,run.session_id?sessions.get(run.session_id):undefined);
      openSession.hidden=!available;unavailable.hidden=available;
      text(savedPrompt,run.prompt);details.hidden=false;content.scrollTop=scroll;
      // A finished run may arrive before the live session list on notification startup.
      // Re-evaluate availability even when the run itself is unchanged.
      refreshTimer=setTimeout(()=>void load(),isRoutineRunActive(run)?2500:5000);
    }catch(error){if(current(epoch)){errorMessage(error);retry.hidden=false;}}
    finally{loading=false;}
  }
  await load();
}
document.addEventListener('click',event=>{if((event.target as Element)?.closest?.('[data-open-routines]'))openRoutines();});
window.addEventListener('many-ai-cli:open-routine',event=>{const id=(event as CustomEvent<{runID?:string}>).detail?.runID;if(id)openRoutines(id);});
let initialRunHandled=false;
function openInitialRun():void{
  if(initialRunHandled)return;
  const id=routineRunFromURL(location.href);if(!id)return;
  initialRunHandled=true;const url=new URL(location.href);url.searchParams.delete('routine_run');history.replaceState(history.state,'',url);openRoutines(id);
}
document.addEventListener('i18n-ready',openInitialRun,{once:true});
// i18n can finish before this module is evaluated. Defer to the end of the import
// graph so session-list/app bindings are initialized before opening the dialog.
if(typeof window.t==='function'){
  if(document.readyState==='loading')document.addEventListener('DOMContentLoaded',openInitialRun,{once:true});
  else queueMicrotask(openInitialRun);
}
