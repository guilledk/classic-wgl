import game from '/classic/state.js';
import { Tilemap, IsometricNavMesh, IsoAgent } from '/classic/isometric.js';
import { Sprite } from '/classic/transforms.js';
import { Collider, Polygon } from '/classic/collision.js';

import { vec2, vec3 } from 'gl-matrix';

declare module '/classic/types.js' {
    interface IGameState {
        editorTarget?: string;
        editorTile?: number;
        editorNavTile?: number;
        editorHeight?: number;
        heightScaleMultiplier?: number;
        heightEditMode?: string;
        agentEnabled?: boolean;
        agentSelected?: boolean;
    }
}

/** Cursor sprite follows the mouse position (ignoreCam). */
export function initCursor(): void {
    const cursor = game.getEntity('cursor')!;
    const compSprite = cursor.getComponent(Sprite)!;

    cursor.registerCall('update', function () {
        vec3.copy(compSprite.position, game.mousePos);
    });
}

/** WASD camera pan + scroll-wheel zoom. Registers an 'update' call. */
export function initCameraControllerWASD(): void {
    const camController = game.getEntity('camController')!;
    camController.registerCall('update', function () {
        if (game.isKeyDown('KeyW')) game.camera.position[1] -= game.scrollSpeed * game.deltaTime;
        if (game.isKeyDown('KeyS')) game.camera.position[1] += game.scrollSpeed * game.deltaTime;
        if (game.isKeyDown('KeyA')) game.camera.position[0] -= game.scrollSpeed * game.deltaTime;
        if (game.isKeyDown('KeyD')) game.camera.position[0] += game.scrollSpeed * game.deltaTime;

        if (Math.abs(game.mouseWheel) > 0.01) {
            game.camera.scale[0] += game.mouseWheel * game.deltaTime;
            game.camera.scale[1] += game.mouseWheel * game.deltaTime;

            vec3.max(game.camera.scale as vec3, game.camera.scale as vec3, [0.1, 0.1, 1]);
        }
    });
}

/** Attaches a world-aligned Polygon collider to the isometric tilemap for click/selection. */
export function initTilemap(): void {
    const tilemap = game.getEntity('tilemap')!;
    const compTilemap = tilemap.getComponent(Tilemap)!;

    const tilemapVerts: vec3[] = [
        vec3.fromValues(0, 0, 0),
        vec3.fromValues(1, 0, 0),
        vec3.fromValues(1, 1, 0),
        vec3.fromValues(0, 1, 0),
    ];
    for (let i = 0; i < tilemapVerts.length; i++) {
        compTilemap.isoToCartesian(tilemapVerts[i]);
    }

    const compTilemapCollider = tilemap.addComponent(
        Collider,
        new Polygon(game, [0, 0, 0], [1, 1, 1], 0, tilemapVerts),
    ) as Collider;

    tilemap.registerCall('update', function () {
        const camDelta = game.camera.getFix();
        vec3.negate(camDelta, camDelta);
        vec3.copy(compTilemapCollider.position, camDelta);

        vec3.copy(compTilemapCollider.scale, [
            compTilemap.mapSize[0] * game.camera.scale[0],
            compTilemap.mapSize[1] * game.camera.scale[1],
            1,
        ]);
        compTilemapCollider.updateRect();
    });
}

const rectVerts: [number, number, number][] = [
    [0, 0, 0],
    [1, 0, 0],
    [1, 1, 0],
    [0, 1, 0],
];

/**
 * Registers the tilemap fill-on-selection handler only.
 * The visual tile palette (UISprite tileset + tile selector +
 * click-to-choose-tile) lives in uiPrefabs.ts `initTilePalette`.
 */
