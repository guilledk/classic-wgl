import { setLoaderSink } from './utils.js';
import {
    startConsoleCapture,
    subscribeConsole,
    appendConsoleEntry,
    buildDebugReport,
    copyTextToClipboard,
    formatConsoleEntry,
    getConsoleErrors,
} from './consoleLog.js';
import type { ConsoleEntry } from './consoleLog.js';
import { APP_NAME, APP_VERSION_DISPLAY } from '../version.js';
import { LOADER_STYLE, LOADER_STYLE_ID } from './loaderStyles.js';

/**
 * Loading screen overlay for the classic-wgl demo, packaged as a small
 * code-defined API so other pages can drop it in with one call:
 *
 *     const loader = createLoader({ exit: 'instant' });
 *     loader.setProgress('Loading assets', 0.25);
 *     // ...
 *     loader.finish();
 *
 * Styles are injected into the document at runtime (see loaderStyles.ts) and
 * the loader's own DOM is cleaned up on finish(), so consumers don't need to
 * wire up a stylesheet or mount element.
 *
 * Features:
 *   - Two render modes, switchable with a corner button (visual: dark gray,
 *     logo + weighted progress bar; terminal: black, green log lines). The
 *     mode toggle persists to localStorage only when `persistMode` is set.
 *   - Browser console messages are captured (see consoleLog.ts) and streamed
 *     into the terminal log; errors/warnings surface in visual mode as a red
 *     panel, with a "copy log" button that pastes a formatted debug report.
 *   - A stall watchdog appends a warning when no progress arrives for
 *     `stallTimeoutMs`.
 *   - On finish() the overlay either fades out (`exit: 'fade'`) or is removed
 *     immediately (`exit: 'instant'`).
 *
 * The engine is decoupled from this overlay through the loader sink in
 * src/classic/utils.ts (setLoaderSink/deleteLoaderLabel), which routes shader
 * error messages and launch cleanup here when a loader is active.
 */

export type LoaderMode = 'visual' | 'terminal';

export type LoaderExit = 'fade' | 'instant';

export interface LoaderOptions {
    /** Initial render mode. Defaults to 'visual'. */
    mode?: LoaderMode;
    /** How the overlay goes away on finish(). Defaults to 'fade'. */
    exit?: LoaderExit;
    /** Mount point for the overlay. Defaults to #loader, else a new div. */
    target?: HTMLElement;
    /** Heading text (visual mode). Defaults to `${APP_NAME} ${APP_VERSION_DISPLAY}`. */
    title?: string;
    /** Logo image URL (visual mode). Omitted (no logo) when not provided. */
    logoUrl?: string;
    /** Stall watchdog threshold in ms. Defaults to STALL_TIMEOUT_MS. */
    stallTimeoutMs?: number;
    /** Show the "copy log" button. Defaults to true. */
    copyLog?: boolean;
    /** Persist the mode toggle to localStorage. Defaults to false. */
    persistMode?: boolean;
}

export const LOADER_STORAGE_KEY = 'classic-wgl:loader-mode';

/** No load progress for this long (ms) triggers a stall warning entry. */
export const STALL_TIMEOUT_MS = 20000;

export function isLoaderMode(value: string | null | undefined): value is LoaderMode {
    return value === 'visual' || value === 'terminal';
}

function getStorage(): Pick<Storage, 'getItem' | 'setItem'> | null {
    try {
        return window.localStorage;
    } catch {
        return null;
    }
}

export function readLoaderMode(): LoaderMode {
    try {
        const value = getStorage()?.getItem(LOADER_STORAGE_KEY);
        return isLoaderMode(value) ? value : 'visual';
    } catch {
        return 'visual';
    }
}

export function writeLoaderMode(mode: LoaderMode): void {
    try {
        getStorage()?.setItem(LOADER_STORAGE_KEY, mode);
    } catch {
        // storage unavailable (private mode etc.) - ignore
    }
}

export interface LoaderController {
    setProgress(label: string, fraction: number): void;
    log(line: string): void;
    error(message: string): void;
    fail(message: string): void;
    note(message: string): void;
    finish(): void;
}

function clamp01(value: number): number {
    return Math.min(1, Math.max(0, value));
}

function el<K extends keyof HTMLElementTagNameMap>(
    tag: K,
    className?: string,
): HTMLElementTagNameMap[K] {
    const node = document.createElement(tag);
    if (className) {
        node.className = className;
    }
    return node;
}

function injectLoaderStyle(): HTMLStyleElement {
    const existing = document.getElementById(LOADER_STYLE_ID);
    if (existing) {
        return existing as HTMLStyleElement;
    }
    const style = document.createElement('style');
    style.id = LOADER_STYLE_ID;
    style.textContent = LOADER_STYLE;
    document.head.appendChild(style);
    return style;
}

