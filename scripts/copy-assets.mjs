#!/usr/bin/env node
import { cp, mkdir, readdir, rm, stat } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const assetsDir = path.join(root, 'assets');
const resDir = path.join(root, 'public', 'res');
const clean = process.argv.includes('--clean');

async function outputs() {
    const out = new Map();
    for (const f of await readdir(path.join(assetsDir, 'demo'))) {
        if (f.endsWith('.png')) out.set(f, path.join(assetsDir, 'demo', f));
    }
    for (const name of await readdir(path.join(assetsDir, 'buildings'))) {
        const sheet = path.join(assetsDir, 'buildings', name, 'spritesheet.png');
        try {
            if ((await stat(sheet)).isFile()) out.set(`${name}.png`, sheet);
        } catch {
            // no spritesheet for this building
        }
    }
    return out;
}

await mkdir(resDir, { recursive: true });
const produced = new Set();
for (const [target, src] of await outputs()) {
    await cp(src, path.join(resDir, target));
    produced.add(target);
}

if (clean) {
    for (const f of await readdir(resDir)) {
        if (f.endsWith('.png') && !produced.has(f)) {
            await rm(path.join(resDir, f), { force: true });
        }
    }
}

console.log(`copied ${produced.size} assets to public/res/`);
