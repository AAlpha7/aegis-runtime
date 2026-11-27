/** Telegram WebApp SDK type declarations — Bot API 6.0 → 9.6 */

// ── User & Theme ──────────────────────────────────────────────────────────────

interface TelegramWebAppUser {
  id: number;
  first_name: string;
  last_name?: string;
  username?: string;
  language_code?: string;
  is_premium?: boolean;
  photo_url?: string;
}

interface TelegramThemeParams {
  bg_color?: string;
  text_color?: string;
  hint_color?: string;
  link_color?: string;
  button_color?: string;
  button_text_color?: string;
  secondary_bg_color?: string;
  header_bg_color?: string;
  accent_text_color?: string;
  section_bg_color?: string;
  section_header_text_color?: string;
  subtitle_text_color?: string;
  destructive_text_color?: string;
  section_separator_color?: string;
  bottom_bar_bg_color?: string;
}

// ── Safe Area ─────────────────────────────────────────────────────────────────

interface SafeAreaInset {
  top: number;
  bottom: number;
  left: number;
  right: number;
}

interface ContentSafeAreaInset {
  top: number;
  bottom: number;
  left: number;
  right: number;
}

// ── Buttons ───────────────────────────────────────────────────────────────────

interface TelegramMainButton {
  text: string;
  color: string;
  textColor: string;
  isVisible: boolean;
  isActive: boolean;
  isProgressVisible: boolean;
  hasShineEffect: boolean;
  position: 'left' | 'right' | 'top' | 'bottom';
  setText(text: string): void;
  onClick(callback: () => void): void;
  offClick(callback: () => void): void;
  show(): void;
  hide(): void;
  enable(): void;
  disable(): void;
  showProgress(leaveActive?: boolean): void;
  hideProgress(): void;
  setParams(params: {
    text?: string;
    color?: string;
    text_color?: string;
    is_active?: boolean;
    is_visible?: boolean;
    has_shine_effect?: boolean;
    position?: 'left' | 'right' | 'top' | 'bottom';
  }): void;
}

interface TelegramBackButton {
  isVisible: boolean;
  onClick(callback: () => void): void;
  offClick(callback: () => void): void;
  show(): void;
  hide(): void;
}

interface TelegramSettingsButton {
  isVisible: boolean;
  onClick(callback: () => void): void;
  offClick(callback: () => void): void;
  show(): void;
  hide(): void;
}

// ── Haptic Feedback ───────────────────────────────────────────────────────────

interface TelegramHapticFeedback {
  impactOccurred(style: 'light' | 'medium' | 'heavy' | 'rigid' | 'soft'): void;
  notificationOccurred(type: 'error' | 'success' | 'warning'): void;
  selectionChanged(): void;
}

// ── Cloud Storage (Bot API 6.9+) ──────────────────────────────────────────────

interface TelegramCloudStorage {
  setItem(key: string, value: string, callback?: (error: string | null, ok?: boolean) => void): void;
  getItem(key: string, callback: (error: string | null, value?: string) => void): void;
  getItems(keys: string[], callback: (error: string | null, values?: Record<string, string>) => void): void;
  removeItem(key: string, callback?: (error: string | null, ok?: boolean) => void): void;
  removeItems(keys: string[], callback?: (error: string | null, ok?: boolean) => void): void;
  getKeys(callback: (error: string | null, keys?: string[]) => void): void;
}

// ── Device Storage (Bot API 9.0+) — 5MB local persistent storage ─────────────

interface TelegramDeviceStorage {
  setItem(key: string, value: string, callback?: (error: string | null, ok?: boolean) => void): void;
  getItem(key: string, callback: (error: string | null, value?: string) => void): void;
  removeItem(key: string, callback?: (error: string | null, ok?: boolean) => void): void;
  clear(callback?: (error: string | null, ok?: boolean) => void): void;
}

// ── Secure Storage (Bot API 9.0+) — hardware-backed keychain ──────────────────

interface TelegramSecureStorage {
  setItem(key: string, value: string, callback?: (error: string | null, ok?: boolean) => void): void;
  getItem(key: string, callback: (error: string | null, value?: string) => void): void;
  removeItem(key: string, callback?: (error: string | null, ok?: boolean) => void): void;
  clear(callback?: (error: string | null, ok?: boolean) => void): void;
}

// ── Biometric Manager (Bot API 7.2+) ─────────────────────────────────────────

interface TelegramBiometricManager {
  isInited: boolean;
  isBiometricAvailable: boolean;
  biometricType: 'finger' | 'face' | 'unknown';
  isAccessRequested: boolean;
  isAccessGranted: boolean;
  isBiometricTokenSaved: boolean;
  deviceId: string;
  init(callback?: () => void): void;
  requestAccess(params: { reason?: string }, callback?: (ok: boolean) => void): void;
  authenticate(params: { reason?: string }, callback?: (ok: boolean, token?: string) => void): void;
  updateBiometricToken(token: string, callback?: (ok: boolean) => void): void;
  openSettings(): void;
}

// ── Location Manager (Bot API 8.0+) ──────────────────────────────────────────

