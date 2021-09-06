import game from "/classic/state.js";
import { Tilemap, IsometricNavMesh, IsoAgent } from "/classic/isometric.js";
import { Rectangle, Sprite, Text } from "/classic/transforms.js";

import { vec2, vec3 } from "/lib/gl-matrix/index.js";



export function initCursor() {

    let cursor = game.getEntity("cursor");
    let compSprite = cursor.getComponent(Sprite);

    cursor.registerCall(
        "update",
        function () {
            vec3.copy(compSprite.position, game.mousePos);
        });

}

export function initFPSCounter() {

    let fpsCounter = game.getEntity("fpsCounter");
    let compFPSText = fpsCounter.getComponent(Text);

    fpsCounter.registerCall(
        "update",
        function() {
            compFPSText.setText(game.fps.toString());
        });

}

export function initInfoText() {

    let label1 = game.getEntity("textLabel1");
    let label1Text = label1.getComponent(Text);
    
    label1Text.setText("CLASSIC ENGINE V0.1A0 ;)");

    let label2 = game.getEntity("textLabel2");
    let label2Text = label2.getComponent(Text);
    
    label2Text.setText("SCROLL WHEEL TO ZOOM, WASD TO MOVE CAM");

}


export function initCameraControllerWASD() {

    let camController = game.getEntity("camController");
    camController.registerCall(
        "update",
        function() {
            if (game.isKeyDown("KeyW"))
                game.camera.position[1] -= game.scrollSpeed * game.deltaTime;
            if (game.isKeyDown("KeyS"))
                game.camera.position[1] += game.scrollSpeed * game.deltaTime;
            if (game.isKeyDown("KeyA"))
                game.camera.position[0] -= game.scrollSpeed * game.deltaTime;
            if (game.isKeyDown("KeyD"))
                game.camera.position[0] += game.scrollSpeed * game.deltaTime;

            if (Math.abs(game.mouseWheel) > .01) {
                game.camera.scale[0] += game.mouseWheel * game.deltaTime;
                game.camera.scale[1] += game.mouseWheel * game.deltaTime;

                vec3.max(game.camera.scale, game.camera.scale, [.1, .1, 1]);
            }
        });

}


export function initTilemap() {

    let tilemap = game.getEntity("tilemap");

    let compTilemap = tilemap.getComponent(Tilemap);
    compTilemap.uploadToGPU();

}

export function initSelectionMonitor() {

    game.editorTarget = "navMesh";

    let tilemap = game.getEntity("tilemap");
    let tileSelector = game.getEntity("tilemapTileSelector");
    let tilemapEditor = game.getEntity("tilemapEditor");

    let navMesh = game.getEntity("tilemapNavigation");  
    let navMeshSelector = game.getEntity("navMeshTileSelector");
    let navMeshEditor = game.getEntity("navMeshEditor");

    let monitor = game.getEntity("selectionMonitor");
    monitor.registerCall(
        "update",
        function() {
            tilemap.enabled = game.editorTarget == "tilemap";
            tileSelector.enabled = game.editorTarget == "tilemap";
            tilemapEditor.enabled = game.editorTarget == "tilemap";

            navMesh.enabled = game.editorTarget == "navMesh";
            navMeshSelector.enabled = game.editorTarget == "navMesh";
            navMeshEditor.enabled = game.editorTarget == "navMesh";
        });

}

export function initTilemapEditor() {

    let tilemap = game.getEntity("tilemap");
    let tileSelector = game.getEntity("tilemapTileSelector");
    let tilemapEditor = game.getEntity("tilemapEditor");

    let compTilemap = tilemap.getComponent(Tilemap);

    const uiBorder = 10;
    let selectedTile = 0;
    let isMouseOverEditor = false;
    let localPos = vec3.fromValues(0, 0, 0);

    let compTilemapSprite = tilemapEditor.getComponent(Sprite);
    let compTilemapSpriteBG = tilemapEditor.getComponent(Rectangle);

    let compTileSelector = tileSelector.getComponent(Rectangle);

    tilemapEditor.registerCall(
        "update",
        function() {

            // Update position of tile selector grafics
            compTilemapSprite.position = vec3.fromValues(
                game.canvas.width - compTilemap.tileSetPixelSize[0] - uiBorder,
                game.canvas.height - compTilemap.tileSetPixelSize[1] - uiBorder,
                compTilemapSprite.position[2]
            );
    
            vec3.copy(compTilemapSpriteBG.position, compTilemapSprite.position);
            compTilemapSpriteBG.scale = [
                compTilemap.tileSetPixelSize[0],
                compTilemap.tileSetPixelSize[1],
                1];

            // Tile selection logic 
            isMouseOverEditor = game.mousePos[0] >= compTilemapSprite.position[0] &&
                game.mousePos[1] >= compTilemapSprite.position[1] &&
                game.mousePos[0] <= compTilemapSprite.position[0] + compTilemap.tileSetPixelSize[0] &&
                game.mousePos[1] <= compTilemapSprite.position[1] + compTilemap.tileSetPixelSize[1];
            
            if (game.wasMouseButtonPressed(0) && isMouseOverEditor) {
                // inside tile selector
                localPos = vec3.clone(game.mousePos);
                vec3.sub(localPos, localPos, compTilemapSprite.position);
                
                localPos[0] = Math.floor(localPos[0] / compTilemap.tilePixelSize[0]);
                localPos[1] = Math.floor(localPos[1] / compTilemap.tilePixelSize[1]);
                localPos[2] = 0;

                selectedTile = Math.min(
                    compTilemap.maxTile, 
                    localPos[0] + (localPos[1] * compTilemap.tileSetSize[0]));
            }

            // Update tile selector rectangle
            compTileSelector.position = vec3.clone(compTilemapSprite.position);
            vec3.add(
                compTileSelector.position,
                compTileSelector.position,
                [localPos[0] * compTilemap.tilePixelSize[0], localPos[1] * compTilemap.tilePixelSize[1], 0]);
        });

    // Actual tilemap editing logic
    tilemapEditor.registerCall(
        "selectionEnd",
        function() {
            if (isMouseOverEditor)
                return;

            const [begin, end] = compTilemap.getSelection();
            if (begin[0] < 0)
                begin[0] = 0;

            if (begin[1] < 0)
                begin[1] = 0;

            if (end[0] > compTilemap.sizeX - 1)
                end[0] = compTilemap.sizeX - 1;

            if (end[1] > compTilemap.sizeY)
                end[1] = compTilemap.sizeY - 1;

            compTilemap.fillRegion(begin, end, selectedTile);
            compTilemap.uploadToGPU();
        });
}


