import { describe, it, expect, vi, beforeEach } from 'vitest';
import { Entity } from '/classic/ecs.js';
import { createMockGame } from '../helpers/mockGame.js';
import type { IGameState } from '/classic/types.js';
import type { SdfText } from '/classic/sdfText.js';
import type { SdfFontMetrics } from '/classic/sdfText.js';

const TEST_METRICS: SdfFontMetrics = {
    name: 'test',
    family: 'Test',
    atlasSize: [512, 512],
    glyphSize: 64,
    baseline: 48,
    lineHeight: 83.2,
    glyphs: {
        A: { x: 0, y: 0, w: 64, h: 64, xOffset: 0, yOffset: -56, xAdvance: 52.0 },
        B: { x: 64, y: 0, w: 64, h: 64, xOffset: 0, yOffset: -56, xAdvance: 52.0 },
        C: { x: 128, y: 0, w: 64, h: 64, xOffset: 0, yOffset: -56, xAdvance: 48.0 },
        ' ': { x: 192, y: 0, w: 64, h: 64, xOffset: 0, yOffset: 0, xAdvance: 18.0 },
        W: { x: 256, y: 0, w: 64, h: 64, xOffset: 0, yOffset: -56, xAdvance: 76.0 },
        '.': { x: 320, y: 0, w: 64, h: 64, xOffset: 0, yOffset: -12, xAdvance: 18.0 },
        i: { x: 384, y: 0, w: 64, h: 64, xOffset: -8, yOffset: -56, xAdvance: 22.0 },
        l: { x: 448, y: 0, w: 64, h: 64, xOffset: 0, yOffset: -56, xAdvance: 22.0 },
        H: { x: 0, y: 64, w: 64, h: 64, xOffset: 0, yOffset: -56, xAdvance: 56.0 },
        e: { x: 64, y: 64, w: 64, h: 64, xOffset: 0, yOffset: -44, xAdvance: 44.0 },
        o: { x: 128, y: 64, w: 64, h: 64, xOffset: 0, yOffset: -44, xAdvance: 44.0 },
    },
};

function createMockTexture() {
    return {
        image: { width: 512, height: 512 } as HTMLImageElement,
        bind: vi.fn(),
        name: 'test-sdf',
        gl: {} as WebGLRenderingContext,
        texture: {} as WebGLTexture,
    };
}

function createMockSolidShader() {
    return {
        bind: vi.fn(),
        unbind: vi.fn(),
        gl: {} as WebGLRenderingContext,
        name: 'solid',
        program: {} as WebGLProgram,
        attr: { vertexPos: 0 },
        unif: {
            modelMatrix: {},
            cameraMatrix: {},
            projectionMatrix: {},
            color: {},
        },
    };
}

function createMockShader() {
    return {
        bind: vi.fn(),
        unbind: vi.fn(),
        gl: {} as WebGLRenderingContext,
        name: 'sdf',
        program: {} as WebGLProgram,
        attr: { vertexPos: 0, texCoord: 1 },
        unif: {
            modelMatrix: {},
            cameraMatrix: {},
            projectionMatrix: {},
            texSampler: {},
            color: {},
            outlineColor: {},
            outlineWidth: {},
            softEdge: {},
        },
    };
}

async function loadSdfText() {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    return (await import('/classic/sdfText.js')) as any;
}

async function loadUiSdfText() {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    return (await import('/classic/ui.js')) as any;
}

