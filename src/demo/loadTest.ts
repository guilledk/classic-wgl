import { setLoadSleepMs } from '/classic/utils.js';
import { appendConsoleEntry } from '/classic/consoleLog.js';

/**
 * Optional query-param test knobs for manually exercising the loading screen:
 *
 *   ?slow          slow each resource-loading step by DEFAULT_TEST_SLEEP_MS
 *   ?slow=250      ... with a custom per-step delay (ms)
 *   ?fail=shader   404 every /shaders/* fetch (real compile failure)
 *   ?fail=manifest 404 /manifest.json
 *   ?fail=state    404 /state.json
 *
 * `applyLoadTestParams()` is called from src/demo/init.ts before loading
 * starts. What gets applied is recorded as console entries so it shows up in
 * the terminal log and the copyable debug report.
 */

export const DEFAULT_TEST_SLEEP_MS = 400;

export type LoadTestFailure = 'shader' | 'manifest' | 'state';

export interface LoadTestOptions {
    /** Per-step delay in ms; 0 disables the slow-down. */
    sleepMs: number;
    fail: LoadTestFailure | null;
}

const FAIL_URL_MAP: Record<LoadTestFailure, string> = {
    shader: '/shaders/',
    manifest: '/manifest.json',
    state: '/state.json',
};

export function parseLoadTestParams(search: string): LoadTestOptions {
    const params = new URLSearchParams(search);

    let sleepMs = 0;
    if (params.has('slow')) {
        const raw = params.get('slow');
        sleepMs =
            raw !== null && raw !== ''
                ? Math.max(0, Math.round(Number(raw)) || 0)
                : DEFAULT_TEST_SLEEP_MS;
    }

    const failParam = params.get('fail');
    const fail: LoadTestFailure | null =
        failParam === 'shader' || failParam === 'manifest' || failParam === 'state'
            ? failParam
            : null;

    return { sleepMs, fail };
}

export function applyLoadTestParams(search?: string): LoadTestOptions {
    const opts = parseLoadTestParams(search ?? window.location.search);

    if (opts.sleepMs > 0) {
        setLoadSleepMs(opts.sleepMs);
        appendConsoleEntry('info', [`Load test: slowing load to ${opts.sleepMs}ms per step`]);
    }

    if (opts.fail) {
        const pattern = FAIL_URL_MAP[opts.fail];
        const originalFetch = window.fetch.bind(window);
        window.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
            const url =
                typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
            if (url.includes(pattern)) {
                return Promise.resolve(new Response('', { status: 404, statusText: 'Not Found' }));
            }
            return originalFetch(input, init);
        }) as typeof fetch;
        appendConsoleEntry('info', [`Load test: requests matching "${pattern}" will fail (404)`]);
    }

    return opts;
}