export async function initNavMeshEditor() {

    let navMesh = game.getEntity("tilemapNavigation");
    let compNavMesh = navMesh.getComponent(IsometricNavMesh);
    await compNavMesh.init();
    compNavMesh.uploadToGPU();

    let navMeshSelector = game.getEntity("navMeshTileSelector");
    let navMeshEditor = game.getEntity("navMeshEditor");

    const uiBorder = 10;
    const uiScale = 4;
    let selectedTile = 0;
    let isMouseOverEditor = false;
    let localPos = vec3.fromValues(0, 0, 0);

    let compTilemapSprite = navMeshEditor.getComponent(Sprite);
    let compTilemapSpriteBG = navMeshEditor.getComponent(Rectangle);

    let compTileSelector = navMeshSelector.getComponent(Rectangle);

    compTileSelector.scale = [
        compNavMesh.tilePixelSize[0] * uiScale,
        compNavMesh.tilePixelSize[1] * uiScale,
        1];
    compTilemapSprite.scale = [uiScale, uiScale, 1];
    compTilemapSpriteBG.scale = [uiScale, uiScale, 1];
    
    navMeshEditor.registerCall(
        "update",
        function() {

            // Update position of tile selector grafics
            compTilemapSprite.position = vec3.fromValues(
                game.canvas.width - (compNavMesh.tileSetPixelSize[0] * uiScale) - uiBorder,
                game.canvas.height - (compNavMesh.tileSetPixelSize[1] * uiScale) - uiBorder,
                compTilemapSprite.position[2]
            );

            vec3.copy(compTilemapSpriteBG.position, compTilemapSprite.position);
            compTilemapSpriteBG.scale = [
                compNavMesh.tileSetPixelSize[0] * uiScale,
                compNavMesh.tileSetPixelSize[1] * uiScale,
                1];

            // Tile selection logic 
            isMouseOverEditor = game.mousePos[0] >= compTilemapSprite.position[0] &&
                game.mousePos[1] >= compTilemapSprite.position[1] &&
                game.mousePos[0] <= compTilemapSprite.position[0] + (compNavMesh.tileSetPixelSize[0] * uiScale) &&
                game.mousePos[1] <= compTilemapSprite.position[1] + (compNavMesh.tileSetPixelSize[1] * uiScale);
            
            if (game.wasMouseButtonPressed(0) && isMouseOverEditor) {
                // inside tile selector
                localPos = vec3.clone(game.mousePos);
                vec3.sub(localPos, localPos, compTilemapSprite.position);
                
                localPos[0] = Math.floor(localPos[0] / (compNavMesh.tilePixelSize[0] * uiScale));
                localPos[1] = Math.floor(localPos[1] / (compNavMesh.tilePixelSize[1] * uiScale));
                localPos[2] = 0;

                selectedTile = Math.min(
                    compNavMesh.maxTile, 
                    localPos[0] + (localPos[1] * (compNavMesh.tileSetSize[0] * uiScale)));
            }

            // Update tile selector rectangle
            compTileSelector.position = vec3.clone(compTilemapSprite.position);
            vec3.add(
                compTileSelector.position,
                compTileSelector.position,
                [
                    localPos[0] * compNavMesh.tilePixelSize[0] * uiScale,
                    localPos[1] * compNavMesh.tilePixelSize[1] * uiScale,
                    0]);
        });

    // Actual tilemap editing logic
    navMeshEditor.registerCall(
        "selectionEnd",
        function() {
            if (isMouseOverEditor)
                return;

            const [begin, end] = compNavMesh.getSelection();
            if (begin[0] < 0)
                begin[0] = 0;

            if (begin[1] < 0)
                begin[1] = 0;

            if (end[0] > compNavMesh.sizeX - 1)
                end[0] = compNavMesh.sizeX - 1;

            if (end[1] > compNavMesh.sizeY)
                end[1] = compNavMesh.sizeY - 1;

            compNavMesh.fillRegion(begin, end, selectedTile);
            compNavMesh.uploadToGPU();

            compNavMesh.updateMap(
                [0, 0],
                [compNavMesh.sizeX, compNavMesh.sizeY],
                compNavMesh.data)
        });

}


export function initAgent() {

    let agent = game.getEntity("navAgent").getComponent(IsoAgent);
    let pathfinder = game.getEntity("tilemapNavigation").getComponent(IsometricNavMesh);
    
    pathfinder.findPath(
        [0, 0], [20, 20]).then((p) => agent.followPath(p))

}
