import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import {
    isLoaderMode,
    readLoaderMode,
    writeLoaderMode,
    createLoader,
    LOADER_STORAGE_KEY,
    STALL_TIMEOUT_MS,
} from '/classic/loader.js';
import { clearConsoleLog, getConsoleLog } from '/classic/consoleLog.js';

describe('isLoaderMode', () => {
    it('accepts the two known modes', () => {
        expect(isLoaderMode('visual')).toBe(true);
        expect(isLoaderMode('terminal')).toBe(true);
    });

    it('rejects unknown values', () => {
        expect(isLoaderMode('dev')).toBe(false);
        expect(isLoaderMode(null)).toBe(false);
        expect(isLoaderMode(undefined)).toBe(false);
    });
});

describe('readLoaderMode / writeLoaderMode', () => {
    beforeEach(() => {
        window.localStorage.clear();
    });

    it('defaults to visual when nothing is stored', () => {
        expect(readLoaderMode()).toBe('visual');
    });

    it('round-trips a stored mode', () => {
        writeLoaderMode('terminal');
        expect(readLoaderMode()).toBe('terminal');

        writeLoaderMode('visual');
        expect(readLoaderMode()).toBe('visual');
    });

    it('falls back to visual for invalid stored values', () => {
        window.localStorage.setItem(LOADER_STORAGE_KEY, 'dev');
        expect(readLoaderMode()).toBe('visual');
    });
});

describe('createLoader', () => {
    let root: HTMLElement;

    beforeEach(() => {
        document.getElementById('loader')?.remove();
        root = document.createElement('div');
        root.id = 'loader';
        document.body.appendChild(root);
    });

    afterEach(() => {
        vi.useRealTimers();
        vi.unstubAllGlobals();
    });

    it('builds the overlay around the #loader element', () => {
        const loader = createLoader();
        expect(root.classList.contains('loader-overlay')).toBe(true);
        expect(root.querySelector('.loader-toggle')).not.toBeNull();
        expect(root.querySelector('.loader-copy')).not.toBeNull();
        expect(root.querySelector('.loader-bar-fill')).not.toBeNull();
        expect(root.querySelector('.loader-log')).not.toBeNull();
        expect(root.querySelector('.loader-errors')).not.toBeNull();
        loader.finish();
    });

    it('reflects progress in the bar fill', () => {
        const loader = createLoader();
        const fill = root.querySelector('.loader-bar-fill') as HTMLElement;
        loader.setProgress('Loading texture: t', 0.5);
        expect(fill.style.width).toBe('50%');
        loader.finish();
    });

    it('surfaces errors in the label', () => {
        const loader = createLoader();
        const label = root.querySelector('.loader-label') as HTMLElement;
        loader.error('Failed to create shader');
        expect(label.classList.contains('loader-label-error')).toBe(true);
        expect(label.textContent).toBe('Failed to create shader');
        loader.finish();
    });

    it('fail() shows the failed state, records the message and keeps the overlay', () => {
        const loader = createLoader();
        clearConsoleLog();
        loader.fail('Failed to load manifest.json');

        expect(root.classList.contains('loader-failed')).toBe(true);
        expect(root.classList.contains('loader-hidden')).toBe(false);
        expect(root.classList.contains('loader-show-errors')).toBe(true);

        const label = root.querySelector('.loader-label') as HTMLElement;
        expect(label.textContent).toBe('Failed to load manifest.json');
        expect(getConsoleLog()).toContain('Failed to load manifest.json');

        const pct = root.querySelector('.loader-pct') as HTMLElement;
        expect(pct.textContent).toBe('FAILED');
        loader.finish();
    });

    it('emits a stall warning after the timeout without progress', () => {
        vi.useFakeTimers();
        const loader = createLoader();
        clearConsoleLog();
        loader.setProgress('Fetching manifest', 0);

        vi.advanceTimersByTime(STALL_TIMEOUT_MS + 1000);

        expect(getConsoleLog()).toContain('stalled');
        expect(getConsoleLog()).toContain('Fetching manifest');
        loader.finish();
    });

    it('finish() with exit: "instant" removes the overlay immediately', () => {
        const loader = createLoader({ exit: 'instant' });
        expect(root.isConnected).toBe(true);
        loader.finish();
        expect(root.isConnected).toBe(false);
    });

    it('finish() with the default fade exit hides the overlay but keeps it until the transition ends', () => {
        const loader = createLoader();
        loader.finish();
        expect(root.isConnected).toBe(true);
        expect(root.classList.contains('loader-hidden')).toBe(true);
    });

    it('renders a load-test note', () => {
        const loader = createLoader();
        const note = root.querySelector('.loader-note') as HTMLElement;
        expect(note).not.toBeNull();
        loader.note('load test: slow 400ms/step / fail=shader');
        expect(note.textContent).toBe('load test: slow 400ms/step / fail=shader');
        expect(note.hidden).toBe(false);
        loader.note('');
        expect(note.hidden).toBe(true);
        loader.finish();
    });

    it('copy button copies the debug report to the clipboard', async () => {
        const writeText = vi.fn().mockResolvedValue(undefined);
        Object.defineProperty(globalThis, 'navigator', {
            value: { ...navigator, clipboard: { writeText } },
            configurable: true,
        });

        const loader = createLoader();
        clearConsoleLog();
        loader.setProgress('Loading textures', 0.5);

        const copyBtn = root.querySelector('.loader-copy') as HTMLButtonElement;
        expect(copyBtn).not.toBeNull();
        copyBtn.click();

        await vi.waitFor(() => expect(writeText).toHaveBeenCalled());
        const text = writeText.mock.calls[0][0] as string;
        expect(text).toContain('classic-wgl');
        expect(text).toContain('console log');
        expect(text).toContain('Loading textures');
        loader.finish();
    });
});
