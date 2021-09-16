import game from "/classic/state.js";
import { Tilemap, IsometricNavMesh, IsoAgent } from "/classic/isometric.js";
import { Rectangle, Sprite, Text } from "/classic/transforms.js";
import { Collider, Polygon } from "/classic/collision.js";
import { isoToCartesian3 } from "/classic/utils.js";

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

    game.editorTarget = "none";

    let tileSelector = game.getEntity("tilemapTileSelector");
    let tilemapEditor = game.getEntity("tilemapEditor");

    let navMesh = game.getEntity("tilemapNavigation");  
    let navMeshSelector = game.getEntity("navMeshTileSelector");
    let navMeshEditor = game.getEntity("navMeshEditor");

    let monitor = game.getEntity("selectionMonitor");
    monitor.registerCall(
        "update",
        function() {
            tileSelector.enabled = game.editorTarget == "tilemap";
            tilemapEditor.enabled = game.editorTarget == "tilemap";

            navMesh.enabled = game.editorTarget == "navMesh";
            navMeshSelector.enabled = game.editorTarget == "navMesh";
            navMeshEditor.enabled = game.editorTarget == "navMesh";
        });

}

const rectVerts =
    [[0, 0, 0],
    [ 1, 0, 0],
    [ 1, 1, 0],
    [ 0, 1, 0]];

export function initTilemapEditor() {

    let tilemap = game.getEntity("tilemap");
    let tileSelector = game.getEntity("tilemapTileSelector");
    let tilemapEditor = game.getEntity("tilemapEditor");

    let compTilemap = tilemap.getComponent(Tilemap);

    const uiBorder = 10;
    let selectedTile = 0;
    let localPos = vec3.fromValues(0, 0, 0);

    let compTilemapSprite = tilemapEditor.getComponent(Sprite);
    let compTilemapSpriteBG = tilemapEditor.getComponent(Rectangle);

    let compTileSelector = tileSelector.getComponent(Rectangle);

    let editorCollider = tilemapEditor.addComponent(
        Collider,
        new Polygon(
            game,
            [0, 0, 0],
            [...compTilemap.tileSetPixelSize, 1],
            0,
            rectVerts));

    editorCollider.clickHandler = function() {
        console.log("test");
        localPos = vec3.clone(game.mousePos);
        vec3.sub(localPos, localPos, compTilemapSprite.position);
        
        localPos[0] = Math.floor(localPos[0] / compTilemap.tilePixelSize[0]);
        localPos[1] = Math.floor(localPos[1] / compTilemap.tilePixelSize[1]);
        localPos[2] = 0;

        selectedTile = Math.min(
            compTilemap.maxTile, 
            localPos[0] + (localPos[1] * compTilemap.tileSetSize[0]));

        return true;
    }

    tilemapEditor.registerCall(
        "update",
        function() {
            // Update position of tile selector graphics
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

            compTileSelector.position = [
                compTilemapSprite.position[0] + localPos[0] * compTilemap.tilePixelSize[0],
                compTilemapSprite.position[1] + localPos[1] * compTilemap.tilePixelSize[1],
                compTileSelector.position[2]
            ];

            vec3.copy(editorCollider.position, compTilemapSprite.position);
            editorCollider.updateRect();
        });

    // Actual tilemap editing logic
    // tilemapEditor.registerCall(
    //     "selectionEnd",
    //     function() {
    //         if (editorCollider.containsPoint(game.mousePos))
    //             return;

    //         const [begin, end] = compTilemap.getSelection();
    //         if (begin[0] < 0)
    //             begin[0] = 0;

    //         if (begin[1] < 0)
    //             begin[1] = 0;

    //         if (end[0] > compTilemap.sizeX - 1)
    //             end[0] = compTilemap.sizeX - 1;

    //         if (end[1] > compTilemap.sizeY)
    //             end[1] = compTilemap.sizeY - 1;

    //         compTilemap.fillRegion(begin, end, selectedTile);
    //         compTilemap.uploadToGPU();
    //     });
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
   
    let editorCollider = navMeshEditor.addComponent(
        Collider,
        new Polygon(
            game,
            [0, 0, 0],
            [
                compNavMesh.tileSetPixelSize[0] * uiScale,
                compNavMesh.tileSetPixelSize[1] * uiScale,
                1
            ],
            0,
            rectVerts));

    editorCollider.clickHandler = function() {
        localPos = vec3.clone(game.mousePos);
        vec3.sub(localPos, localPos, compTilemapSprite.position);
        
        localPos[0] = Math.floor(localPos[0] / (compNavMesh.tilePixelSize[0] * uiScale));
        localPos[1] = Math.floor(localPos[1] / (compNavMesh.tilePixelSize[1] * uiScale));
        localPos[2] = 0;

        selectedTile = Math.min(
            compNavMesh.maxTile, 
            localPos[0] + (localPos[1] * (compNavMesh.tileSetSize[0] * uiScale)));

        return true;
    }

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

            // Update tile selector rectangle
            compTileSelector.position = vec3.clone(compTilemapSprite.position);
            vec3.add(
                compTileSelector.position,
                compTileSelector.position,
                [
                    localPos[0] * compNavMesh.tilePixelSize[0] * uiScale,
                    localPos[1] * compNavMesh.tilePixelSize[1] * uiScale,
                    0]);

            vec3.copy(editorCollider.position, compTilemapSprite.position)
            editorCollider.updateRect();
        });

    // Actual tilemap editing logic
    // navMeshEditor.registerCall(
    //     "selectionEnd",
    //     function() {
    //         if (editorCollider.containsPoint(game.mousePos))
    //             return;

    //         const [begin, end] = compNavMesh.getSelection();
    //         if (begin[0] < 0)
    //             begin[0] = 0;

    //         if (begin[1] < 0)
    //             begin[1] = 0;

    //         if (end[0] > compNavMesh.sizeX - 1)
    //             end[0] = compNavMesh.sizeX - 1;

    //         if (end[1] > compNavMesh.sizeY)
    //             end[1] = compNavMesh.sizeY - 1;

    //         compNavMesh.fillRegion(begin, end, selectedTile);
    //         compNavMesh.uploadToGPU();

    //         compNavMesh.updateMap(
    //             [0, 0],
    //             [compNavMesh.sizeX, compNavMesh.sizeY],
    //             compNavMesh.data)
    //     });

}


