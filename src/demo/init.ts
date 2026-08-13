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
} from './prefabs.js';
import { initUI } from './uiPrefabs.js';
import { initLighting } from '/classic/lighting.js';

import { isoToCartesian4 } from '/classic/utils.js';
import { mat4, vec3 } from 'gl-matrix';

async function initContext(): Promise<void> {
    window.game = game;

    game.init();
    await game.loadResources();

    await game.load('/state.json');

    initCursor();
    initCameraControllerWASD();
    initTilemap();
    initTilemapEditorLogic();
    initNavMeshEditorLogic();
    initHeightEditorLogic();
    initAgent();
    generateDemoSlopes();

    initUI();
    initLighting();

    game.showGrid = true;

    // Centre camera on demo slope area around tile (90, 85)
    const isoToWorld = mat4.clone(isoToCartesian4);
    mat4.scale(isoToWorld, isoToWorld, [45, 45, 1]);
    const centre = vec3.fromValues(90, 85, 0);
    vec3.transformMat4(centre, centre, isoToWorld);
    game.camera.position[0] = centre[0];
    game.camera.position[1] = centre[1];

    game.launch();
}

window.addEventListener('load', initContext, false);