describe('SdfText', () => {
    let game: IGameState;

    beforeEach(() => {
        game = createMockGame({
            textures: { 'test-sdf': createMockTexture() } as any,
            shaders: { sdf: createMockShader(), solid: createMockSolidShader() } as any,
            sdfFonts: { test: TEST_METRICS } as any,
            getSdfFont: vi.fn((name: string) => {
                return TEST_METRICS;
            }) as any,
        });
    });

    describe('construction', () => {
        it('loads metrics synchronously from game state', async () => {
            const { SdfText: SdfTextClass } = await loadSdfText();
            const entity = new Entity(game, 1, 'sdftext');
            const instance = entity.addComponent(
                SdfTextClass as any,
                [0, 0, 0],
                [1, 1, 1],
                'test',
                [1, 1, 1, 1],
                [0, 0, 0, 0],
                true,
            ) as SdfText;
            expect(instance.metrics).toBe(TEST_METRICS);
            expect(instance.vertexCount).toBe(0);
            expect(instance.color).toEqual([1, 1, 1, 1]);
            expect(instance.bgcolor).toEqual([0, 0, 0, 0]);
            expect(instance.outlineWidth).toBe(0);
            expect(instance.ignoreCam).toBe(true);
        });
    });

    describe('setText', () => {
        it('builds vertex data synchronously', async () => {
            const { SdfText: SdfTextClass } = await loadSdfText();
            const entity = new Entity(game, 1, 'sdftext');
            const instance = entity.addComponent(
                SdfTextClass as any,
                [0, 0, 0],
                [1, 1, 1],
                'test',
                [1, 1, 1, 1],
                [0, 0, 0, 0],
                true,
            ) as SdfText;
            instance.setText('A');

            expect(instance.vertexCount).toBe(6);
            expect(instance.glyphData.length).toBe(6 * 4);
        });

        it('computes correct textWidth from advance values', async () => {
            const { SdfText: SdfTextClass } = await loadSdfText();
            const entity = new Entity(game, 1, 'sdftext');
            const instance = entity.addComponent(
                SdfTextClass as any,
                [0, 0, 0],
                [1, 1, 1],
                'test',
                [1, 1, 1, 1],
                [0, 0, 0, 0],
                true,
            ) as SdfText;
            instance.setText('ABC');

            const expectedWidth = 52 + 52 + 48;
            expect(instance.textWidth).toBeCloseTo(expectedWidth, 0);
        });

        it('distinguishes wide and narrow characters', async () => {
            const { SdfText: SdfTextClass } = await loadSdfText();
            const entity = new Entity(game, 1, 'sdftext');
            const instance = entity.addComponent(
                SdfTextClass as any,
                [0, 0, 0],
                [1, 1, 1],
                'test',
                [1, 1, 1, 1],
                [0, 0, 0, 0],
                true,
            ) as SdfText;
            instance.setText('W');
            const wideWidth = instance.textWidth;

            instance.setText('i');
            const narrowWidth = instance.textWidth;

            expect(wideWidth).toBeGreaterThan(narrowWidth);
        });

        it('handles spaces with space advance', async () => {
            const { SdfText: SdfTextClass } = await loadSdfText();
            const entity = new Entity(game, 1, 'sdftext');
            const instance = entity.addComponent(
                SdfTextClass as any,
                [0, 0, 0],
                [1, 1, 1],
                'test',
                [1, 1, 1, 1],
                [0, 0, 0, 0],
                true,
            ) as SdfText;
            instance.setText('A A');

            const expected = 52 + 18 + 52;
            expect(instance.textWidth).toBeCloseTo(expected, 0);
        });

        it('produces valid UV atlas coordinates', async () => {
            const { SdfText: SdfTextClass } = await loadSdfText();
            const entity = new Entity(game, 1, 'sdftext');
            const instance = entity.addComponent(
                SdfTextClass as any,
                [0, 0, 0],
                [1, 1, 1],
                'test',
                [1, 1, 1, 1],
                [0, 0, 0, 0],
                true,
            ) as SdfText;
            instance.setText('A');

            const data = instance.glyphData;

            for (let v = 0; v < 6; v++) {
                const uvX = data[v * 4 + 2];
                const uvY = data[v * 4 + 3];

                expect(uvX).toBeGreaterThanOrEqual(0);
                expect(uvX).toBeLessThanOrEqual(1);
                expect(uvY).toBeGreaterThanOrEqual(0);
                expect(uvY).toBeLessThanOrEqual(1);
            }
        });

        it('clears vertex data for empty string', async () => {
            const { SdfText: SdfTextClass } = await loadSdfText();
            const entity = new Entity(game, 1, 'sdftext');
            const instance = entity.addComponent(
                SdfTextClass as any,
                [0, 0, 0],
                [1, 1, 1],
                'test',
                [1, 1, 1, 1],
                [0, 0, 0, 0],
                true,
            ) as SdfText;
            instance.setText('A');
            expect(instance.vertexCount).toBe(6);

            instance.setText('');
            expect(instance.vertexCount).toBe(0);
        });
    });

    describe('outline', () => {
        it('defaults to no outline', async () => {
            const { SdfText: SdfTextClass } = await loadSdfText();
            const entity = new Entity(game, 1, 'sdftext');
            const instance = entity.addComponent(
                SdfTextClass as any,
                [0, 0, 0],
                [1, 1, 1],
                'test',
                [1, 1, 1, 1],
                [0, 0, 0, 0],
                true,
            ) as SdfText;
            expect(instance.outlineWidth).toBe(0);
        });

        it('setOutline configures outline width and color', async () => {
            const { SdfText: SdfTextClass } = await loadSdfText();
            const entity = new Entity(game, 1, 'sdftext');
            const instance = entity.addComponent(
                SdfTextClass as any,
                [0, 0, 0],
                [1, 1, 1],
                'test',
                [1, 1, 1, 1],
                [0, 0, 0, 0],
                true,
            ) as SdfText;
            instance.setOutline(0.1, [1, 0, 0, 1]);
            expect(instance.outlineWidth).toBe(0.1);
            expect(instance.outlineColor).toEqual([1, 0, 0, 1]);
        });
    });

    describe('shadow', () => {
        it('defaults to no shadow offset', async () => {
            const { SdfText: SdfTextClass } = await loadSdfText();
            const entity = new Entity(game, 1, 'sdftext');
            const instance = entity.addComponent(
                SdfTextClass as any,
                [0, 0, 0],
                [1, 1, 1],
                'test',
                [1, 1, 1, 1],
                [0, 0, 0, 0],
                true,
            ) as SdfText;
            expect(instance.shadowOffset).toEqual([0, 0]);
        });

        it('setShadow configures shadow offset and color', async () => {
            const { SdfText: SdfTextClass } = await loadSdfText();
            const entity = new Entity(game, 1, 'sdftext');
            const instance = entity.addComponent(
                SdfTextClass as any,
                [0, 0, 0],
                [1, 1, 1],
                'test',
                [1, 1, 1, 1],
                [0, 0, 0, 0],
                true,
            ) as SdfText;
            instance.setShadow(2, 3, [0, 0, 0, 0.8], 1);
            expect(instance.shadowOffset).toEqual([2, 3]);
            expect(instance.shadowColor).toEqual([0, 0, 0, 0.8]);
        });
    });

    describe('rawDraw', () => {
        it('binds shader and draws vertices', async () => {
            const { SdfText: SdfTextClass } = await loadSdfText();
            const entity = new Entity(game, 1, 'sdftext');
            const instance = entity.addComponent(
                SdfTextClass as any,
                [0, 0, 0],
                [1, 1, 1],
                'test',
                [1, 1, 1, 1],
                [0, 0, 0, 0],
                true,
            ) as SdfText;
            instance.setText('ABC');

            const drawArraysSpy = game.gl.drawArrays as ReturnType<typeof vi.fn>;
            instance.rawDraw();

            expect(drawArraysSpy).toHaveBeenCalled();
            const callArgs = drawArraysSpy.mock.calls[0];
            expect(callArgs[0]).toBe(0x0004);
            expect(callArgs[2]).toBe(instance.vertexCount);
        });

        it('skips draw call when vertexCount is 0', async () => {
            const { SdfText: SdfTextClass } = await loadSdfText();
            const entity = new Entity(game, 1, 'sdftext');
            const instance = entity.addComponent(
                SdfTextClass as any,
                [0, 0, 0],
                [1, 1, 1],
                'test',
                [1, 1, 1, 1],
                [0, 0, 0, 0],
                true,
            ) as SdfText;

            const drawArraysSpy = game.gl.drawArrays as ReturnType<typeof vi.fn>;
            drawArraysSpy.mockClear();
            instance.rawDraw();

            expect(drawArraysSpy).not.toHaveBeenCalled();
        });
    });
});

