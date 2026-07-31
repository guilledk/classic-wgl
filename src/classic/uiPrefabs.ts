import { APP_VERSION_DISPLAY } from '../version.ts';
import game from '/classic/state.js';
import {
    UIManager,
    UIText,
    UISprite,
    UIElement,
    UIArray,
    UIContainer,
    UIPadding,
} from '/classic/ui.js';
import type { ICollider } from './types.js';

type Color = [number, number, number, number];

// --- Text scale variables ---
const tHuge = 1.6; // very big
const tBig = 0.8; // big
const tMid = 0.5; // normal
const tSmall = 0.4; // small
// const tTiny = 0.2; // very small (unused)

// --- sideMenu state and actions ---
let sideMenuIsOpen = false;
function toggleSideMenu(): void {
    sideMenuIsOpen = !sideMenuIsOpen;
    console.log(sideMenuIsOpen ? 'Menu opened' : 'Menu closed');
}

// --- mainView state and actions ---
let viewState = 0;
function setView(index: number): void {
    viewState = index;
}

// Define main init func for this first example
export function initUI(): void {
    // init UIManager
    const UI = new UIManager(game);

    // add components
    initTopBar(UI);
    initMainView(UI);
    initSideMenu(UI);
}

function initTopBar(UI: UIManager): void {
    const topBarContainer = UI.spawnContainer(undefined, undefined, [0, 0.08, 0, 1]);
    UI.root.addChild(topBarContainer, 'top-center', 'top-center');

    // instantiate sub-parts
    const FPS = initFPS(UI);
    const MenuBtn = initBtn(UI, 'menu', tMid, () => {
        if (!sideMenuIsOpen) {
            toggleSideMenu();
        }
    });
    const title = UI.spawnText(
        'Classic Engine + UI',
        undefined,
        1000,
        [0, 0.6, 0, 1],
        [0, 0, 0, 0],
    );

    // set positions
    topBarContainer.addChild(FPS, 'mid-left', 'mid-left');
    topBarContainer.addChild(MenuBtn, 'mid-right', 'mid-right');
    topBarContainer.addChild(title, 'mid-center', 'mid-center');

    // make reactive based on screen breakpoints,
    // only reevaluated when the browser canvas resizes
    const applyBreakpoints = (): void => {
        // mobile
        if (UI.root.width < 700) {
            title.setTextScale(tSmall);
            title.setText('classic + UI');
            topBarContainer.setSize(UI.root.width, FPS.height);
        }
        // desktop
        else if (UI.root.width < 1100) {
            title.setTextScale(tMid);
            title.setText('Classic Engine + UI');
            topBarContainer.setSize(UI.root.width, FPS.height);
        }
        // wide desktop
        else {
            title.setTextScale(tBig);
            title.setText('Classic Engine + UI');
            topBarContainer.setSize(UI.root.width, FPS.height + 15);
        }
    };
    UI.root.entity.registerCall('canvasResize', applyBreakpoints);
    applyBreakpoints();
}

function initFPS(UI: UIManager): UIPadding {
    // Static comp
    const FPSContainer = UI.spawnPadding([10, 20, 10, 20], [0, 0, 0, 0]);
    const FPSText = UI.spawnText('FPS', tMid);
    FPSContainer.addChild(FPSText);
    UI.root.addChild(FPSContainer, 'top-left', 'top-left');

    // Dynamic comp
    let lastFPS = 0;
    let timeAccumulator = 0;
    UI.root.entity.registerCall('update', () => {
        timeAccumulator += game.deltaTime;
        if (timeAccumulator >= 0.1) {
            lastFPS = game.fps;
            FPSText.setText(lastFPS.toString());
            timeAccumulator = 0;
        }
        if (lastFPS >= 30) {
            FPSText.setTextColor([0, 0.6, 0, 1]);
        } else {
            FPSText.setTextColor([0.8, 0, 0, 1]);
        }
    });
    return FPSContainer;
}

