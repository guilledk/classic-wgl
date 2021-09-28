import game from "/classic/state.js"

import {
    initCursor,
    initFPSCounter,
    initInfoText,
    initCameraControllerWASD,
    initSelectionMonitor,
    initTilemap,
    initTilemapEditor,
    initNavMeshEditor,
    initAgent,
    initDEVButtons
} from "/classic/prefabs.js";


async function initContext() {

    window.game = game;

    await game.init();
    await game.loadResources();

    await game.load("state.json");
    
    initCursor();
    initFPSCounter();
    initInfoText();
    initCameraControllerWASD();
    initSelectionMonitor();
    initTilemap();
    initTilemapEditor();
    await initNavMeshEditor();

    initAgent();

    initDEVButtons();

    game.camera.position[0] += 800;

    game.launch();

}

window.addEventListener("load", initContext, false);
