const CACHE_PREFIX = 'aegis-owner-shell-';
const CACHE_NAME = 'aegis-owner-shell-v2';

self.addEventListener('install', (event) => {
  event.waitUntil(Promise.resolve());
  self.skipWaiting();
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches.keys().then((keys) =>
      Promise.all(keys.filter((key) => key.startsWith(CACHE_PREFIX)).map((key) => caches.delete(key))),
    ),
  );
  self.clients.claim();
});

self.addEventListener('push', (event) => {
  let payload = {};
  try {
    payload = event.data ? event.data.json() : {};
  } catch {
    payload = { title: 'Aegis', body: event.data ? event.data.text() : 'Aegis has an update.' };
  }
  const title = typeof payload.title === 'string' && payload.title.trim() ? payload.title : 'Aegis';
  const options = {
    body: typeof payload.body === 'string' ? payload.body : 'Open Aegis to review the update.',
    tag: typeof payload.tag === 'string' ? payload.tag : 'aegis-owner-notice',
    data: {
      url: typeof payload.url === 'string' && payload.url.startsWith('/ui/') ? payload.url : '/ui/',
    },
  };
  event.waitUntil(self.registration.showNotification(title, options));
});

self.addEventListener('notificationclick', (event) => {
  event.notification.close();
  const targetUrl = event.notification.data && event.notification.data.url ? event.notification.data.url : '/ui/';
  event.waitUntil(
    clients.matchAll({ type: 'window', includeUncontrolled: true }).then((clientList) => {
      for (const client of clientList) {
        const url = new URL(client.url);
        if (url.pathname.startsWith('/ui/')) {
          client.focus();
          return client.navigate(targetUrl);
        }
      }
      return clients.openWindow(targetUrl);
    }),
  );
});
