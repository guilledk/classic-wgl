import game from '/classic/state.js';
import { Tilemap, IsometricNavMesh } from '/classic/isometric.js';
import { UIManager, UIText, UISprite, UIElement, UIContainer } from '/classic/ui.js';
import { Collider } from '/classic/collision.js';
import { vec3 } from 'gl-matrix';

declare module '/classic/types.js' {
    interface IGameState {
        editorTarget?: string;
        editorTile?: number;
        editorNavTile?: number;
    }
}

let _tilePalette: UIContainer | null = null;
let _navPalette: UIContainer | null = null;
let _uiScale = 1;

/**
 * Entry point for all UI-system components.
 * Creates the UIManager, computes a viewport-based scale factor, then
 * assembles the top bar, tool buttons, tile/nav palettes, and mode control.
 */
export function initUI(): void {
    const UI = new UIManager(game);
    _uiScale = Math.max(1, Math.min(3, (game.canvas?.height ?? 1080) / 1080));

    game.editorTarget = 'none';
    game.editorTile = 0;
    game.editorNavTile = 0;

    initTopBar(UI);
    initToolButtons(UI);
    _tilePalette = initTilePalette(UI);
    _navPalette = initNavPalette(UI);
    initEditorModeControl(UI);
}

/** Top bar: FPS counter (left), title (center), controls hint (right). */
function initTopBar(UI: UIManager): void {
    const barH = Math.round(32 * _uiScale);
    const textScale = 0.4 * _uiScale;
    const titleScale = 0.6 * _uiScale;

    const topBar = UI.spawnContainer(UI.root.width, barH, [0, 0, 0, 0.5]);
    UI.root.addChild(topBar, 'top-center', 'top-center');

    const fpsText = UI.spawnText('FPS', textScale, 100, [0, 0.6, 0, 1], [0, 0, 0, 0]);
    topBar.addChild(fpsText, 'mid-left', 'mid-left');

    const title = UI.spawnText('CLASSIC WGL', titleScale, 400, [1, 0.53, 0.3, 1], [0, 0, 0, 0]);
    topBar.addChild(title, 'mid-center', 'mid-center');

    const infoText = UI.spawnText(
        'WASD MOVE | SCROLL ZOOM',
        textScale,
        400,
        [1, 0.2, 0.6, 1],
        [0, 0, 0, 0],
    );
    topBar.addChild(infoText, 'mid-right', 'mid-right');

    let lastFPS = 0;
    let timeAccumulator = 0;
    UI.root.entity.registerCall('update', () => {
        timeAccumulator += game.deltaTime;
        if (timeAccumulator >= 0.2) {
            lastFPS = game.fps;
            fpsText.setText(lastFPS.toString());
            timeAccumulator = 0;
        }
        if (lastFPS >= 30) {
            fpsText.setTextColor([0, 0.6, 0, 1]);
        } else {
            fpsText.setTextColor([0.8, 0, 0, 1]);
        }
    });

    UI.root.entity.registerCall('canvasResize', () => {
        topBar.setSize(UI.root.width, barH);
    });
}

/**
 * Bottom-left tool buttons: DEV toggle + tilemap/navmesh mode selectors.
 * Replicates the old raw-Sprite slide-out animation (sine wiggle on hover,
 * lerp slide, click pulse) using UISprite + UI.addColliderToElem.
 */
function initToolButtons(UI: UIManager): void {
    const btnPixel = Math.round(64 * _uiScale);
    const slideDist = Math.round(300 * _uiScale);
    const wiggleFactor = 24;

    function makeBtn(
        texture: string,
        w: number,
        h: number,
        frame: number,
        tileSetSize: [number, number],
    ): { container: UIContainer; sprite: UISprite; collider: Collider } {
        const container = UI.spawnContainer(w, h, [0, 0, 0, 0]);
        const sprite = UI.spawnSprite(texture, w, h, frame, tileSetSize);
        container.addChild(sprite, 'mid-center', 'mid-center');
        const collider = UI.addColliderToElem(container);
        return { container, sprite, collider };
    }

    const dev = makeBtn('editorIcons', btnPixel, btnPixel, 0, [4, 4]);
    const tile = makeBtn('editorIcons', btnPixel, btnPixel, 1, [4, 4]);
    const nav = makeBtn('editorIcons', btnPixel, btnPixel, 2, [4, 4]);

    let sineCounter = 0;
    let timeSinceClick = 10;
    let isOpen = false;

    dev.collider.addHandler('click', () => {
        timeSinceClick = 0;
        isOpen = !isOpen;
        if (!isOpen) game.editorTarget = 'none';
        return true;
    });

    tile.collider.addHandler('click', () => {
        game.editorTarget = 'tilemap';
        return true;
    });

    nav.collider.addHandler('click', () => {
        game.editorTarget = 'navMesh';
        return true;
    });

    UI.root.entity.registerCall('update', () => {
        const ch = game.canvas!.height;
        const wiggle = Math.sin(Math.PI * sineCounter) / wiggleFactor;
        const clickPulse =
            timeSinceClick < 0.8 ? Math.sin((timeSinceClick + Math.PI / 4) * 2) / 8 : 0;

        const half = btnPixel / 2;
        const openX = btnPixel - half;
        const closedX = btnPixel - slideDist - half;
        const tileY = ch - btnPixel * 3 - half;
        const navY = ch - btnPixel * 4 - half;

        dev.container.setPosition(btnPixel - half, ch - btnPixel - half);

        const t = Math.min(timeSinceClick, 1);
        const targetX = isOpen ? openX : closedX;

        const tileCur = tile.container.position[0];
        const navCur = nav.container.position[0];
        if (timeSinceClick <= 1) {
            tile.container.setPosition(tileCur + (targetX - tileCur) * t, tileY);
            nav.container.setPosition(navCur + (targetX - navCur) * t, navY);
        } else {
            tile.container.setPosition(targetX, tileY);
            nav.container.setPosition(targetX, navY);
        }

        const applyHover = (sprite: UISprite, collider: Collider) => {
            if (game.physics!.gjk(collider, game.physics!.mouse)) {
                const s = btnPixel + (wiggle + clickPulse) * btnPixel;
                sprite.setSize(s, s);
            } else {
                sprite.setSize(btnPixel, btnPixel);
            }
        };

        applyHover(dev.sprite, dev.collider);
        applyHover(tile.sprite, tile.collider);
        applyHover(nav.sprite, nav.collider);

        sineCounter = (sineCounter + game.deltaTime) % 1;
        timeSinceClick += game.deltaTime * 3;
    });
}

