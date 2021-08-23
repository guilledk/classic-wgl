import game from "/classic/state.js"

import { Sprite, Tilemap, Text } from "/classic/transforms.js";

import { vec3 } from "/lib/gl-matrix/index.js";


async function initContext() {

    await game.init();
    await game.loadResources();

    game.camera.position = [2000, 0, 1];
    
    var tilemap = game.spawnEntity("tilemap");
    var compTilemap = tilemap.addComponent(
        Tilemap,
        [0, 0, -100], [1, 1, 1],
        1000, 1000,  // size in tiles
        [64, 64],    // map pixel size
        game.textures.tileSet,
        [16, 16],    // tileset tile pixel size
        10);          // max tile flat id
    compTilemap.uploadToGPU();

    var cursor = game.spawnEntity("cursor");
    var compSprite = cursor.addComponent(
        Sprite,
        [0, 0, 0], [32, 32, 1],
        game.textures.cursor,
        true);

    cursor.registerCall(
        "update",
        function () {
            vec3.copy(compSprite.position, game.mousePos);
        });

    var fpsCounter = game.spawnEntity("fpsCounter");
    var compFPSText = fpsCounter.addComponent(
        Text,
        [0, 0, 0], [1, 1, 1],
        game.textures.font,
        [3, 1],  // max char size
        [16, 16],   // font size
        [32, 32],   // glyph pixel size
        // glyph str
        "!\"#$%&\'()*+,-./?0123456789:;<=>@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`{|}~",
        [1, 1, 1, 1],  // color
        [0, 0, 0, 1],  // bgcolor
        true);     // ignore cam

    fpsCounter.registerCall(
        "update",
        function() {
            compFPSText.setText(game.fps.toString());
        });

    var label1 = game.spawnEntity("textLabel1");
    var label1Text = label1.addComponent(
        Text,
        [100, 0, 0], [.8, .8, 1],
        game.textures.font,
        [24, 1],  // max char size
        [16, 16],   // font size
        [32, 32],   // glyph pixel size
        // glyph str
        "!\"#$%&\'()*+,-./?0123456789:;<=>@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`{|}~",
        [1.00,0.53,0.30, 1],  // color
        [0, 0, 0, 1],  // bgcolor
        true);     // ignore cam
    
    label1Text.setText("CLASSIC ENGINE V0.1A0 ;)");

    var label2 = game.spawnEntity("textLabel2");
    var label2Text = label1.addComponent(
        Text,
        [100, 80, 0], [.5, .5, 1],
        game.textures.font,
        [42, 1],  // max char size
        [16, 16],   // font size
        [32, 32],   // glyph pixel size
        // glyph str
        "!\"#$%&\'()*+,-./?0123456789:;<=>@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`{|}~",
        [1.00,0.20,0.60, 1],  // color
        [0.30,0.00,0.15, 1],  // bgcolor
        true);     // ignore cam
    
    label2Text.setText("SCROLL WHEEL TO ZOOM, DRAG TO SELECT TILES");

    game.launch();

    const mapSize = [5, 5];
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
            to: [4, 4]
        }
    });


    // font = new Text(
    //     [100, 0, 0], [.8, .8, 1],
    //     textures.font,
    //     [24, 1],  // max char size
    //     [16, 16],   // font size
    //     [32, 32],   // glyph pixel size
    //     // glyph str
    //     "!\"#$%&\'()*+,-./?0123456789:;<=>@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`{|}~",
    //     [1.00,0.53,0.30, 1],  // color
    //     true);     // ignore cam
    // font.setText("CLASSIC ENGINE V0.1A0 ;)");
}

window.addEventListener("load", initContext, false);
