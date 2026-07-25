import game from '/classic/state.js';
import { Rectangle, Text, Sprite } from '/classic/transforms.js';
import { Polygon, Collider } from '/classic/collision.js';
import type { IEntity, IGameState, IComponent, ICollider } from './types.js';
import { vec3 } from 'gl-matrix';

type Color = [number, number, number, number] | number[];
type Vec3Like = vec3 | [number, number, number] | number[];

// Common interface for all UI components
export interface UIComponentBase {
    entity: IEntity;
    game: IGameState;
    position: vec3 | number[];
    width: number;
    height: number;
    setPosition(x: number, y: number): this;
    setEnabled(flag: boolean): this;
}

// Union type for any UI component that can be a child
export type UIChild = UIElement | UIText | UISprite;

// UIManager interface extension for game state
declare module './types.js' {
    interface IGameState {
        ui?: UIManager;
    }
}

// Flag the UI system so it performs a single layout pass on the next frame
function markUIDirty(component: { game: IGameState }): void {
    if (component.game.ui) {
        component.game.ui.markDirty();
    }
}

// UIElement is a Rectangle component with pixel-based sizing
export class UIElement extends Rectangle {
    constructor(entity: IEntity, color: Color, width: number, height: number, zlayer: number) {
        super(entity, [0, 0, zlayer], [width, height, 1], color, true);
    }

    get width(): number {
        return this.scale[0];
    }
    set width(value: number) {
        this.scale[0] = value;
    }

    get height(): number {
        return this.scale[1];
    }
    set height(value: number) {
        this.scale[1] = value;
    }

    setPosition(x: number, y: number): this {
        this.position[0] = x;
        this.position[1] = y;

        if (
            typeof (this as unknown as { setChildrenPos?: () => void }).setChildrenPos ===
            'function'
        ) {
            (this as unknown as { setChildrenPos: () => void }).setChildrenPos();
        }
        return this;
    }

    setSize(width: number, height: number): this {
        if (this.width === width && this.height === height) return this;

        this.width = width;
        this.height = height;
        markUIDirty(this);
        return this;
    }

    setColor(rgba: Color): this {
        this.color = rgba;
        return this;
    }

    getChildren(): UIChild[] {
        return [];
    }

    setEnabled(flag: boolean): this {
        if (this.entity.enabled !== flag) markUIDirty(this);
        this.entity.enabled = flag;

        for (const child of this.getChildren()) {
            if (child.setEnabled) child.setEnabled(flag);
        }

        return this;
    }
}

// UIText extends Text with word wrap and layout behavior
export class UIText extends Text {
    rawText: string;
    maxWidth: number;
    maxCharPerLine: number = 1;

    constructor(
        entity: IEntity,
        text: string,
        textScale: number,
        maxWidth: number,
        color: Color,
        bgColor: Color,
        zlayer: number,
    ) {
        const fontSize: [number, number] = [16, 16];
        const glyphSize: [number, number] = [32, 32];
        const glyphStr = '!"#$%&\'()*+,-./?0123456789:;<=>@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`{|}~';

        super(
            entity,
            [0, 0, zlayer],
            [textScale, textScale, 1],
            'font',
            [1, 1],
            fontSize,
            glyphSize,
            glyphStr,
            color,
            bgColor,
            true,
        );

        this.rawText = text;
        this.maxWidth = maxWidth;
        this._recalculateTextElement();
    }

    get width(): number {
        return this.maxCharSize[0] * this.glyphSize[0] * this.scale[0];
    }
    get height(): number {
        return this.maxCharSize[1] * this.glyphSize[1] * this.scale[1];
    }
    get textScale(): number {
        return this.scale[0];
    }

    static wrapText(str: string, maxCharPerLine: number): string[] {
        const words = str.split(' ');
        const lines: string[] = [];
        let line = '';

        for (const word of words) {
            if ((line + (line.length ? ' ' : '') + word).length <= maxCharPerLine) {
                line += (line.length ? ' ' : '') + word;
            } else {
                if (line.length > 0) lines.push(line);
                line = word;
            }
        }
        if (line.length) lines.push(line);

        return lines;
    }