function initSideMenu(UI: UIManager): void {
    // Static comp
    const overlay = UI.spawnContainer(UI.root.width, UI.root.height, [0, 0.05, 0, 0.92]);
    UI.root.addChild(overlay, 'top-left', 'top-left');
    const sideContainer = UI.spawnContainer(200, UI.root.height);
    UI.root.addChild(sideContainer, 'top-right', 'top-right');
    const content = initMenuContent(UI);
    sideContainer.addChild(content, 'top-left', 'top-left');

    // Dynamic comp
    const overlayCollider = UI.addColliderToElem(overlay);

    // clicking the overlay closes the menu, dispatched by the
    // physics collider system
    overlayCollider.addHandler('click', () => {
        if (sideMenuIsOpen) {
            toggleSideMenu();
            return true; // stop propagation
        }
        return false;
    });

    UI.root.entity.registerCall('update', () => {
        // in open state
        if (sideMenuIsOpen) {
            sideContainer.setColor([0, 0.1, 0, 1]);
            sideContainer.setSize(UI.interpolation(sideContainer.width, 200), UI.root.height);
            overlay.setSize(UI.root.width - sideContainer.width, UI.root.height);

            // idle / hover
            overlay.setColor([0, 0.05, 0, 0.92]);
            if (game.physics!.gjk(overlayCollider, game.physics!.mouse)) {
                overlay.setColor([0.05, 0, 0, 0.92]);
            }
        }
        // in close state
        else {
            sideContainer.setSize(UI.interpolation(sideContainer.width, 0), UI.root.height);
            overlay.setColor([0, 0, 0, 0]);
            overlay.setSize(0, 0);
        }
    });
}

function initMenuContent(UI: UIManager): UIPadding {
    const container = UI.spawnPadding([56, 36, 36, 18], [0, 0.1, 0, 0]);
    const group = UI.spawnArray(true, 'left', 2, [0, 0, 0, 0]);
    container.addChild(group);

    const btn = initBtn(UI, 'init', tMid, () => {
        setView(0);
        toggleSideMenu();
    });
    const btn2 = initBtn(UI, 'gameover', tMid, () => {
        setView(1);
        toggleSideMenu();
    });
    const btn3 = initBtn(UI, 'skygpu', tMid, () => {
        setView(2);
        toggleSideMenu();
    });
    const btn4 = initBtn(UI, 'Box Grid', tMid, () => {
        setView(3);
        toggleSideMenu();
    });

    group.addChild(btn);
    group.addChild(btn2);
    group.addChild(btn3);
    group.addChild(btn4);

    return container;
}

function initMainView(UI: UIManager): void {
    const container = UI.spawnContainer(1, 1, [1, 1, 1, 0]);
    const pad = UI.spawnPadding([20, 20, 20, 20], [0, 0.08, 0, 0.98]);
    const array = UI.spawnArray(true, 'center', 0, [0, 0, 0, 0]);
    const btn = initBtn(UI, 'next', tMid, () => {
        if (viewState <= 2) {
            viewState += 1;
        } else {
            viewState = 0;
        }
    });
    pad.addChild(array);
    container.addChild(pad);
    container.addChild(btn, 'bot-center', 'top-center');
    UI.root.addChild(container);

    // init each view...
    const v0 = init00(UI);
    array.addChild(v0);
    const v1 = init01(UI).setEnabled(false);
    array.addChild(v1);
    const v2 = init02(UI).setEnabled(false);
    array.addChild(v2);
    const v3 = init03(UI).setEnabled(false);
    array.addChild(v3);

    type ViewType = UIArray | UIPadding;
    const views: ViewType[] = [v0, v1, v2, v3];
    let prevView: ViewType = v0;

    UI.root.entity.registerCall('update', () => {
        container.setSize(pad.width, pad.height + 40);

        const v = views[viewState] || v0;
        if (v !== prevView) vSet(v);
    });

    function vSet(v: ViewType): void {
        prevView.setEnabled(false);
        v.setEnabled(true);
        prevView = v;
    }
}

function init00(UI: UIManager): UIArray {
    const array = UI.spawnArray(true, 'left', 20, [1, 0, 0, 0]);
    const title = UI.spawnText('Welcome', tBig, 1000, undefined, [0, 0, 0, 0]);
    array.addChild(title);

    const txt = UI.spawnText('', tSmall, 320, undefined, [0, 0, 0, 0]);
    array.addChild(txt);
    typeWriterFx(
        txt,
        `This front is constructed as a testing example of the new layout system built on top of classic-wgl ${APP_VERSION_DISPLAY}.`,
        20,
    );

    const txt2 = UI.spawnText('', tSmall, 320, undefined, [0, 0, 0, 0]);
    array.addChild(txt2);
    typeWriterFx(
        txt2,
        'Layouts are designed to work on desktop and mobile updating automatically.',
        60,
    );

    // clickable links for repo and main UI file...
    const link = initLink(UI, '> UI manager file', undefined, () => {
        window.open(
            'https://github.com/vgMonky/classic-wgl/blob/00-layout-sys-first-approach/src/classic/ui.js',
            '_blank',
        );
    });
    array.addChild(link);

    const link2 = initLink(UI, '> this front file', undefined, () => {
        window.open(
            'https://github.com/vgMonky/classic-wgl/blob/00-layout-sys-first-approach/src/classic/uiPrefabs.js',
            '_blank',
        );
    });
    array.addChild(link2);

    return array;
}

