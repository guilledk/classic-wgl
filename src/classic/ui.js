import game from "/classic/state.js";
import { Rectangle, Text, Sprite } from "/classic/transforms.js";

import { Polygon, Collider } from "/classic/collision.js";
import { vec3 } from "/lib/gl-matrix/index.js";

// // Objective: Final API based on UIManager class,

// --- Most basic UI element of the system is UIElement,
//     from this other elements can be extended ---

// A UIElement is just an entity with a rectangle component,
// its just an object that ocupies some 2d space in the screen
class UIElement {
    constructor(
        name, //: string
        color, //: [r, g, b, a] -> number between 0-1
        width, //: number -> pixels
        height, //: number -> pixels
        zlayer //: number
    ) { 
        this.width = width;
        this.height = height;
        this.position = [0, 0];
        this.color = color;
        this.enabled = true

        // Spawn the entity
        this.entity = game.spawnEntity(name);

        // Add Rectangle component
        const [x, y] = this.position;
        this.rectangle = this.entity.addComponent(
            Rectangle,
            [x, y, zlayer], // pos
            [this.width, this.height, 1], // scale
            color, // color
            true // ignoreCam
        );
    }
    
    setPosition(x, y) {
        this.position = [x, y];
        this.rectangle.position = [x, y, this.rectangle.position[2]];
        
        if (typeof this.setChildrenPos === "function") {
            this.setChildrenPos();
        }
        return this;
    }

    setSize(width, height) {
        this.width = width;
        this.height = height;
        this.rectangle.scale = [this.width, this.height, 1];
        return this;
    }

    setColor(rgba) {
        this.color = rgba;  // array [r,g,b,a]
        this.rectangle.color = this.color;
        return this;
    }

    setEnabled(flag) { // Desactivates the element's entity(render & collider) and ocupied space in the ui layout.
        this.entity.enabled = flag;

        // cascade to children
        // !!! Something is not working right, padding container seems tu be the fuck up
        if (this.children) {
            for (const entry of this.children) {
                const child = entry[0] || entry;
                if (child.setEnabled) {
                    child.setEnabled(flag);
                }
            }
        }
        if (this.child && this.child.setEnabled) {
            this.child.setEnabled(flag);
        }

        return this;
    }   
}

class UIText extends UIElement {
    constructor(name, text, textScale, maxWidth, color, bgColor, zlayer) {
        const fontSize = [16, 16];
        const glyphSize = [32, 32];
        const glyphStr = "!\"#$%&'()*+,-./?0123456789:;<=>@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`{|}~";

        super(name, bgColor, 0, 0, zlayer);

        // Core data
        this.fontSize = fontSize;
        this.glyphSize = glyphSize;
        this.glyphStr = glyphStr;
        this.textComps = [];

        this.rawText = text;
        this.textScale = textScale;
        this.maxWidth = maxWidth;
        this.color = color;
        this.lineHeight = 1.3;

        // Initialize
        this._recalculateTextElement();

        this.entity.registerCall("refreshUI", () => this._refreshPositions());
    }

    static wrapText(str, maxCharPerLine) {
        const words = str.split(' ');
        const lines = [];
        let line = "";

        for (let word of words) {
            if ((line + (line.length ? " " : "") + word).length <= maxCharPerLine) {
                line += (line.length ? " " : "") + word;
            } else {
                if (line.length > 0) lines.push(line);
                line = word;
            }
        }
        if (line.length) lines.push(line);

        return lines;
    }

    setText(str) {
        this.rawText = str;
        this._recalculateTextElement();
        return this;
    }

    setTextScale(newScale) {
        this.textScale = newScale;
        this._recalculateTextElement();
        return this;
    }

    setTextColor(rgba) {
        this.color = rgba; // array [r,g,b,a]
        this._recalculateTextElement();
        return this;
    }

    setMaxWidth(number) {
        this.maxWidth = number
        this._recalculateTextElement();
        return this;
    }