class Loader implements LoaderController {
    private _mode: LoaderMode;
    private _exit: LoaderExit;
    private _persistMode: boolean;
    private _stallTimeoutMs: number;
    private _style: HTMLStyleElement;
    private _root: HTMLElement;
    private _copy!: HTMLButtonElement;
    private _toggle: HTMLButtonElement;
    private _barFill: HTMLElement;
    private _pct: HTMLElement;
    private _label: HTMLElement;
    private _note: HTMLElement;
    private _log: HTMLElement;
    private _progress: HTMLElement;
    private _errors: HTMLElement;
    private _errorsList: HTMLElement;
    private _fraction = 0;
    private _lastLoggedLabel = '';
    private _finished = false;
    private _status: 'loading' | 'failed' = 'loading';
    private _startedAt = Date.now();
    private _lastProgressAt = Date.now();
    private _stallReportedAt = Date.now();
    private _stallTimer: number | null = null;
    private _unsubscribeConsole: (() => void) | null = null;

    constructor(options: LoaderOptions = {}) {
        this._persistMode = options.persistMode ?? false;
        this._exit = options.exit ?? 'fade';
        this._stallTimeoutMs = options.stallTimeoutMs ?? STALL_TIMEOUT_MS;
        this._mode = this._resolveInitialMode(options);

        this._style = injectLoaderStyle();

        this._root =
            options.target ??
            document.getElementById('loader') ??
            (() => {
                const div = el('div');
                div.id = 'loader';
                document.body.appendChild(div);
                return div;
            })();

        this._root.className = 'loader-overlay';
        this._root.innerHTML = '';

        const actions = el('div', 'loader-actions');

        const copyLog = options.copyLog ?? true;
        if (copyLog) {
            this._copy = el('button', 'loader-copy');
            this._copy.type = 'button';
            this._copy.textContent = 'copy log';
            this._copy.addEventListener('click', this._copyLog.bind(this));
            actions.appendChild(this._copy);
        }

        this._toggle = el('button', 'loader-toggle');
        this._toggle.type = 'button';
        this._toggle.addEventListener('click', this._toggleMode.bind(this));

        actions.appendChild(this._toggle);

        const visual = el('div', 'loader-visual');

        if (options.logoUrl) {
            const logo = el('img', 'loader-logo');
            logo.src = options.logoUrl;
            logo.alt = APP_NAME;
            logo.addEventListener('error', () => {
                logo.style.display = 'none';
            });
            visual.appendChild(logo);
        }

        const title = el('div', 'loader-title');
        title.textContent = options.title ?? `${APP_NAME} ${APP_VERSION_DISPLAY}`;

        const bar = el('div', 'loader-bar');
        this._barFill = el('div', 'loader-bar-fill');
        bar.appendChild(this._barFill);

        const meta = el('div', 'loader-meta');
        this._pct = el('span', 'loader-pct');
        this._label = el('span', 'loader-label');
        meta.append(this._pct, this._label);

        const note = el('div', 'loader-note');
        this._note = note;

        const errors = el('div', 'loader-errors');
        const errorsHead = el('div', 'loader-errors-head');
        errorsHead.textContent = 'console errors';
        this._errorsList = el('pre', 'loader-errors-list');
        errors.append(errorsHead, this._errorsList);
        this._errors = errors;

        visual.append(title, bar, meta, note, errors);

        const terminal = el('div', 'loader-terminal');
        this._log = el('div', 'loader-log');
        this._progress = el('div', 'loader-terminal-progress');
        terminal.append(this._log, this._progress);

        this._root.append(actions, visual, terminal);

        this._updateMode();

        startConsoleCapture();
        this._unsubscribeConsole = subscribeConsole((entry) => this._onConsoleEntry(entry));
        this._stallTimer = window.setInterval(() => this._checkStall(), 1000);

        setLoaderSink(this);
    }

    private _resolveInitialMode(options: LoaderOptions): LoaderMode {
        if (options.persistMode) {
            const stored = getStorage()?.getItem(LOADER_STORAGE_KEY);
            if (isLoaderMode(stored)) {
                return stored;
            }
        }
        return options.mode ?? 'visual';
    }

    setProgress(label: string, fraction: number): void {
        this._lastProgressAt = Date.now();
        this._fraction = clamp01(fraction);
        const pct = Math.round(this._fraction * 100);

        this._label.textContent = label;
        this._label.classList.remove('loader-label-error');
        this._pct.textContent = `${pct}%`;
        this._barFill.style.width = `${this._fraction * 100}%`;

        if (label !== this._lastLoggedLabel) {
            this._lastLoggedLabel = label;
            this._logLine(`> ${label}`);
        }
        this._renderProgress();
    }

