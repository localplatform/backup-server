import { logger } from './logger.js';

export interface RetryOptions {
  maxRetries: number;
  baseDelay: number;
  maxDelay: number;
  label?: string;
}

const defaults: RetryOptions = {
  maxRetries: 5,
  baseDelay: 1000,
  maxDelay: 60000,
};

export async function retry<T>(
  fn: () => Promise<T>,
  opts: Partial<RetryOptions> = {}
): Promise<T> {
  const options = { ...defaults, ...opts };
  let lastError: Error | undefined;

  for (let attempt = 0; attempt <= options.maxRetries; attempt++) {
    try {
      return await fn();
    } catch (err) {
      lastError = err as Error;
      if (attempt >= options.maxRetries) break;

      const delay = Math.min(
        options.baseDelay * Math.pow(2, attempt) + Math.random() * 1000,
        options.maxDelay
      );
      logger.warn({ attempt, delay, label: options.label, error: lastError.message }, 'Retrying...');
      await new Promise(resolve => setTimeout(resolve, delay));
    }
  }

  throw lastError;
}