    _recalculateTextElement() {
        const scaledGlyphSize = [
            this.glyphSize[0] * this.textScale,
            this.glyphSize[1] * this.textScale
        ];
        this.maxCharPerLine = Math.max(1, Math.floor(this.maxWidth / scaledGlyphSize[0]));
        const lines = UIText.wrapText(this.rawText || "", this.maxCharPerLine);
    
        // Recycle existing components
        for (let i = 0; i < this.textComps.length; i++) {
            if (i < lines.length) {
                const lineText = lines[i].toUpperCase();
    
                // 1) ensure capacity (only if needed)
                if (this.textComps[i].maxCharSize[0] < lineText.length) {
                    this.textComps[i].setMaxCharSize(lineText.length, 1);
                }
    
                // 2) make visible BEFORE setText so setText actually updates the FBO
                this.textComps[i].visible = true;
    
                // 3) update content & appearance
                this.textComps[i].setText(lineText);
                this.textComps[i].scale = [this.textScale, this.textScale, 1];
                this.textComps[i].color = this.color;
    
            } else {
                this.textComps[i].visible = false;
            }
        }
    
        // Add new components if needed
        for (let i = this.textComps.length; i < lines.length; i++) {
            const lineText = lines[i].toUpperCase();
            const textComp = this.entity.addComponent(
                Text,
                [0, 0, this.rectangle.position[2]],
                [this.textScale, this.textScale, 1],
                "font",
                [lineText.length, 1],     // initial capacity
                this.fontSize,
                this.glyphSize,
                this.glyphStr,
                this.color,
                [0, 0, 0, 0],
                true
            );
    
            // visible BEFORE setText
            textComp.visible = true;
            textComp.setText(lineText);
    
            this.textComps.push(textComp);
        }
    
        // Update background size from the *actual* content lengths
        const maxLineLength = Math.max(1, ...lines.map(l => l.length));
        const lineCount = lines.length;
        this.width  = scaledGlyphSize[0] * maxLineLength;
        this.height = scaledGlyphSize[1] + (scaledGlyphSize[1] * this.lineHeight * (lineCount - 1));
        this.rectangle.scale = [this.width, this.height, 1];
    
        this._refreshPositions();
    }
    
    setSize() {
        console.error("to adjust text size use setTextScale(newScale)");
        
    }

    _refreshPositions() {
        const [x, y] = this.position;
        const lineHeight = this.glyphSize[1] * this.textScale * this.lineHeight;

        for (let i = 0; i < this.textComps.length; i++) {
            this.textComps[i].position = [x, y + i * lineHeight, this.rectangle.position[2]];
        }
    }
}

class UISprite extends UIElement {
    constructor(
        name,   //: string
        texture,       //: string -> texture name from manifest.json
        width,         //: number -> pixels
        height,        //: number -> pixels
        frame,     //: number -> sprite sheet frame
        tileSetSize, //: number -> tiles in texture
        color, //: [r,g,b,a]
        zlayer //: number
    ) {
        super(name, color, width, height, zlayer);

        // Add Sprite component
        this.spriteComp = this.entity.addComponent(
            Sprite,
            [this.position[0], this.position[1], zlayer],
            [width / (64 * tileSetSize[0]) , height / (64 * tileSetSize[1]), 1], // scale in terms of pixels / texture? adjust if needed
            texture,
            true,             // ignoreCam → screen-space
            frame,
            tileSetSize,
            [0, 0] // dont change this anchor, use container element instead (e.g. UIContainer)
        );

        this.tileSetSize = tileSetSize

        // Optional: update position on refresh
        this.entity.registerCall("refreshUI", () => {
            this._refreshPosition();
        });
    }

    _refreshPosition() {
        const [x, y] = this.position;
        this.spriteComp.position = [x, y, this.spriteComp.position[2]];
    }

    setPosition(x, y) {
        super.setPosition(x, y);
        this._refreshPosition();
        return this;
    }

    setSize(width, height) {
        super.setSize(width, height);
        // Update scale accordingly
        this.spriteComp.scale = [width / (64 * this.tileSetSize[0]), height / (64 * this.tileSetSize[1]), 1]; // adjust 64 if needed
        return this;
    }

