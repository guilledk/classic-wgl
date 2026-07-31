import game from "/classic/state.js";
import { Rectangle, Text, Sprite } from "/classic/transforms.js";

import { Polygon, Collider } from "/classic/collision.js";
import { vec3 } from "/lib/gl-matrix/index.js";

// // Objective: Final API based on UIManager class,

// Flag the UI system (game.ui, a UIManager) so it performs a single
// layout pass on the next frame. Called by UI components whenever a
// mutation affects the layout (sizes, children, enabled state, ...).
function markUIDirty(component) {
    if (component.game.ui)
        component.game.ui.markDirty();
}

// --- Most basic UI element of the system is UIElement,
//     from this other elements can be extended ---

// A UIElement is a Rectangle component (Component -> Transform -> Drawable
// -> Rectangle), following the standard component inheritance model: it gets
// this.game and this.entity automatically when added with addComponent.
// Its just an object that ocupies some 2d space in the screen.
// Entities are spawned by the UIManager system, never by constructors.
class UIElement extends Rectangle {
    constructor(
        entity, //: Entity
        color, //: [r, g, b, a] -> number between 0-1
        width, //: number -> pixels
        height, //: number -> pixels
        zlayer //: number
    ) {
        super(
            entity,
            [0, 0, zlayer], // pos
            [width, height, 1], // scale
            color, // color
            true // ignoreCam
        );
    }

    // width/height in pixels map directly onto the Transform scale
    get width() { return this.scale[0]; }
    set width(value) { this.scale[0] = value; }

    get height() { return this.scale[1]; }
    set height(value) { this.scale[1] = value; }

    setPosition(x, y) {
        this.position[0] = x;
        this.position[1] = y;

        if (typeof this.setChildrenPos === "function") {
            this.setChildrenPos();
        }
        return this;
    }

    setSize(width, height) {
        if (this.width === width && this.height === height)
            return this;

        this.width = width;
        this.height = height;
        markUIDirty(this);
        return this;
    }

    setColor(rgba) {
        this.color = rgba;  // array [r,g,b,a]
        return this;
    }

    // Container subclasses override this to expose their children with a
    // uniform interface, regardless of how they store them internally.
    getChildren() {
        return [];
    }

    setEnabled(flag) { // Desactivates the element's entity(render & collider) and ocupied space in the ui layout.
        if (this.entity.enabled !== flag)
            markUIDirty(this);
        this.entity.enabled = flag;

        // cascade to children
        for (const child of this.getChildren()) {
            if (child.setEnabled)
                child.setEnabled(flag);
        }

        return this;
    }   
}

// UIText extends the engine Text component directly, adding word wrap and
// layout behaviour on top of it. Wrapped lines live on the internal glyph
// grid of the Text component (maxCharSize = [cols, rows]), so a single
// component handles multi-line text.
class UIText extends Text {
    constructor(entity, text, textScale, maxWidth, color, bgColor, zlayer) {
        const fontSize = [16, 16];
        const glyphSize = [32, 32];
        const glyphStr = "!\"#$%&'()*+,-./?0123456789:;<=>@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`{|}~";

        super(
            entity,
            [0, 0, zlayer], // pos
            [textScale, textScale, 1], // scale
            "font", // texture font
            [1, 1], // initial capacity (cols, rows)
            fontSize,
            glyphSize,
            glyphStr,
            color,
            bgColor,
            true // ignoreCam
        );

        this.rawText = text;
        this.maxWidth = maxWidth;

        // Initialize
        this._recalculateTextElement();
    }

    // element size in pixels comes from the glyph grid and the scale
    get width() { return this.maxCharSize[0] * this.glyphSize[0] * this.scale[0]; }
    get height() { return this.maxCharSize[1] * this.glyphSize[1] * this.scale[1]; }

    get textScale() { return this.scale[0]; }

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
        if (str === this.rawText)
            return this;