    // Override Text.setText to use word-wrap logic
    setText(str: string): this {
        if (str === this.rawText) return this;

        this.rawText = str;
        this._recalculateTextElement();
        markUIDirty(this);
        return this;
    }

    setTextScale(newScale: number): this {
        if (newScale === this.scale[0]) return this;

        this.scale = vec3.fromValues(newScale, newScale, 1);
        this._recalculateTextElement();
        markUIDirty(this);
        return this;
    }

    setTextColor(rgba: Color): this {
        this.color = rgba;
        return this;
    }

    setColor(rgba: Color): this {
        this.bgcolor = rgba;
        this._recalculateTextElement();
        return this;
    }

    setMaxWidth(number: number): this {
        if (number === this.maxWidth) return this;

        this.maxWidth = number;
        this._recalculateTextElement();
        markUIDirty(this);
        return this;
    }

    _recalculateTextElement(): void {
        const scaledGlyphWidth = this.glyphSize[0] * this.scale[0];
        this.maxCharPerLine = Math.max(1, Math.floor(this.maxWidth / scaledGlyphWidth));
        const lines = UIText.wrapText((this.rawText || '').toUpperCase(), this.maxCharPerLine);

        const cols = Math.max(1, ...lines.map((l) => l.length));
        const rows = Math.max(1, lines.length);
        this.setMaxCharSize(cols, rows);

        super.setText(lines.map((l) => l.padEnd(cols, ' ')).join(''));
    }

    setPosition(x: number, y: number): this {
        this.position[0] = x;
        this.position[1] = y;
        return this;
    }

    setEnabled(flag: boolean): this {
        if (this.entity.enabled !== flag) markUIDirty(this);
        this.entity.enabled = flag;
        return this;
    }
}

// UISprite extends Sprite with pixel-based sizing
export class UISprite extends Sprite {
    constructor(
        entity: IEntity,
        texture: string,
        width: number,
        height: number,
        frame: number,
        tileSetSize: [number, number],
        zlayer: number,
    ) {
        super(entity, [0, 0, zlayer], [1, 1, 1], texture, true, frame, tileSetSize, [0, 0]);
        this.setSize(width, height);
    }

    get width(): number {
        return this.scale[0] * this.texture.image.width;
    }
    set width(value: number) {
        this.scale[0] = value / this.texture.image.width;
    }

    get height(): number {
        return this.scale[1] * this.texture.image.height;
    }
    set height(value: number) {
        this.scale[1] = value / this.texture.image.height;
    }

    setPosition(x: number, y: number): this {
        this.position[0] = x;
        this.position[1] = y;
        return this;
    }

    setSize(width: number, height: number): this {
        if (this.width === width && this.height === height) return this;
        this.width = width;
        this.height = height;
        markUIDirty(this);
        return this;
    }

    setFrame(frame: number): this {
        this.frame = frame;
        return this;
    }

    setEnabled(flag: boolean): this {
        if (this.entity.enabled !== flag) markUIDirty(this);
        this.entity.enabled = flag;
        return this;
    }
}

type AnchorType =
    | 'top-left'
    | 'top-center'
    | 'top-right'
    | 'mid-left'
    | 'mid-center'
    | 'mid-right'
    | 'bot-left'
    | 'bot-center'
    | 'bot-right';

interface ChildEntry {
    child: UIChild;
    selfAnchor: AnchorType;
    childAnchor: AnchorType;
}

// UIContainer positions children based on anchors
export class UIContainer extends UIElement {
    children: ChildEntry[];
    anchor: AnchorType;

    constructor(entity: IEntity, color: Color, width: number, height: number, zlayer: number) {
        super(entity, color, width, height, zlayer);
        this.children = [];
        this.anchor = 'mid-center';
    }