export function initAgent() {

    let navAgent = game.getEntity("navAgent");
    let agent = navAgent.getComponent(IsoAgent);
    let pathfinder = game.getEntity("tilemapNavigation").getComponent(IsometricNavMesh);
   
    let collider = navAgent.addComponent(
        Collider,
        new Polygon(
            game,
            [0, 0, 0], [1, 1, 1], 0, rectVerts));

    collider.clickHandler = function() {
        console.log("clicked agent");
    }

    collider.handleEnter = function(other) {
        collider.debugColor = [1, 1, 1, .2];
    }

    collider.handleExit = function(other) {
        collider.debugColor = [.1, .1, .1, .2];
    }

    navAgent.registerCall(
        "update",
        function() {
            const cPos = vec3.clone(agent.position);
            const camFix = game.camera.getFix();
            vec3.transformMat3(cPos, cPos, isoToCartesian3);
    
            cPos[0] -= agent.tilePixelSize[0] * agent.anchor[0];
            cPos[1] -= agent.tilePixelSize[1] * agent.anchor[1];
            cPos[0] *= game.camera.scale[0];
            cPos[1] *= game.camera.scale[1];
            vec3.sub(cPos, cPos, camFix);

            let scale = [
                agent.tilePixelSize[0] * game.camera.scale[0],
                agent.tilePixelSize[1] * game.camera.scale[1],
                1];
            vec3.copy(collider.position, cPos);
            vec3.copy(collider.scale, scale);
            collider.updateRect();
        });

    pathfinder.findPath(
        [1, 14], [25, 25]).then((p) => agent.followPath(p))


}