    setFrame(frame) {
        this.spriteComp.frame = frame;
        return this;
    }
}

// --- There are also "container elements" that recalculate their
//     children screen position based on some parameters
//     and logics -> with addChild(params) & setChildrenPos(params) ---

// UIContainer: Container element that repositions its children in the global
//              pos based on its own position and the anchor concept, which is
//              just a property of the container (a default self/child anchor
//              pair that can be overridden per child on addChild).
class UIContainer extends UIElement {
    constructor(
        name, //: string
        color, //: [n, n, n, n] -> number between 0-1
        width, //: number -> pixels
        height, //: number -> pixels
        zlayer //: number
    ) {
        super(name, color, width, height, zlayer);
        this.children = [];
        this.anchor = "mid-center"; // default anchor used for self & children

        this.entity.registerCall("refreshUI", () => {
            this.setChildrenPos();
        });
    }

    addChild(child, selfAnchor = this.anchor, childAnchor = this.anchor) {
        this.children.push({ child, selfAnchor, childAnchor });
        return this;
    }

    getAnchorOffset(anchor, w, h) {
        const map = {
            'top-left': { x: 0, y: 0 },
            'top-center': { x: w / 2, y: 0 },
            'top-right': { x: w, y: 0 },
            'mid-left': { x: 0, y: h / 2 },
            'mid-center': { x: w / 2, y: h / 2 },
            'mid-right': { x: w, y: h / 2 },
            'bot-left': { x: 0, y: h },
            'bot-center': { x: w / 2, y: h },
            'bot-right': { x: w, y: h }
        };
        return map[anchor];
    }

    setChildrenPos() {
        const [panelX, panelY] = this.position;

        for (const { child, selfAnchor, childAnchor } of this.children) {
            if (!child.entity.enabled) continue;  // <-- skip disabled elements

            const panelOffset = this.getAnchorOffset(selfAnchor, this.width, this.height);
            const childOffset = this.getAnchorOffset(childAnchor, child.width, child.height);

            const x = panelX + panelOffset.x - childOffset.x;
            const y = panelY + panelOffset.y - childOffset.y;

            child.setPosition(x, y);

        }
    }
}

// UIArray: Container element that positions its children based
//          on an array layout rule, and it adquires the width and 
//          height of the total size of its children and gaps
class UIArray extends UIElement {
    constructor(
        name, //: string
        vertical, //: bool
        align, //: left" | "center" | "right"
        spacing, //: number -> pixels
        color, //: [r, g, b, a] -> number between 0-1
        zlayer //: number
    ) {
        super(name, color, 10, 10, zlayer);
        this.vertical = vertical;
        this.align = align;
        this.spacing = spacing;
        this.children = [];

        this.entity.registerCall("refreshUI", () => {
            this.setChildrenPos();
        });
    }

    addChild(child) {
        this.children.push(child);
        return this;
    }

    setVertical(flag) {
        this.vertical = !!flag;
        this.setChildrenPos();
        return this;
    }

    setAlign(option) {
        this.align = option //: left" | "center" | "right"
        this.setChildrenPos();
        return this;
    }

    setChildrenPos() {
        const isVertical = this.vertical;
    
        // Step 1: Measure layout size
        let totalMain = 0;
        let maxCross = 0;
    
        for (const child of this.children) {
            if (!child.entity.enabled) continue;  // <-- skip disabled elements

            const main = isVertical ? child.height : child.width;
            const cross = isVertical ? child.width : child.height;
            totalMain += main + this.spacing;
            maxCross = Math.max(maxCross, cross);
        }
    
        totalMain = Math.max(0, totalMain - this.spacing);
    
        // Step 2: Resize self
        this.width = isVertical ? maxCross : totalMain;
        this.height = isVertical ? totalMain : maxCross;
        this.rectangle.scale = [this.width, this.height, 1];
    
        // Step 3: Position each child
        const [startX, startY] = this.position;
        let offset = 0;
    
        for (const child of this.children) {
            if (!child.entity.enabled) continue;  // <-- skip disabled elements

            const main = isVertical ? child.height : child.width;
            const cross = isVertical ? child.width : child.height;
    
            let crossOffset = 0;
            if (this.align === "center") {
                crossOffset = (isVertical ? this.width : this.height) / 2 - cross / 2;
            } else if (this.align === "right") {
                crossOffset = (isVertical ? this.width : this.height) - cross;
            }
    
            const x = isVertical ? startX + crossOffset : startX + offset;
            const y = isVertical ? startY + offset : startY + crossOffset;
    
            child.setPosition(x, y);
            offset += main + this.spacing;
        }
    }    
}

