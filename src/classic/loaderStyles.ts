/**
 * Loading-screen styles for createLoader().
 *
 * Kept as a plain CSS string and injected into a scoped <style> element at
 * runtime (see loader.ts), so library consumers get a working loading screen
 * by just calling createLoader() — no separate stylesheet to wire up. Every
 * rule is scoped under `#loader` to avoid leaking onto consumer pages.
 */

export const LOADER_STYLE_ID = 'classic-wgl-loader-style';

export const LOADER_STYLE = `
#loader {
    --loader-bg: #1a1a1a;
    --loader-fg: #ddd;
    --loader-accent: #4caf50;
    --loader-panel: #0a0a0a;
    --loader-bevel-hi: #4a4a4a;
    --loader-bevel-lo: #000;
}

#loader.loader-overlay {
    position: fixed;
    inset: 0;
    z-index: 10000;
    display: flex;
    align-items: center;
    justify-content: center;
    box-sizing: border-box;
    background: var(--loader-bg);
    color: var(--loader-fg);
    font-family: system-ui, sans-serif;
    transition: opacity 0.5s ease;
}

#loader.loader-hidden {
    opacity: 0;
    pointer-events: none;
}

/* Top-right action buttons (copy log + mode toggle) */

#loader .loader-actions {
    position: absolute;
    top: 12px;
    right: 12px;
    display: flex;
    gap: 8px;
}

#loader .loader-actions button {
    padding: 4px 12px;
    border: 2px solid transparent;
    border-radius: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    font-size: 12px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    cursor: pointer;
}

#loader .loader-copy.copied {
    color: var(--loader-accent);
}

/* terminal: subtle translucent buttons */

#loader.loader-terminal-mode .loader-actions button {
    border-color: rgba(255, 255, 255, 0.35);
    background: rgba(255, 255, 255, 0.08);
}

#loader.loader-terminal-mode .loader-actions button:hover {
    background: rgba(255, 255, 255, 0.18);
}

/* visual: dark-mode win95 raised bevel buttons */

#loader.loader-visual-mode .loader-actions button {
    border-color: var(--loader-bevel-hi) var(--loader-bevel-lo) var(--loader-bevel-lo)
        var(--loader-bevel-hi);
    background: #2a2a2a;
    color: var(--loader-fg);
    font-weight: 700;
    letter-spacing: 0.04em;
}

#loader.loader-visual-mode .loader-actions button:hover {
    background: #333;
}

#loader.loader-visual-mode .loader-actions button:active {
    border-color: var(--loader-bevel-lo) var(--loader-bevel-hi) var(--loader-bevel-hi)
        var(--loader-bevel-lo);
    background: #1f1f1f;
}

/* Visual mode */

#loader .loader-visual {
    display: none;
    flex-direction: column;
    align-items: center;
    gap: 18px;
    max-width: 420px;
    padding: 32px;
}

#loader.loader-visual-mode .loader-visual {
    display: flex;
}

#loader.loader-visual-mode {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}

#loader .loader-logo {
    width: clamp(120px, 25vmin, 280px);
    max-width: 40vw;
    height: auto;
    image-rendering: pixelated;
    filter: drop-shadow(0 6px 18px rgba(0, 0, 0, 0.6));
}

#loader .loader-title {
    font-size: 14px;
    font-weight: 700;
    text-transform: uppercase;
    color: rgba(255, 255, 255, 0.8);
}

#loader .loader-bar {
    width: 100%;
    height: 22px;
    border: 2px solid;
    border-color: var(--loader-bevel-lo) var(--loader-bevel-hi) var(--loader-bevel-hi)
        var(--loader-bevel-lo);
    border-radius: 0;
    background: var(--loader-panel);
    box-sizing: border-box;
    padding: 2px;
    overflow: hidden;
}

#loader .loader-bar-fill {
    height: 100%;
    width: 0%;
    border-radius: 0;
    background-color: var(--loader-accent);
    background-image: repeating-linear-gradient(
        90deg,
        rgba(0, 0, 0, 0) 0 12px,
        rgba(0, 0, 0, 0.45) 12px 14px
    );
    box-shadow:
        inset 0 2px 0 rgba(255, 255, 255, 0.35),
        inset 0 -2px 0 rgba(0, 0, 0, 0.55);
    transition: width 0.15s ease;
}

#loader .loader-meta {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    width: 100%;
    font-size: 12px;
    font-weight: 700;
    color: rgba(255, 255, 255, 0.85);
}

#loader .loader-pct {
    color: var(--loader-accent);
}

#loader .loader-label-error {
    color: #ff5555;
    font-weight: 700;
}

/* Load-test hint (shown when ?slow / ?fail= query params are active) */

#loader .loader-note {
    max-width: 100%;
    box-sizing: border-box;
    font-size: 11px;
    line-height: 1.4;
    color: #e0a83a;
    text-align: center;
}

/* Console error panel (visual mode) */

#loader .loader-errors {
    display: none;
    width: 100%;
    box-sizing: border-box;
    border: 2px solid;
    border-color: var(--loader-bevel-hi) var(--loader-bevel-lo) var(--loader-bevel-lo)
        var(--loader-bevel-hi);
    background: rgba(255, 85, 85, 0.08);
    color: #ff8888;
    text-align: left;
}

#loader.loader-show-errors .loader-errors,
#loader.loader-failed .loader-errors {
    display: block;
}

#loader .loader-errors-head {
    padding: 4px 8px;
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: #ff5555;
}

#loader .loader-errors-list {
    margin: 0;
    padding: 0 8px 8px;
    max-height: 160px;
    overflow-y: auto;
    white-space: pre-wrap;
    word-break: break-word;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 11px;
    line-height: 1.5;
}

/* Failed load state: red accent, FAILED readout, keep overlay visible */

#loader.loader-failed {
    --loader-accent: #e53935;
}

#loader.loader-failed .loader-bar-fill {
    background-color: #e53935;
}

/* Terminal mode */

#loader .loader-terminal {
    display: none;
    flex: 1 1 auto;
    align-self: stretch;
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
    box-sizing: border-box;
    flex-direction: column;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}

#loader.loader-terminal-mode .loader-terminal {
    display: flex;
}

#loader.loader-terminal-mode {
    --loader-bg: #000;
    --loader-fg: #33ff33;
    --loader-accent: #33ff33;
    align-items: stretch;
    justify-content: flex-start;
    padding: 24px;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}

#loader .loader-log {
    flex: 1;
    overflow-y: auto;
    font-size: 13px;
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-word;
}

#loader .loader-log-error {
    color: #ff5555;
}

#loader .loader-terminal-progress {
    margin-top: 12px;
    font-size: 13px;
    white-space: pre;
}
`;
