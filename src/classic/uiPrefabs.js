import {UIManager} from "/classic/ui.js";

// Define global UI variables, could go on a uiVar.js file
// --- Color variables hsla (hue, saturation, light, alpha) ---
// h0 = accent hue (any)
// h1 = secondary hue (any)
// h-alert = allert hue (red)
// h-succses = succses hue (green)
// ***we need a util func hslaToRgba(h,s,l,a) in UIManager and maybe directlly input hsla in all elements construction
// --- Text scale variables ---
let tHuge = 1.6 // very big
let tBig = 0.8 // big
let tMid = 0.5 // normal
let tSmall = 0.4 // small
let tTiny = 0.2 // very small

// Define global UI state, could go on a uiState.js file
// --- sideMenu state and actions ---
let sideMenuIsOpen = false;
function toggleSideMenu() {
    sideMenuIsOpen = !sideMenuIsOpen;
    console.log(sideMenuIsOpen ? "Menu opened" : "Menu closed");
}
// --- mainView state and actions ---
let viewState = 0
function setView(index) {viewState = index}

// Define main init func for this first example
export function initUI() {
    // init UIManager
    let UI = new UIManager(game);

    // add components
    initTopBar(UI)
    initMainView(UI)
    initSideMenu(UI)

    //UI.root.setEnabled(false) //skip all recurive
}

function initTopBar(UIManager) {
    let UI = UIManager

    let topBarContainer = UI.spawnContainer(undefined, undefined, [0,0.08,0,1])
    UI.root.addChild(topBarContainer, "top-center", "top-center")
    // instantiate sub-parts
    let FPS = initFPS(UI)
    let MenuBtn = initBtn(UI, "menu", tMid, () => {
        if (sideMenuIsOpen == false) {
            toggleSideMenu()
        }
    });
    let title = UI.spawnText("Classic Engine + UI", undefined, 1000, [0,0.6,0,1], [0,0,0,0])
    // set positions
    topBarContainer.addChild(FPS, "mid-left", "mid-left")
    topBarContainer.addChild(MenuBtn, "mid-right", "mid-right")
    topBarContainer.addChild(title, "mid-center", "mid-center")
    
    // make reactive based on screen breakpoints
    UI.root.entity.registerCall("refreshUI", () => {
        topBarContainer.setSize(UI.root.width, FPS.height);
        // mobile
        if (UI.root.width < 700) {
            title.setTextScale(tSmall);
            title.setText("classic + UI");
        } 
        // desktop
        else if (UI.root.width < 1100) {
            title.setTextScale(tMid);
            title.setText("Classic Engine + UI");
        } 
        // wide desktop
        else {
            title.setTextScale(tBig);
            title.setText("Classic Engine + UI");
            topBarContainer.setSize(UI.root.width, FPS.height + 15);
        }
    });  
}

function initFPS(UIManager) {
    let UI = UIManager
    // Static comp
    let FPSContainer = UI.spawnPadding([10,20,10,20], [0,0,0,0])
    let FPSText = UI.spawnText("FPS", tMid)
    FPSContainer.addChild(FPSText)
    UI.root.addChild(FPSContainer, "top-left", "top-left")
    // Dynamic comp
    let lastFPS = 0
    let timeAccumulator = 0
    UI.root.entity.registerCall("refreshUI", () => {
        timeAccumulator += game.deltaTime;
        if (timeAccumulator >= 0.1) {
            lastFPS = game.fps;
            FPSText.setText(lastFPS.toString());
            timeAccumulator = 0;
        }
        if (lastFPS >= 30) {
            FPSText.setTextColor([0,0.6,0,1])
        }else {FPSText.setTextColor([0.8,0,0,1])}
    })
    return FPSContainer
}