// UIPadding: Container element that repositions its child
//            considering a padding size for each side.
class UIPadding extends UIElement {
    constructor(
        name, //: string
        padding, //: [top, right, bottom, left]
        color, //: [r, g, b, a]
        zlayer, //: number
    ) {
        // Start with dummy size; will be recalculated later
        super(name, color, 10, 10, zlayer);

        this.padding = padding;
        this.child = null;

        this.entity.registerCall("refreshUI", () => {
            this.setChildrenPos();
        });
    }

    addChild(child) {
        if (this.child) {
            throw new Error("UIPadding can only have one child!");
        }
        this.child = child;
        this.setChildrenPos();
        return this;
    }

    setPadding(padding) {
        this.padding = padding;
        this.setChildrenPos();
        return this;
    }

    setChildrenPos() {        
        if (!this.child && this.child.entity.enabled) return;

        const [top, right, bottom, left] = this.padding;

        // Recalculate self size: child size + padding
        this.width = this.child.width + left + right;
        this.height = this.child.height + top + bottom;
        this.rectangle.scale = [this.width, this.height, 1];

        // Reposition child inside
        const [x, y] = this.position;
        const childX = x + left;
        const childY = y + top;
        this.child.setPosition(childX, childY);
    }

    setPosition(x, y) {
        this.position = [x, y];
        this.rectangle.position = [x, y, this.rectangle.position[2]];

        // Update child pos too
        this.setChildrenPos();
        return this;
    }
}

// UIPanel: Container element that has a specific size, making the inner content
//          only render what fits in the panel. It should include scroll
//          behaiviour and scroll bar if inner content excedes the panel size.
// class UIPanel extends UIElement {...



// --- OKAY!!!
// --- How to use all this elements above? ---
export class UIManager {
    constructor(gameInstance) {
        this.game = gameInstance;
        this.elements = new Map(); // name -> UIElement
        this.indexCounter = 0;
        this.zlayer = -1000;

        // Root element (screen)
        this.root = this.spawnContainer(game.canvas.width, game.canvas.height, [0,0.06,0,0.94 ])
        this.root.entity.registerCall("refreshUI", () => {
            this.root.setSize(game.canvas.width, game.canvas.height)
        });

    }

    // spawn methods
    spawnElement(
        width = 100,
        height = 100,
        color = [1, 1, 1, 0.1],
    ) {
        const name = this._generateName("element");
        const element = new UIElement(name, color, width, height, this.zlayer);
        this.elements.set(name, element);
        return element;
    }

    spawnText(
        text = "Text",
        textScale = 1,
        maxWidth = 260,
        color = [0, 0.7, 0, 1],
        bgColor = [0, 0.1, 0, 1],
    ) {
        const name = this._generateName("text");
        const textElement = new UIText(name, text, textScale, maxWidth, color, bgColor, this.zlayer);
        this.elements.set(name, textElement);
        return textElement;
    }    

    spawnSprite(
        texture = "editorIcons",   // texture name from manifest.json
        width = 64,                // width in pixels
        height = 64,               // height in pixels
        frame = 0,                 // sprite sheet frame
        tileSetSize = [1, 1],      // tiles in texture
        color = [1,1,1,0.2]
    ) {
        const name = this._generateName("sprite");
        const sprite = new UISprite(name, texture, width, height, frame, tileSetSize, color, this.zlayer);
        this.elements.set(name, sprite);
        return sprite;
    }    

