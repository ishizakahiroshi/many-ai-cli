import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { createRequire } from 'node:module';

const esbuild = createRequire(import.meta.url)('esbuild');
const source = readFileSync(new URL('../src/app/remote-auth.ts', import.meta.url), 'utf8');
const clean = source.replace(/^import[\s\S]*?;\r?$/gm, '').replace(/^export /gm, '');
const code = esbuild.transformSync(clean + '\nglobalThis.init = initRemoteAuth;', { loader:'ts',format:'cjs',target:'es2022' }).code;
const tick = async () => { for (let i=0;i<20;i++) await Promise.resolve(); };

function fixture() {
  const overlay = { hidden:true };
  let focused = 0;
  let status = { pin_enabled:true,remote:false,authed:true };
  let calls = 0;
  let fail = false;
  const input = Object.assign(new EventTarget(), { focus:()=>focused++ });
  const doc = Object.assign(new EventTarget(), { getElementById:(id:string) => id === 'pin-login-overlay' ? overlay : id === 'pin-login-input' ? input : null });
  const context:any = { document:doc,location:{hostname:'127.0.0.1'},apiFetch:async(path:string)=>{
    assert.equal(path,'/api/auth/status'); calls++;
    if(fail) throw new Error('synthetic offline');
    return {ok:true,json:async()=>status};
  }};
  runInNewContext(code,context);
  context.init();
  return {overlay,doc,setStatus:(value:typeof status)=>{status=value;},setOffline:()=>{fail=true;},calls:()=>calls,focused:()=>focused};
}

test('server logical-remote PIN status prompts again after a WS disconnect, even for a loopback URL',async()=>{
  const f=fixture(); await tick();
  assert.equal(f.overlay.hidden,true);
  f.setStatus({pin_enabled:true,remote:true,authed:false});
  f.doc.dispatchEvent(new Event('remote-auth-check')); await tick();
  assert.equal(f.overlay.hidden,false);
  assert.equal(f.focused(),1);
  assert.equal(f.calls(),2);
  f.setStatus({pin_enabled:true,remote:true,authed:true});
  f.doc.dispatchEvent(new Event('remote-auth-check')); await tick();
  assert.equal(f.overlay.hidden,true);
});

test('disabled PIN and a local screen do not prompt, and an offline status check keeps the current screen',async()=>{
  const f=fixture(); await tick();
  f.setStatus({pin_enabled:false,remote:true,authed:false});
  f.doc.dispatchEvent(new Event('remote-auth-check')); await tick();
  assert.equal(f.overlay.hidden,true);
  f.setStatus({pin_enabled:true,remote:true,authed:false});
  f.doc.dispatchEvent(new Event('remote-auth-check')); await tick();
  assert.equal(f.overlay.hidden,false);
  f.setOffline(); f.doc.dispatchEvent(new Event('remote-auth-check')); await tick();
  assert.equal(f.overlay.hidden,false);
});