function initSideMenu(UIManager) {
    // Static comp
    let UI = UIManager
    let overlay = UI.spawnContainer(UI.root.width, UI.root.height, [0,0.05,0,0.92])
    UI.root.addChild(overlay, "top-left", "top-left")
    let sideContainer = UI.spawnContainer(200, UI.root.height)
    UI.root.addChild(sideContainer, "top-right", "top-right")
    let content = initMenuContent(UI)
    sideContainer.addChild(content, "top-left", "top-left")

    // Dynamic comp
    let overlayCollider = UI.addColliderToElem(overlay);

    UI.root.entity.registerCall("refreshUI", () => {
        // idle
        overlay.setSize(UI.root.width-sideContainer.width, UI.root.height)
        sideContainer.setSize(sideContainer.width, UI.root.height)
        overlay.setColor([0,0.05,0,0.92]);
        // hover
        if (game.physics.gjk(overlayCollider, game.physics.mouse)) {
            overlay.setColor([0.05, 0, 0, 0.92]);
        }
        // click
        if (game.wasMouseButtonReleased(0) && game.physics.gjk(overlayCollider, game.physics.mouse)) {
            if (sideMenuIsOpen==true) {
                toggleSideMenu();
            }
        }
    });
    UI.root.entity.registerCall("refreshUI", () => {
        // in open state
        if (sideMenuIsOpen==true) {
            sideContainer.setColor([0,0.1,0,1])
            sideContainer.setSize(UI.interpolation(sideContainer.width, 200), UI.root.height)
        }
        // in close state
        else {
            sideContainer.setSize(UI.interpolation(sideContainer.width, 0), UI.root.height);
            overlay.setColor([0,0,0,0])
            overlay.setSize(0,0)
        }
    });
}

function initMenuContent(UIManager) {
    let UI = UIManager
    let container = UI.spawnPadding([56,36,36,18],[0,0.1,0,0]) 
    let group = UI.spawnArray(true, "left", 2, [0,0,0,0])
    container.addChild(group)
    let btn = initBtn(UI, "init", tMid, () => {
        setView(0)
        toggleSideMenu()
    })
    let btn2 = initBtn(UI, "gameover", tMid, () => {
        setView(1)
        toggleSideMenu()
    })
    let btn3 = initBtn(UI, "skygpu", tMid, () => {
        setView(2)
        toggleSideMenu()
    })
    let btn4 = initBtn(UI, "Box Grid", tMid, () => {
        setView(3)
        toggleSideMenu()
    })
    group.addChild(btn)
    group.addChild(btn2)
    group.addChild(btn3)
    group.addChild(btn4)

    return container
}

function initMainView(UIManager) {
    let UI = UIManager
    let container = UI.spawnContainer(1, 1, [1,1,1,0])
    let pad = UI.spawnPadding([20,20,20,20], [0,0.08,0,0.98])
    let array = UI.spawnArray(true, "center", 0, [0,0,0,0])
    let btn = initBtn(UI, "next", tMid, () => {
        if (viewState <= 2){
            viewState += 1
        }else {viewState = 0}
    })
    pad.addChild(array)
    container.addChild(pad)
    container.addChild(btn, "bot-center", "top-center")
    UI.root.addChild(container)
    
    // init each view...
    let v0 = init00(UI)
    array.addChild(v0)
    let v1 = init01(UI).setEnabled(false)
    array.addChild(v1)
    let v2 = init02(UI).setEnabled(false)
    array.addChild(v2)
    let v3 = init03(UI).setEnabled(false)
    array.addChild(v3)

    let prevView = v0
    UI.root.entity.registerCall("refreshUI", () => {
        container.setSize(pad.width, pad.height + 40)

        if (viewState == 0) {vSet(v0)}
        if (viewState == 1) {vSet(v1)}
        if (viewState == 2) {vSet(v2)}
        if (viewState == 3) {vSet(v3)}
    })

    function vSet(v){
        prevView.setEnabled(false)
        v.setEnabled(true)
        prevView = v
    }
}