function init01(UI: UIManager): UIPadding {
    // game over component
    // create the elements
    const gameover = UI.spawnPadding([40, 40, 40, 40], [0, 0.1, 0, 0]);
    const content = UI.spawnArray(true, 'center', 12, [0, 0, 0, 0]);
    const text1 = UI.spawnText('Game over', 1.4, 200, [0.8, 0.2, 0.2, 1]);
    const text2 = UI.spawnText('start again', 0.5, 300, undefined, [0, 0.3, 0, 0.05]);

    // nest the elements
    content.addChild(text1);
    content.addChild(text2);
    gameover.addChild(content);

    const text2Collider = UI.addColliderToElem(text2 as unknown as UIElement);

    // test animation
    UI.root.entity.registerCall('update', () => {
        // idle
        text1.setTextColor([UI.newSine(0.7, 0.9, 400), 0, 0, 1]);
        text2.setColor([0, 0, 0, UI.newSine(0, 0.2, 200)]);
        text2.setTextColor([0, UI.newSine(0.6, 0.9, 200), 0, 1]);

        // hover
        if (game.physics!.gjk(text2Collider, game.physics!.mouse)) {
            text2.setTextScale(UI.newSine(0.45, 0.5, 150));
        } else {
            text2.setTextScale(0.5);
        }
    });

    // click
    text2Collider.addHandler('click', () => {
        console.log('start again clicked!!!');
        return true; // returning true stops propagation
    });

    return gameover;
}

function init02(UI: UIManager): UIArray {
    const array = UI.spawnArray(true, 'left', 10, [1, 0, 0, 0]);
    const arrayH = UI.spawnArray(false, 'center', 8, [1, 0, 0, 0]);
    const iso = UI.spawnSprite('skynetLogo', 110, 110, 0, [1, 1]);
    const title = UI.spawnText('SKYGPU.NET', tHuge, 1000, undefined, [0, 0, 0, 0]);
    const desc = UI.spawnText('', tMid, 500, undefined, [0, 0, 0, 0]);
    typeWriterFx(desc, 'decentralized compute layer', 60);

    array.addChild(title);
    array.addChild(desc);
    arrayH.addChild(iso);
    arrayH.addChild(array);

    // reevaluated only when the browser canvas resizes
    const applyBreakpoints = (): void => {
        // mobile
        if (UI.root.width < 700) {
            arrayH.setVertical(true);
            iso.setSize(140, 140);
            title.setTextScale(tBig);
            desc.setTextScale(0);
        }
        // desktop
        else {
            arrayH.setVertical(false);
            iso.setSize(110, 110);
            title.setTextScale(tHuge);
            desc.setTextScale(tMid);
        }
    };
    UI.root.entity.registerCall('canvasResize', applyBreakpoints);
    applyBreakpoints();

    return arrayH;
}

function init03(UI: UIManager): UIArray {
    const array = UI.spawnArray(true, 'center', 15, [1, 0, 0, 0]);
    const title = UI.spawnText('box grid', tBig, 1000, undefined, [0, 0, 0, 0]);
    const desc = UI.spawnText('hover grid to interact', tSmall, 300, undefined, [0, 0, 0, 0]);

    // build grid directly inside array
    const gridSize = 10;
    const gap = 4;
    const boxSize = 25;

    const grid = UI.spawnArray(true, 'center', gap, [0, 0, 0, 0]); // vertical stack of rows
    array.addChild(grid);
    array.addChild(title);
    array.addChild(desc);

    for (let y = 0; y < gridSize; y++) {
        const row = UI.spawnArray(false, 'center', gap, [0, 0, 0, 0]); // horizontal row
        grid.addChild(row);
        for (let x = 0; x < gridSize; x++) {
            const box = createReactiveBox(UI, { size: boxSize });
            row.addChild(box);
        }
    }

    return array;
}

interface ReactiveBoxOpts {
    size?: number;
    minScale?: number;
    maxDist?: number;
    colorNear?: Color;
    colorFar?: Color;
}