    addChild(
        child: UIChild,
        selfAnchor: AnchorType = this.anchor,
        childAnchor: AnchorType = this.anchor,
    ): this {
        this.children.push({ child, selfAnchor, childAnchor });
        markUIDirty(this);
        return this;
    }

    getChildren(): UIChild[] {
        return this.children.map((entry) => entry.child);
    }

    getAnchorOffset(anchor: AnchorType, w: number, h: number): { x: number; y: number } {
        const map: Record<AnchorType, { x: number; y: number }> = {
            'top-left': { x: 0, y: 0 },
            'top-center': { x: w / 2, y: 0 },
            'top-right': { x: w, y: 0 },
            'mid-left': { x: 0, y: h / 2 },
            'mid-center': { x: w / 2, y: h / 2 },
            'mid-right': { x: w, y: h / 2 },
            'bot-left': { x: 0, y: h },
            'bot-center': { x: w / 2, y: h },
            'bot-right': { x: w, y: h },
        };
        return map[anchor];
    }

    setChildrenPos(): void {
        const [panelX, panelY] = this.position;

        for (const { child, selfAnchor, childAnchor } of this.children) {
            if (!child.entity.enabled) continue;

            const panelOffset = this.getAnchorOffset(selfAnchor, this.width, this.height);
            const childOffset = this.getAnchorOffset(childAnchor, child.width, child.height);

            const x = panelX + panelOffset.x - childOffset.x;
            const y = panelY + panelOffset.y - childOffset.y;

            child.setPosition(x, y);
        }
    }
}

type AlignType = 'left' | 'center' | 'right';

// UIArray positions children in a flex-like layout
export class UIArray extends UIElement {
    vertical: boolean;
    align: AlignType;
    spacing: number;
    children: UIChild[];

    constructor(
        entity: IEntity,
        vertical: boolean,
        align: AlignType,
        spacing: number,
        color: Color,
        zlayer: number,
    ) {
        super(entity, color, 10, 10, zlayer);
        this.vertical = vertical;
        this.align = align;
        this.spacing = spacing;
        this.children = [];
    }

    addChild(child: UIChild): this {
        this.children.push(child);
        this.setChildrenPos();
        markUIDirty(this);
        return this;
    }

    getChildren(): UIChild[] {
        return this.children;
    }

    setVertical(flag: boolean): this {
        if (this.vertical === !!flag) return this;
        this.vertical = !!flag;
        markUIDirty(this);
        return this;
    }

    setAlign(option: AlignType): this {
        if (this.align === option) return this;
        this.align = option;
        markUIDirty(this);
        return this;
    }

    setChildrenPos(): void {
        const isVertical = this.vertical;

        // Step 1: Measure layout size
        let totalMain = 0;
        let maxCross = 0;

        for (const child of this.children) {
            if (!child.entity.enabled) continue;

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
            if (!child.entity.enabled) continue;

            const main = isVertical ? child.height : child.width;
            const cross = isVertical ? child.width : child.height;

            let crossOffset = 0;
            if (this.align === 'center') {
                crossOffset = (isVertical ? this.width : this.height) / 2 - cross / 2;
            } else if (this.align === 'right') {
                crossOffset = (isVertical ? this.width : this.height) - cross;
            }

            const x = isVertical ? startX + crossOffset : startX + offset;
            const y = isVertical ? startY + offset : startY + crossOffset;

            child.setPosition(x, y);
            offset += main + this.spacing;
        }
    }
}

// UIPadding wraps a single child with padding
export class UIPadding extends UIElement {
    padding: [number, number, number, number];
    child: UIChild | null;

    constructor(
        entity: IEntity,
        padding: [number, number, number, number],
        color: Color,
        zlayer: number,
    ) {
        super(entity, color, 10, 10, zlayer);
        this.padding = padding;
        this.child = null;
    }

