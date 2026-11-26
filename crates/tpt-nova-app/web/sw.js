// Service worker for the TPT Nova PWA.
//
// Strategy: the app shell (page, manifest, icons, generated JS and WASM) is
// precached on install so the editor opens offline after the first visit.
// Everything else goes to the network. Bump CACHE when a shell file changes,
// so clients drop the old cache on activate.
const CACHE = 'tpt-nova-shell-v2';
const SHELL = [
  './',
  './index.html',
  './manifest.webmanifest',
  './icons/icon-192.png',
  './icons/icon-512.png',
  './tpt-nova-app.js',
  './tpt-nova-app_bg.wasm',
];

self.addEventListener('install', (event) => {
  event.waitUntil(
    caches.open(CACHE).then((cache) => cache.addAll(SHELL)).then(() => self.skipWaiting())
  );
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) => Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k))))
      .then(() => self.clients.claim())
  );
});

self.addEventListener('fetch', (event) => {
  const request = event.request;
  if (request.method !== 'GET' || new URL(request.url).origin !== self.location.origin) {
    return;
  }
  // Cache-first for the shell; network fallback for anything not precached.
  event.respondWith(
    caches.match(request).then((hit) => hit || fetch(request))
  );
});
