import test from 'node:test';
import assert from 'node:assert/strict';
import { normalizeHeadlessProgress, receiveHeadlessProgress, elapsedHeadlessMs, headlessProgressText, decodeHeadlessText, setHeadlessConnected, renderHeadlessProgressValues } from './headless-progress.js';
const fixture = () => ({version:1,run_id:'1:1',sequence:1,billing_mode:'plan-login',observed_turns:2,result_turns:null,active_tools:[{id:'tool1',name:'Read'}],started_at_ms:1000,elapsed_ms:2300,ended_at_ms:null,cost:{amount_usd:0.1,currency:'USD',kind:'estimate',coverage:'observed-main'},last_assistant:'safe &lt;img src=x&gt;',state:'running',data_incomplete:false,final_cost_missing:false});
const t = (key:string, vars?:Record<string,unknown>) => key + (vars ? JSON.stringify(vars) : '');
test('headless snapshot rejects malformed numeric/cost/lifecycle input', () => {
 assert.ok(normalizeHeadlessProgress(fixture()));
 for (const bad of [NaN, Infinity, -1, '2']) assert.equal(normalizeHeadlessProgress({...fixture(),elapsed_ms:bad}),null);
 assert.equal(normalizeHeadlessProgress({...fixture(),cost:{amount_usd:-1,currency:'USD',kind:'estimate',coverage:'partial'}}),null);
 assert.equal(normalizeHeadlessProgress({...fixture(),active_tools:Array(17).fill({id:'x',name:'Read'})}),null);
 assert.equal(normalizeHeadlessProgress({...fixture(),active_tools:[null]}),null);
 assert.equal(normalizeHeadlessProgress({...fixture(),ended_at_ms:1}),null);
 assert.equal(normalizeHeadlessProgress({...fixture(),observed_turns:1_000_001}),null);
});
test('run and sequence isolation; duplicate cumulative snapshots never add', () => {
 const one=receiveHeadlessProgress(fixture(),null,100)!;
 assert.equal(receiveHeadlessProgress(fixture(),one,200),null);
 assert.equal(receiveHeadlessProgress({...fixture(),run_id:'2:1',sequence:2},one,200),null);
 const two=receiveHeadlessProgress({...fixture(),sequence:2,cost:{...fixture().cost,amount_usd:0.15}},one,200)!;
 assert.equal(two.cost.amount_usd,0.15);
 assert.equal(receiveHeadlessProgress({...fixture(),sequence:3,elapsed_ms:1},two,300),null);
 assert.equal(receiveHeadlessProgress({...fixture(),sequence:3,started_at_ms:2},two,300),null);
});
test('one monotonic clock drives both views and stops for end/disconnect', () => {
 const one=receiveHeadlessProgress(fixture(),null,100)!;
 assert.equal(elapsedHeadlessMs(one,1100),3300);
 assert.equal(elapsedHeadlessMs(one,1100),elapsedHeadlessMs(one,1100));
 assert.equal(elapsedHeadlessMs(one,1100,false),2300);
 const done=receiveHeadlessProgress({...fixture(),state:'completed',ended_at_ms:3300,active_tools:[]},null,100)!;
 assert.equal(elapsedHeadlessMs(done,5000),2300);
 setHeadlessConnected(false); assert.equal(elapsedHeadlessMs(one,5000),2300); setHeadlessConnected(true);
});
test('unknown values remain unavailable; plan dollar amounts are reference; final replaces estimate', () => {
 const p=receiveHeadlessProgress(fixture())!;
 assert.match(headlessProgressText(p,t)[4],/headless_cost_estimate/);
 assert.match(headlessProgressText(p,t)[4],/headless_reference/);
 p.cost={amount_usd:null,currency:'USD',kind:'unavailable',coverage:'unavailable'};
 assert.match(headlessProgressText(p,t)[4],/headless_cost_unknown/);
 p.cost={amount_usd:0.34,currency:'USD',kind:'result',coverage:'cli-result'};p.result_turns=7;
 assert.match(headlessProgressText(p,t)[1],/7/);assert.match(headlessProgressText(p,t)[4],/headless_cost_result/);
});
test('safe projection is decoded exactly once as plain text, never expanded HTML', () => {
 assert.equal(decodeHeadlessText('&amp;lt;img&gt;'), '&lt;img>');
 const p=receiveHeadlessProgress(fixture())!; assert.match(headlessProgressText(p,t)[5],/safe <img src=x>/);
 assert.ok(!JSON.stringify(normalizeHeadlessProgress({...fixture(),raw_secret:'excluded'})).includes('excluded'));
});

