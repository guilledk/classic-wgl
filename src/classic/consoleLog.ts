import { APP_NAME, APP_VERSION_DISPLAY } from '../version.js';

/**
 * Console message capture for the loading screen (src/classic/loader.ts).
 *
 * Wraps the browser console so every message logged while the loader is on
 * screen (engine init, shader/asset fetches, errors, unhandled rejections)
 * is recorded in an in-memory ring buffer. The loader renders the captured
 * lines and offers a "copy log" button that pastes a formatted debug report
 * for sending to developers.
 */

export type ConsoleLevel = 'log' | 'info' | 'warn' | 'error' | 'debug' | 'trace';

export interface ConsoleEntry {
    time: number;
    level: ConsoleLevel;
    text: string;
}

const MAX_ENTRIES = 500;

const CAPTURED_LEVELS: ConsoleLevel[] = ['log', 'info', 'warn', 'error', 'debug', 'trace'];

let capturing = false;
let entries: ConsoleEntry[] = [];
const listeners = new Set<(entry: ConsoleEntry) => void>();
const originals: Partial<Record<ConsoleLevel, (...args: unknown[]) => void>> = {};

function push(level: ConsoleLevel, args: unknown[]): void {
    const entry = { time: Date.now(), level, text: formatConsoleArgs(args) };
    entries.push(entry);
    if (entries.length > MAX_ENTRIES) {
        entries.shift();
    }
    for (const listener of listeners) {
        listener(entry);
    }
}

// ============================================================================
// Capture
// ============================================================================

export function startConsoleCapture(): void {
    if (capturing) {
        return;
    }
    capturing = true;

    for (const level of CAPTURED_LEVELS) {
        const original = console[level].bind(console);
        originals[level] = original;
        const wrapped: (...args: unknown[]) => void = (...args) => {
            original(...args);
            push(level, args);
        };
        console[level] = wrapped;
    }

    const originalAssert = console.assert.bind(console);
    const wrappedAssert: (condition?: boolean, ...data: unknown[]) => void = (
        condition,
        ...data
    ) => {
        originalAssert(condition, ...data);
        if (!condition) {
            push('error', data.length ? data : ['Assertion failed']);
        }
    };
    console.assert = wrappedAssert;

    window.addEventListener('error', onUncaughtError);
    window.addEventListener('unhandledrejection', onUnhandledRejection);
}

function onUncaughtError(event: ErrorEvent): void {
    push('error', [
        `Uncaught ${event.message} at ${event.filename}:${event.lineno}:${event.colno}`,
    ]);
}

function onUnhandledRejection(event: PromiseRejectionEvent): void {
    push('error', ['Unhandled promise rejection:', event.reason]);
}

/**
 * Records an entry (and forwards it to the real console) without relying on
 * the wrapped console methods. Used by the loader for messages it generates
 * itself, so they always end up in the copyable log.
 */
export function appendConsoleEntry(level: ConsoleLevel, args: unknown[]): void {
    const original = originals[level];
    if (original) {
        original(...args);
    } else if (!capturing) {
        console[level](...args);
    }
    push(level, args);
}

export function subscribeConsole(listener: (entry: ConsoleEntry) => void): () => void {
    listeners.add(listener);
    return () => {
        listeners.delete(listener);
    };
}

export function clearConsoleLog(): void {
    entries = [];
}

// ============================================================================
// Formatting
// ============================================================================

function jsonStringify(value: unknown): string | undefined {
    const seen = new Set<unknown>();
    try {
        return JSON.stringify(value, (_key, v) => {
            if (typeof v === 'object' && v !== null) {
                if (seen.has(v)) {
                    return '[Circular]';
                }
                seen.add(v);
            }
            return v;
        });
    } catch {
        return undefined;
    }
}

export function formatConsoleArg(arg: unknown): string {
    if (arg === null) {
        return 'null';
    }
    if (arg === undefined) {
        return 'undefined';
    }
    if (typeof arg === 'string') {
        return arg;
    }
    if (typeof arg === 'number' || typeof arg === 'boolean' || typeof arg === 'bigint') {
        return String(arg);
    }
    if (arg instanceof Error) {
        return arg.stack ?? `${arg.name}: ${arg.message}`;
    }
    if (typeof (arg as { tagName?: unknown }).tagName === 'string') {
        return String(arg);
    }
    const json = jsonStringify(arg);
    return json ?? String(arg);
}

export function formatConsoleArgs(args: unknown[]): string {
    return args.map(formatConsoleArg).join(' ');
}

function pad(value: number, width = 2): string {
    return String(value).padStart(width, '0');
}

function formatTime(time: number): string {
    const d = new Date(time);
    return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(
        d.getMilliseconds(),
        3,
    )}`;
}

export function formatConsoleEntry(entry: ConsoleEntry): string {
    return `[${formatTime(entry.time)}] [${entry.level}] ${entry.text}`;
}

// ============================================================================
// Reading / copying
// ============================================================================

export function getConsoleLog(): string {
    return entries.map(formatConsoleEntry).join('\n');
}

/** Formatted error/warn lines, most recent last, for the visual error panel. */
export function getConsoleErrors(max = 8): string[] {
    const errors = entries.filter((e) => e.level === 'error' || e.level === 'warn');
    return errors.slice(-max).map(formatConsoleEntry);
}

export interface DebugReportMeta {
    status: string;
    step: string;
    progress: number;
    elapsedMs: number;
}

export function buildDebugReport(meta?: DebugReportMeta): string {
    const lines: string[] = [];
    lines.push(`${APP_NAME} ${APP_VERSION_DISPLAY}`);
    lines.push(`Captured at: ${new Date().toISOString()}`);
    if (meta) {
        lines.push(`Load status: ${meta.status}`);
        lines.push(`Current step: ${meta.step}`);
        lines.push(`Progress: ${Math.round(meta.progress * 100)}%`);
        lines.push(`Elapsed: ${(meta.elapsedMs / 1000).toFixed(1)}s`);
    }
    lines.push('');
    lines.push('--- console log ---');
    lines.push(getConsoleLog() || '(no console output captured)');
    return lines.join('\n');
}

export async function copyTextToClipboard(text: string): Promise<boolean> {
    if (navigator.clipboard?.writeText) {
        try {
            await navigator.clipboard.writeText(text);
            return true;
        } catch {
            // fall through to the legacy path below
        }
    }

    const textarea = document.createElement('textarea');
    textarea.value = text;
    textarea.style.position = 'fixed';
    textarea.style.opacity = '0';
    document.body.appendChild(textarea);
    textarea.select();
    let ok = false;
    try {
        ok = document.execCommand('copy');
    } catch {
        ok = false;
    }
    textarea.remove();
    return ok;
}
