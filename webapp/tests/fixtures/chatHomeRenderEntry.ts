// Test-only bundle entry: re-exports both the view and the LocaleProvider from a
// SINGLE esbuild bundle so they share one LocaleContext instance. Importing the
// provider from a separate bundle would create a distinct context object, and the
// view (reading its own bundled context) would ignore the provider's locale.
export { ChatHomeView, appendApprovalMessage, replaceApprovalMessagesFromServer, setApprovalState, setApprovalProgress, composerSizeActionLabel, chatDateDividerLabels, collapsedChatHomeMessages, messageBodyText, messageCiteToken, messagePublicRef, isGatePendingOwnerAck, omitGatePendingOwnerAcks, ownerVisibleSearchOutcome, searchHoldIdentity } from '../../src/vnext/ChatHomeView';
export { LocaleProvider } from '../../src/i18n/LocaleContext';
// Re-exported so the render test can compute the EXACT localized live-progress
// working text (Slice 3) from the same bundled catalog the view renders.
export { tf, t } from '../../src/i18n/catalog';