test('synthetic DOM keeps two children isolated across card/pane switch and snapshot reload', () => {
 class Element {
   children: Element[] = []; textContent = ''; className = '';
   ownerDocument = {createElement: (_tag:string) => new Element()};
   appendChild(child:Element) { this.children.push(child); return child; }
 }
 const one=receiveHeadlessProgress(fixture(),null,0)!;
 const two=receiveHeadlessProgress({...fixture(),run_id:'2:1',observed_turns:9,last_assistant:'second'},null,0)!;
 const sidebarOne=new Element(), card=new Element(), paneTwo=new Element();
 const render=(el:Element,p:typeof one) => renderHeadlessProgressValues(el as unknown as HTMLElement,headlessProgressText(p,t,1000));
 render(sidebarOne,one);render(card,one);render(paneTwo,two);
 assert.deepEqual(card.children.map(c=>c.textContent),sidebarOne.children.map(c=>c.textContent));
 assert.notEqual(card.children[1].textContent,paneTwo.children[1].textContent);
 render(card,two); assert.deepEqual(card.children.map(c=>c.textContent),paneTwo.children.map(c=>c.textContent));
 render(card,receiveHeadlessProgress(JSON.parse(JSON.stringify(one)),null,0)!);
 assert.deepEqual(card.children.map(c=>c.textContent),sidebarOne.children.map(c=>c.textContent));
 assert.match(card.children[5].textContent, /<img/); assert.equal(card.children[5].children.length,0);
});

test('prelaunch and unsupported provider metrics remain unknown rather than zero', () => {
 const initial=receiveHeadlessProgress({...fixture(),sequence:0})!;
 assert.match(headlessProgressText(initial,t)[0], /headless_waiting/);
 assert.equal(headlessProgressText(initial,t)[1], 'headless_turns_unknown');
 const textOnly=receiveHeadlessProgress(fixture())!;
 assert.equal(headlessProgressText(textOnly,t,0,false)[1], 'headless_turns_unknown');
 assert.equal(headlessProgressText(textOnly,t,0,false)[2], 'headless_tool_unknown');
});

import { noteSpawnConfirmationRequested, refreshSpawnConfirmationDisclosure, selectOldestPendingConfirmationFor, closeSpawnConfirmation, clearAllSpawnConfirmationsForHubRestart } from './spawn-confirm-store.js';
test('recipe change re-discloses only the same pending request and never resurrects closed approval', () => {
 clearAllSpawnConfirmationsForHubRestart();
 const message=(label:string) => ({type:'spawn_confirmation_requested',spawn_confirmation_id:'synthetic-billing',session_id:17,provider:'claude',execution_mode:'headless',data:btoa(JSON.stringify({version:1,billing_mode:'api-key',billing_profile:'synthetic',max_budget_usd:1,billing_profiles:[{id:'synthetic',display_name:label,provider:'claude',command:'PRIVATE_RECIPE_SENTINEL'}]}))});
 noteSpawnConfirmationRequested(message('old label'));
 assert.equal(refreshSpawnConfirmationDisclosure('synthetic-billing',18,message('new label')),false);
 assert.equal(refreshSpawnConfirmationDisclosure('synthetic-billing',17,message('new label')),true);
 const fresh=selectOldestPendingConfirmationFor(17)!;
 assert.equal(fresh.billingProfiles![0].display_name,'new label');
 assert.ok(!JSON.stringify(fresh).includes('PRIVATE_RECIPE_SENTINEL'));
 closeSpawnConfirmation({spawn_confirmation_id:'synthetic-billing'});
 assert.equal(refreshSpawnConfirmationDisclosure('synthetic-billing',17,message('late label')),false);
 assert.equal(selectOldestPendingConfirmationFor(17),null);
});
