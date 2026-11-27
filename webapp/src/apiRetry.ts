export type FetchLike = (url: string, init: RequestInit) => Promise<Response>;
export type SleepLike = (ms: number) => Promise<void>;

function isLikelyNetworkError(error: unknown): boolean {
  return error instanceof TypeError
    || (error instanceof Error && /failed to fetch|network|load failed/i.test(error.message));
}

function defaultDelay(ms: number): Promise<void> {
  return new Promise((resolve) => {
    window.setTimeout(resolve, ms);
  });
}

function defaultFetch(url: string, init: RequestInit): Promise<Response> {
  return fetch(url, init);
}

export async function fetchJsonWithOneNetworkRetry(
  url: string,
  init: RequestInit,
  fetchImpl: FetchLike = defaultFetch,
  sleep: SleepLike = defaultDelay,
): Promise<Response> {
  try {
    return await fetchImpl(url, init);
  } catch (error) {
    if (!isLikelyNetworkError(error)) throw error;
    await sleep(600);
    return fetchImpl(url, init);
  }
}

function isRetryableAssistantStatus(status: number): boolean {
  return status === 408 || status === 429 || status === 503 || status === 504;
}

export async function fetchAssistantCommandWithTransientRetry(
  url: string,
  init: RequestInit,
  fetchImpl: FetchLike = defaultFetch,
  sleep: SleepLike = defaultDelay,
): Promise<Response> {
  const delaysMs = [500, 1200, 2400];
  let lastNetworkError: unknown = null;

  for (let attempt = 0; attempt <= delaysMs.length; attempt += 1) {
    try {
      const response = await fetchImpl(url, init);
      if (!isRetryableAssistantStatus(response.status) || attempt === delaysMs.length) {
        return response;
      }
    } catch (error) {
      if (!isLikelyNetworkError(error)) throw error;
      lastNetworkError = error;
      if (attempt === delaysMs.length) throw error;
    }
    await sleep(delaysMs[attempt]);
  }

  throw lastNetworkError instanceof Error ? lastNetworkError : new Error('Assistant command retry failed');
}
