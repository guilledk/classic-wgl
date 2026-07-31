import game from '/classic/state.js';

import {
    initCursor,
    initCameraControllerWASD,
    initTilemap,
    initTilemapEditorLogic,
    initNavMeshEditorLogic,
    initAgent,
} from './prefabs.js';
import { initUI } from './uiPrefabs.js';

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
    initAgent();

    initUI();

    game.camera.position[0] += 800;

    game.launch();
}

window.addEventListener('load', initContext, false);
