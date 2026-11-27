import type { ReactNode } from 'react';
import { useLocale } from '../i18n/LocaleContext';
import { isResearchProgress, ownerFacingSearchReceipt, type SearchReceipt } from './searchReceipt';

type ReceiptTranslator = { t: (key: string) => string };

export function SearchReceiptCard({
  receipt,
  tr,
  ownerQuestion = '',
  inline = false,
}: {
  receipt: SearchReceipt;
  tr: ReceiptTranslator;
  ownerQuestion?: string;
  inline?: boolean;
}) {
  const { locale } = useLocale();
  const zh = locale === 'zh';
  const shown = ownerFacingSearchReceipt(receipt, ownerQuestion);
  if (shown.sources.length === 0) return null;
  const content = <>
      {shown.services?.length ? <p className="vnext-search-service">{shown.services.map(service => {
        const name = ({ codex_subscription: zh ? 'Codex 订阅' : 'Codex subscription',
          xai_subscription_web_search: zh ? 'Grok 订阅' : 'Grok subscription',
          openrouter_native_xai_search: 'OpenRouter', duckduckgo_wasm: 'DuckDuckGo' } as Record<string,string>)[service.source]
          ?? (zh ? '已配置的搜索服务' : 'Configured search service');
        return name + (service.fallback_used ? (zh ? '（兜底）' : ' (fallback)') : '');
      }).join(' · ')}</p> : null}
      <ol className="vnext-search-source-list">
        {shown.sources.map((source, index) => (
          <li key={`${source.url}:${index}`}>
            {source.url ? <a href={source.url} target="_blank" rel="noopener noreferrer">{source.title || new URL(source.url).hostname}</a> : <span>{source.title}</span>}
            {source.excerpt ? <p>{source.excerpt}</p> : null}
          </li>
        ))}
      </ol>
    </>;
  return inline ? <div className="vnext-search-sources" data-testid="search-receipt">{content}</div> : (
    <details className="vnext-answer-card tone-evidence vnext-search-sources" data-testid="search-receipt">
      <summary>{tr.t('chatHome.search.sources')} · {shown.sources.length}</summary>
      {content}
    </details>
  );
}

export function ResearchStopControl({
  text,
  stopped,
  onStop,
  label,
  stoppedLabel,
}: {
  text: string;
  stopped: boolean;
  onStop: () => void;
  label: string;
  stoppedLabel: string;
}): ReactNode {
  if (!isResearchProgress(text)) return null;
  if (stopped) {
    return <p data-testid="research-stopped">{stoppedLabel}</p>;
  }
  return (
    <button type="button" data-testid="research-stop" onClick={onStop}>
      {label}
    </button>
  );
}
