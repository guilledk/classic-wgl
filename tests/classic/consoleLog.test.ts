import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import {
    startConsoleCapture,
    clearConsoleLog,
    getConsoleLog,
    getConsoleErrors,
    appendConsoleEntry,
    subscribeConsole,
    formatConsoleArg,
    buildDebugReport,
    copyTextToClipboard,
} from '/classic/consoleLog.js';
import type { ConsoleEntry } from '/classic/consoleLog.js';

startConsoleCapture();

const originalNavigator = window.navigator;

function stubNavigator(overrides: Record<string, unknown>): void {
    const stub = { ...originalNavigator, ...overrides } as unknown as Navigator;
    Object.defineProperty(globalThis, 'navigator', { value: stub, configurable: true });
}

function restoreNavigator(): void {
    Object.defineProperty(globalThis, 'navigator', {
        value: originalNavigator,
        configurable: true,
    });
}

afterEach(() => {
    restoreNavigator();
    vi.restoreAllMocks();
});

describe('startConsoleCapture', () => {
    beforeEach(() => {
        clearConsoleLog();
    });

    it('records console messages with their level', () => {
        console.error('boom');
        console.log('hi');
        const log = getConsoleLog();
        expect(log).toContain('[error] boom');
        expect(log).toContain('[log] hi');
    });

    it('is idempotent - a single call records a single entry', () => {
        startConsoleCapture();
        startConsoleCapture();
        console.log('once');
        const matches = getConsoleLog().match(/once/g);
        expect(matches).toHaveLength(1);
    });
});

describe('formatConsoleArg', () => {
    it('renders primitives', () => {
        expect(formatConsoleArg(null)).toBe('null');
        expect(formatConsoleArg(undefined)).toBe('undefined');
        expect(formatConsoleArg(42)).toBe('42');
        expect(formatConsoleArg('str')).toBe('str');
        expect(formatConsoleArg(true)).toBe('true');
    });

    it('stringifies objects without crashing on circular refs', () => {
        const obj: Record<string, unknown> = { name: 'circle' };
        obj.self = obj;
        const out = formatConsoleArg(obj);
        expect(out).toContain('circle');
        expect(out).toContain('[Circular]');
    });

    it('uses the error stack when available', () => {
        const err = new Error('nope');
        const out = formatConsoleArg(err);
        expect(out).toContain('nope');
    });
});

describe('appendConsoleEntry', () => {
    it('records an entry directly', () => {
        clearConsoleLog();
        appendConsoleEntry('error', ['manual message']);
        expect(getConsoleLog()).toContain('[error] manual message');
    });
});

describe('subscribeConsole', () => {
    it('notifies subscribers with new entries and stops on unsubscribe', () => {
        const received: ConsoleEntry[] = [];
        const unsubscribe = subscribeConsole((entry) => received.push(entry));
        clearConsoleLog();

        appendConsoleEntry('warn', ['hello sub']);
        expect(received).toHaveLength(1);
        expect(received[0].text).toContain('hello sub');

        unsubscribe();
        appendConsoleEntry('error', ['after unsubscribe']);
        expect(received).toHaveLength(1);
    });
});

describe('getConsoleErrors', () => {
    it('filters to error and warn entries', () => {
        clearConsoleLog();
        appendConsoleEntry('log', ['ignored log line']);
        appendConsoleEntry('error', ['b']);
        appendConsoleEntry('warn', ['c']);
        const errors = getConsoleErrors().join('\n');
        expect(errors).toContain('b');
        expect(errors).toContain('c');
        expect(errors).not.toContain('ignored log line');
    });
});

describe('buildDebugReport', () => {
    it('includes the header, load meta and console log', () => {
        clearConsoleLog();
        appendConsoleEntry('log', ['line1']);

        const report = buildDebugReport({
            status: 'failed',
            step: 'Loading textures',
            progress: 0.4,
            elapsedMs: 5000,
        });

        expect(report).toContain('classic-wgl');
        expect(report).toContain('Load status: failed');
        expect(report).toContain('Current step: Loading textures');
        expect(report).toContain('40%');
        expect(report).toContain('5.0s');
        expect(report).toContain('line1');
    });
});

describe('copyTextToClipboard', () => {
    it('uses the async clipboard API when available', async () => {
        const writeText = vi.fn().mockResolvedValue(undefined);
        stubNavigator({ clipboard: { writeText } });

        await expect(copyTextToClipboard('payload')).resolves.toBe(true);
        expect(writeText).toHaveBeenCalledWith('payload');
    });

    it('falls back to execCommand when the clipboard API is unavailable', async () => {
        stubNavigator({});
        Object.defineProperty(document, 'execCommand', {
            value: () => true,
            configurable: true,
        });
        const execCommand = vi.spyOn(document, 'execCommand').mockReturnValue(true);

        await expect(copyTextToClipboard('payload')).resolves.toBe(true);
        expect(execCommand).toHaveBeenCalled();
    });
});