/**
 * Bottom-right tile-set palette. Shows the full tileset via UISprite,
 * click maps pixel coords → tile index → game.editorTile.
 * The fill-on-selection logic lives in prefabs.ts `initTilemapEditorLogic`.
 * Starts disabled; enabled when game.editorTarget === 'tilemap'.
 */
function initTilePalette(UI: UIManager): UIContainer {
    const compTilemap = game.getEntity('tilemap')!.getComponent(Tilemap)!;
    const tSize = compTilemap.tilePixelSize;
    const tsSize = compTilemap.tileSetSize;
    const tsPixelSize = compTilemap.tileSetPixelSize;
    const maxTile = compTilemap.maxTile;
    const uiBorder = Math.round(10 * _uiScale);

    const paletteW = tsPixelSize[0];
    const paletteH = tsPixelSize[1];

    const container = UI.spawnContainer(paletteW, paletteH, [0, 0, 0, 0.2]);
    const sprite = UI.spawnSprite('tileSet', paletteW, paletteH, 0, [1, 1]);
    container.addChild(sprite, 'top-left', 'top-left');

    const selector = UI.spawnElement(tSize[0], tSize[1], [1, 1, 1, 0.3]);
    container.addChild(selector, 'top-left', 'top-left');

    const collider = UI.addColliderToElem(container);

    let localX = 0;
    let localY = 0;

    collider.addHandler('click', () => {
        const mouseLocal = vec3.clone(game.mousePos);
        vec3.sub(mouseLocal, mouseLocal, container.position);

        localX = Math.floor(mouseLocal[0] / tSize[0]);
        localY = Math.floor(mouseLocal[1] / tSize[1]);

        game.editorTile = Math.min(maxTile, localX + localY * tsSize[0]);

        return true;
    });

    UI.root.entity.registerCall('update', () => {
        container.setPosition(
            game.canvas!.width - paletteW - uiBorder,
            game.canvas!.height - paletteH - uiBorder,
        );

        selector.setPosition(
            container.position[0] + localX * tSize[0],
            container.position[1] + localY * tSize[1],
        );
    });

    container.setEnabled(false);
    return container;
}

/**
 * Bottom-right nav-tile palette (4x display scale, same as old prefab).
 * Click maps pixel coords → tile index → game.editorNavTile.
 * The fill-on-selection logic lives in prefabs.ts `initNavMeshEditorLogic`.
 * Starts disabled; enabled when game.editorTarget === 'navMesh'.
 */
function initNavPalette(UI: UIManager): UIContainer {
    const navMesh = game.getEntity('tilemapNavigation')!.getComponent(IsometricNavMesh)!;
    const tSize = navMesh.tilePixelSize;
    const tsSize = navMesh.tileSetSize;
    const tsPixelSize = navMesh.tileSetPixelSize;
    const maxTile = navMesh.maxTile;
    const uiBorder = Math.round(10 * _uiScale);
    const uiScale = 4;

    const paletteW = tsPixelSize[0] * uiScale;
    const paletteH = tsPixelSize[1] * uiScale;

    const container = UI.spawnContainer(paletteW, paletteH, [0, 0, 0, 0.2]);
    const sprite = UI.spawnSprite('navTileset', paletteW, paletteH, 0, [1, 1]);
    container.addChild(sprite, 'top-left', 'top-left');

    const selector = UI.spawnElement(tSize[0] * uiScale, tSize[1] * uiScale, [1, 1, 1, 0.3]);
    container.addChild(selector, 'top-left', 'top-left');

    const collider = UI.addColliderToElem(container);

    let localX = 0;
    let localY = 0;

    collider.addHandler('click', () => {
        const mouseLocal = vec3.clone(game.mousePos);
        vec3.sub(mouseLocal, mouseLocal, container.position);

        localX = Math.floor(mouseLocal[0] / (tSize[0] * uiScale));
        localY = Math.floor(mouseLocal[1] / (tSize[1] * uiScale));

        game.editorNavTile = Math.min(maxTile, localX + localY * tsSize[0]);

        return true;
    });

    UI.root.entity.registerCall('update', () => {
        container.setPosition(
            game.canvas!.width - paletteW - uiBorder,
            game.canvas!.height - paletteH - uiBorder,
        );

        selector.setPosition(
            container.position[0] + localX * tSize[0] * uiScale,
            container.position[1] + localY * tSize[1] * uiScale,
        );
    });

    container.setEnabled(false);
    return container;
}

/** Toggles palette/nav-mesh visibility each frame based on game.editorTarget. */
function initEditorModeControl(UI: UIManager): void {
    const navMeshEntity = game.getEntity('tilemapNavigation')!;

    UI.root.entity.registerCall('update', () => {
        if (_tilePalette) _tilePalette.setEnabled(game.editorTarget === 'tilemap');
        if (_navPalette) _navPalette.setEnabled(game.editorTarget === 'navMesh');
        navMeshEntity.enabled = game.editorTarget === 'navMesh';
    });
}
