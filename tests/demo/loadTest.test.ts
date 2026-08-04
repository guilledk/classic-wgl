import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { parseLoadTestParams, applyLoadTestParams, DEFAULT_TEST_SLEEP_MS } from '/demo/loadTest.js';
import { startConsoleCapture, clearConsoleLog, getConsoleLog } from '/classic/consoleLog.js';
import { getLoadSleepMs, setLoadSleepMs } from '/classic/utils.js';

startConsoleCapture();

describe('parseLoadTestParams', () => {
    it('returns disabled options for an empty query', () => {
        expect(parseLoadTestParams('')).toEqual({ sleepMs: 0, fail: null });
    });

    it('enables the default slow-down with ?slow', () => {
        expect(parseLoadTestParams('?slow')).toEqual({
            sleepMs: DEFAULT_TEST_SLEEP_MS,
            fail: null,
        });
    });

    it('honours a custom delay with ?slow=<ms>', () => {
        expect(parseLoadTestParams('?slow=250')).toEqual({ sleepMs: 250, fail: null });
        expect(parseLoadTestParams('?slow=0')).toEqual({ sleepMs: 0, fail: null });
        expect(parseLoadTestParams('?slow=abc')).toEqual({ sleepMs: 0, fail: null });
    });

    it('parses the fail mode', () => {
        expect(parseLoadTestParams('?fail=shader')).toEqual({ sleepMs: 0, fail: 'shader' });
        expect(parseLoadTestParams('?fail=manifest')).toEqual({ sleepMs: 0, fail: 'manifest' });
        expect(parseLoadTestParams('?fail=state')).toEqual({ sleepMs: 0, fail: 'state' });
        expect(parseLoadTestParams('?fail=bogus')).toEqual({ sleepMs: 0, fail: null });
    });

    it('combines slow and fail', () => {
        expect(parseLoadTestParams('?slow=150&fail=state')).toEqual({
            sleepMs: 150,
            fail: 'state',
        });
    });
});

describe('applyLoadTestParams', () => {
    let realFetch: typeof fetch | undefined;

    beforeEach(() => {
        clearConsoleLog();
        setLoadSleepMs(0);
        realFetch = window.fetch;
        vi.unstubAllGlobals();
    });

    afterEach(() => {
        window.fetch = realFetch as typeof fetch;
        vi.unstubAllGlobals();
    });

    it('turns on the slow-down and logs what it applied', () => {
        const opts = applyLoadTestParams('?slow=200');
        expect(opts).toEqual({ sleepMs: 200, fail: null });
        expect(getLoadSleepMs()).toBe(200);
        expect(getConsoleLog()).toContain('Load test: slowing load to 200ms per step');
    });

    it('patches fetch to 404 matching URLs when ?fail= is present', async () => {
        const original = vi.fn().mockResolvedValue(new Response('ok', { status: 200 }));
        vi.stubGlobal('fetch', original);

        const opts = applyLoadTestParams('?fail=shader');
        expect(opts.fail).toBe('shader');

        const failed = await window.fetch('/shaders/solid.vert');
        expect(failed.status).toBe(404);
        expect(original).not.toHaveBeenCalled();

        const passed = await window.fetch('/manifest.json');
        expect(passed.status).toBe(200);
        expect(original).toHaveBeenCalledWith('/manifest.json', undefined);
        expect(getConsoleLog()).toContain('requests matching "/shaders/"');
    });

    it('leaves unrelated requests untouched', async () => {
        const original = vi.fn().mockResolvedValue(new Response('ok', { status: 200 }));
        vi.stubGlobal('fetch', original);

        applyLoadTestParams('?fail=manifest');

        const res = await window.fetch('/res/cool_snek.png');
        expect(res.status).toBe(200);
        expect(original).toHaveBeenCalledTimes(1);
    });
});
