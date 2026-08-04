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
// Font cell-pixel size. Must match the "cell pixel" unit used throughout the
// engine: cell px = (GLYPH_SIZE * 1.2 * old_RENDER_SCALE) / old_CELL_SCALE
// old_RENDER_SCALE was 16, old_CELL_SCALE was 48.  Simplifies to GLYPH_SIZE * 0.4.
const FONT_CELL_SIZE = GLYPH_SIZE * 0.4;

const CHARS =
    ' !"#$%&\'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~';

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
    };

    for (let i = 1; i < args.length; i++) {
        const a = args[i];
        if (a === '--family' && i + 1 < args.length) opts.family = args[++i];
        else if (a === '--ss' && i + 1 < args.length) opts.ss = parseInt(args[++i], 10);
        else if (a === '--spread' && i + 1 < args.length) opts.spread = parseInt(args[++i], 10);
        else if (a === '--max-size' && i + 1 < args.length) opts.maxSize = parseInt(args[++i], 10);
        else if (a === '--no-cache') opts.noCache = true;
    }

    return { fontPath, fontName, opts };
}

async function main() {
    const { fontPath, fontName, opts } = parseArgs(process.argv.slice(2));
    const baseName = fontName.toLowerCase().replace(/\s+/g, '-');

    // Check cache
    if (!opts.noCache) {
        const fontBuf = await readFile(fontPath);
        const key = cacheKey(fontBuf, CHARS, opts);
        if (await cacheHit(baseName, key)) {
            console.log(`SDF atlas cache hit for ${baseName} (${CHARS.length} glyphs)`);
            return;
        }
    }

    console.log(`Loading font: ${fontPath}`);
    GlobalFonts.registerFromPath(fontPath, opts.family);

    const fontSize = FONT_CELL_SIZE;

    // Shared 2D context, resized per glyph
    const gfx = createCanvas(8, 8).getContext('2d');

    const glyphResults = [];
    console.log(`Generating ${CHARS.length} glyphs (ss=${opts.ss}, spread=${opts.spread})...`);
    const t0 = Date.now();

    for (const char of CHARS) {
        const result = renderGlyphSDF(gfx, opts.family, fontSize, opts.ss, opts.spread, char);
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
    await writeFile(jsonPath, JSON.stringify(metrics));

    // Write cache signature (after successful output)
    if (!opts.noCache) {
        const fontBuf = await readFile(fontPath);
        await cacheWrite(baseName, cacheKey(fontBuf, CHARS, opts));
    }

    console.log(`Generated: ${pngPath}  (${atlasSize}x${atlasSize})`);
    console.log(`Generated: ${jsonPath}`);
    console.log(`Glyphs:   ${Object.keys(glyphMap).length} characters`);
}

main().catch((err) => {
    console.error(err);
    process.exit(1);
});