    log(line: string): void {
        this._logLine(line);
    }

    error(message: string): void {
        this._lastProgressAt = Date.now();
        this._label.textContent = message;
        this._label.classList.add('loader-label-error');
        this._pct.textContent = '!';
        appendConsoleEntry('error', [message]);
    }

    note(message: string): void {
        this._note.textContent = message;
        this._note.hidden = message === '';
    }

    fail(message: string): void {
        if (this._status === 'failed') {
            return;
        }
        this._status = 'failed';
        this._stopStallTimer();
        this._lastProgressAt = Date.now();

        appendConsoleEntry('error', [message]);

        this._root.classList.add('loader-failed');
        this._label.textContent = message;
        this._label.classList.add('loader-label-error');
        this._pct.textContent = 'FAILED';
        this._barFill.style.width = '100%';
        this._showErrors();
    }

    finish(): void {
        if (this._finished) {
            return;
        }
        this._finished = true;
        this._stopStallTimer();
        if (this._unsubscribeConsole) {
            this._unsubscribeConsole();
            this._unsubscribeConsole = null;
        }
        this.setProgress('Ready', 1);
        setLoaderSink(null);
        if (this._exit === 'instant') {
            this._destroy();
        } else {
            this._root.classList.add('loader-hidden');
            window.setTimeout(() => this._destroy(), 600);
        }
    }

    private _destroy(): void {
        this._style.remove();
        this._root.remove();
    }

    private _toggleMode(): void {
        this._mode = this._mode === 'visual' ? 'terminal' : 'visual';
        if (this._persistMode) {
            writeLoaderMode(this._mode);
        }
        this._updateMode();
    }

    private _updateMode(): void {
        this._root.classList.toggle('loader-visual-mode', this._mode === 'visual');
        this._root.classList.toggle('loader-terminal-mode', this._mode === 'terminal');
        this._toggle.textContent = this._mode === 'visual' ? 'terminal' : 'visual';
    }

    private _logLine(text: string, cls?: string): void {
        const line = el('div', 'loader-log-line');
        if (cls) {
            line.classList.add(cls);
        }
        line.textContent = text;
        this._log.appendChild(line);
        this._log.scrollTop = this._log.scrollHeight;
    }

    private _renderProgress(): void {
        const width = 24;
        const filled = Math.round(this._fraction * width);
        const bar = '#'.repeat(filled) + '-'.repeat(width - filled);
        const pct = Math.round(this._fraction * 100);
        this._progress.textContent = `[${bar}] ${pct}% ${this._lastLoggedLabel}`;
    }

    private _onConsoleEntry(entry: ConsoleEntry): void {
        this._logLine(formatConsoleEntry(entry));
        if (entry.level === 'error' || entry.level === 'warn') {
            this._showErrors();
        } else {
            this._renderErrors();
        }
    }

    private _showErrors(): void {
        this._root.classList.add('loader-show-errors');
        this._renderErrors();
    }

    private _renderErrors(): void {
        const lines = getConsoleErrors();
        this._errorsList.textContent = lines.length ? lines.join('\n') : '(no errors)';
    }

    private _checkStall(): void {
        if (this._finished || this._status === 'failed') {
            return;
        }
        const idleMs = Date.now() - this._lastProgressAt;
        if (
            idleMs >= this._stallTimeoutMs &&
            Date.now() - this._stallReportedAt >= this._stallTimeoutMs
        ) {
            this._stallReportedAt = Date.now();
            appendConsoleEntry('warn', [
                `Loading stalled — no progress for ${this._stallTimeoutMs / 1000}s. ` +
                    `Current step: "${this._lastLoggedLabel || 'n/a'}", progress ${Math.round(
                        this._fraction * 100,
                    )}%.`,
            ]);
        }
    }

    private async _copyLog(): Promise<void> {
        const report = buildDebugReport({
            status: this._status,
            step: this._lastLoggedLabel,
            progress: this._fraction,
            elapsedMs: Date.now() - this._startedAt,
        });
        const ok = await copyTextToClipboard(report);
        if (ok) {
            this._copy.classList.add('copied');
            this._copy.textContent = 'copied!';
            window.setTimeout(() => {
                this._copy.classList.remove('copied');
                this._copy.textContent = 'copy log';
            }, 1500);
        }
    }

    private _stopStallTimer(): void {
        if (this._stallTimer !== null) {
            window.clearInterval(this._stallTimer);
            this._stallTimer = null;
        }
    }
}

export function createLoader(options?: LoaderOptions): LoaderController {
    return new Loader(options);
}
