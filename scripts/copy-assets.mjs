#!/usr/bin/env node
import { cp, mkdir, readdir, rm, stat } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const assetsDir = path.join(root, 'assets');
const resDir = path.join(root, 'public', 'res');
const animsDir = path.join(root, 'public', 'animations');
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
    const rocketDir = path.join(assetsDir, 'vehicles', 'us-rocket');
    for (const file of ['landing_spritesheet.png', 'launch_spritesheet.png']) {
        const source = path.join(rocketDir, file);
        try {
            if ((await stat(source)).isFile()) out.set(file, source);
        } catch {
            // vehicle animation output is optional
        }
    }
    return out;
}

await mkdir(resDir, { recursive: true });
await mkdir(animsDir, { recursive: true });
const produced = new Set();
for (const [target, src] of await outputs()) {
    await cp(src, path.join(resDir, target));
    produced.add(target);
}

// Copy the rocket's per-frame animation metadata into public/animations/,
// keyed by the animation name the ROM manifest declares (`rocketLanding`).
const rocketMeta = [
    ['landing_meta.json', 'rocketLanding.json'],
    ['launch_meta.json', 'rocketLaunch.json'],
];
for (const [src, target] of rocketMeta) {
    const source = path.join(assetsDir, 'vehicles', 'us-rocket', src);
    try {
        if ((await stat(source)).isFile()) {
            await cp(source, path.join(animsDir, target));
        }
    } catch {
        // vehicle animation output is optional
    }
}

if (clean) {
    for (const f of await readdir(resDir)) {
        if (f.endsWith('.png') && !produced.has(f)) {
            await rm(path.join(resDir, f), { force: true });
        }
    }
}

console.log(`copied ${produced.size} assets to public/res/`);
