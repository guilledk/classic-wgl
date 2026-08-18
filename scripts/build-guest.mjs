#!/usr/bin/env node
// Compile the per-scene ROM guest crates (standalone #![no_std] cdylibs under
// guest/) to wasm32-unknown-unknown and copy them into public/code/, where
// build-roms.mjs bundles them into each scene ROM.
//
// Regenerate via `npm run assets`.

import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const targetDir = path.join(root, 'target');

const guests = [
    { crate: 'demo-guest', out: 'demo.wasm' },
    { crate: 'lunar-guest', out: 'lunar.wasm' },
];

mkdirSync(path.join(root, 'public', 'code'), { recursive: true });

for (const { crate, out } of guests) {
    const manifest = path.join(root, 'guest', crate, 'Cargo.toml');
    execFileSync(
        'cargo',
        ['build', '--manifest-path', manifest, '--target', 'wasm32-unknown-unknown', '--release'],
        { stdio: 'inherit', env: { ...process.env, CARGO_TARGET_DIR: targetDir } },
    );
    const src = path.join(
        targetDir,
        'wasm32-unknown-unknown',
        'release',
        `${crate.replace(/-/g, '_')}.wasm`,
    );
    copyFileSync(src, path.join(root, 'public', 'code', out));
    console.log(`wrote public/code/${out} from ${crate}`);
}
