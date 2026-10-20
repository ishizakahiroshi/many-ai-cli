import test from 'node:test';
import assert from 'node:assert/strict';
import { attachSessionCardPopover, hideSessionCardPopover, refreshSessionCardPopover } from './session-card-popover.js';

class Events {
  listeners = new Map<string, Array<(event: any) => void>>();
  addEventListener(type: string, listener: (event: any) => void): void {
    this.listeners.set(type, [...(this.listeners.get(type) || []), listener]);
  }
  emit(type: string, data: Record<string, unknown> = {}): void {
    for (const listener of this.listeners.get(type) || []) listener({ target: this, ...data });
  }
}

class ElementStub extends Events {
  id = '';
  className = '';
  hidden = true;
  isConnected = true;
  innerHTML = '';
  dataset: Record<string, string> = {};
  style: Record<string, string> = {};
  attributes = new Map<string, string>();
  offsetWidth = 240;
  offsetHeight = 80;
  setAttribute(key: string, value: string): void { this.attributes.set(key, value); }
  removeAttribute(key: string): void { this.attributes.delete(key); }
  contains(other: unknown): boolean { return other === this; }
  matches(selector: string): boolean { return selector === ':focus-visible'; }
  getBoundingClientRect() { return { left: 20, top: 40, bottom: 68 }; }
}

test('compact details lifecycle with synthetic DOM and deterministic timers', async context => {
  const priorWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
  const priorDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
  const documentStub = new Events() as Events & { body: { appendChild: (el: ElementStub) => void }; createElement: () => ElementStub };
  const windowStub = new Events() as Events & Record<string, any>;
  const appended: ElementStub[] = [];
  documentStub.body = { appendChild: el => { appended.push(el); } };
  documentStub.createElement = () => new ElementStub();
  let now = 0;
  let serial = 0;
  let hover = true;
  const timers = new Map<number, { at: number; run: () => void }>();
  windowStub.setTimeout = (run: () => void, delay: number) => { const id = ++serial; timers.set(id, { at: now + delay, run }); return id; };
  windowStub.clearTimeout = (id: number) => timers.delete(id);
  windowStub.matchMedia = () => ({ matches: hover });
  windowStub.innerWidth = 960;
  windowStub.innerHeight = 600;
  const advance = (ms: number) => {
    now += ms;
    for (const [id, timer] of [...timers]) if (timer.at <= now) { timers.delete(id); timer.run(); }
  };
  const card = new ElementStub();
  const node = card as unknown as HTMLElement;
  let detail = '<div>Synthetic details</div>';
  Object.defineProperty(globalThis, 'window', { configurable: true, value: windowStub });
  Object.defineProperty(globalThis, 'document', { configurable: true, value: documentStub });
  try {
    attachSessionCardPopover(node, () => detail);
    await context.test('400ms stationary delay, overlay and immediate leave', () => {
      card.emit('pointerenter', { pointerType: 'mouse' });
      advance(399);
      assert.equal(appended.length, 0);
      card.emit('pointermove', { pointerType: 'mouse' });
      advance(399);
      assert.equal(appended.length, 0);
      advance(1);
      assert.equal(appended.length, 1);
      assert.equal(appended[0].hidden, false);
      assert.equal(appended[0].innerHTML, detail);
      assert.equal(appended[0].style.top, '70px');
      assert.equal(card.attributes.get('aria-describedby'), 'session-card-popover');
      card.emit('pointerleave');
      assert.equal(appended[0].hidden, true);
      assert.equal(card.attributes.has('aria-describedby'), false);
    });
    await context.test('scroll cancels pending and visible details', () => {
      card.emit('pointerenter', { pointerType: 'mouse' });
      documentStub.emit('scroll');
      advance(400);
      assert.equal(appended[0].hidden, true);
      card.emit('pointerenter', { pointerType: 'mouse' });
      advance(400);
      documentStub.emit('scroll');
      assert.equal(appended[0].hidden, true);
    });
    await context.test('render/tab cleanup, drag, click, blur and resize cancel pending details', () => {
      const dismissals = [hideSessionCardPopover, () => documentStub.emit('dragstart'), () => documentStub.emit('pointerdown'),
        () => windowStub.emit('blur'), () => windowStub.emit('resize'), () => documentStub.emit('visibilitychange')];
      for (const dismiss of dismissals) {
        card.emit('pointerenter', { pointerType: 'mouse' });
        dismiss(); advance(400);
        assert.equal(appended[0].hidden, true);
        assert.equal(timers.size, 0);
      }
    });
    await context.test('keyboard focus reads details and Escape/focusout dismiss them', () => {
      card.emit('focusin'); advance(400);
      assert.equal(appended[0].hidden, false);
      documentStub.emit('keydown', { key: 'Escape' });
      assert.equal(appended[0].hidden, true);
      card.emit('focusin'); advance(400);
      card.emit('focusout', { relatedTarget: null });
      assert.equal(appended[0].hidden, true);
    });
    await context.test('live refresh uses current data and never reopens a dismissed popup', () => {
      card.emit('pointerenter', { pointerType: 'mouse' }); advance(400);
      detail = '<div>Updated synthetic details</div>';
      refreshSessionCardPopover(node);
      assert.equal(appended[0].innerHTML, detail);
      hideSessionCardPopover(); refreshSessionCardPopover(node);
      assert.equal(appended[0].hidden, true);
    });
    await context.test('touch and no-hover environments never open details', () => {
      card.emit('pointerenter', { pointerType: 'touch' }); advance(400);
      assert.equal(appended[0].hidden, true);
      hover = false;
      card.emit('pointerenter', { pointerType: 'mouse' }); card.emit('focusin'); advance(400);
      assert.equal(appended[0].hidden, true);
      hover = true;
    });
    await context.test('removed anchors cannot show stale content', () => {
      card.emit('pointerenter', { pointerType: 'mouse' });
      card.isConnected = false; advance(400);
      assert.equal(appended[0].hidden, true);
      assert.equal(timers.size, 0);
      card.isConnected = true;
    });
  } finally {
    hideSessionCardPopover();
    if (priorWindow) Object.defineProperty(globalThis, 'window', priorWindow);
    else Reflect.deleteProperty(globalThis, 'window');
    if (priorDocument) Object.defineProperty(globalThis, 'document', priorDocument);
    else Reflect.deleteProperty(globalThis, 'document');
  }
});