interface TelegramLocationManager {
  isInited: boolean;
  isLocationAvailable: boolean;
  isAccessRequested: boolean;
  isAccessGranted: boolean;
  init(callback?: () => void): void;
  getLocation(callback: (location: { latitude: number; longitude: number; altitude?: number; course?: number; speed?: number; horizontal_accuracy?: number; vertical_accuracy?: number; course_accuracy?: number; speed_accuracy?: number } | null) => void): void;
  openSettings(): void;
}

// ── Sensors (Bot API 8.0+) ────────────────────────────────────────────────────

interface TelegramSensor {
  isStarted: boolean;
  start(params?: { refresh_rate?: number }, callback?: (ok: boolean) => void): void;
  stop(callback?: (ok: boolean) => void): void;
}

interface TelegramAccelerometer extends TelegramSensor {
  x: number;
  y: number;
  z: number;
}

interface TelegramGyroscope extends TelegramSensor {
  x: number;
  y: number;
  z: number;
}

interface TelegramDeviceOrientation extends TelegramSensor {
  absolute: boolean;
  alpha: number;
  beta: number;
  gamma: number;
}

// ── Popup ─────────────────────────────────────────────────────────────────────

interface PopupParams {
  title?: string;
  message: string;
  buttons?: PopupButton[];
}

interface PopupButton {
  id?: string;
  type?: 'default' | 'ok' | 'close' | 'cancel' | 'destructive';
  text?: string;
}

// ── Main WebApp Interface ─────────────────────────────────────────────────────

interface TelegramWebApp {
  // ── Identity & Init ──
  initData: string;
  initDataUnsafe: {
    query_id?: string;
    user?: TelegramWebAppUser;
    auth_date?: number;
    hash?: string;
    chat_instance?: string;
    start_param?: string;
  };
  version: string;
  platform: string;

  // ── Theme ──
  colorScheme: 'light' | 'dark';
  themeParams: TelegramThemeParams;
  headerColor: string;
  backgroundColor: string;
  bottomBarColor: string;
  setHeaderColor(color: string): void;
  setBackgroundColor(color: string): void;
  setBottomBarColor(color: string): void;

  // ── Viewport ──
  isExpanded: boolean;
  viewportHeight: number;
  viewportStableHeight: number;

  // ── Fullscreen (Bot API 8.0+) ──
  isFullscreen: boolean;
  requestFullscreen(): void;
  exitFullscreen(): void;

  // ── Orientation (Bot API 8.0+) ──
  isOrientationLocked: boolean;
  lockOrientation(): void;
  unlockOrientation(): void;

  // ── Lifecycle (Bot API 8.0+) ──
  isActive: boolean;

  // ── Safe Area (Bot API 8.0+) ──
  safeAreaInset: SafeAreaInset;
  contentSafeAreaInset: ContentSafeAreaInset;

  // ── Closing ──
  isClosingConfirmationEnabled: boolean;
  enableClosingConfirmation(): void;
  disableClosingConfirmation(): void;

  // ── Vertical Swipes (Bot API 7.7+) ──
  isVerticalSwipesEnabled: boolean;
  enableVerticalSwipes(): void;
  disableVerticalSwipes(): void;

  // ── Buttons ──
  MainButton: TelegramMainButton;
  SecondaryButton: TelegramMainButton;
  BackButton: TelegramBackButton;
  SettingsButton: TelegramSettingsButton;

  // ── Haptic ──
  HapticFeedback: TelegramHapticFeedback;

  // ── Storage ──
  CloudStorage: TelegramCloudStorage;
  DeviceStorage: TelegramDeviceStorage;
  SecureStorage: TelegramSecureStorage;

  // ── Biometric ──
  BiometricManager: TelegramBiometricManager;

  // ── Location ──
  LocationManager: TelegramLocationManager;

  // ── Sensors ──
  Accelerometer: TelegramAccelerometer;
  Gyroscope: TelegramGyroscope;
  DeviceOrientation: TelegramDeviceOrientation;

  // ── Core Methods ──
  close(): void;
  expand(): void;
  ready(): void;
  sendData(data: string): void;

  // ── Popups & Dialogs ──
  showPopup(params: PopupParams, callback?: (buttonId: string) => void): void;
  showAlert(message: string, callback?: () => void): void;
  showConfirm(message: string, callback?: (ok: boolean) => void): void;
  showScanQrPopup(params?: { text?: string }, callback?: (text: string) => boolean): void;
  closeScanQrPopup(): void;

  // ── Clipboard ──
  readTextFromClipboard(callback?: (text: string | null) => void): void;

  // ── File Download (Bot API 8.0+) ──
  downloadFile(params: { url: string; file_name: string }, callback?: (ok: boolean) => void): void;

  // ── Sharing ──
  shareToStory(media_url: string, params?: { text?: string; widget_link?: { url: string; name?: string } }): void;
  shareMessage(msg_id: string, callback?: (ok: boolean) => void): void;

  // ── Home Screen (Bot API 8.0+) ──
  addToHomeScreen(): void;
  checkHomeScreenStatus(callback?: (status: 'unsupported' | 'unknown' | 'added' | 'missed') => void): void;