function init00(UIManager) {
    let UI = UIManager
    let array = UI.spawnArray(true, "left", 20 , [1,0,0,0])
    let title = UI.spawnText("Welcome", tBig, 1000, undefined, [0,0,0,0])
    array.addChild(title)
    let txt = UI.spawnText("", tSmall, 320, undefined, [0,0,0,0])
    array.addChild(txt)
    typeWriterFx(txt,
        "This front is constructed as a testing example of the new layout system built on top of clasic-wgl v0.1a0.",
        20
    );
    let txt2 = UI.spawnText("", tSmall, 320, undefined, [0,0,0,0])
    array.addChild(txt2)
    typeWriterFx(txt2,
        "Layouts are designed to work on desktop and mobile updating automatically.",
        60
    );

    // clickable links for repo and main UI file...
    let link = initLink(UI, "> UI manager file", undefined, () => {
        window.open(
            "https://github.com/vgMonky/classic-wgl/blob/00-layout-sys-first-approach/src/classic/ui.js",
            "_blank");
    });
    array.addChild(link)
    let link2 = initLink(UI, "> this front file", undefined, () => {
        window.open(
            "https://github.com/vgMonky/classic-wgl/blob/00-layout-sys-first-approach/src/classic/uiPrefabs.js",
            "_blank");
    });
    array.addChild(link2)


    
    return array
}
function init01(UIManager) {
    let UI = UIManager
    // game over component
    // create the elements
    let gameover = UI.spawnPadding([40, 40, 40, 40], [0,0.1,0,0])
    let content = UI.spawnArray(true, "center", 12, [0,0,0,0])
    let text1 = UI.spawnText("Game over", 1.4, 200, [0.8,0.2,0.2,1])        
    let text2 = UI.spawnText("start again", 0.5, 300, undefined, [0,0.3,0,0.05])
    // nest the elements
    content.addChild(text1)
    content.addChild(text2)
    gameover.addChild(content)

    let text2Collider = UI.addColliderToElem(text2)
    // test animation
    UI.root.entity.registerCall("refreshUI", () => {
        // idle
        text1.setTextColor([UI.newSine(0.7, 0.9, 400), 0, 0, 1]);
        text2.setColor([0, 0, 0, UI.newSine(0, 0.2, 200)]);
        text2.setTextColor([0, UI.newSine(0.6, 0.9, 200), 0, 1]);
        
        // hover
        if (game.physics.gjk(text2Collider, game.physics.mouse)) {
            text2.setTextScale(UI.newSine(0.45, 0.50, 150))
        } else {
            text2.setTextScale(0.5)
        }
    });
    // click
    text2Collider.addHandler("click", () => {
        console.log("start again clicked!!!")
        return true; // returning true stops propagation
    });
    

    return gameover
}
function init02(UIManager) {
    let UI = UIManager
    let array = UI.spawnArray(true, "left", 10, [1,0,0,0])
    let arrayH = UI.spawnArray(false, "center", 8, [1,0,0,0])
    let iso = UI.spawnSprite("skynetLogo", 110, 110, 0, [1,1])
    let title = UI.spawnText("SKYGPU.NET", tHuge, 1000, undefined, [0,0,0,0])
    let desc = UI.spawnText("", tMid, 500, undefined, [0,0,0,0])
    typeWriterFx(desc, "decentralized compute layer", 60)
    array.addChild(title)
    array.addChild(desc)
    arrayH.addChild(iso)
    arrayH.addChild(array)

    UI.root.entity.registerCall("refreshUI", () => {
        // mobile
        if (UI.root.width < 700) {
            arrayH.setVertical(true)
            iso.setSize(140,140)
            title.setTextScale(tBig)
            desc.setTextScale(0)
        } 
        // desktop
        else {
            arrayH.setVertical(false)
            iso.setSize(110,110)
            title.setTextScale(tHuge)
            desc.setTextScale(tMid)
        }
    })

    return arrayH
}
function init03(UIManager) {
    let UI = UIManager
    let array = UI.spawnArray(true, "center", 15, [1,0,0,0])
    let title = UI.spawnText("box grid", tBig, 1000, undefined, [0,0,0,0])
    let desc = UI.spawnText("hover grid to interact", tSmall, 300, undefined, [0,0,0,0])

    // build grid directly inside array
    const gridSize = 10;
    const gap = 4;
    const boxSize = 25;

    const grid = UI.spawnArray(true, "center", gap, [0,0,0,0]); // vertical stack of rows
    array.addChild(grid);
    array.addChild(title)
    array.addChild(desc)

    for (let y = 0; y < gridSize; y++) {
        let row = UI.spawnArray(false, "center", gap, [0,0,0,0]); // horizontal row
        grid.addChild(row);
        for (let x = 0; x < gridSize; x++) {
            const box = createReactiveBox(UI, { size: boxSize });
            row.addChild(box);
        }
    }

    return array

    function createReactiveBox(UI, opts = {}) {
        const size = opts.size || 30;        
        const minScale = opts.minScale || 0.25;
        const maxDist = opts.maxDist || 120;
        const colorNear = opts.colorNear || [0, 0.1, 0, 1];
        const colorFar  = opts.colorFar  || [0, 0.5, 0, 1];

        const box = UI.spawnContainer(size, size, colorFar);

        box.entity.registerCall("refreshUI", () => {
            const centerX = box.position[0] + box.width * 0.5;
            const centerY = box.position[1] + box.height * 0.5;
            const mx = game.mousePos[0];
            const my = game.mousePos[1];
            const dx = mx - centerX;
            const dy = my - centerY;
            const dist = Math.sqrt(dx*dx + dy*dy);
            const t = Math.max(0, Math.min(1, 1 - dist / maxDist));
            const scale = 1 - (1 - minScale) * t;
            const newSize = Math.max(2, Math.round(size * scale));
            box.setSize(newSize, newSize);
            box.setPosition(centerX - newSize * 0.5, centerY - newSize * 0.5);

            const lerp = (a,b,f) => a + (b-a)*f;
            const col = [
                lerp(colorFar[0], colorNear[0], t),
                lerp(colorFar[1], colorNear[1], t),
                lerp(colorFar[2], colorNear[2], t),
                lerp(colorFar[3] ?? 1, colorNear[3] ?? 1, t)
            ];
            box.setColor(col);
        });

        return box;
    }
}

