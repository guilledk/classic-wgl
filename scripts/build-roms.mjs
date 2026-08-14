#!/usr/bin/env node
// Build the shipped scene ROMs (demo.rom / lunar.rom) as gzip'd tarballs.
//
// Each ROM bundles `manifest.json` (with `format_version` + `entrypoint`
// injected), the scene's `state.json`, and every manifest-declared texture /
// SDF font, at paths matching the manifest's `src`/`metrics` (leading `/`
// stripped, e.g. `res/humanoid.png`).  The Rust `RomArchive` reads these
// tar.gz archives; shaders are not bundled (the engine ships a built-in GLSL
// base and ROMs may override it later).
//
// Regenerate via `npm run assets`.

import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import zlib from 'node:zlib';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const publicDir = path.join(root, 'public');
const resDir = path.join(publicDir, 'res');
const animsDir = path.join(publicDir, 'animations');

const romPath = (p) => p.replace(/^\/+/, ''); // "/res/foo.png" -> "res/foo.png"
const basename = (p) => p.split('/').pop();

function parseManifest(name) {
    return JSON.parse(readFileSync(path.join(publicDir, name), 'utf8'));
}

// Merge a per-scene manifest overlay (`manifest_lunar.json`) onto the shared
// base `manifest.json`, so scene-specific assets ship only in that scene's ROM.
function parseSceneManifest(overlayName) {
    const manifest = parseManifest('manifest.json');
    if (!overlayName) return manifest;
    const overlay = parseManifest(overlayName);
    for (const key of ['shaders', 'textures', 'sdfFonts', 'animations']) {
        if (overlay[key] && overlay[key].length) {
            manifest[key] = (manifest[key] || []).concat(overlay[key]);
        }
    }
    return manifest;
}

function tarHeader(name, size) {
    const h = Buffer.alloc(512, 0);
    h.write(name, 0, 100, 'utf8'); // name
    h.write('0000644\0', 100, 8, 'utf8'); // mode
    h.write('0000000\0', 108, 8, 'utf8'); // uid
    h.write('0000000\0', 116, 8, 'utf8'); // gid
    h.write(size.toString(8).padStart(11, '0') + '\0', 124, 12, 'utf8'); // size
    h.write('00000000000\0', 136, 12, 'utf8'); // mtime
    h.write('        ', 148, 8, 'utf8'); // chksum (spaces while computing)
    h.write('0', 156, 1, 'utf8'); // typeflag: regular file
    h.write('ustar\0', 257, 6, 'utf8'); // magic
    h.write('00', 263, 2, 'utf8'); // version
    // chksum: sum of header bytes (chksum field as spaces)
    let sum = 0;
    for (let i = 0; i < 512; i++) sum += h[i];
    h.write(sum.toString(8).padStart(6, '0') + '\0 ', 148, 8, 'utf8');
    return h;
}

function pack(entrypoint, stateFile, outName, guestWasm, overlayName) {
    const manifest = parseSceneManifest(overlayName);
    manifest.format_version = 1;
    manifest.entrypoint = entrypoint;
    manifest.state = 'state.json';
    manifest.host_features = true;
    manifest.trusted = true;
    manifest.code = [{ name: 'main', src: `/code/${guestWasm}` }];

    const chunks = [];
    const addFile = (name, data) => {
        chunks.push(tarHeader(name, data.length));
        chunks.push(data);
        const pad = (512 - (data.length % 512)) % 512;
        if (pad) chunks.push(Buffer.alloc(pad, 0));
    };

    addFile('manifest.json', Buffer.from(JSON.stringify(manifest, null, 4), 'utf8'));
    addFile('state.json', readFileSync(path.join(publicDir, stateFile)));

    for (const t of manifest.textures || []) {
        addFile(romPath(t.src), readFileSync(path.join(resDir, basename(t.src))));
    }
    for (const f of manifest.sdfFonts || []) {
        addFile(romPath(f.metrics), readFileSync(path.join(resDir, basename(f.metrics))));
    }
    for (const a of manifest.animations || []) {
        if (a.metadata) {
            addFile(romPath(a.metadata), readFileSync(path.join(animsDir, basename(a.metadata))));
        }
    }
    for (const c of manifest.code || []) {
        addFile(romPath(c.src), readFileSync(path.join(publicDir, 'code', basename(c.src))));
    }

    chunks.push(Buffer.alloc(1024, 0)); // end-of-archive
    const tar = Buffer.concat(chunks);
    const gz = zlib.gzipSync(tar, { level: 9 });
    writeFileSync(path.join(publicDir, outName), gz);
    console.log(`wrote public/${outName}: ${gz.length} bytes`);
}

pack('demo', 'state.json', 'demo.rom', 'demo.wasm');
pack('lunar', 'state_lunar.json', 'lunar.rom', 'lunar.wasm', 'manifest_lunar.json');