  // ── Emoji Status (Bot API 8.0+) ──
  setEmojiStatus(custom_emoji_id: string, params?: { duration?: number }, callback?: (ok: boolean) => void): void;

  // ── Contact & Write Access ──
  requestContact(callback?: (ok: boolean) => void): void;
  requestWriteAccess(callback?: (ok: boolean) => void): void;

  // ── Chat (Bot API 9.5+) ──
  requestChat(params: { chat_type: 'users' | 'bots' | 'groups' | 'supergroups' | 'channels' }, callback?: (chat: { id: number; type: string; title?: string; username?: string; photo_url?: string } | null) => void): void;

  // ── Keyboard (Bot API 9.1+) ──
  hideKeyboard(): void;

  // ── Invoice / Payment ──
  openInvoice(url: string, callback?: (status: 'paid' | 'cancelled' | 'failed' | 'pending') => void): void;

  // ── Links ──
  openLink(url: string, options?: { try_instant_view?: boolean }): void;
  openTelegramLink(url: string): void;
  switchInlineQuery(query: string, choose_chat_types?: string[]): void;

  // ── Events ──
  onEvent(eventType: 'themeChanged', callback: () => void): void;
  onEvent(eventType: 'viewportChanged', callback: (event: { isStateStable: boolean }) => void): void;
  onEvent(eventType: 'mainButtonClicked', callback: () => void): void;
  onEvent(eventType: 'secondaryButtonClicked', callback: () => void): void;
  onEvent(eventType: 'backButtonClicked', callback: () => void): void;
  onEvent(eventType: 'settingsButtonClicked', callback: () => void): void;
  onEvent(eventType: 'invoiceClosed', callback: (event: { url: string; status: string }) => void): void;
  onEvent(eventType: 'popupClosed', callback: (event: { button_id: string | null }) => void): void;
  onEvent(eventType: 'qrTextReceived', callback: (event: { data: string }) => void): void;
  onEvent(eventType: 'clipboardTextReceived', callback: (event: { data: string | null }) => void): void;
  onEvent(eventType: 'writeAccessRequested', callback: (event: { status: string }) => void): void;
  onEvent(eventType: 'contactRequested', callback: (event: { status: string }) => void): void;
  onEvent(eventType: 'biometricManagerUpdated', callback: () => void): void;
  onEvent(eventType: 'biometricAuthRequested', callback: (event: { isAuthenticated: boolean; biometricToken?: string }) => void): void;
  onEvent(eventType: 'activated', callback: () => void): void;
  onEvent(eventType: 'deactivated', callback: () => void): void;
  onEvent(eventType: 'safeAreaChanged', callback: () => void): void;
  onEvent(eventType: 'contentSafeAreaChanged', callback: () => void): void;
  onEvent(eventType: 'fullscreenChanged', callback: () => void): void;
  onEvent(eventType: 'fullscreenFailed', callback: (event: { error: string }) => void): void;
  onEvent(eventType: 'homeScreenAdded', callback: () => void): void;
  onEvent(eventType: 'homeScreenChecked', callback: (event: { status: string }) => void): void;
  onEvent(eventType: 'accelerometerStarted', callback: () => void): void;
  onEvent(eventType: 'accelerometerStopped', callback: () => void): void;
  onEvent(eventType: 'accelerometerChanged', callback: () => void): void;
  onEvent(eventType: 'accelerometerFailed', callback: (event: { error: string }) => void): void;
  onEvent(eventType: 'deviceOrientationStarted', callback: () => void): void;
  onEvent(eventType: 'deviceOrientationStopped', callback: () => void): void;
  onEvent(eventType: 'deviceOrientationChanged', callback: () => void): void;
  onEvent(eventType: 'deviceOrientationFailed', callback: (event: { error: string }) => void): void;
  onEvent(eventType: 'gyroscopeStarted', callback: () => void): void;
  onEvent(eventType: 'gyroscopeStopped', callback: () => void): void;
  onEvent(eventType: 'gyroscopeChanged', callback: () => void): void;
  onEvent(eventType: 'gyroscopeFailed', callback: (event: { error: string }) => void): void;
  onEvent(eventType: 'locationManagerUpdated', callback: () => void): void;
  onEvent(eventType: 'locationRequested', callback: (event: { locationData: unknown }) => void): void;
  onEvent(eventType: 'emojiStatusSet', callback: () => void): void;
  onEvent(eventType: 'emojiStatusFailed', callback: (event: { error: string }) => void): void;
  onEvent(eventType: 'emojiStatusAccessRequested', callback: (event: { status: string }) => void): void;
  onEvent(eventType: 'fileDownloadRequested', callback: (event: { status: string }) => void): void;
  onEvent(eventType: string, callback: (...args: unknown[]) => void): void;

  offEvent(eventType: string, callback: (...args: unknown[]) => void): void;

  // ── Version Check ──
  isVersionAtLeast(version: string): boolean;
}

interface Window {
  Telegram?: {
    WebApp: TelegramWebApp;
  };
}