    addChild(child: UIChild): this {
        if (this.child) {
            throw new Error('UIPadding can only have one child!');
        }
        this.child = child;
        this.setChildrenPos();
        markUIDirty(this);
        return this;
    }

    getChildren(): UIChild[] {
        return this.child ? [this.child] : [];
    }

    setPadding(padding: [number, number, number, number]): this {
        this.padding = padding;
        markUIDirty(this);
        return this;
    }

    setChildrenPos(): void {
        if (!this.child || !this.child.entity.enabled) return;

        const [top, right, bottom, left] = this.padding;

        this.width = this.child.width + left + right;
        this.height = this.child.height + top + bottom;

        const [x, y] = this.position;
        const childX = x + left;
        const childY = y + top;
        this.child.setPosition(childX, childY);
    }
}

interface ColliderEntry {
    elem: UIElement;
    shape: Polygon;
    collider: Collider;
}

// UIManager system for UI elements
export class UIManager {
    game: IGameState;
    elements: Map<string, UIElement>;
    indexCounter: number;
    zlayer: number;
    dirty: boolean;
    _elementColliders: ColliderEntry[];
    root: UIContainer;

    constructor(gameInstance: IGameState) {
        this.game = gameInstance;
        (this.game as IGameState & { ui: UIManager }).ui = this;

        this.elements = new Map();
        this.indexCounter = 0;
        this.zlayer = -1000;

        this.dirty = true;
        this._elementColliders = [];

        // Root element (screen)
        this.root = this.spawnContainer(
            this.game.canvas!.width,
            this.game.canvas!.height,
            [0, 0, 0, 0],
        );

        this.root.entity.registerCall('canvasResize', () => {
            this.root.setSize(this.game.canvas!.width, this.game.canvas!.height);
        });

        this.root.entity.registerCall('update', () => {
            if (this.dirty) this.refreshLayout();
        });
    }

    markDirty(): void {
        this.dirty = true;
    }

    refreshLayout(): void {
        this._measure(this.root);
        this.root.setChildrenPos();
        this._syncColliders();
        this.dirty = false;
    }

    _measure(element: UIChild): void {
        if (!element.entity.enabled) return;

        if (typeof (element as UIElement).getChildren === 'function') {
            for (const child of (element as UIElement).getChildren()) {
                this._measure(child);
            }
        }

        if (typeof (element as UIContainer).setChildrenPos === 'function') {
            (element as UIContainer).setChildrenPos();
        }
    }

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    _spawnUIComponent<T extends UIElement>(
        type: string,
        componentClass: new (entity: IEntity, ...args: any[]) => T,
        ...args: any[]
    ): T {
        const name = this._generateName(type);
        const entity = this.game.spawnEntity(name);
        const element = entity.addComponent(componentClass, ...args) as T;
        this.elements.set(name, element);
        return element;
    }

    spawnElement(
        width: number = 100,
        height: number = 100,
        color: Color = [1, 1, 1, 0.1],
    ): UIElement {
        return this._spawnUIComponent('element', UIElement, color, width, height, this.zlayer);
    }

    spawnText(
        text: string = 'Text',
        textScale: number = 1,
        maxWidth: number = 260,
        color: Color = [0, 0.7, 0, 1],
        bgColor: Color = [0, 0.1, 0, 1],
    ): UIText {
        const name = this._generateName('text');
        const entity = this.game.spawnEntity(name);
        const element = entity.addComponent(
            UIText,
            text,
            textScale,
            maxWidth,
            color,
            bgColor,
            this.zlayer,
        ) as UIText;
        this.elements.set(name, element as unknown as UIElement);
        return element;
    }

    spawnSprite(
        texture: string = 'editorIcons',
        width: number = 64,
        height: number = 64,
        frame: number = 0,
        tileSetSize: [number, number] = [1, 1],
    ): UISprite {
        const name = this._generateName('sprite');
        const entity = this.game.spawnEntity(name);
        const element = entity.addComponent(
            UISprite,
            texture,
            width,
            height,
            frame,
            tileSetSize,
            this.zlayer,
        ) as UISprite;
        this.elements.set(name, element as unknown as UIElement);
        return element;
    }