        this.rawText = str;
        this._recalculateTextElement();
        markUIDirty(this);
        return this;
    }

    setTextScale(newScale) {
        if (newScale === this.scale[0])
            return this;

        this.scale = [newScale, newScale, 1];
        this._recalculateTextElement();
        markUIDirty(this);
        return this;
    }

    setTextColor(rgba) {
        // Text.rawDraw colorizes with this.color, no glyph redraw needed
        this.color = rgba; // array [r,g,b,a]
        return this;
    }

    setColor(rgba) {
        // background color, applied when the glyph buffer is cleared
        this.bgcolor = rgba; // array [r,g,b,a]
        this._recalculateTextElement();
        return this;
    }

    setMaxWidth(number) {
        if (number === this.maxWidth)
            return this;

        this.maxWidth = number
        this._recalculateTextElement();
        markUIDirty(this);
        return this;
    }

    _recalculateTextElement() {
        const scaledGlyphWidth = this.glyphSize[0] * this.scale[0];
        this.maxCharPerLine = Math.max(1, Math.floor(this.maxWidth / scaledGlyphWidth));
        const lines = UIText.wrapText(
            (this.rawText || "").toUpperCase(), this.maxCharPerLine);

        // resize the glyph grid to fit the wrapped content
        const cols = Math.max(1, ...lines.map(l => l.length));
        const rows = Math.max(1, lines.length);
        this.setMaxCharSize(cols, rows);

        // pad every line with spaces so each one fills a full row
        // of the glyph grid (the cursor wraps at maxCharSize[0])
        super.setText(lines.map(l => l.padEnd(cols, ' ')).join(''));
    }
    
    setSize() {
        console.error("to adjust text size use setTextScale(newScale)");
        
    }

    setPosition(x, y) {
        this.position[0] = x;
        this.position[1] = y;
        return this;
    }

    setEnabled(flag) {
        if (this.entity.enabled !== flag)
            markUIDirty(this);
        this.entity.enabled = flag;
        return this;
    }
}

// UISprite extends the engine Sprite component directly, exposing the same
// pixel based width/height layout interface as the other UI elements.
class UISprite extends Sprite {
    constructor(
        entity, //: Entity
        texture,       //: string -> texture name from manifest.json
        width,         //: number -> pixels
        height,        //: number -> pixels
        frame,     //: number -> sprite sheet frame
        tileSetSize, //: number -> tiles in texture
        zlayer //: number
    ) {
        super(
            entity,
            [0, 0, zlayer], // pos
            [1, 1, 1], // scale, set right after through setSize
            texture,
            true,             // ignoreCam → screen-space
            frame,
            tileSetSize,
            [0, 0] // dont change this anchor, use container element instead (e.g. UIContainer)
        );

        this.setSize(width, height);
    }

    // width/height in pixels map onto the texture relative Sprite scale
    get width() { return this.scale[0] * this.texture.image.width; }
    set width(value) { this.scale[0] = value / this.texture.image.width; }

    get height() { return this.scale[1] * this.texture.image.height; }
    set height(value) { this.scale[1] = value / this.texture.image.height; }

    setPosition(x, y) {
        this.position[0] = x;
        this.position[1] = y;
        return this;
    }

    setSize(width, height) {
        if (this.width === width && this.height === height)
            return this;

        this.width = width;
        this.height = height;
        markUIDirty(this);
        return this;
    }

    setFrame(frame) {
        this.frame = frame;
        return this;
    }

    setEnabled(flag) {
        if (this.entity.enabled !== flag)
            markUIDirty(this);
        this.entity.enabled = flag;
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
        entity, //: Entity
        color, //: [n, n, n, n] -> number between 0-1
        width, //: number -> pixels
        height, //: number -> pixels
        zlayer //: number
    ) {
        super(entity, color, width, height, zlayer);
        this.children = [];
        this.anchor = "mid-center"; // default anchor used for self & children
    }

    addChild(child, selfAnchor = this.anchor, childAnchor = this.anchor) {
        this.children.push({ child, selfAnchor, childAnchor });
        markUIDirty(this);
        return this;
    }