// Base components - generic reusable components:
// generic button 
function initBtn(UIManager, txt = "btn", txtSize = tMid, onClick = null) {
    let UI = UIManager;

    // Static comp
    let container = UI.spawnPadding([10, 20, 10, 20], [0, 0.15, 0, 0]);
    let text = UI.spawnText(txt.toString(), txtSize, 200, [0, 0.7, 0, 1], [0, 0.15, 0, 0]);
    container.addChild(text);

    // Dynamic comp
    let container2Collider = UI.addColliderToElem(container);
    let speed = 150;

    container.entity.registerCall("refreshUI", () => {
        // idle
        text.setTextColor([UI.newSine(0, 0.4, speed), UI.newSine(0.6, 0.9, speed), 0, 1]);
        container.setColor([0, 0.15, 0, 0]);

        // hover
        if (game.physics.gjk(container2Collider, game.physics.mouse)) {
            container.setColor([0, UI.newSine(0.5, 0.8, speed), 0, 1]);
            text.setTextColor([0, 0.1, 0, 1]);
        }
        // click
        if (game.wasMouseButtonReleased(0) && game.physics.gjk(container2Collider, game.physics.mouse)) {
            if (onClick) {
                onClick();   // run custom action
            } else {
                console.log("clicked!!!");
            }
        }
    });

    return container;
}
// generic link 
function initLink(UIManager, txt = "link", txtSize = tSmall, onClick = null) {
    let UI = UIManager;

    // Static comp
    let container = UI.spawnPadding([0, 0, 0, 0], [0, 0.15, 0, 0]);
    let text = UI.spawnText(txt.toString(), txtSize, 400, [0.7,0.4,0,1], [0, 0.15, 0, 0]);
    container.addChild(text);

    // Dynamic comp
    let container2Collider = UI.addColliderToElem(container);
    let speed = 10;

    container.entity.registerCall("refreshUI", () => {
        // idle
        text.setTextColor([UI.newSine(0.6, 0.8, speed), 0.45, 0, 1]);
        container.setColor([0, 0, 0, 0]);

        // hover
        if (game.physics.gjk(container2Collider, game.physics.mouse)) {
            container.setColor([UI.newSine(0.7, 0.9, speed), 0.45, 0, 1]);
            text.setTextColor([0, 0.1, 0, 1]);
        }
        // click
        if (game.wasMouseButtonReleased(0) && game.physics.gjk(container2Collider, game.physics.mouse)) {
            if (onClick) {
                onClick();   // run custom action
            } else {
                console.log("clicked!!!");
            }
        }
    });
    
    return container;
}


function typeWriterFx(textElement, fullText, speed = 100) {
    let index = 0;
    let lastTime = Date.now();

    textElement.entity.registerCall("refreshUI", () => {
        const now = Date.now();
        if (index < fullText.length && now - lastTime > speed) {
            index++;
            textElement.setText(fullText.slice(0, index));
            lastTime = now;
        }
    });
}