    spawnArray(
        vertical: boolean = true,
        align: AlignType = 'left',
        spacing: number = 5,
        color: Color = [0.1, 0.2, 0.1, 0.8],
    ): UIArray {
        return this._spawnUIComponent(
            'array',
            UIArray,
            vertical,
            align,
            spacing,
            color,
            this.zlayer,
        );
    }

    spawnContainer(
        width: number = 300,
        height: number = 200,
        color: Color = [0.06, 0.15, 0.06, 1],
    ): UIContainer {
        return this._spawnUIComponent('container', UIContainer, color, width, height, this.zlayer);
    }

    spawnPadding(
        padding: [number, number, number, number] = [10, 10, 10, 10],
        color: Color = [0.1, 0.1, 0.1, 0.1],
    ): UIPadding {
        return this._spawnUIComponent('padding', UIPadding, padding, color, this.zlayer);
    }

    _generateName(type: string): string {
        return `ui-${this.indexCounter++}-${type}`;
    }

    destroyElement(element: UIChild): void {
        if (typeof (element as UIElement).getChildren === 'function') {
            for (const child of (element as UIElement).getChildren()) {
                this.destroyElement(child);
            }
        }

        this.elements.delete(element.entity.name);
        this.game.destroyEntity(element.entity);
    }

    clearAll(): void {
        for (const [_name, element] of this.elements.entries()) {
            this.game.destroyEntity(element.entity);
        }
        this.elements.clear();
    }

    addColliderToElem(elem: UIElement): Collider {
        const elemVerts: Vec3Like[] = [
            [0, 0, 0],
            [elem.width, 0, 0],
            [elem.width, elem.height, 0],
            [0, elem.height, 0],
        ];

        const elemShape = new Polygon(
            this.game,
            [elem.position[0], elem.position[1], 0],
            [1, 1, 1],
            0,
            elemVerts,
        );

        const elemCollider = elem.entity.addComponent(Collider, elemShape) as Collider;

        this._elementColliders.push({
            elem: elem,
            shape: elemShape,
            collider: elemCollider,
        });

        return elemCollider;
    }

    _syncColliders(): void {
        for (const { elem, shape, collider } of this._elementColliders) {
            shape.position = vec3.fromValues(elem.position[0], elem.position[1], 0);

            const newVerts: vec3[] = [
                vec3.fromValues(0, 0, 0),
                vec3.fromValues(elem.width, 0, 0),
                vec3.fromValues(elem.width, elem.height, 0),
                vec3.fromValues(0, elem.height, 0),
            ];
            shape.rawVerts = newVerts;
            shape._flatVertArray = newVerts.flatMap((v) => [...v]);
            shape._rawCenter = vec3.create();
            for (const vert of newVerts) {
                vec3.add(shape._rawCenter, shape._rawCenter, vert);
            }
            vec3.scale(shape._rawCenter, shape._rawCenter, 1 / newVerts.length);

            shape._rawMin = vec3.create();
            shape._rawMax = vec3.fromValues(elem.width, elem.height, 0);

            collider.updateRect();
        }
    }

    newSine(min: number, max: number, speed: number = 1000, offset: number = 0): number {
        const t = Date.now() / speed + offset;
        const sine = Math.sin(t);
        return min + ((sine + 1) / 2) * (max - min);
    }

    interpolation(
        current: number,
        target: number,
        speed: number = 10,
        snapThreshold: number = 0.5,
        easing: boolean = true,
    ): number {
        let t = Math.min(speed * game.deltaTime, 1);

        if (easing) {
            t = t * t * (3 - 2 * t);
        }

        const value = current + (target - current) * t;

        if (Math.abs(value - target) < snapThreshold) {
            return target;
        }

        return value;
    }
}