    getChildren() {
        // children are stored as {child, selfAnchor, childAnchor} entries
        return this.children.map(entry => entry.child);
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
        entity, //: Entity
        vertical, //: bool
        align, //: left" | "center" | "right"
        spacing, //: number -> pixels
        color, //: [r, g, b, a] -> number between 0-1
        zlayer //: number
    ) {
        super(entity, color, 10, 10, zlayer);
        this.vertical = vertical;
        this.align = align;
        this.spacing = spacing;
        this.children = [];
    }

    addChild(child) {
        this.children.push(child);
        // recompute own size right away so it can be queried during init,
        // final positions get resolved on the next layout pass
        this.setChildrenPos();
        markUIDirty(this);
        return this;
    }

    getChildren() {
        // children are stored directly
        return this.children;
    }

    setVertical(flag) {
        if (this.vertical === !!flag)
            return this;

        this.vertical = !!flag;
        markUIDirty(this);
        return this;
    }

    setAlign(option) {
        if (this.align === option)
            return this;

        this.align = option //: left" | "center" | "right"
        markUIDirty(this);
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
        entity, //: Entity
        padding, //: [top, right, bottom, left]
        color, //: [r, g, b, a]
        zlayer, //: number
    ) {
        // Start with dummy size; will be recalculated later
        super(entity, color, 10, 10, zlayer);

        this.padding = padding;
        this.child = null;
    }

    addChild(child) {
        if (this.child) {
            throw new Error("UIPadding can only have one child!");
        }
        this.child = child;
        // recompute own size right away so it can be queried during init,
        // final positions get resolved on the next layout pass
        this.setChildrenPos();
        markUIDirty(this);
        return this;
    }

    getChildren() {
        // single child container
        return this.child ? [this.child] : [];
    }

    setPadding(padding) {
        this.padding = padding;
        markUIDirty(this);
        return this;
    }

    setChildrenPos() {        
        if (!this.child || !this.child.entity.enabled) return;

        const [top, right, bottom, left] = this.padding;

        // Recalculate self size: child size + padding
        this.width = this.child.width + left + right;
        this.height = this.child.height + top + bottom;

        // Reposition child inside
        const [x, y] = this.position;
        const childX = x + left;
        const childY = y + top;
        this.child.setPosition(childX, childY);
    }
}

// UIPanel: Container element that has a specific size, making the inner content
//          only render what fits in the panel. It should include scroll
//          behaiviour and scroll bar if inner content excedes the panel size.
// class UIPanel extends UIElement {...



// --- OKAY!!!
// --- How to use all this elements above? ---
// UIManager is a "system": a piece of code that operates on a set of
// entities / components in a specific way (like PhysicsProvider does for
// Collider components). It owns entity spawning for all UI elements.
export class UIManager {
    constructor(gameInstance) {
        this.game = gameInstance;
        this.game.ui = this; // expose the system, like game.physics

        this.elements = new Map(); // name -> UIElement
        this.indexCounter = 0;
        this.zlayer = -1000;

        this.dirty = true;
        this._elementColliders = [];

        // Root element (screen)
        this.root = this.spawnContainer(this.game.canvas.width, this.game.canvas.height, [0,0.06,0,0.94 ])

        // The UI only refreshes positions and sizes / scaling after the
        // canvas resize event we get from the browser...
        this.root.entity.registerCall("canvasResize", () => {
            this.root.setSize(this.game.canvas.width, this.game.canvas.height);
        });

        // ...or when a mutation marked the layout dirty: a single layout
        // pass runs on the next frame (elements that need per frame logic
        // just use the normal "update" call).
        this.root.entity.registerCall("update", () => {
            if (this.dirty)
                this.refreshLayout();
        });
    }

    markDirty() {
        this.dirty = true;
    }

    refreshLayout() {
        // 1) measure bottom-up: containers that size to content
        //    (UIArray, UIPadding) get correct sizes from the leaves up
        this._measure(this.root);

        // 2) position top-down: setChildrenPos cascades through
        //    setPosition recursively from the root
        this.root.setChildrenPos();

        // 3) keep collider shapes in sync with the new layout
        this._syncColliders();

        this.dirty = false;
    }

