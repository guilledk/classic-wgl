import game from "/classic/state.js"

import { Rectangle, Sprite, Text } from "/classic/transforms.js";
import { Tilemap,  IsoSprite } from "/classic/isometric.js";
import { Animator } from "/classic/animator.js";
import { cartesianToIso3, isoToCartesian3 } from "/classic/utils.js";

import { vec2, vec3 } from "/lib/gl-matrix/index.js";


async function initContext() {

    window.game = game;

    await game.init();
    await game.loadResources();

    await game.load("state.json");

    var camController = game.getEntity("camController");
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
    
    const tileMax = 32;
    const tileSize = [32, 32];
    const tileSetSize = [
        game.textures.tileSet.image.width,
        game.textures.tileSet.image.height
    ];
    var tileSetSizeTiles = [
        tileSetSize[0] / tileSize[0],
        tileSetSize[1] / tileSize[1]
    ];
    var tilemap = game.getEntity("tilemap");
    var compTilemap = tilemap.getComponent(Tilemap);
    compTilemap.uploadToGPU();

    window.tmap = compTilemap;

    const uiBorder = 10;
    var selectedTile = 0;

    var tilemapEditor = game.getEntity("tilemapEditor");
    var compTilemapSprite = tilemapEditor.getComponent(Sprite);

    var compTilemapSpriteBackground = tilemapEditor.getComponent(Rectangle);

    compTilemapSprite.position = vec3.fromValues(
        game.canvas.width - tileSetSize[0] - uiBorder,
        game.canvas.height - tileSetSize[1] - uiBorder,
        -10
    );

    vec3.copy(compTilemapSpriteBackground.position, compTilemapSprite.position);
    compTilemapSpriteBackground.scale = [
        tileSetSize[0],
        tileSetSize[1],
        1];
   
    var isMouseOverEditor = false;
    var localPos = vec3.fromValues(0, 0, 0);

    tilemapEditor.registerCall(
        "update",
        function() {
            isMouseOverEditor = game.mousePos[0] >= compTilemapSprite.position[0] &&
                game.mousePos[1] >= compTilemapSprite.position[1] &&
                game.mousePos[0] <= compTilemapSprite.position[0] + tileSetSize[0] &&
                game.mousePos[1] <= compTilemapSprite.position[1] + tileSetSize[1];
            
            if (game.wasMouseButtonPressed(0) && isMouseOverEditor) {
                // inside tile selector
                localPos = vec3.clone(game.mousePos);
                vec3.sub(localPos, localPos, compTilemapSprite.position);
                
                localPos[0] = Math.floor(localPos[0] / tileSize[0]);
                localPos[1] = Math.floor(localPos[1] / tileSize[1]);
                localPos[2] = 0;

                selectedTile = Math.min(
                    tileMax,
                    localPos[0] + (localPos[1] * tileSetSizeTiles[0]));
            }
        });
    
    var tileSelector = game.getEntity("tileSelector");
    var compTileSelector = tilemapEditor.getComponent(Rectangle);

    tileSelector.registerCall(
        "update",
        function() {
            compTileSelector.position = vec3.clone(compTilemapSprite.position);
            vec3.add(
                compTileSelector.position,
                compTileSelector.position,
                [localPos[0] * tileSize[0], localPos[1] * tileSize[1], 0]);
        });

    tilemapEditor.registerCall(
        "selectionEnd",
        function() {
            if (isMouseOverEditor)
                return;

            const [begin, end] = compTilemap.getSelection();
            if (begin[0] < 0 || begin[1] < 0)
                return;

            if (end[1] > compTilemap.sizeX || end[1] > compTilemap.sizeY)
                return;

            compTilemap.fillRegion(begin, end, selectedTile);
            compTilemap.uploadToGPU();
        });


    var fpsCounter = game.getEntity("fpsCounter");
    var compFPSText = fpsCounter.getComponent(Text);

    fpsCounter.registerCall(
        "update",
        function() {
            compFPSText.setText(game.fps.toString());
        });

    var label1 = game.getEntity("textLabel1");
    var label1Text = label1.getComponent(Text);
    
    label1Text.setText("CLASSIC ENGINE V0.1A0 ;)");

    var label2 = game.getEntity("textLabel2");
    var label2Text = label2.getComponent(Text);
    
    label2Text.setText("SCROLL WHEEL TO ZOOM, DRAG TO SELECT TILES");

    var semaphore01 = game.getEntity("semaphore01");
    var compSemaphore01Sprite = semaphore01.getComponent(IsoSprite);
    compSemaphore01Sprite.anchor = [0.125, .975];

    var semaphore02 = game.getEntity("semaphore02");
    var compSemaphore02Sprite = semaphore02.getComponent(IsoSprite);
    compSemaphore02Sprite.anchor = [.5, .975];

    // var humanoid = game.getEntity("humanoid");
    // var compHumanSprite = humanoid.addComponent(
    //     Sprite,
    //     [0, 0, -10], [.25, .5, 1],
    //     game.textures.humanoid,
    //     false);
    // compHumanSprite.tileSetSize = [32, 16];
    // compHumanSprite.anchor = [.5, .875];

    // var compHumanAnimator = humanoid.addComponent(Animator, compHumanSprite);
    // const anims = [
    //     "walkSouth",
    //     "walkSouthEast",
    //     "walkEast",
    //     "walkNorthEast",
    //     "walkNorth",
    //     "walkNorthWest",
    //     "walkWest",
    //     "walkSouthWest"
    // ];

    // humanoid.registerCall(
    //     "update",
    //     function() {
    //         let isoPos = vec3.clone(compHumanSprite.position);
    //         vec3.transformMat3(isoPos, isoPos, cartesianToIso3);

    //         let deltaX = isoPos[0] - game.mouseIsoPos[0];
    //         let deltaY = isoPos[1] - game.mouseIsoPos[1];
    //         let radians = Math.atan2(deltaX,  deltaY);
    //         let angle = ((radians * 180) / Math.PI) + 180;

    //         let index = Math.floor(angle / 45.0);
    //          
    //         compHumanAnimator.play(game.animations[anims[index]], true);
    //     });

    var cursor = game.getEntity("cursor");
    var compSprite = cursor.getComponent(Sprite);

    cursor.registerCall(
        "update",
        function () {
            vec3.copy(compSprite.position, game.mousePos);
        });

    game.launch();

    const mapSize = [100, 100];
    const mapData = new Array(mapSize[0] * mapSize[1]).fill(true);

    var pathfinder = new Worker("/classic/pathfinder.js");
    pathfinder.onmessage = msg => console.log(msg.data);
    pathfinder.postMessage({
        op: "initmap",
        args: { 
            name: "test",
            size: mapSize,
            data: mapData 
        }
    });
    pathfinder.postMessage({
        op: "findpath",
        args: {
            name: "test",
            from: [0, 0],
            to: [99, 99]
        }
    });
}

window.addEventListener("load", initContext, false);
