#!/usr/bin/env node
/**
 * Generates a signed-distance-field (SDF) font atlas for pure-GL text rendering.
 *
 * Usage:  node scripts/make-font-atlas.mjs <font.ttf> [options]
 *
 * Options:
 *   --family <name>   Font family name (default: font file basename)
 *   --ss <n>          Supersampling factor (default: 12)
 *   --spread <n>      SDF spread in cell pixels (default: 4)
 *   --max-size <n>    Maximum atlas width/height (default: 4096)
 *   --no-cache        Skip the content-hash cache
 *
 * Produces:
 *   public/res/<basename>-sdf.png   – grayscale SDF atlas texture
 *   public/res/<basename>-sdf.json  – glyph metrics + atlas positions
 */

import canvasModule from '@napi-rs/canvas';
const { GlobalFonts, createCanvas, ImageData } = canvasModule;
import { createHash } from 'node:crypto';
import { mkdir, readFile, stat, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const resDir = path.join(root, 'public', 'res');

const GLYPH_SIZE = 64;
const PAD = 2;
// Font cell-pixel size matching the engine's unit system:
// cell px = (GLYPH_SIZE * 1.2 * old_RENDER_SCALE) / old_CELL_SCALE = 64 * 0.4
const FONT_CELL_SIZE = GLYPH_SIZE * 0.4;

// ---------------------------------------------------------------------------
// Charset groups
// ---------------------------------------------------------------------------

function charRange(a, b) {
    const o = [];
    for (let cp = a; cp <= b; cp++) o.push(String.fromCodePoint(cp));
    return o;
}

const CHARSET_GROUPS = {
    ascii: charRange(0x0020, 0x007e),
    latin1: charRange(0x00a0, 0x00ff),
    punct: [
        ...'\u2010\u2011\u2012\u2013\u2014\u2015\u2018\u2019\u201a\u201b\u201c\u201d\u201e',
        ...'\u2020\u2021\u2022\u2023\u2026\u2030\u2032\u2033\u2039\u203a\u203b\u203c\u2044',
    ].join(''),
    supsub: [
        ...'\u2070\u2074\u2075\u2076\u2077\u2078\u2079\u207a\u207b\u207c\u207d\u207e\u207f',
        ...'\u2080\u2081\u2082\u2083\u2084\u2085\u2086\u2087\u2088\u2089\u208a\u208b\u208c\u208d\u208e',
    ].join(''),
    fractions: charRange(0x2150, 0x215f),
    currency: charRange(0x20a0, 0x20b5).concat(['\u20b9', '\u20bd', '\u0192']),
    roman: charRange(0x2160, 0x217f),
    arrows: charRange(0x2190, 0x21ff),
    math: [
        ...'\u2200\u2202\u2203\u2205\u2206\u2207\u2208\u2209\u220f\u2211\u2212\u2213\u2215',
        ...'\u2217\u2219\u221a\u221d\u221e\u221f\u2220\u2229\u222a\u222b\u2248\u2260\u2261',
        ...'\u2264\u2265\u226a\u226b\u2282\u2283\u2295\u2297\u22a5\u22c5',
    ].join(''),
    box: charRange(0x2500, 0x257f),
    blocks: charRange(0x2580, 0x2590).concat(charRange(0x2594, 0x259f)), // exclude ░▒▓ (dither, incompatible with SDF)
    geometric: charRange(0x25a0, 0x25ff),
    symbols: [
        ...'\u2600\u2601\u2602\u2603\u2604\u2605\u2606\u2609\u260e\u2610\u2611\u2612\u2618',
        ...'\u261b\u261e\u2620\u2622\u2623\u262f\u2639\u263a\u263c\u2640\u2642',
        ...'\u2648\u2649\u264a\u264b\u264c\u264d\u264e\u264f\u2650\u2651\u2652\u2653',
        ...'\u2654\u2655\u2656\u2657\u2658\u2659\u265a\u265b\u265c\u265d\u265e\u265f',
        ...'\u2660\u2661\u2662\u2663\u2664\u2665\u2666\u2667\u2668\u2669\u266a\u266b\u266c',
        ...'\u266d\u266e\u266f\u267b',
        ...'\u2680\u2681\u2682\u2683\u2684\u2685',
        ...'\u2690\u2691\u2692\u2693\u2694\u2695\u2696\u2697\u2698\u2699\u269c\u26a0\u26a1',
    ].join(''),
    dingbats: [
        ...'\u2708\u2712\u2713\u2714\u2715\u2716\u2717\u2718\u271a\u271b\u271c\u2720\u2721',
        ...'\u2726\u2727\u2729\u272a\u272b\u272c\u272d\u272e\u272f\u2730\u2731\u2732\u2733',
        ...'\u2734\u2735\u2736\u2739\u273d\u2740\u2744\u2756\u2764\u2765\u2766\u2767',
        ...'\u2794\u2798\u279c\u27a1\u27a4\u27b2',
    ].join(''),
    enclosed: [
        ...charRange(0x2460, 0x2469),
        ...charRange(0x2776, 0x277f),
        ...charRange(0x2780, 0x2789),
    ],
    keys: [
        ...'\u2318\u2325\u2303\u2324\u23ce\u232b\u2326\u21ea\u2423\u21b5\u21b9\u2380\u2387',
    ].join(''),
    greek: [...charRange(0x0391, 0x03a9), ...charRange(0x03b1, 0x03c9)],
};

function resolveCharset(spec) {
    if (!spec) return CHARSET_GROUPS.ascii.concat(makeDefaultCharset());
    let chars = '';
    for (const tok of spec.split(',')) {
        const t = tok.trim();
        if (!t) continue;
        if (t === 'all') {
            for (const g of Object.values(CHARSET_GROUPS)) chars += g;
            continue;
        }
        if (t.startsWith('-')) {
            // exclude groups will be handled in a second pass
            continue;
        }
        const group = CHARSET_GROUPS[t];
        if (group) {
            chars += group;
            continue;
        }
        console.error(
            `Unknown charset group: "${t}". Available: ${Object.keys(CHARSET_GROUPS).join(', ')}`,
        );
        process.exit(1);
    }
    // Apply exclusions
    for (const tok of spec.split(',')) {
        const t = tok.trim();
        if (t.startsWith('-')) {
            const name = t.slice(1);
            const group = CHARSET_GROUPS[name];
            if (!group) continue;
            for (const ch of group) chars = chars.replaceAll(ch, '');
        }
    }
    return [...new Set([...chars])].join('');
}

function makeDefaultCharset() {
    const groups = [
        'latin1',
        'punct',
        'supsub',
        'fractions',
        'currency',
        'roman',
        'arrows',
        'math',
        'box',
        'blocks',
        'geometric',
        'symbols',
        'dingbats',
        'enclosed',
        'keys',
        'greek',
    ];
    return groups.map((g) => CHARSET_GROUPS[g]).join('');
}

const CHARS = resolveCharset('all');

function nearestPow2(n) {
    let v = 1;
    while (v < n) v *= 2;
    return v;
}

// ---------------------------------------------------------------------------
// Separable squared-distance transform (Felzenszwalb)
// ---------------------------------------------------------------------------

/**
 * 1D squared-distance transform of an input function.
 *  f[n]     - input function values per column / row
 *  d[n]     - output distance values
 *  v[n],z[] - scratch space
 */
function dt1d(f, n, d, v, z) {
    let k = 0;
    v[0] = 0;
    z[0] = -1e20;
    z[1] = 1e20;
    for (let q = 1; q < n; q++) {
        let s = (f[q] + q * q - (f[v[k]] + v[k] * v[k])) / (2 * q - 2 * v[k]);
        while (s <= z[k]) {
            k--;
            s = (f[q] + q * q - (f[v[k]] + v[k] * v[k])) / (2 * q - 2 * v[k]);
        }
        k++;
        v[k] = q;
        z[k] = s;
        z[k + 1] = 1e20;
    }
    k = 0;
    for (let q = 0; q < n; q++) {
        while (z[k + 1] < q) k++;
        d[q] = (q - v[k]) * (q - v[k]) + f[v[k]];
    }
}

/**
 * Separable 2D squared-distance transform.
 * bufOut receives the result; inputs mask where mask[p] is true for inside.
 */
function edt2d(mask, w, h, bufOut) {
    const INF = 1e20;
    const m = Math.max(w, h);
    const f = new Float64Array(m);
    const d = new Float64Array(m);
    const v = new Int32Array(m);
    const z = new Float64Array(m + 1);

    for (let x = 0; x < w; x++) {
        for (let y = 0; y < h; y++) f[y] = mask[y * w + x] ? 0 : INF;
        dt1d(f, h, d, v, z);
        for (let y = 0; y < h; y++) bufOut[y * w + x] = d[y];
    }
    for (let y = 0; y < h; y++) {
        for (let x = 0; x < w; x++) f[x] = bufOut[y * w + x];
        dt1d(f, w, d, v, z);
        for (let x = 0; x < w; x++) bufOut[y * w + x] = d[x];
    }
}

// ---------------------------------------------------------------------------
// Atlas packing
// ---------------------------------------------------------------------------

function packAtlas(glyphs, pad, maxSize = 4096) {
    const sorted = [...glyphs].sort((a, b) => b.bh - a.bh || b.bw - a.bw);
    const totalArea = sorted.reduce((s, g) => s + (g.bw + pad) * (g.bh + pad), 0);
    let size = nearestPow2(Math.ceil(Math.sqrt(totalArea) * 1.3));

    while (true) {
        let x = pad,
            y = pad,
            rowH = 0;
        let ok = true;

        for (const g of sorted) {
            if (x + g.bw + pad > size) {
                y += rowH + pad;
                x = pad;
                rowH = 0;
            }
            if (y + g.bh + pad > size) {
                ok = false;
                break;
            }
            g.atlasX = x;
            g.atlasY = y;
            x += g.bw + pad;
            rowH = Math.max(rowH, g.bh);
        }

        if (ok) return size;
        if (size >= maxSize) {
            throw new Error(
                `Atlas packer could not fit ${sorted.length} glyphs at max size ${maxSize}×${maxSize}. ` +
                    'Try reducing the charset or passing --max-size with a larger value.',
            );
        }
        size *= 2;
    }
}

// ---------------------------------------------------------------------------
// SDF generation
// ---------------------------------------------------------------------------

/**
 * Compute the SDF for a single glyph using separable EDT.
 * Renders the glyph at high resolution into a per-glyph-sized raster,
 * runs inside/outside distance transforms, and point-samples at cell centres.
 *
 * @param {*} gfx - canvas 2D context (canvas will be resized per glyph)
 * @param {string} fontFamily
 * @param {number} fontSize - cell pixels
 * @param {number} ss - supersampling factor
 * @param {number} spread - SDF spread in cell pixels
 * @param {string} char - character to render
 * @returns glyph result object
 */
function renderGlyphSDF(gfx, fontFamily, fontSize, ss, spread, char) {
    const renderSize = fontSize * ss;
    const spreadPx = spread * ss;
    const padPx = PAD * ss;

    gfx.font = `${renderSize}px "${fontFamily}"`;
    gfx.textBaseline = 'alphabetic';
    gfx.textAlign = 'left';
    const m = gfx.measureText(char);

    const inkW = Math.max(1, Math.ceil((m.actualBoundingBoxLeft + m.actualBoundingBoxRight) / ss));
    const inkH = Math.max(
        1,
        Math.ceil((m.actualBoundingBoxAscent + m.actualBoundingBoxDescent) / ss),
    );
    const cellW = inkW + spread * 2 + PAD * 2;
    const cellH = inkH + spread * 2 + PAD * 2;
    const srcW = cellW * ss;
    const srcH = cellH * ss;

    gfx.canvas.width = srcW;
    gfx.canvas.height = srcH;

    gfx.fillStyle = '#ffffff';
    gfx.font = `${renderSize}px "${fontFamily}"`;
    gfx.textBaseline = 'alphabetic';
    gfx.textAlign = 'left';

    gfx.clearRect(0, 0, srcW, srcH);
    const drawX = padPx + spreadPx;
    const drawY = padPx + spreadPx + renderSize * 0.78;
    gfx.fillText(char, drawX, drawY);

    const imgData = gfx.getImageData(0, 0, srcW, srcH).data;

    const inside = new Uint8Array(srcW * srcH);
    const outside = new Uint8Array(srcW * srcH);
    for (let i = 0, p = 0; i < imgData.length; i += 4, p++) {
        const on = imgData[i + 3] > 128;
        inside[p] = on ? 1 : 0;
        outside[p] = on ? 0 : 1;
    }

    const bufA = new Float64Array(srcW * srcH);
    const bufB = new Float64Array(srcW * srcH);
    edt2d(outside, srcW, srcH, bufA);
    edt2d(inside, srcW, srcH, bufB);

    const maxDist = spread * ss;
    const cellBuf = new Uint8ClampedArray(cellW * cellH * 4);

    for (let cy = 0; cy < cellH; cy++) {
        for (let cx = 0; cx < cellW; cx++) {
            const sx = cx * ss + (ss >> 1);
            const sy = cy * ss + (ss >> 1);
            const p = sy * srcW + sx;
            const sd = inside[p] ? Math.sqrt(bufA[p]) : -Math.sqrt(bufB[p]);
            const norm = Math.max(-1, Math.min(1, sd / maxDist));
            const byte = Math.round(128 + norm * 127);
            const di = (cy * cellW + cx) * 4;
            cellBuf[di] = byte;
            cellBuf[di + 1] = byte;
            cellBuf[di + 2] = byte;
            cellBuf[di + 3] = 255;
        }
    }

    let anyVariant = false;
    for (let i = 0; i < cellBuf.length; i += 4) {
        if (cellBuf[i] !== 128) {
            anyVariant = true;
            break;
        }
    }

    // Compute offsets relative to the glyph origin (left baseline point)
    // The glyph origin within the cell in cell-pixel coordinates is at:
    //   originX = PAD + spread   (drawX / ss)
    //   originY = PAD + spread + baseline  (drawY / ss = PAD + spread + fontSize*0.78)
    const baseline = fontSize * 0.78;
    const originCX = PAD + spread;
    const originCY = PAD + spread + baseline;

    // xOffset = cell_left_edge - origin_x  (BBox left relative to origin)
    // yOffset = cell_top_edge - origin_y   (BBox top relative to origin)
    const xOffset = 0 - originCX;
    const yOffset = 0 - originCY;

    return {
        imageData: new ImageData(cellBuf, cellW, cellH),
        xAdvance: m.width / ss,
        xOffset,
        yOffset,
        bw: cellW,
        bh: cellH,
        char,
        anyVariant,
    };
}

// ---------------------------------------------------------------------------
// Content-hash cache
// ---------------------------------------------------------------------------

const CACHE_VERSION = 1;

function cacheKey(fontBuf, charset, opts) {
    const h = createHash('sha256');
    h.update(String(CACHE_VERSION));
    h.update(fontBuf);
    h.update(charset);
    h.update(String(opts.ss));
    h.update(String(opts.spread));
    h.update(String(GLYPH_SIZE));
    h.update(String(PAD));
    return h.digest('hex');
}

async function cacheHit(baseName, key) {
    const sigPath = path.join(resDir, `${baseName}-sdf.sig`);
    try {
        const existing = (await readFile(sigPath, 'utf-8')).trim();
        if (existing !== key) return false;
        // Verify output files still exist
        await stat(path.join(resDir, `${baseName}-sdf.png`));
        await stat(path.join(resDir, `${baseName}-sdf.json`));
        return true;
    } catch {
        return false;
    }
}

async function cacheWrite(baseName, key) {
    await writeFile(path.join(resDir, `${baseName}-sdf.sig`), key);
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

function parseArgs(args) {
    if (args.length < 1) {
        console.error('Usage: make-font-atlas.mjs <font.ttf> [options]');
        process.exit(1);
    }

    const fontPath = path.resolve(args[0]);
    const fontName = path.basename(fontPath, path.extname(fontPath));

    const opts = {
        family: fontName,
        ss: 12,
        spread: 4,
        maxSize: 4096,
        noCache: false,
        charset: null,
    };

    for (let i = 1; i < args.length; i++) {
        const a = args[i];
        if (a === '--family' && i + 1 < args.length) opts.family = args[++i];
        else if (a === '--ss' && i + 1 < args.length) opts.ss = parseInt(args[++i], 10);
        else if (a === '--spread' && i + 1 < args.length) opts.spread = parseInt(args[++i], 10);
        else if (a === '--max-size' && i + 1 < args.length) opts.maxSize = parseInt(args[++i], 10);
        else if (a === '--charset' && i + 1 < args.length) opts.charset = args[++i];
        else if (a === '--no-cache') opts.noCache = true;
    }

    return { fontPath, fontName, opts };
}

async function main() {
    const { fontPath, fontName, opts } = parseArgs(process.argv.slice(2));
    const baseName = fontName.toLowerCase().replace(/\s+/g, '-');
    const fontSize = FONT_CELL_SIZE;
    const charset = opts.charset ? resolveCharset(opts.charset) : CHARS;

    // Check cache
    if (!opts.noCache) {
        const fontBuf = await readFile(fontPath);
        const key = cacheKey(fontBuf, charset, opts);
        if (await cacheHit(baseName, key)) {
            console.log(`SDF atlas cache hit for ${baseName} (${charset.length} glyphs)`);
            return;
        }
    }

    console.log(`Loading font: ${fontPath}`);
    GlobalFonts.registerFromPath(fontPath, opts.family);

    // Shared 2D context, resized per glyph
    const gfx = createCanvas(8, 8).getContext('2d');

    // Render .notdef references (known-absent code points) and hash them
    const notdefA = renderGlyphSDF(gfx, opts.family, fontSize, opts.ss, opts.spread, '\u{1F600}');
    const notdefB = renderGlyphSDF(gfx, opts.family, fontSize, opts.ss, opts.spread, '\u4e00');
    const notdefHash = (b) => Buffer.from(b).toString('base64');
    const ndHashes = new Set([
        notdefHash(notdefA.imageData.data),
        notdefHash(notdefB.imageData.data),
    ]);
    // Also consider fully-blank glyphs (all byte=128) as absent
    function isAbsent(result) {
        if (result.anyVariant === false) return true;
        return ndHashes.has(notdefHash(result.imageData.data));
    }

    // Dedup bitmap-identical glyphs (e.g. NBSP ≡ space)
    const seen = new Map();

    const glyphResults = [];
    console.log(`Generating ${charset.length} glyphs (ss=${opts.ss}, spread=${opts.spread})...`);
    const t0 = Date.now();

    for (const char of charset) {
        const result = renderGlyphSDF(gfx, opts.family, fontSize, opts.ss, opts.spread, char);
        if (isAbsent(result)) continue;
        const h = notdefHash(result.imageData.data);
        const prev = seen.get(h);
        if (prev) {
            glyphResults.push({ ...result, char, xAdvance: result.xAdvance }); // use own advance
            continue;
        }
        seen.set(h, result);
        glyphResults.push(result);
    }

    const genMs = Date.now() - t0;
    console.log(
        `  ${glyphResults.length} glyphs rendered in ${(genMs / 1000).toFixed(1)} s ` +
            `(${(genMs / glyphResults.length).toFixed(1)} ms/glyph)`,
    );

    const atlasSize = packAtlas(glyphResults, PAD, opts.maxSize);

    const atlasCanvas = createCanvas(atlasSize, atlasSize);
    const atlasCtx = atlasCanvas.getContext('2d');

    atlasCtx.fillStyle = '#000000';
    atlasCtx.fillRect(0, 0, atlasSize, atlasSize);

    const glyphMap = {};
    let spaceAdvance = fontSize * 0.28;

    for (const g of glyphResults) {
        atlasCtx.putImageData(g.imageData, g.atlasX, g.atlasY);

        const xAdvance = g.char === ' ' ? spaceAdvance : g.xAdvance;
        if (g.char === ' ' && g.xAdvance > 0) spaceAdvance = g.xAdvance;

        glyphMap[g.char] = {
            x: g.atlasX,
            y: g.atlasY,
            w: g.bw,
            h: g.bh,
            xOffset: g.xOffset,
            yOffset: g.yOffset,
            xAdvance: xAdvance,
        };
    }

    const metrics = {
        name: baseName,
        family: opts.family,
        atlasSize: [atlasSize, atlasSize],
        glyphSize: GLYPH_SIZE,
        spread: opts.spread,
        baseline: fontSize * 0.78,
        lineHeight: fontSize * 1.3,
        glyphs: glyphMap,
    };

    await mkdir(resDir, { recursive: true });

    const pngBuf = await atlasCanvas.encode('png');
    const pngPath = path.join(resDir, `${baseName}-sdf.png`);
    const jsonPath = path.join(resDir, `${baseName}-sdf.json`);

    await writeFile(pngPath, pngBuf);
    await writeFile(
        jsonPath,
        JSON.stringify(metrics, (key, value) =>
            typeof value === 'number' ? Math.round(value * 1000) / 1000 : value,
        ),
    );

    // Write cache signature (after successful output)
    if (!opts.noCache) {
        const fontBuf = await readFile(fontPath);
        await cacheWrite(baseName, cacheKey(fontBuf, charset, opts));
    }

    console.log(`Generated: ${pngPath}  (${atlasSize}x${atlasSize})`);
    console.log(`Generated: ${jsonPath}`);
    console.log(`Glyphs:   ${Object.keys(glyphMap).length} characters`);
}

main().catch((err) => {
    console.error(err);
    process.exit(1);
});
