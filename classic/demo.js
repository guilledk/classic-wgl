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
    initAgent
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

    game.camera.scale[0] = .7;
    game.camera.scale[1] = .7;

    game.camera.position[0] += 550;

    game.launch();

}

window.addEventListener("load", initContext, false);
