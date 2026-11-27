import { createRoot } from 'react-dom/client';
import VNextApp from './vnext/App';
import { registerOwnerServiceWorker } from './vnext/serviceWorker';
import { startAutoUpdate } from './vnext/autoUpdate';

document.documentElement.dataset.aegisOwnerUiBuild = '2026-05-20-cache-bypass-v3';

createRoot(document.getElementById('root')!).render(<VNextApp />);
registerOwnerServiceWorker();
// Auto-reload to a freshly-deployed bundle on return-from-background, so the
// owner PWA never stays stranded on a stale build (owner feedback 2026-06-16).
startAutoUpdate();
