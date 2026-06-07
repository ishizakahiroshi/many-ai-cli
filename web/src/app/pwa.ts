import { token } from './util.js';
import { activateSession } from './session-list.js';

let serviceWorkerReadyPromise = null;

export function registerServiceWorker() {
  if (!('serviceWorker' in navigator)) return null;
  if (!serviceWorkerReadyPromise) {
    serviceWorkerReadyPromise = new Promise((resolve) => {
      window.addEventListener('load', () => {
        navigator.serviceWorker.register('/sw.js', { scope: '/' })
          .then((registration) => {
            rememberHubToken(registration);
            resolve(registration);
          })
          .catch(() => resolve(null));
      });
    });
    navigator.serviceWorker.addEventListener('controllerchange', () => {
      navigator.serviceWorker.ready.then(rememberHubToken).catch(() => {});
    });
  }
  return serviceWorkerReadyPromise;
}

export async function serviceWorkerReady() {
  if (!('serviceWorker' in navigator)) return null;
  registerServiceWorker();
  try {
    const registration = await navigator.serviceWorker.ready;
    rememberHubToken(registration);
    return registration;
  } catch (_) {
    return null;
  }
}

export function pushNotificationsSupported() {
  return !!(
    'serviceWorker' in navigator &&
    'PushManager' in window &&
    'Notification' in window &&
    window.isSecureContext
  );
}

export function isLikelyIOSBrowserTabWithoutStandalone() {
  const ua = navigator.userAgent || '';
  const isiOS = /iPad|iPhone|iPod/.test(ua) || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1);
  const standalone = window.matchMedia('(display-mode: standalone)').matches || navigator.standalone === true;
  return isiOS && !standalone;
}

export async function getPushSubscription() {
  if (!pushNotificationsSupported()) return null;
  const registration = await serviceWorkerReady();
  if (!registration || !registration.pushManager) return null;
  return registration.pushManager.getSubscription();
}

export async function subscribeWebPush() {
  if (!pushNotificationsSupported()) throw new Error('unsupported');
  const registration = await serviceWorkerReady();
  if (!registration || !registration.pushManager) throw new Error('unsupported');
  let permission = Notification.permission;
  if (permission !== 'granted') {
    permission = await Notification.requestPermission();
  }
  if (permission !== 'granted') throw new Error(permission || 'permission_denied');
  let subscription = await registration.pushManager.getSubscription();
  if (!subscription) {
    const publicKey = await fetchPushPublicKey();
    subscription = await registration.pushManager.subscribe({
      userVisibleOnly: true,
      applicationServerKey: urlBase64ToUint8Array(publicKey),
    });
  }
  await savePushSubscription(subscription);
  rememberHubToken(registration);
  return subscription;
}

export async function unsubscribeWebPush() {
  const subscription = await getPushSubscription();
  if (!subscription) return;
  await deletePushSubscription(subscription).catch(() => {});
  await subscription.unsubscribe();
}

export async function fetchPushStatus() {
  const res = await fetch(`/api/push/status?token=${encodeURIComponent(token || '')}`);
  if (!res.ok) return { supported: false, subscriptions: 0 };
  return res.json();
}

async function fetchPushPublicKey() {
  const res = await fetch(`/api/push/vapid-public-key?token=${encodeURIComponent(token || '')}`);
  if (!res.ok) throw new Error(`vapid key ${res.status}`);
  const data = await res.json();
  if (!data.public_key) throw new Error('missing_vapid_key');
  return data.public_key;
}

async function savePushSubscription(subscription) {
  const res = await fetch(`/api/push/subscriptions?token=${encodeURIComponent(token || '')}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(subscription.toJSON ? subscription.toJSON() : subscription),
  });
  if (!res.ok) throw new Error(`save subscription ${res.status}`);
}

async function deletePushSubscription(subscription) {
  const json = subscription.toJSON ? subscription.toJSON() : subscription;
  const res = await fetch(`/api/push/subscriptions?token=${encodeURIComponent(token || '')}`, {
    method: 'DELETE',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ endpoint: json.endpoint, keys: json.keys || {} }),
  });
  if (!res.ok) throw new Error(`delete subscription ${res.status}`);
}

function rememberHubToken(registration) {
  if (!registration) return;
  const target = registration.active || navigator.serviceWorker.controller;
  if (target) target.postMessage({ type: 'any-ai-cli-token', token: token || '' });
}

function urlBase64ToUint8Array(base64String) {
  const padding = '='.repeat((4 - base64String.length % 4) % 4);
  const base64 = (base64String + padding).replace(/-/g, '+').replace(/_/g, '/');
  const raw = atob(base64);
  const output = new Uint8Array(raw.length);
  for (let i = 0; i < raw.length; i++) output[i] = raw.charCodeAt(i);
  return output;
}

if ('serviceWorker' in navigator) {
  navigator.serviceWorker.addEventListener('message', (event) => {
    const data = event.data || {};
    if (data.type !== 'any-ai-cli-open-session') return;
    const sessionId = Number(data.session_id || 0);
    if (!Number.isFinite(sessionId) || sessionId <= 0) return;
    activateSession(sessionId);
  });
}

registerServiceWorker();
