import game from '/classic/state.js';
import { Tilemap, IsometricNavMesh } from '/classic/isometric.js';
import { UIManager, UIText, UISprite, UIElement, UIContainer } from '/classic/ui.js';
import { Collider } from '/classic/collision.js';
import {
    PRESET_ORDER,
    LIGHT_PRESETS,
    applyLightPreset,
    updateLightDirection,
} from '/classic/lighting.js';
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
        lightPreset?: string;
        lightAmbient?: [number, number, number];
        lightDir?: [number, number, number];
        lightColor?: [number, number, number];
        showGrid?: boolean;
        lightAzimuth?: number;
        lightElevation?: number;
        debugFootprints?: boolean;
    }
}

let _tilePalette: UIContainer | null = null;
let _navPalette: UIContainer | null = null;
let _heightWidget: UIContainer | null = null;
let _lightWidget: UIContainer | null = null;
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
    game.debugFootprints = false;

    initTopBar(UI);
    initToolButtons(UI);
    _tilePalette = initTilePalette(UI);
    _navPalette = initNavPalette(UI);
    _heightWidget = initHeightWidget(UI);
    _lightWidget = initLightWidget(UI);
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

    const coordFontScale = 0.3 * _uiScale;
    const coordMaxW = Math.ceil(12 * 32 * coordFontScale);
    const txCoord = UI.spawnText(
        'X:0.0',
        coordFontScale,
        coordMaxW,
        [1.0, 0.3, 0.3, 1],
        [0, 0, 0, 0],
    );
    const tyCoord = UI.spawnText(
        'Y:0.0',
        coordFontScale,
        coordMaxW,
        [0.3, 1.0, 0.3, 1],
        [0, 0, 0, 0],
    );
    const tzCoord = UI.spawnText(
        'Z:0',
        coordFontScale,
        coordMaxW,
        [0.4, 0.4, 1.0, 1],
        [0, 0, 0, 0],
    );

    const axisFontScale = 0.35 * _uiScale;
    const axisMaxW = Math.ceil(4 * 32 * axisFontScale);
    const roseN = UI.spawnText('N', axisFontScale, axisMaxW, [1.0, 1.0, 0.8, 1], [0, 0, 0, 0]);
    const roseE = UI.spawnText('E', axisFontScale, axisMaxW, [1.0, 1.0, 0.8, 1], [0, 0, 0, 0]);
    const roseS = UI.spawnText('S', axisFontScale, axisMaxW, [1.0, 1.0, 0.8, 1], [0, 0, 0, 0]);
    const roseW = UI.spawnText('W', axisFontScale, axisMaxW, [1.0, 1.0, 0.8, 1], [0, 0, 0, 0]);
    const axisX = UI.spawnText('X', axisFontScale, axisMaxW, [1.0, 0.3, 0.3, 1], [0, 0, 0, 0]);
    const axisY = UI.spawnText('Y', axisFontScale, axisMaxW, [0.3, 1.0, 0.3, 1], [0, 0, 0, 0]);
    const axisZ = UI.spawnText('Z', axisFontScale, axisMaxW, [0.4, 0.4, 1.0, 1], [0, 0, 0, 0]);

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

        if (game.debugFootprints) {
            const ox = 310 * _uiScale;
            const oy = 40 * _uiScale;
            const rowH = Math.round(16 * _uiScale);

            txCoord.setEnabled(true);
            txCoord.setPosition(ox, oy);
            tyCoord.setEnabled(true);
            tyCoord.setPosition(ox, oy + rowH);
            tzCoord.setEnabled(true);
            tzCoord.setPosition(ox, oy + rowH * 2);

            const rcX = 100 * _uiScale;
            const rcY = 65 * _uiScale;
            const rR = 28 * _uiScale;
            const axX = 200 * _uiScale;
            const axY = 65 * _uiScale;
            const aL = 35 * _uiScale;

            roseN.setEnabled(true);
            roseN.setPosition(rcX - 5 * _uiScale, rcY - rR - 14 * _uiScale);
            roseE.setEnabled(true);
            roseE.setPosition(rcX + rR + 4, rcY - 6 * _uiScale);
            roseS.setEnabled(true);
            roseS.setPosition(rcX - 5 * _uiScale, rcY + rR + 4);
            roseW.setEnabled(true);
            roseW.setPosition(rcX - rR - 14 * _uiScale, rcY - 6 * _uiScale);

            axisX.setEnabled(true);
            axisX.setPosition(axX + aL + 4, axY - aL / 2 - 8 * _uiScale);
            axisY.setEnabled(true);
            axisY.setPosition(axX + aL + 4, axY + aL / 2 - 10 * _uiScale);
            axisZ.setEnabled(true);
            axisZ.setPosition(axX - 5 * _uiScale, axY - aL - 16 * _uiScale);

            const tm = game.getEntity('tilemap')?.getComponent(Tilemap);
            if (tm) {
                const px = tm.mouseIsoPos[0];
                const py = tm.mouseIsoPos[1];
                txCoord.setText('X:' + px.toFixed(1));
                tyCoord.setText('Y:' + py.toFixed(1));

                const ftx = Math.floor(px);
                const fty = Math.floor(py);
                const fx = px - ftx;
                const fy = py - fty;
                const hd = tm.heightData;
                const sX = tm.sizeX;
                const sY = tm.sizeY;
                const at = (tx: number, ty: number) =>
                    hd[
                        Math.min(Math.max(tx, 0), sX - 1) + Math.min(Math.max(ty, 0), sY - 1) * sX
                    ] ?? 0;
                const hNW = at(ftx, fty);
                const hNE = at(ftx + 1, fty);
                const hSW = at(ftx, fty + 1);
                const hSE = at(ftx + 1, fty + 1);
                const h =
                    hNW + (hNE - hNW) * fx + (hSW - hNW) * fy + (hNW - hNE - hSW + hSE) * fx * fy;
                tzCoord.setText('Z:' + (h * tm.heightScale).toFixed(0));
            }
        } else {
            txCoord.setEnabled(false);
            tyCoord.setEnabled(false);
            tzCoord.setEnabled(false);
            roseN.setEnabled(false);
            roseE.setEnabled(false);
            roseS.setEnabled(false);
            roseW.setEnabled(false);
            axisX.setEnabled(false);
            axisY.setEnabled(false);
            axisZ.setEnabled(false);
        }
    });

    UI.root.entity.registerCall('canvasResize', () => {
        topBar.setSize(UI.root.width, barH);
    });
}

