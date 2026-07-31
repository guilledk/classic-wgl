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
        editorHeight?: number;
        heightScaleMultiplier?: number;
        heightEditMode?: string;
        agentSelected?: boolean;
    }
}

let _tilePalette: UIContainer | null = null;
let _navPalette: UIContainer | null = null;
let _heightWidget: UIContainer | null = null;
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
    game.editorHeight = 0;
    game.heightScaleMultiplier = 1;
    game.heightEditMode = 'blend';

    initTopBar(UI);
    initToolButtons(UI);
    _tilePalette = initTilePalette(UI);
    _navPalette = initNavPalette(UI);
    _heightWidget = initHeightWidget(UI);
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

    // Height button: uses a coloured container + text label instead of
    // editorIcons sprite because frame 3 of that spritesheet is blank.
    const heightContainer = UI.spawnContainer(btnPixel, btnPixel, [0.2, 0.4, 0.8, 1]);
    const heightLabel = UI.spawnText('H', 0.6 * _uiScale, 100, [1, 1, 1, 1], [0, 0, 0, 0]);
    heightContainer.addChild(heightLabel, 'mid-center', 'mid-center');
    const heightCollider = UI.addColliderToElem(heightContainer);

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

    heightCollider.addHandler('click', () => {
        game.editorTarget = 'height';
        return true;
    });

    // Agent selection indicator: small [A] button, always visible
    const agentInd = Math.round(48 * _uiScale);
    const agentBox = UI.spawnContainer(agentInd, agentInd, [0.1, 0.6, 0.1, 0.8]);
    const agentTxt = UI.spawnText('A', 0.55 * _uiScale, 100, [1, 1, 1, 1], [0, 0, 0, 0]);
    agentBox.addChild(agentTxt, 'mid-center', 'mid-center');
    const agentCol = UI.addColliderToElem(agentBox);

    agentCol.addHandler('click', () => {
        game.agentSelected = !(game.agentSelected ?? true);
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
        const heightY = ch - btnPixel * 5 - half;

        dev.container.setPosition(btnPixel - half, ch - btnPixel - half);

        const t = Math.min(timeSinceClick, 1);
        const targetX = isOpen ? openX : closedX;

        const tileCur = tile.container.position[0];
        const navCur = nav.container.position[0];
        const heightCur = heightContainer.position[0];
        if (timeSinceClick <= 1) {
            tile.container.setPosition(tileCur + (targetX - tileCur) * t, tileY);
            nav.container.setPosition(navCur + (targetX - navCur) * t, navY);
            heightContainer.setPosition(heightCur + (targetX - heightCur) * t, heightY);
        } else {
            tile.container.setPosition(targetX, tileY);
            nav.container.setPosition(targetX, navY);
            heightContainer.setPosition(targetX, heightY);
        }

        const applyHover = (sprite: UISprite | null, collider: Collider) => {
            if (game.physics!.gjk(collider, game.physics!.mouse)) {
                if (sprite != null) {
                    const s = btnPixel + (wiggle + clickPulse) * btnPixel;
                    sprite.setSize(s, s);
                }
            } else {
                if (sprite != null) {
                    sprite.setSize(btnPixel, btnPixel);
                }
            }
        };

        applyHover(dev.sprite, dev.collider);
        applyHover(tile.sprite, tile.collider);
        applyHover(nav.sprite, nav.collider);
        applyHover(null, heightCollider);

        // Agent indicator: bottom-left, below dev button
        const selected = game.agentSelected ?? true;
        agentBox.color = selected ? [0.1, 0.7, 0.1, 0.8] : [0.3, 0.3, 0.3, 0.6];
        agentBox.setPosition(agentInd / 2, ch - btnPixel * 2 - agentInd / 2);

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

/** Height editing tool widget: value row + scale row + set/blend toggle. */
function initHeightWidget(UI: UIManager): UIContainer {
    const btnSize = Math.round(32 * _uiScale);
    const labelW = Math.round(50 * _uiScale);
    const gap = Math.round(4 * _uiScale);
    const widgetW = btnSize * 2 + labelW + Math.round(12 * _uiScale);
    const rowH = btnSize;
    const widgetH = rowH * 3 + gap * 4;
    const uiBorder = Math.round(10 * _uiScale);

    function updateHeightScale(): void {
        const tm = game.getEntity('tilemap')?.getComponent(Tilemap);
        if (tm) {
            tm.heightScale = tm.tilePixelSize[0] * (game.heightScaleMultiplier ?? 1);
            tm._meshDirty = true;
        }
    }

    const container = UI.spawnContainer(widgetW, widgetH, [0, 0, 0, 0.4]);

    // Row 1: height delta value
    const hMinus = UI.spawnContainer(btnSize, btnSize, [0.6, 0.1, 0.1, 1]);
    const hMinusTxt = UI.spawnText('-', 0.6 * _uiScale, 100, [1, 1, 1, 1], [0, 0, 0, 0]);
    hMinus.addChild(hMinusTxt, 'mid-center', 'mid-center');
    container.addChild(hMinus, 'top-left', 'top-left');

    const hLabel = UI.spawnText('0', 0.5 * _uiScale, 60, [1, 1, 1, 1], [0, 0, 0, 0]);
    container.addChild(hLabel, 'top-left', 'top-left');

    const hPlus = UI.spawnContainer(btnSize, btnSize, [0.1, 0.6, 0.1, 1]);
    const hPlusTxt = UI.spawnText('+', 0.6 * _uiScale, 100, [1, 1, 1, 1], [0, 0, 0, 0]);
    hPlus.addChild(hPlusTxt, 'mid-center', 'mid-center');
    container.addChild(hPlus, 'top-left', 'top-left');

    // Row 2: height scale multiplier
    const sMinus = UI.spawnContainer(btnSize, btnSize, [0.1, 0.1, 0.6, 1]);
    const sMinusTxt = UI.spawnText('s-', 0.45 * _uiScale, 100, [1, 1, 1, 1], [0, 0, 0, 0]);
    sMinus.addChild(sMinusTxt, 'mid-center', 'mid-center');
    container.addChild(sMinus, 'top-left', 'top-left');

    const sLabel = UI.spawnText('x1', 0.45 * _uiScale, 60, [1, 1, 1, 1], [0, 0, 0, 0]);
    container.addChild(sLabel, 'top-left', 'top-left');

    const sPlus = UI.spawnContainer(btnSize, btnSize, [0.1, 0.1, 0.6, 1]);
    const sPlusTxt = UI.spawnText('s+', 0.45 * _uiScale, 100, [1, 1, 1, 1], [0, 0, 0, 0]);
    sPlus.addChild(sPlusTxt, 'mid-center', 'mid-center');
    container.addChild(sPlus, 'top-left', 'top-left');

    // Row 3: set / blend mode toggle
    const modeBtn = UI.spawnContainer(widgetW, rowH, [0.2, 0.2, 0.2, 1]);
    const modeTxt = UI.spawnText('blend', 0.4 * _uiScale, 100, [1, 1, 1, 1], [0, 0, 0, 0]);
    modeBtn.addChild(modeTxt, 'mid-center', 'mid-center');
    container.addChild(modeBtn, 'top-left', 'top-left');

    // Colliders
    const hMinusCol = UI.addColliderToElem(hMinus);
    const hPlusCol = UI.addColliderToElem(hPlus);
    const sMinusCol = UI.addColliderToElem(sMinus);
    const sPlusCol = UI.addColliderToElem(sPlus);
    const modeCol = UI.addColliderToElem(modeBtn);

    hMinusCol.addHandler('click', () => {
        game.editorHeight = (game.editorHeight ?? 0) - 1;
        return true;
    });
    hPlusCol.addHandler('click', () => {
        game.editorHeight = (game.editorHeight ?? 0) + 1;
        return true;
    });
    sMinusCol.addHandler('click', () => {
        game.heightScaleMultiplier = Math.max(1, (game.heightScaleMultiplier ?? 1) - 1);
        updateHeightScale();
        return true;
    });
    sPlusCol.addHandler('click', () => {
        game.heightScaleMultiplier = (game.heightScaleMultiplier ?? 1) + 1;
        updateHeightScale();
        return true;
    });
    modeCol.addHandler('click', () => {
        game.heightEditMode = game.heightEditMode === 'set' ? 'blend' : 'set';
        return true;
    });

    // Manual positioning each frame
    UI.root.entity.registerCall('update', () => {
        const cw = game.canvas!.width;
        const ch = game.canvas!.height;
        const x0 = cw - uiBorder - widgetW;
        const y0 = ch - uiBorder - widgetH;
        const cx = gap;
        const cy1 = gap;
        const cy2 = rowH + gap * 2;
        const cy3 = rowH * 2 + gap * 3;

        container.setPosition(x0 + widgetW / 2, y0 + widgetH / 2);

        hMinus.setPosition(x0 + cx, y0 + cy1);
        hLabel.setPosition(x0 + cx + btnSize + gap, y0 + cy1);
        hPlus.setPosition(x0 + cx + btnSize + gap + labelW, y0 + cy1);

        sMinus.setPosition(x0 + cx, y0 + cy2);
        sLabel.setPosition(x0 + cx + btnSize + gap, y0 + cy2);
        sPlus.setPosition(x0 + cx + btnSize + gap + labelW, y0 + cy2);

        modeBtn.setPosition(x0 + cx, y0 + cy3);

        hLabel.setText((game.editorHeight ?? 0).toString());
        sLabel.setText('x' + (game.heightScaleMultiplier ?? 1).toString());
        modeTxt.setText(game.heightEditMode ?? 'blend');
    });

    container.setEnabled(false);
    return container;
}

/** Toggles palette/nav-mesh/height-widget visibility each frame based on game.editorTarget. */
function initEditorModeControl(UI: UIManager): void {
    const navMeshEntity = game.getEntity('tilemapNavigation')!;

    UI.root.entity.registerCall('update', () => {
        if (_tilePalette) _tilePalette.setEnabled(game.editorTarget === 'tilemap');
        if (_navPalette) _navPalette.setEnabled(game.editorTarget === 'navMesh');
        if (_heightWidget) _heightWidget.setEnabled(game.editorTarget === 'height');
        navMeshEntity.enabled = game.editorTarget === 'navMesh';
    });
}