export function initTilemapEditorLogic(): void {
    const tilemap = game.getEntity('tilemap')!;
    const compTilemap = tilemap.getComponent(Tilemap)!;
    const compTilemapCollider = tilemap.getComponent(Collider)!;

    compTilemapCollider.addHandler('selection', function () {
        if (game.editorTarget !== 'tilemap') return;
        const [begin, end] = compTilemap.getSelection();

        vec2.max(begin, begin, [0, 0]);
        vec2.min(end, end, compTilemap.mapSize as vec2);

        compTilemap.fillRegion(begin, end, game.editorTile ?? 0);
        compTilemap.uploadToGPU();
    });
}

/**
 * Registers the nav-mesh fill-on-selection handler only.
 * The visual nav-tile palette (UISprite navTileset + selector +
 * click-to-choose-tile) lives in uiPrefabs.ts `initNavPalette`.
 */
export function initNavMeshEditorLogic(): void {
    const navMesh = game.getEntity('tilemapNavigation')!;
    const compNavMesh = navMesh.getComponent(IsometricNavMesh)!;

    const tilemap = game.getEntity('tilemap')!;
    const compTilemapCollider = tilemap.getComponent(Collider)!;

    compTilemapCollider.addHandler('selection', function () {
        if (game.editorTarget !== 'navMesh') return;
        const [begin, end] = compNavMesh.getSelection();

        vec2.max(begin, begin, [0, 0]);
        vec2.min(end, end, compNavMesh.mapSize as vec2);

        compNavMesh.fillRegion(begin, end, game.editorNavTile ?? 0);
        compNavMesh.uploadToGPU();

        compNavMesh.updateMap([0, 0], [compNavMesh.sizeX, compNavMesh.sizeY], compNavMesh.data);
    });
}

/**
 * Registers the height-fill-on-selection handler.
 * The height value widget (UISprite +/- buttons + label) lives in
 * uiPrefabs.ts `initHeightWidget`.
 */
export function initHeightEditorLogic(): void {
    const tilemap = game.getEntity('tilemap')!;
    const compTilemap = tilemap.getComponent(Tilemap)!;
    const compTilemapCollider = tilemap.getComponent(Collider)!;

    compTilemapCollider.addHandler('selection', function () {
        if (game.editorTarget !== 'height') return;
        const [begin, end] = compTilemap.getSelection();

        vec2.max(begin, begin, [0, 0]);
        vec2.min(end, end, compTilemap.mapSize as vec2);

        const val = game.editorHeight ?? 0;
        const isSet = game.heightEditMode === 'set';

        console.log(
            '[height]',
            isSet ? 'set' : 'blend',
            [begin[0], begin[1]],
            'to',
            [end[0], end[1]],
            'value',
            val,
        );

        for (let y = begin[1]; y < end[1]; y++) {
            for (let x = begin[0]; x < end[0]; x++) {
                if (isSet) {
                    compTilemap.setHeight(x, y, Math.max(0, val));
                } else {
                    const idx = x + compTilemap.sizeX * y;
                    const cur = compTilemap.heightData[idx] ?? 0;
                    compTilemap.setHeight(x, y, Math.max(0, cur + val));
                }
            }
        }
    });
}

/** Generates demo slope patterns in the centre of the tilemap to showcase height features. */
export function generateDemoSlopes(): void {
    const tm = game.getEntity('tilemap')?.getComponent(Tilemap);
    if (!tm) return;

    const cx = 80;
    const cy = 80;

    // Flat platform (height 3)
    for (let y = cy; y < cy + 10; y++) for (let x = cx; x < cx + 10; x++) tm.setHeight(x, y, 3);

    // Pyramid steps (1→5→1)
    for (let y = cy - 5; y < cy + 15; y++) {
        for (let x = cx + 12; x < cx + 22; x++) {
            const dx = x - (cx + 17);
            const dy = y - (cy + 5);
            const dist = Math.max(Math.abs(dx), Math.abs(dy));
            tm.setHeight(x, y, Math.max(0, 4 - dist));
        }
    }

    // Ramp going SE
    for (let i = 0; i < 12; i++) tm.setHeight(cx + 24 + i, cy + i, Math.floor(i / 3) + 1);

    // Ramp going SW
    for (let i = 0; i < 12; i++) tm.setHeight(cx + 24 + i, cy + 14 - i, Math.floor(i / 3) + 1);

    // Valley (height 0 depression surrounded by height 2)
    for (let y = cy + 20; y < cy + 30; y++)
        for (let x = cx; x < cx + 10; x++) tm.setHeight(x, y, y === cy + 25 ? 0 : 3);

    // Alternating plateau
    for (let y = cy + 20; y < cy + 30; y++)
        for (let x = cx + 12; x < cx + 22; x++) tm.setHeight(x, y, (x + y) % 2 === 0 ? 5 : 1);

    console.log('[demo] slope terrain generated');
}