/**
 * Bottom-left dev tools: DEV button toggles a pop-up Start Menu-style
 * text menu with tool selectors. Click-outside closes the menu.
 */
function initToolButtons(UI: UIManager): void {
    const btnPixel = Math.round(64 * _uiScale);
    const menuItemH = Math.round(32 * _uiScale);
    const menuPadding = Math.round(6 * _uiScale);
    const menuGap = Math.round(2 * _uiScale);
    const menuFontScale = 0.5 * _uiScale;
    const menuPanelGap = Math.round(0 * _uiScale);

    let isOpen = false;

    // DEV button
    const devBtn = UI.spawnButton(
        btnPixel,
        btnPixel,
        [0, 0, 0, 0],
        () => {
            game.uiConsumedClick = true;
            isOpen = !isOpen;
            if (!isOpen) game.editorTarget = 'none';
            return true;
        },
        { sprite: 'editorIcons', spriteFrame: 0, spriteTileSet: [4, 4] },
    );

    interface MenuItem {
        label: string;
        action(): void;
        isActive(): boolean;
    }

    const menuItems: MenuItem[] = [
        {
            label: 'Tile Editor',
            action: () => {
                game.editorTarget = game.editorTarget === 'tilemap' ? 'none' : 'tilemap';
                game.agentSelected = false;
                game.uiConsumedClick = true;
                isOpen = false;
            },
            isActive: () => game.editorTarget === 'tilemap',
        },
        {
            label: 'Nav Editor',
            action: () => {
                game.editorTarget = game.editorTarget === 'navMesh' ? 'none' : 'navMesh';
                game.agentSelected = false;
                game.uiConsumedClick = true;
                isOpen = false;
            },
            isActive: () => game.editorTarget === 'navMesh',
        },
        {
            label: 'Height Editor',
            action: () => {
                game.editorTarget = game.editorTarget === 'height' ? 'none' : 'height';
                game.agentSelected = false;
                game.uiConsumedClick = true;
                isOpen = false;
            },
            isActive: () => game.editorTarget === 'height',
        },
        {
            label: 'Light Config',
            action: () => {
                game.editorTarget = game.editorTarget === 'light' ? 'none' : 'light';
                game.agentSelected = false;
                game.uiConsumedClick = true;
                isOpen = false;
            },
            isActive: () => game.editorTarget === 'light',
        },
        {
            label: 'Footprints',
            action: () => {
                game.debugFootprints = !(game.debugFootprints ?? false);
                game.uiConsumedClick = true;
                isOpen = false;
            },
            isActive: () => game.debugFootprints ?? false,
        },
    ];

    const maxLabelLen = Math.max(...menuItems.map((m) => m.label.length));
    const glyphPixelW = 16 * _uiScale;
    const menuW = maxLabelLen * glyphPixelW + menuPadding * 2;
    const menuH = menuItems.length * menuItemH + menuGap * (menuItems.length - 1) + menuPadding * 2;

    // Agent selection indicator: small [A] button, always visible
    const agentInd = Math.round(48 * _uiScale);
    const agentBtn = UI.spawnButton(
        agentInd,
        agentInd,
        [0.1, 0.6, 0.1, 0.8],
        () => {
            game.uiConsumedClick = true;
            game.editorTarget = 'none';
            game.agentSelected = !(game.agentSelected ?? true);
            return true;
        },
        { text: 'A', textScale: 0.55 * _uiScale, priority: 1, hover: true, clickFeedback: 5 },
    );

    // Menu panel background
    const menuPanel = UI.spawnButton(menuW, menuH, [0.1, 0.1, 0.1, 0.95], () => true, {
        priority: 2,
    }).container;

    // Menu item rows
    const itemRows = menuItems.map((item) => {
        const rowW = menuW - menuPadding * 2;
        const { container: row, collider: col } = UI.spawnButton(
            rowW,
            menuItemH,
            [0.15, 0.15, 0.15, 1],
            () => {
                item.action();
                return true;
            },
            { priority: 3 },
        );
        const label = UI.spawnText(
            item.label,
            menuFontScale,
            maxLabelLen + 2,
            [1, 1, 1, 1],
            [0, 0, 0, 0],
        );
        row.addChild(label, 'mid-left', 'mid-left');
        menuPanel.addChild(row, 'top-left', 'top-left');
        return { row, label, col, item };
    });

    // Full-screen backdrop for click-outside-to-close
    const backdrop = UI.spawnButton(1, 1, [0, 0, 0, 0.01], () => {
        if (!isOpen) return false;
        const mx = game.mousePos[0];
        const my = game.mousePos[1];
        const dp = devBtn.container.position;
        const mp = menuPanel.position;
        const onDev =
            mx >= dp[0] && mx <= dp[0] + btnPixel && my >= dp[1] && my <= dp[1] + btnPixel;
        const onMenu = mx >= mp[0] && mx <= mp[0] + menuW && my >= mp[1] && my <= mp[1] + menuH;
        if (!onDev && !onMenu) {
            game.uiConsumedClick = true;
            isOpen = false;
        }
        return false;
    }).container;

    // Update loop
    UI.root.entity.registerCall('update', () => {
        const ch = game.canvas!.height;
        const h = btnPixel;
        const half = h / 2;

        // DEV button position
        devBtn.container.setPosition(h - half, ch - h - half);

        // Agent indicator: above DEV button when menu closed
        const agentTop = ch - h * 2 - agentInd / 2;
        const selected = game.agentSelected ?? true;
        (agentBtn.container as unknown as { _btnBase: number[] })._btnBase = selected
            ? [0.1, 0.7, 0.1, 0.8]
            : [0.3, 0.3, 0.3, 0.6];
        agentBtn.container.setPosition(agentInd / 2, agentTop);

        // Menu panel: above DEV button
        const menuTop = ch - h - menuPanelGap - menuH;
        menuPanel.setPosition(h, menuTop);
        menuPanel.setEnabled(isOpen);
        game.panelMenuOpen = isOpen;

        // Menu items stacked inside panel
        let y = menuTop + menuPadding;
        for (const { row } of itemRows) {
            row.setPosition(menuPanel.position[0] + menuPadding, y);
            y += menuItemH + menuGap;
        }

        // Backdrop
        backdrop.setSize(game.canvas!.width, game.canvas!.height);
        backdrop.setPosition(0, 0);
        backdrop.setEnabled(isOpen);

        // Hover and active-tool highlights
        for (const { row, col, item } of itemRows) {
            const hovered = game.physics!.gjk(col, game.physics!.mouse);
            const active = item.isActive();
            if (hovered) {
                row.color = [0.25, 0.45, 0.75, 1];
            } else if (active) {
                row.color = [0.2, 0.35, 0.6, 1];
            } else {
                row.color = [0.15, 0.15, 0.15, 1];
            }
        }

        UI.refreshLayout();
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
    collider.consumesClick = true;

    let localX = 0;
    let localY = 0;

    collider.addHandler('click', () => {
        const mouseLocal = vec3.clone(game.mousePos);
        vec3.sub(mouseLocal, mouseLocal, container.position);

        localX = Math.floor(mouseLocal[0] / tSize[0]);
        localY = Math.floor(mouseLocal[1] / tSize[1]);

        game.editorTile = Math.min(maxTile, localX + localY * tsSize[0]);

        game.uiConsumedClick = true;
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
    collider.consumesClick = true;

    let localX = 0;
    let localY = 0;

    collider.addHandler('click', () => {
        const mouseLocal = vec3.clone(game.mousePos);
        vec3.sub(mouseLocal, mouseLocal, container.position);

        localX = Math.floor(mouseLocal[0] / (tSize[0] * uiScale));
        localY = Math.floor(mouseLocal[1] / (tSize[1] * uiScale));

        game.editorNavTile = Math.min(maxTile, localX + localY * tsSize[0]);

        game.uiConsumedClick = true;
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
    const widgetW = gap * 4 + btnSize * 2 + labelW;
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
    const { container: hMinus } = UI.spawnButton(
        btnSize,
        btnSize,
        [0.6, 0.1, 0.1, 1],
        () => {
            game.uiConsumedClick = true;
            game.editorHeight = (game.editorHeight ?? 0) - 1;
            return true;
        },
        { text: '-', textScale: 0.6 * _uiScale, hover: true, clickFeedback: 5 },
    );
    container.addChild(hMinus, 'top-left', 'top-left');

    const hLabel = UI.spawnText('0', 0.5 * _uiScale, 60, [1, 1, 1, 1], [0, 0, 0, 0]);
    container.addChild(hLabel, 'top-left', 'top-left');

    const { container: hPlus } = UI.spawnButton(
        btnSize,
        btnSize,
        [0.1, 0.6, 0.1, 1],
        () => {
            game.uiConsumedClick = true;
            game.editorHeight = (game.editorHeight ?? 0) + 1;
            return true;
        },
        { text: '+', textScale: 0.6 * _uiScale, hover: true, clickFeedback: 5 },
    );
    container.addChild(hPlus, 'top-left', 'top-left');

    // Row 2: height scale multiplier
    const { container: sMinus } = UI.spawnButton(
        btnSize,
        btnSize,
        [0.1, 0.1, 0.6, 1],
        () => {
            game.uiConsumedClick = true;
            game.heightScaleMultiplier = Math.max(1, (game.heightScaleMultiplier ?? 1) - 1);
            updateHeightScale();
            return true;
        },
        { text: 's-', textScale: 0.45 * _uiScale, hover: true, clickFeedback: 5 },
    );
    container.addChild(sMinus, 'top-left', 'top-left');

    const sLabel = UI.spawnText('x1', 0.45 * _uiScale, 60, [1, 1, 1, 1], [0, 0, 0, 0]);
    container.addChild(sLabel, 'top-left', 'top-left');

    const { container: sPlus } = UI.spawnButton(
        btnSize,
        btnSize,
        [0.1, 0.1, 0.6, 1],
        () => {
            game.uiConsumedClick = true;
            game.heightScaleMultiplier = (game.heightScaleMultiplier ?? 1) + 1;
            updateHeightScale();
            return true;
        },
        { text: 's+', textScale: 0.45 * _uiScale, hover: true, clickFeedback: 5 },
    );
    container.addChild(sPlus, 'top-left', 'top-left');

    // Row 3: set / blend mode toggle
    const { container: modeBtn, child: modeTxt } = UI.spawnButton(
        widgetW - gap * 2,
        rowH,
        [0.2, 0.2, 0.2, 1],
        () => {
            game.uiConsumedClick = true;
            game.heightEditMode = game.heightEditMode === 'set' ? 'blend' : 'set';
            return true;
        },
        { text: 'blend', textScale: 0.4 * _uiScale, hover: true, clickFeedback: 5 },
    );
    container.addChild(modeBtn, 'top-left', 'top-left');

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

        container.setPosition(x0, y0);

        hMinus.setPosition(x0 + cx, y0 + cy1);
        hLabel.setPosition(x0 + cx + btnSize + gap, y0 + cy1);
        hPlus.setPosition(x0 + cx + btnSize + gap + labelW, y0 + cy1);

        sMinus.setPosition(x0 + cx, y0 + cy2);
        sLabel.setPosition(x0 + cx + btnSize + gap, y0 + cy2);
        sPlus.setPosition(x0 + cx + btnSize + gap + labelW, y0 + cy2);

        modeBtn.setPosition(x0 + cx, y0 + cy3);

        hLabel.setText((game.editorHeight ?? 0).toString());
        sLabel.setText('x' + (game.heightScaleMultiplier ?? 1).toString());
        (modeTxt as UIText).setText(game.heightEditMode ?? 'blend');
        UI.refreshLayout();
    });

    container.setEnabled(false);
    return container;
}

/** Light config widget: preset cycle + azimuth/elevation tweak buttons. */
function initLightWidget(UI: UIManager): UIContainer {
    const btnSize = Math.round(36 * _uiScale);
    const smallBtn = Math.round(28 * _uiScale);
    const labelW = Math.round(110 * _uiScale);
    const dirW = Math.round(60 * _uiScale);
    const gap = Math.round(4 * _uiScale);
    const buttonGap = Math.round(12 * _uiScale);
    const presetRowW = gap * 4 + btnSize * 2 + labelW;
    const adjustRowW = gap * 4 + dirW + smallBtn * 2 + buttonGap * 2;
    const widgetW = Math.max(presetRowW, adjustRowW);
    const rowH = btnSize;
    const widgetH = rowH * 3 + gap * 4;
    const uiBorder = Math.round(10 * _uiScale);

    const container = UI.spawnContainer(widgetW, widgetH, [0, 0, 0, 0.4]);

    const AZ_STEP = 15;
    const EL_STEP = 10;

    // Row 1: preset cycle
    const { container: presetPrev } = UI.spawnButton(
        btnSize,
        btnSize,
        [0.3, 0.3, 0.6, 1],
        () => {
            game.uiConsumedClick = true;
            const keys = PRESET_ORDER;
            const cur = game.lightPreset ?? 'sunny';
            const idx = keys.indexOf(cur);
            const prev = keys[(idx - 1 + keys.length) % keys.length];
            applyLightPreset(prev);
            return true;
        },
        { text: '<<', textScale: 0.4 * _uiScale, hover: true, clickFeedback: 5 },
    );
    container.addChild(presetPrev, 'top-left', 'top-left');

    const presetLabel = UI.spawnText('Sunny Day', 0.45 * _uiScale, 120, [1, 1, 1, 1], [0, 0, 0, 0]);
    container.addChild(presetLabel, 'top-left', 'top-left');

    const { container: presetNext } = UI.spawnButton(
        btnSize,
        btnSize,
        [0.3, 0.3, 0.6, 1],
        () => {
            game.uiConsumedClick = true;
            const keys = PRESET_ORDER;
            const cur = game.lightPreset ?? 'sunny';
            const idx = keys.indexOf(cur);
            const next = keys[(idx + 1) % keys.length];
            applyLightPreset(next);
            return true;
        },
        { text: '>>', textScale: 0.4 * _uiScale, hover: true, clickFeedback: 5 },
    );
    container.addChild(presetNext, 'top-left', 'top-left');

    // Row 2: azimuth
    const azLabel = UI.spawnText('az: 45deg', 0.45 * _uiScale, 80, [1, 1, 1, 1], [0, 0, 0, 0]);
    container.addChild(azLabel, 'top-left', 'top-left');

    const { container: azMinus } = UI.spawnButton(
        smallBtn,
        smallBtn,
        [0.6, 0.3, 0.1, 1],
        () => {
            game.uiConsumedClick = true;
            game.lightAzimuth = ((game.lightAzimuth ?? 0) - AZ_STEP + 360) % 360;
            updateLightDirection();
            game.lightPreset = 'custom';
            return true;
        },
        { text: '-', textScale: 0.4 * _uiScale, hover: true, clickFeedback: 5 },
    );
    container.addChild(azMinus, 'top-left', 'top-left');

    const { container: azPlus } = UI.spawnButton(
        smallBtn,
        smallBtn,
        [0.1, 0.6, 0.3, 1],
        () => {
            game.uiConsumedClick = true;
            game.lightAzimuth = ((game.lightAzimuth ?? 0) + AZ_STEP) % 360;
            updateLightDirection();
            game.lightPreset = 'custom';
            return true;
        },
        { text: '+', textScale: 0.4 * _uiScale, hover: true, clickFeedback: 5 },
    );
    container.addChild(azPlus, 'top-left', 'top-left');

    // Row 3: elevation
    const elLabel = UI.spawnText('el: 45deg', 0.45 * _uiScale, 80, [1, 1, 1, 1], [0, 0, 0, 0]);
    container.addChild(elLabel, 'top-left', 'top-left');

    const { container: elMinus } = UI.spawnButton(
        smallBtn,
        smallBtn,
        [0.6, 0.3, 0.1, 1],
        () => {
            game.uiConsumedClick = true;
            game.lightElevation = Math.max(0, (game.lightElevation ?? 45) - EL_STEP);
            updateLightDirection();
            game.lightPreset = 'custom';
            return true;
        },
        { text: '-', textScale: 0.4 * _uiScale, hover: true, clickFeedback: 5 },
    );
    container.addChild(elMinus, 'top-left', 'top-left');

    const { container: elPlus } = UI.spawnButton(
        smallBtn,
        smallBtn,
        [0.1, 0.6, 0.3, 1],
        () => {
            game.uiConsumedClick = true;
            game.lightElevation = Math.min(90, (game.lightElevation ?? 45) + EL_STEP);
            updateLightDirection();
            game.lightPreset = 'custom';
            return true;
        },
        { text: '+', textScale: 0.4 * _uiScale, hover: true, clickFeedback: 5 },
    );
    container.addChild(elPlus, 'top-left', 'top-left');

    UI.root.entity.registerCall('update', () => {
        const cw = game.canvas!.width;
        const ch = game.canvas!.height;
        const y0 = ch - uiBorder - widgetH;
        const x0 = cw - uiBorder - widgetW;
        const cx = gap;
        const cy1 = gap;
        const cy2 = rowH + gap * 2;
        const cy3 = rowH * 2 + gap * 3;

        container.setPosition(x0, y0);

        presetPrev.setPosition(x0 + cx, y0 + cy1);
        presetLabel.setPosition(x0 + cx + btnSize + gap, y0 + cy1);
        presetNext.setPosition(x0 + cx + btnSize + gap + labelW, y0 + cy1);

        azLabel.setPosition(x0 + cx, y0 + cy2);
        azMinus.setPosition(x0 + widgetW - gap - smallBtn * 2 - buttonGap, y0 + cy2);
        azPlus.setPosition(x0 + widgetW - gap - smallBtn, y0 + cy2);

        elLabel.setPosition(x0 + cx, y0 + cy3);
        elMinus.setPosition(x0 + widgetW - gap - smallBtn * 2 - buttonGap, y0 + cy3);
        elPlus.setPosition(x0 + widgetW - gap - smallBtn, y0 + cy3);

        const preset = game.lightPreset ?? 'sunny';
        const info = LIGHT_PRESETS[preset];
        presetLabel.setText(info ? info.name : preset);

        const az = Math.round(game.lightAzimuth ?? 0);
        azLabel.setText('az: ' + az + 'deg');

        const el = Math.round(game.lightElevation ?? 45);
        elLabel.setText('el: ' + el + 'deg');
        UI.refreshLayout();
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
        if (_lightWidget) _lightWidget.setEnabled(game.editorTarget === 'light');
        navMeshEntity.enabled = game.editorTarget === 'navMesh';
    });
}
