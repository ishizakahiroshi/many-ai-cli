const CACHE_VERSION = 'many-ai-cli-sw-v1';
const TOKEN_CACHE = 'many-ai-cli-token-v1';
const TOKEN_URL = '/__many-ai-cli-token__';

self.addEventListener('install', (event) => {
  event.waitUntil(self.skipWaiting());
});

self.addEventListener('activate', (event) => {
  event.waitUntil((async () => {
    const keys = await caches.keys();
    await Promise.all(keys.filter((key) => key !== CACHE_VERSION && key !== TOKEN_CACHE).map((key) => caches.delete(key)));
    await self.clients.claim();
  })());
});

self.addEventListener('message', (event) => {
  const data = event.data || {};
  if (data.type !== 'many-ai-cli-token' || !data.token) return;
  event.waitUntil((async () => {
    const cache = await caches.open(TOKEN_CACHE);
    await cache.put(TOKEN_URL, new Response(JSON.stringify({ token: data.token }), {
      headers: { 'Content-Type': 'application/json' },
    }));
  })());
});

self.addEventListener('push', (event) => {
  event.waitUntil((async () => {
    const payload = parsePushPayload(event);
    const windows = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
    if (windows.some((client) => client.visibilityState === 'visible')) {
      return;
    }
    const title = payload.title || 'MANY-AI-CLI';
    const body = payload.body || 'Approval is waiting.';
    await self.registration.showNotification(title, {
      body,
      icon: '/icon.svg',
      badge: '/icon.svg',
      tag: payload.session_id ? `many-ai-cli-approval-${payload.session_id}` : payload.id || 'many-ai-cli-approval',
      data: {
        session_id: payload.session_id || 0,
        url: payload.url || '',
      },
      requireInteraction: false,
    });
  })());
});

self.addEventListener('notificationclick', (event) => {
  event.notification.close();
  event.waitUntil((async () => {
    const data = event.notification.data || {};
    const sessionId = Number(data.session_id || 0);
    const windows = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
    for (const client of windows) {
      client.postMessage({ type: 'many-ai-cli-open-session', session_id: sessionId });
      return client.focus();
    }
    const url = await notificationURL(data.url || '', sessionId);
    return self.clients.openWindow(url);
  })());
});

self.addEventListener('fetch', () => {
  // Network-only. The Hub is a localhost app with token-bound state, so caching
  // authenticated UI responses would create more risk than value.
});

function parsePushPayload(event) {
  if (!event.data) return {};
  try {
    return event.data.json() || {};
  } catch (_) {
    try {
      return JSON.parse(event.data.text() || '{}');
    } catch (_) {
      return {};
    }
  }
}

async function notificationURL(url, sessionId) {
  const base = url || '/';
  let target;
  try {
    target = new URL(base, self.location.origin);
  } catch (_) {
    target = new URL('/', self.location.origin);
  }
  // 同一オリジン以外（payload.url に外部絶対 URL が来た場合）には Hub トークンを付けず、
  // 安全側に自オリジンのトップへフォールバックする（トークンの外部オリジン漏洩を防ぐ）。
  if (target.origin !== self.location.origin) {
    target = new URL('/', self.location.origin);
  }
  // 既にトークン付き URL（自オリジン）なら、そのトークンを尊重してそのまま使う。
  if (target.searchParams.has('token')) {
    if (sessionId > 0 && !target.searchParams.has('session_id')) {
      target.searchParams.set('session_id', String(sessionId));
    }
    return target.href;
  }
  const token = await readHubToken();
  if (token) target.searchParams.set('token', token);
  if (sessionId > 0) target.searchParams.set('session_id', String(sessionId));
  return target.href;
}

async function readHubToken() {
  try {
    const cache = await caches.open(TOKEN_CACHE);
    const res = await cache.match(TOKEN_URL);
    if (!res) return '';
    const data = await res.json();
    return data.token || '';
  } catch (_) {
    return '';
  }
}
