'use strict';
// Service worker of the Bookx web client: no network access of its own, it only answers requests
// to this server. Books and covers are served from the device once saved; the catalogue and the
// app files are fetched fresh and kept for offline use.
const BASE = new URL('..', self.location.href).pathname.replace(/\/$/, '');
const SHELL = 'bookx-shell-v1';
const BOOKS = 'bookx-books';
const CATALOGUE = 'bookx-catalogue-v1';
const SHELL_FILES = ['app/', 'app/app.js', 'app/app.css', 'app/manifest.webmanifest', 'app/icon.svg'].map(f => BASE + '/' + f);

self.addEventListener('install', event => {
  event.waitUntil(caches.open(SHELL).then(c => c.addAll(SHELL_FILES)).then(() => self.skipWaiting()));
});

self.addEventListener('activate', event => {
  event.waitUntil(
    caches.keys()
      .then(keys => Promise.all(keys.filter(k => ![SHELL, BOOKS, CATALOGUE].includes(k)).map(k => caches.delete(k))))
      .then(() => self.clients.claim()));
});

function ownPath(url) {
  return url.origin === self.location.origin && url.pathname.startsWith(BASE + '/') ? url.pathname.slice(BASE.length) : null;
}

async function networkFirst(request, cacheName) {
  const cache = await caches.open(cacheName);
  try {
    const fresh = await fetch(request);
    if (fresh.ok) await cache.put(request, fresh.clone());
    return fresh;
  } catch (e) {
    const hit = await cache.match(request);
    if (hit) return hit;
    throw e;
  }
}

async function cacheFirst(request, cacheName) {
  const hit = await (await caches.open(cacheName)).match(request);
  return hit || fetch(request);
}

self.addEventListener('fetch', event => {
  const request = event.request;
  if (request.method !== 'GET') return;
  const path = ownPath(new URL(request.url));
  if (path === null) return;
  if (path.startsWith('/download/')) {
    event.respondWith(cacheFirst(request, BOOKS));
  } else if (path.startsWith('/cover/')) {
    event.respondWith(cacheFirst(request, BOOKS).catch(() => fetch(request)));
  } else if (path.startsWith('/opds')) {
    event.respondWith(networkFirst(request, CATALOGUE));
  } else if (path === '/app' || path.startsWith('/app/')) {
    event.respondWith(networkFirst(request, SHELL));
  }
  // everything else (the sync routes under /bookx, the plain web page) is not touched
});