    _measure(element) {
        if (!element.entity.enabled)
            return;

        if (typeof element.getChildren === "function")
            for (const child of element.getChildren())
                this._measure(child);

        if (typeof element.setChildrenPos === "function")
            element.setChildrenPos();
    }

    // Generic spawner: creates the entity and attaches the UI component
    // to it, following the same pattern used by the rest of the engine
    // (spawnEntity + addComponent), instead of spawning inside constructors.
    _spawnUIComponent(type, componentClass, ...args) {
        const name = this._generateName(type);
        const entity = this.game.spawnEntity(name);
        const element = entity.addComponent(componentClass, ...args);
        this.elements.set(name, element);
        return element;
    }

    // spawn methods
    spawnElement(
        width = 100,
        height = 100,
        color = [1, 1, 1, 0.1],
    ) {
        return this._spawnUIComponent(
            "element", UIElement, color, width, height, this.zlayer);
    }

    spawnText(
        text = "Text",
        textScale = 1,
        maxWidth = 260,
        color = [0, 0.7, 0, 1],
        bgColor = [0, 0.1, 0, 1],
    ) {
        return this._spawnUIComponent(
            "text", UIText, text, textScale, maxWidth, color, bgColor, this.zlayer);
    }    

    spawnSprite(
        texture = "editorIcons",   // texture name from manifest.json
        width = 64,                // width in pixels
        height = 64,               // height in pixels
        frame = 0,                 // sprite sheet frame
        tileSetSize = [1, 1],      // tiles in texture
    ) {
        return this._spawnUIComponent(
            "sprite", UISprite, texture, width, height, frame, tileSetSize, this.zlayer);
    }    

    spawnArray(
        vertical = true,
        align = "left", // or "center", "right"
        spacing = 5,
        color = [0.1, 0.2, 0.1, 0.8],
    ) {
        return this._spawnUIComponent(
            "array", UIArray, vertical, align, spacing, color, this.zlayer);
    }

    spawnContainer(
        width = 300,
        height = 200,
        color = [0.06, 0.15, 0.06, 1],
    ) {
        return this._spawnUIComponent(
            "container", UIContainer, color, width, height, this.zlayer);
    }

    spawnPadding(
        padding = [10, 10, 10, 10],
        color = [0.1, 0.1, 0.1, 0.1],
    ) {
        return this._spawnUIComponent(
            "padding", UIPadding, padding, color, this.zlayer);
    }

    // other methods
    _generateName(type) {
        return `ui-${this.indexCounter++}-${type}`;
    }

    destroyElement(element) {
        // Recursively destroy children if any
        if (typeof element.getChildren === "function")
            for (const child of element.getChildren())
                this.destroyElement(child);

        this.elements.delete(element.entity.name);
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
            this.game,
            [elem.position[0], elem.position[1], 0],
            [1, 1, 1],
            0,
            elemVerts
        );

        // 2. Add collider component
        const elemCollider = elem.entity.addComponent(Collider, elemShape);

        // 3. Keep it in sync with the element after every layout pass
        //    (see _syncColliders, called from refreshLayout)
        this._elementColliders.push({
            elem: elem,
            shape: elemShape,
            collider: elemCollider
        });

        return elemCollider;
    }

    _syncColliders() {
        for (const { elem, shape, collider } of this._elementColliders) {
            // position
            shape.position = [elem.position[0], elem.position[1], 0];

            // verts (element size may have changed)
            const newVerts = [
                [0, 0, 0],
                [elem.width, 0, 0],
                [elem.width, elem.height, 0],
                [0, elem.height, 0]
            ];
            shape.rawVerts = newVerts;
            shape._flatVertArray = newVerts.flat();
            shape._rawCenter = vec3.create();
            for (const vert of newVerts)
                vec3.add(shape._rawCenter, shape._rawCenter, vert);
            vec3.scale(shape._rawCenter, shape._rawCenter, 1 / newVerts.length);

            // keep the aabb used by the quadtree in sync too
            shape._rawMin = vec3.create();
            shape._rawMax = vec3.fromValues(elem.width, elem.height, 0);

            collider.updateRect();
        }
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