function createReactiveBox(UI: UIManager, opts: ReactiveBoxOpts = {}): UIContainer {
    const size = opts.size || 30;
    const minScale = opts.minScale || 0.25;
    const maxDist = opts.maxDist || 120;
    const colorNear = opts.colorNear || [0, 0.1, 0, 1];
    const colorFar = opts.colorFar || [0, 0.5, 0, 1];

    const box = UI.spawnContainer(size, size, colorFar);

    box.entity.registerCall('update', () => {
        const centerX = box.position[0] + box.width * 0.5;
        const centerY = box.position[1] + box.height * 0.5;
        const mx = game.mousePos[0];
        const my = game.mousePos[1];
        const dx = mx - centerX;
        const dy = my - centerY;
        const dist = Math.sqrt(dx * dx + dy * dy);
        const t = Math.max(0, Math.min(1, 1 - dist / maxDist));
        const scale = 1 - (1 - minScale) * t;
        const newSize = Math.max(2, Math.round(size * scale));
        box.setSize(newSize, newSize);
        box.setPosition(centerX - newSize * 0.5, centerY - newSize * 0.5);

        const lerp = (a: number, b: number, f: number): number => a + (b - a) * f;
        const col: Color = [
            lerp(colorFar[0], colorNear[0], t),
            lerp(colorFar[1], colorNear[1], t),
            lerp(colorFar[2], colorNear[2], t),
            lerp(colorFar[3] ?? 1, colorNear[3] ?? 1, t),
        ];
        box.setColor(col);
    });

    return box;
}

// Base components - generic reusable components:
// generic button
function initBtn(
    UI: UIManager,
    txt: string = 'btn',
    txtSize: number = tMid,
    onClick: (() => void) | null = null,
): UIPadding {
    // Static comp
    const container = UI.spawnPadding([10, 20, 10, 20], [0, 0.15, 0, 0]);
    const text = UI.spawnText(txt.toString(), txtSize, 200, [0, 0.7, 0, 1], [0, 0.15, 0, 0]);
    container.addChild(text);

    // Dynamic comp
    const container2Collider = UI.addColliderToElem(container);
    const speed = 150;

    // click, dispatched by the physics collider system
    container2Collider.addHandler('click', () => {
        if (onClick) {
            onClick(); // run custom action
        } else {
            console.log('clicked!!!');
        }
        return true; // stop propagation
    });

    container.entity.registerCall('update', () => {
        // idle
        text.setTextColor([UI.newSine(0, 0.4, speed), UI.newSine(0.6, 0.9, speed), 0, 1]);
        container.setColor([0, 0.15, 0, 0]);

        // hover
        if (game.physics!.gjk(container2Collider, game.physics!.mouse)) {
            container.setColor([0, UI.newSine(0.5, 0.8, speed), 0, 1]);
            text.setTextColor([0, 0.1, 0, 1]);
        }
    });

    return container;
}

// generic link
function initLink(
    UI: UIManager,
    txt: string = 'link',
    txtSize: number = tSmall,
    onClick: (() => void) | null = null,
): UIPadding {
    // Static comp
    const container = UI.spawnPadding([0, 0, 0, 0], [0, 0.15, 0, 0]);
    const text = UI.spawnText(txt.toString(), txtSize, 400, [0.7, 0.4, 0, 1], [0, 0.15, 0, 0]);
    container.addChild(text);

    // Dynamic comp
    const container2Collider = UI.addColliderToElem(container);
    const speed = 10;

    // click, dispatched by the physics collider system
    container2Collider.addHandler('click', () => {
        if (onClick) {
            onClick(); // run custom action
        } else {
            console.log('clicked!!!');
        }
        return true; // stop propagation
    });

    container.entity.registerCall('update', () => {
        // idle
        text.setTextColor([UI.newSine(0.6, 0.8, speed), 0.45, 0, 1]);
        container.setColor([0, 0, 0, 0]);

        // hover
        if (game.physics!.gjk(container2Collider, game.physics!.mouse)) {
            container.setColor([UI.newSine(0.7, 0.9, speed), 0.45, 0, 1]);
            text.setTextColor([0, 0.1, 0, 1]);
        }
    });

    return container;
}

function typeWriterFx(textElement: UIText, fullText: string, speed: number = 100): void {
    let index = 0;
    let lastTime = Date.now();

    textElement.entity.registerCall('update', () => {
        const now = Date.now();
        if (index < fullText.length && now - lastTime > speed) {
            index++;
            textElement.setText(fullText.slice(0, index));
            lastTime = now;
        }
    });
}
