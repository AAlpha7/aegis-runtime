export const ASSISTANT_STILL_WORKING_RETRY_DELAYS_MS = [2400, 5000, 10000] as const;
export const ASSISTANT_STILL_WORKING_RETRY_MESSAGE = 'Still working. Checking whether the answer is ready...';

export type AssistantProgressCallback = (message: string) => void;
export type AssistantRetryPredicate = (
  error: unknown,
  idempotencyKey: string | undefined,
  checksUsed: number,
) => boolean;

/** Result of one `/command/status` poll. */
export type StillWorkingStatus<T> =
  | { kind: 'pending' }
  | { kind: 'gone' }
  | { kind: 'done'; value: T };

/**
 * Poll schedule after the command request failed with a "still working"
 * signal (504 from the route, 524 from the edge, or a dropped connection).
 * About 3.5 minutes in total; each tick is a cheap GET, never a re-POST of
 * the (possibly multi-megabyte) command body.
 */
export const ASSISTANT_STATUS_POLL_DELAYS_MS = [
  3000, 5000, 5000, 5000, 7000, 10000, 10000, 10000, 15000, 15000, 15000, 20000, 20000, 20000, 20000, 20000,
] as const;

export async function requestWithStillWorkingAutoCheck<T, Args extends { idempotencyKey?: string }>({
  args,
  request,
  sleep,
  shouldRetry,
  onStillWorking,
  checkStatus,
  pollDelaysMs = ASSISTANT_STATUS_POLL_DELAYS_MS,
}: {
  args: Args;
  request: (args: Args) => Promise<T>;
  sleep: (ms: number) => Promise<void>;
  shouldRetry: AssistantRetryPredicate;
  onStillWorking?: AssistantProgressCallback;
  /** When given, a still-working failure is resolved by polling instead of re-posting. */
  checkStatus?: () => Promise<StillWorkingStatus<T>>;
  pollDelaysMs?: readonly number[];
}): Promise<T> {
  let checksUsed = 0;
  for (;;) {
    try {
      return await request(args);
    } catch (error) {
      if (!shouldRetry(error, args.idempotencyKey, checksUsed)) {
        throw error;
      }
      if (checkStatus) {
        for (const delayMs of pollDelaysMs) {
          onStillWorking?.(ASSISTANT_STILL_WORKING_RETRY_MESSAGE);
          await sleep(delayMs);
          const status = await checkStatus();
          if (status.kind === 'done') return status.value;
          if (status.kind === 'gone') throw error;
        }
        throw error;
      }
      const delayMs = ASSISTANT_STILL_WORKING_RETRY_DELAYS_MS[checksUsed] ?? 0;
      checksUsed += 1;
      onStillWorking?.(ASSISTANT_STILL_WORKING_RETRY_MESSAGE);
      await sleep(delayMs);
    }
  }
}