describe('UISdfText', () => {
    let game: IGameState;

    beforeEach(() => {
        game = createMockGame({
            textures: { 'dejavusans-sdf': createMockTexture() } as any,
            shaders: { sdf: createMockShader(), solid: createMockSolidShader() } as any,
            sdfFonts: { dejavusans: TEST_METRICS } as any,
            getSdfFont: vi.fn((name: string) => {
                return { dejavusans: TEST_METRICS }[name];
            }) as any,
        });
    });

    it('builds text synchronously in constructor', async () => {
        const { UISdfText: UISdfTextClass } = await loadUiSdfText();
        const entity = new Entity(game, 1, 'uisdf');
        const instance = entity.addComponent(
            UISdfTextClass as any,
            'Hello World',
            1,
            500,
            [1, 1, 1, 1],
            [0, 0, 0, 0],
            0,
        ) as SdfText;

        expect(instance.text).toBe('Hello World');
        expect(instance.vertexCount).toBe(54);
    });

    it('breaks long text into multiple lines', async () => {
        const { UISdfText: UISdfTextClass } = await loadUiSdfText();
        const entity = new Entity(game, 1, 'uisdf');
        const instance = entity.addComponent(
            UISdfTextClass as any,
            'A B',
            1,
            60,
            [1, 1, 1, 1],
            [0, 0, 0, 0],
            0,
        ) as SdfText;

        expect(instance.text).toBe('A\nB');
    });

    it('handles empty string', async () => {
        const { UISdfText: UISdfTextClass } = await loadUiSdfText();
        const entity = new Entity(game, 1, 'uisdf');
        const instance = entity.addComponent(
            UISdfTextClass as any,
            '',
            1,
            100,
            [1, 1, 1, 1],
            [0, 0, 0, 0],
            0,
        ) as SdfText;

        expect(instance.text).toBe('');
        expect(instance.vertexCount).toBe(0);
    });

    it('updates text synchronously via setText', async () => {
        const { UISdfText: UISdfTextClass } = await loadUiSdfText();
        const entity = new Entity(game, 1, 'uisdf');
        const instance = entity.addComponent(
            UISdfTextClass as any,
            'ABC',
            1,
            400,
            [1, 1, 1, 1],
            [0, 0, 0, 0],
            0,
        ) as SdfText;

        instance.setText('CBA');
        expect(instance.text).toBe('CBA');
    });
});