/** Click-to-pathfind IsoAgent: attaches collider + click handler on the tilemap. */
export function initAgent(): void {
    const tilemap = game.getEntity('tilemap')!.getComponent(Tilemap)!;
    const navAgent = game.getEntity('navAgent')!;
    const agent = navAgent.getComponent(IsoAgent)!;
    const pathfinder = game.getEntity('tilemapNavigation')!.getComponent(IsometricNavMesh)!;

    const collider = navAgent.addComponent(
        Collider,
        new Polygon(game, [0, 0, 0], [1, 1, 1], 0, rectVerts),
    ) as Collider;

    collider.addHandler('enter', function () {
        // collider.debugColor = [1, 1, 1, .2];
    });

    collider.addHandler('exit', function () {
        // collider.debugColor = [.1, .1, .1, .2];
    });

    navAgent.registerCall('update', function () {
        const cPos = vec3.clone(agent.position);
        tilemap.isoToCartesian(cPos);

        cPos[0] -= agent.tilePixelSize[0] * (agent.anchor[0] as number);
        cPos[1] -= agent.tilePixelSize[1] * (agent.anchor[1] as number);
        cPos[0] *= game.camera.scale[0];
        cPos[1] *= game.camera.scale[1];
        vec3.sub(cPos, cPos, game.camera.getFix());

        const scale: vec3 = [
            agent.tilePixelSize[0] * game.camera.scale[0],
            agent.tilePixelSize[1] * game.camera.scale[1],
            1,
        ];
        vec3.copy(collider.position, cPos);
        vec3.copy(collider.scale, scale);
        collider.updateRect();
    });

    const compTilemapCollider = tilemap.entity.getComponent(Collider)!;

    game.agentEnabled = true;
    game.agentSelected = false;

    // Init agent height to terrain surface
    const aTx = Math.floor(agent.position[0]);
    const aTy = Math.floor(agent.position[1]);
    const aIdx =
        Math.min(aTx, tilemap.sizeX - 1) + Math.min(aTy, tilemap.sizeY - 1) * tilemap.sizeX;
    agent.position[2] = (tilemap.heightData[aIdx] ?? 0) * tilemap.heightScale;

    compTilemapCollider.addHandler('click', function () {
        if (!game.agentEnabled) return;

        const clickX = tilemap.mouseIsoPos[0];
        const clickY = tilemap.mouseIsoPos[1];
        const agentX = agent.position[0];
        const agentY = agent.position[1];

        // Click very close to agent → toggle selection
        if (Math.hypot(clickX - agentX, clickY - agentY) < 0.8) {
            game.agentSelected = !(game.agentSelected ?? false);
            return true;
        }

        // Click elsewhere → move agent if selected
        if (!game.agentSelected) return;

        const start: [number, number] = [agentX, agentY];
        const end: [number, number] = [clickX, clickY];

        if (vec2.dist(start, end) > 2) {
            pathfinder.findPath(start, end).then((p) => {
                if (p != null) agent.followPath(p as [number, number][]);
            });
        }
    });

    // Selection ring and P-key toggle
    navAgent.registerCall('update', function () {
        if (game.wasKeyPressed('KeyP')) {
            game.agentEnabled = !(game.agentEnabled ?? true);
            console.log('[agent] movement', game.agentEnabled ? 'enabled' : 'disabled');
        }
    });
}
