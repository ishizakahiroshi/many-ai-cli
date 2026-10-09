import test from 'node:test';
import assert from 'node:assert/strict';
import { beginSpawnPending as begin, normalizeSpawnPending as normalize, setSpawnHubInstance as hub,
  matchSpawnCorrelation as match, recordSpawnMatch as record, liveSpawnMatches, pruneSpawnPending,
  SPAWN_PENDING_LIMIT, SPAWN_PENDING_TTL_MS, type SpawnCorrelation } from './session-spawn-state.js';
import type { SessionSnapshot } from '../types/proto.js';
const now=100000;
const session=(id:number,started_at='start-'+id):SessionSnapshot=>({id,started_at,cwd:'/repos/example'} as SessionSnapshot);
const event=(request:string,id=1,started='start-'+id,instance='hub-A'):SpawnCorrelation=>({client_request_id:request,session_id:id,started_at:started,hub_instance:instance});
const initial=()=>{const state=normalize(null,now);hub(state,'hub-A');return state;};

test('correlation nonce validation and request capacity are bounded without evicting accepted work',()=>{
 const state=initial();for(let i=0;i<SPAWN_PENDING_LIMIT;i++)assert.ok(begin(state,'request-'+i,'tab-A',1,now,true));
 assert.equal(begin(state,'overflow','tab-B',1,now,true),null);assert.equal(state.pending[0].requestId,'request-0');
 assert.equal(begin(initial(),'bad value','tab-A',1,now,true),null);assert.equal(begin(initial(),'x'.repeat(129),'tab-A',1,now,true),null);
});
test('malformed/reloaded pending data retains identities only and respects expiry',()=>{
 const raw={version:1,hubInstance:'hub-A',pending:[{requestId:'r1',tabId:'A',createdAt:now,expected:2,hubInstance:'hub-A',matched:[{id:1,startedAt:'first',secret:'not-copied'}],message:'not-copied'}, {requestId:'bad space',tabId:'B',createdAt:now}]};
 const state=normalize(raw,now);assert.equal(state.pending.length,1);assert.ok(!JSON.stringify(state).includes('not-copied'));
 pruneSpawnPending(state,now+SPAWN_PENDING_TTL_MS);assert.equal(state.pending.length,0);
});
test('another browser and externally created sessions cannot consume this browser request',()=>{
 const state=initial();begin(state,'ours','tab-A',1,now,true);
 assert.equal(match(state,event('theirs'),[session(1)],now),null);assert.equal(state.pending[0].matched.length,0);
 assert.equal(match(state,event('ours'),[session(1)],now)?.tabId,'tab-A');
});
test('concurrent requests preserve each captured tab and all grid siblings',()=>{
 const state=initial();const one=begin(state,'one','A',2,now,true)!;begin(state,'two','B',1,now,true);
 assert.equal(match(state,event('two',3),[session(3)],now)?.tabId,'B');
 record(match(state,event('one',1),[session(1)],now)!,{id:1,startedAt:'start-1'});
 assert.equal(match(state,event('one',2),[session(2)],now),one);record(one,{id:2,startedAt:'start-2'});
 assert.deepEqual(liveSpawnMatches(one,[session(1),session(2)]).map(s=>s.id),[1,2]);assert.equal(state.pending.length,2);
});
test('WS before response and before session upsert is resolved only against the actual lifecycle',()=>{
 const state=initial();begin(state,'request','A',1,now,true);
 assert.equal(match(state,event('request'),[],now),null);
 assert.ok(match(state,event('request'),[session(1)],now));
 assert.equal(match(state,event('request'),[session(1,'reused')],now),null);
});
test('first snapshot after a page reload checks persisted Hub scope, not only a memory variable',()=>{
 const old=initial();begin(old,'old-request','A',1,now,true);
 const state=normalize(JSON.parse(JSON.stringify(old)),now+1);hub(state,'hub-B');assert.equal(state.pending.length,0);
 assert.equal(match(state,event('old-request',1,'start-1','hub-B'),[session(1)],now+1),null);
});
test('requests created after disconnect bind only at the next snapshot',()=>{
 const state=initial();begin(state,'old','A',1,now,true);begin(state,'new','B',1,now+1,false);
 hub(state,'hub-B');assert.deepEqual(state.pending.map(p=>p.requestId),['new']);
 assert.equal(match(state,event('new',1,'start-1','hub-A'),[session(1)],now+2),null);
 assert.equal(match(state,event('new',1,'start-1','hub-B'),[session(1)],now+2)?.tabId,'B');
});
test('same-Hub reconnect and repeated snapshots do not replay a consumed manual affiliation',()=>{
 const state=initial();const pending=begin(state,'r','A',1,now,true)!;record(pending,{id:1,startedAt:'start-1'});
 const reloaded=normalize(JSON.parse(JSON.stringify(state)),now+1);hub(reloaded,'hub-A');
 assert.equal(match(reloaded,event('r'),[session(1)],now+1),null);
});
test('HTTP failure, observer cancellation, and a partial grid do not drop accepted siblings',()=>{
 const state=initial();const pending=begin(state,'grid','A',3,now,true)!;
 record(pending,{id:1,startedAt:'start-1'}); // response failed after this accepted start
 hub(state,'hub-A');assert.equal(match(state,event('grid',2),[session(1),session(2)],now+100)?.tabId,'A');
 assert.equal(state.pending.length,1);assert.equal(liveSpawnMatches(pending,[session(1,'reused')]).length,0);
});
