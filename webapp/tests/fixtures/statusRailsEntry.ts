// Test-only bundle entry for the runtime-rails tests (tests/vnextStatusRails.test.mjs):
// the Telegram banner state machine, the offline store, the nightly line and the
// two banner components, re-exported from ONE esbuild graph together with the
// LocaleProvider they consume (a separately-bundled provider would create a
// distinct context instance the components never read). Not imported by the app.
export {
  TELEGRAM_RAIL_DISMISSED_KEY,
  TELEGRAM_UNREACHABLE_BANNER_AFTER_SECS,
  nightlyStatusLine,
  readNightlyStatus,
  readTelegramRail,
  readTelegramRailDismissedSince,
  telegramRailBannerState,
  telegramRailRow,
  writeTelegramRailDismissedSince,
} from '../../src/vnext/statusRails';
export { createOnlineStatusStore, readOnlineStatus } from '../../src/vnext/onlineStatus';
export { TelegramRailBanner } from '../../src/vnext/TelegramRailBanner';
export { OfflineBanner } from '../../src/vnext/OfflineBanner';
export { LocaleProvider } from '../../src/i18n/LocaleContext';
export { t, tf } from '../../src/i18n/catalog';