export function initDEVButtons() {

    const centeredRectVerts = [
        [-0.5, -0.5, 0],
        [ 0.5, -0.5, 0],
        [ 0.5,  0.5, 0],
        [-0.5,  0.5, 0]
    ];

    const btnDEVScale = .5;
    const btnTilemapScale = .25;
    const btnNavMeshScale = .25;
    const wiggleFactor = 24;
    const btnPixelSize = [64, 64]
    const margin = 30;

    let btnToolTilemap = game.spawnEntity("btnTilemap");
    let btnTilemap = btnToolTilemap.addComponent(
        Sprite, [-400, 0, -10], [btnTilemapScale, btnTilemapScale, 1],
        game.textures.editorIcons.name, true, 1, [4, 4], [.5, .5]);
    let btnTilemapCollider = btnToolTilemap.addComponent(
        Collider,
        new Polygon(
            game,
            [0, 0, 0],
            [btnPixelSize[0], btnPixelSize[1], 1],
            0,
            centeredRectVerts));

    btnTilemapCollider.clickHandler = function() {
        game.editorTarget = "tilemap";
        return true;
    };

    let btnToolNavMesh = game.spawnEntity("btnNavMesh");
    let btnNavMesh = btnToolNavMesh.addComponent(
        Sprite, [-400, 0, -10], [btnNavMeshScale, btnNavMeshScale, 1],
        game.textures.editorIcons.name, true, 2, [4, 4], [.5, .5]);
    let btnNavMeshCollider = btnToolNavMesh.addComponent(
        Collider,
        new Polygon(
            game,
            [0, 0, 0],
            [btnPixelSize[0], btnPixelSize[1], 1],
            0,
            centeredRectVerts));

    btnNavMeshCollider.clickHandler = function() {
        game.editorTarget = "navMesh";
        return true;
    };

    let btnDEVEntity = game.spawnEntity("btnDEV");
    let btnDEV = btnDEVEntity.addComponent(
        Sprite, [0, 0, -10], [btnDEVScale, btnDEVScale, 1],
        game.textures.editorIcons.name, true, 0, [4, 4], [0.5, 0.5]);

    let verts = [
        [.1,  .15, 0],
        [.35, .1,  0],
        [.92, .55, 0],
        [.94, .85, 0],
        [.68, .88, 0],
        [.06, .45, 0]
    ]
    for (let i = 0; i < verts.length; i++)
        vec3.sub(verts[i], verts[i], [.5, .5, 0]);

    let btnDEVCollider = btnDEVEntity.addComponent(
        Collider,
        new Polygon(
            game,
            [0, 0, 0],
            [btnPixelSize[0] * 2, btnPixelSize[1] * 2, 1],
            0,
            verts));

    // TODO: create general purpose timer class with: sine timer,
    // time since click, etc.
    let sineCounter = 0;
    let timeSinceClick = 10;

    let targetTools = btnDEV.position[0] - 300;
    let startPos = btnDEV.position[0];

    btnDEVCollider.clickHandler = function() {
        console.log("click");
        timeSinceClick = 0;
        if (targetTools == btnDEV.position[0]) {
            targetTools = btnDEV.position[0] - 300;
            startPos = btnDEV.position[0];
            game.editorTarget = "none";
        } else {
            targetTools = btnDEV.position[0];
            startPos = btnDEV.position[0] - 300;
        }
        return true;
    };

    btnDEVEntity.registerCall(
        "update",
        function() {
            btnDEV.position = [
                btnPixelSize[0],
                game.canvas.height - (btnPixelSize[1]),
                btnDEV.position[2]
            ];

            if (timeSinceClick <= 1) {
                vec3.lerp(
                    btnTilemap.position,
                    [
                        startPos,
                        game.canvas.height - (btnPixelSize[1] * 3),
                        btnTilemap.position[2]
                    ],
                    [
                        targetTools,
                        game.canvas.height - (btnPixelSize[1] * 3),
                        btnTilemap.position[2]
                    ], timeSinceClick);

                btnTilemapCollider.updateRect();

                vec3.lerp(
                    btnNavMesh.position,
                    [
                        startPos,
                        game.canvas.height - (btnPixelSize[1] * 4),
                        btnNavMesh.position[2]
                    ],
                    [
                        targetTools,
                        game.canvas.height - (btnPixelSize[1] * 4),
                        btnNavMesh.position[2]
                    ], timeSinceClick);

                btnNavMeshCollider.updateRect();
            }

            vec3.copy(btnDEVCollider.position, btnDEV.position);
            btnDEVCollider.updateRect();

            vec3.copy(btnTilemapCollider.position, btnTilemap.position);
            btnTilemapCollider.updateRect();

            vec3.copy(btnNavMeshCollider.position, btnNavMesh.position);
            btnNavMeshCollider.updateRect();

            sineCounter += game.deltaTime;
            timeSinceClick += game.deltaTime * 3;
            if (sineCounter > 1)
                sineCounter = 0;

            if (game.physics.gjk(btnDEVCollider, game.physics.mouse)) {
                let clickScale = 0;
                if (timeSinceClick < .8)
                    clickScale = Math.sin((timeSinceClick + (Math.PI / 4)) * 2) / 8;
                btnDEV.scale[0] = 
                    btnDEVScale +
                    (Math.sin(Math.PI * sineCounter) / wiggleFactor) +
                    clickScale;
                btnDEV.scale[1] = btnDEVScale + 
                    (Math.sin(Math.PI * sineCounter) / wiggleFactor) + clickScale;
            } else
                btnDEV.scale[0] = btnDEVScale;

            if (game.physics.gjk(btnTilemapCollider, game.physics.mouse)) {
                let clickScale = 0;
                if (timeSinceClick < .8)
                    clickScale = Math.sin((timeSinceClick + (Math.PI / 4)) * 2) / 8;
                btnTilemap.scale[0] = 
                    btnTilemapScale +
                    (Math.sin(Math.PI * sineCounter) / wiggleFactor) +
                    clickScale;
                btnTilemap.scale[1] = btnTilemapScale + 
                    (Math.sin(Math.PI * sineCounter) / wiggleFactor) + clickScale;
            } else
                btnTilemap.scale[0] = btnTilemapScale;

            if (game.physics.gjk(btnNavMeshCollider, game.physics.mouse)) {
                let clickScale = 0;
                if (timeSinceClick < .8)
                    clickScale = Math.sin((timeSinceClick + (Math.PI / 4)) * 2) / 8;
                btnNavMesh.scale[0] = 
                    btnNavMeshScale +
                    (Math.sin(Math.PI * sineCounter) / wiggleFactor) +
                    clickScale;
                btnNavMesh.scale[1] = btnNavMeshScale + 
                    (Math.sin(Math.PI * sineCounter) / wiggleFactor) + clickScale;
            } else
                btnNavMesh.scale[0] = btnNavMeshScale;
        });

}