    spawnArray(
        vertical = true,
        align = "left", // or "center", "right"
        spacing = 5,
        color = [0.1, 0.2, 0.1, 0.8],
    ) {
        const name = this._generateName("array");
        const array = new UIArray(name, vertical, align, spacing, color, this.zlayer);
        this.elements.set(name, array);
        return array;
    }

    spawnContainer(
        width = 300,
        height = 200,
        color = [0.06, 0.15, 0.06, 1],
    ) {
        const name = this._generateName("container");
        const container = new UIContainer(name, color, width, height, this.zlayer);
        this.elements.set(name, container);
        return container;
    }

    spawnPadding(
        padding = [10, 10, 10, 10],
        color = [0.1, 0.1, 0.1, 0.1],
    ) {
        const name = this._generateName("padding");
        const pad = new UIPadding(name, padding, color, this.zlayer);
        this.elements.set(name, pad);
        return pad;
    }

    // other methods
    _generateName(type) {
        return `ui-${this.indexCounter++}-${type}`;
    }

    destroyElement(element) {
        // Recursively destroy children if any
        if (element.children) {
            for (const child of element.children) {
                // UIContainer keeps {child, selfAnchor, childAnchor}, others keep child directly
                this.destroyElement(child.child || child);
            }
        }
        if (element.child) { // UIPadding has a single child
            this.destroyElement(element.child);
        }
    
        this.elements.delete(element.name);
        this.game.destroyEntity(element.entity);
    }    

    clearAll() {
        for (const [name, element] of this.elements.entries()) {
            this.game.destroyEntity(element.entity);
        }
        this.elements.clear();
    }

    addColliderToElem(elem) {
        // 1. Create polygon shape
        const elemVerts = [
            [0, 0, 0],
            [elem.width, 0, 0],
            [elem.width, elem.height, 0],
            [0, elem.height, 0]
        ];

        const elemShape = new Polygon(
            game,
            [elem.position[0], elem.position[1], 0],
            [1, 1, 1],
            0,
            elemVerts
        );

        // 2. Add collider component
        const elemCollider = elem.entity.addComponent(Collider, elemShape);

        // 3. Update collider position automatically on UI refresh
        elem.entity.registerCall("refreshUI", () => {
            elemShape.position = [elem.position[0], elem.position[1], 0];
            elemCollider.updateRect();
        });

        // update polygon verts if element size changes
        elem.entity.registerCall("refreshUI", () => {
            const newVerts = [
                [0, 0, 0],
                [elem.width, 0, 0],
                [elem.width, elem.height, 0],
                [0, elem.height, 0]
            ];
            elemShape.rawVerts = newVerts;
            elemShape._flatVertArray = newVerts.flat();
            elemShape._rawCenter = vec3.create();
            for (const vert of newVerts) vec3.add(elemShape._rawCenter, elemShape._rawCenter, vert);
            vec3.scale(elemShape._rawCenter, elemShape._rawCenter, 1 / newVerts.length);
            elemCollider.updateRect();
        });

        return elemCollider;
    }

    newSine(min, max, speed = 1000, offset = 0) {
        // speed = duration of one full sine cycle in ms
        // offset = phase shift (optional, defaults to 0)
        const t = Date.now() / speed + offset;
        const sine = Math.sin(t); // oscillates between -1 and 1
        return min + (sine + 1) / 2 * (max - min);
    }

    interpolation(current, target, speed = 10, snapThreshold = 0.5, easing = true) {
        // how far we need to move this frame
        let t = Math.min(speed * game.deltaTime, 1);
    
        // optional ease in/out (smoothstep)
        if (easing) {
            t = t * t * (3 - 2 * t); // smoothstep curve
        }
    
        const value = current + (target - current) * t;
    
        // snap to target if we're very close (avoids jitter)
        if (Math.abs(value - target) < snapThreshold) {
            return target;
        }
    
        return value;
    }
    
}