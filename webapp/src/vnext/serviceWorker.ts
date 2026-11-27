export function registerOwnerServiceWorker(): void {
  if (!('serviceWorker' in navigator)) {
    return;
  }

  let refreshing = false;
  let reloadPending = false;
  const composerHoldsDraft = () => {
    const active = document.activeElement;
    if (active instanceof HTMLTextAreaElement && active.value.trim()) return true;
    return Array.from(document.querySelectorAll('textarea')).some((el) => el.value.trim());
  };
  const reloadNow = () => {
    if (refreshing) return;
    refreshing = true;
    window.location.reload();
  };
  navigator.serviceWorker.addEventListener('controllerchange', () => {
    // A background update used to hard-reload mid-compose, discarding the
    // draft and any pending attachment. Defer until the composer is empty
    // or the page is hidden.
    if (composerHoldsDraft()) {
      reloadPending = true;
      return;
    }
    reloadNow();
  });
  document.addEventListener('visibilitychange', () => {
    if (reloadPending && document.visibilityState === 'hidden') reloadNow();
  });

  window.addEventListener('load', () => {
    navigator.serviceWorker
      .register('/ui/service-worker.js', { scope: '/ui/', updateViaCache: 'none' })
      .then((registration) => {
        registration.update().catch(() => {});
      })
      .catch(() => {
        // The owner UI must stay usable if install support is unavailable.
      });
  });
}
