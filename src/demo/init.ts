import game from '/classic/state.js';

import {
    initCursor,
    initCameraControllerWASD,
    initTilemap,
    initTilemapEditorLogic,
    initNavMeshEditorLogic,
    initHeightEditorLogic,
    initAgent,
    generateDemoSlopes,
    initFootprintColliders,
} from './prefabs.js';
import { initUI } from './uiPrefabs.js';
import { initLighting } from '/classic/lighting.js';
import { createLoader } from '/classic/loader.js';
import type { LoaderController } from '/classic/loader.js';
import { applyLoadTestParams } from './loadTest.js';

import { isoToCartesian4 } from '/classic/utils.js';
import { mat4, vec3 } from 'gl-matrix';

// Fraction of the loading bar assigned to each phase of startup.
const SLOTS = {
    init: [0.0, 0.03],
    resources: [0.03, 0.85],
    state: [0.85, 0.97],
    scene: [0.97, 1.0],
} as const;

const loader: LoaderController = createLoader({
    exit: 'instant',
    persistMode: true,
    logoUrl: '/res/cool_snek.png',
});

// Optional manual-test knobs (?slow, ?fail=...) - see src/demo/loadTest.ts.
const testOpts = applyLoadTestParams();
const testParts: string[] = [];
if (testOpts.sleepMs > 0) {
    testParts.push(`slow ${testOpts.sleepMs}ms/step`);
}
if (testOpts.fail) {
    testParts.push(`fail=${testOpts.fail}`);
}
if (testParts.length > 0) {
    loader.note(`load test: ${testParts.join(' / ')}`);
}

// Maps a phase-local progress fraction (0..1) onto its global slot on the bar.
function report(slotStart: number, slotSpan: number) {
    return (label: string, fraction: number) =>
        loader.setProgress(label, slotStart + fraction * slotSpan);
}

async function initContext(): Promise<void> {
    try {
        window.game = game;

        loader.setProgress('Initializing WebGL', SLOTS.init[0]);
        game.init();
        loader.setProgress('Initializing WebGL', SLOTS.init[1]);

        const [resStart, resEnd] = SLOTS.resources;
        await game.loadResources(report(resStart, resEnd - resStart));

        const [stateStart, stateEnd] = SLOTS.state;
        await game.load('/state.json', report(stateStart, stateEnd - stateStart));

        loader.setProgress('Building scene', SLOTS.scene[0]);
        initCursor();
        initCameraControllerWASD();
        initTilemap();
        initTilemapEditorLogic();
        initNavMeshEditorLogic();
        initHeightEditorLogic();
        initAgent();
        generateDemoSlopes();
        initFootprintColliders();

        initUI();
        initLighting();

        game.showGrid = true;

        // Centre camera on demo slope area around tile (90, 85)
        const isoToWorld = mat4.clone(isoToCartesian4);
        mat4.scale(isoToWorld, isoToWorld, [45, 45, 1]);
        const centre = vec3.fromValues(0, 0, 0);
        vec3.transformMat4(centre, centre, isoToWorld);
        game.camera.position[0] = centre[0];
        game.camera.position[1] = centre[1];

        loader.setProgress('Building scene', SLOTS.scene[1]);

        game.launch();
    } catch (err) {
        console.error('Failed to initialize classic-wgl:', err);
        loader.fail(err instanceof Error ? (err.stack ?? err.message) : String(err));
    }
}

window.addEventListener('load', initContext, false);
