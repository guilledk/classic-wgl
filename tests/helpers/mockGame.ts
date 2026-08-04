/**
 * Minimal test doubles for the parts of IGameState / WebGLRenderingContext
 * that core-engine unit tests actually touch.
 *
 * These are intentionally *not* full implementations of the real
 * interfaces - only the members exercised by the code under test are
 * implemented, then cast to the real type. This keeps tests fast and free
 * of any real WebGL/DOM dependency.
 */
import { vi } from 'vitest';
import { vec3 } from 'gl-matrix';
import type { IGameState, IPhysicsProvider } from '/classic/types.js';

/**
 * A fake WebGLRenderingContext that only implements the buffer-related
 * calls used by `Buffer` (src/classic/utils.ts) and its consumers
 * (Circle, Polygon, PhysicsProvider).
 */
export function createMockGL(): WebGLRenderingContext {
    const gl = {
        ARRAY_BUFFER: 0x8892,
        ELEMENT_ARRAY_BUFFER: 0x8893,
        STATIC_DRAW: 0x88e4,
        DYNAMIC_DRAW: 0x88e8,
        FLOAT: 0x1406,
        UNSIGNED_SHORT: 0x1403,
        LINE_LOOP: 0x0002,
        TRIANGLES: 0x0004,
        TEXTURE0: 0x84c0,
        TEXTURE_2D: 0x0de1,
        TEXTURE_BINDING_2D: 0x8069,
        TEXTURE_MIN_FILTER: 0x2801,
        TEXTURE_MAG_FILTER: 0x2800,
        LINEAR: 0x2601,
        NEAREST: 0x2600,
        CURRENT_PROGRAM: 0x8b8d,
        createBuffer: vi.fn(() => ({})),
        deleteBuffer: vi.fn(),
        bindBuffer: vi.fn(),
        bufferData: vi.fn(),
        vertexAttribPointer: vi.fn(),
        enableVertexAttribArray: vi.fn(),
        uniformMatrix4fv: vi.fn(),
        uniform4fv: vi.fn(),
        uniform1i: vi.fn(),
        uniform1f: vi.fn(),
        drawElements: vi.fn(),
        drawArrays: vi.fn(),
        activeTexture: vi.fn(),
        bindTexture: vi.fn(),
        texParameteri: vi.fn(),
        getParameter: vi.fn(() => null),
        getUniform: vi.fn(() => 0),
        getError: vi.fn(() => 0),
    };

    return gl as unknown as WebGLRenderingContext;
}

/**
 * A minimal IPhysicsProvider stub exposing just registerCollider /
 * unregisterCollider so `Collider` (src/classic/collision.ts) can be
 * constructed and cleaned up without a real PhysicsProvider.
 */
export function createMockPhysics(): IPhysicsProvider {
    return {
        registerCollider: vi.fn(),
        unregisterCollider: vi.fn(),
    } as unknown as IPhysicsProvider;
}

/**
 * Builds a fake IGameState with just enough surface area for constructing
 * Entities/Components/Shapes in tests. Pass `overrides` to customize
 * specific fields (e.g. `canvas` for PhysicsProvider.resizeScreen tests).
 */
export function createMockGame(overrides: Partial<IGameState> = {}): IGameState {
    const gl = createMockGL();

    const textures: Record<string, unknown> = {};
    const shaders: Record<string, unknown> = {};
    const sdfFonts: Record<string, unknown> = {};

    const base = {
        gl,
        physics: createMockPhysics(),
        entities: {},
        calls: {},
        canvas: { width: 800, height: 600 } as unknown as HTMLCanvasElement,
        mousePos: vec3.create(),
        projectionMatrix: new Float32Array(16),
        camera: {
            matrix: vi.fn(() => new Float32Array(16)),
        },
        textures,
        shaders,
        buffers: {
            quad: {
                verts: {
                    bind: vi.fn(),
                    unbind: vi.fn(),
                    gl: gl as unknown as WebGLRenderingContext,
                    buffer: {},
                },
                indices: {
                    bind: vi.fn(),
                    unbind: vi.fn(),
                    gl: gl as unknown as WebGLRenderingContext,
                    buffer: {},
                },
                uvs: {
                    bind: vi.fn(),
                    unbind: vi.fn(),
                    gl: gl as unknown as WebGLRenderingContext,
                    buffer: {},
                },
            },
        },
        sdfFonts,
        getTexture: vi.fn((name: string) => {
            if (textures[name]) return textures[name];
            return {
                image: { width: 512, height: 512 } as HTMLImageElement,
                bind: vi.fn(),
                name,
                gl: gl as unknown as WebGLRenderingContext,
                texture: {} as unknown as WebGLTexture,
            };
        }),
        getSdfFont: vi.fn((name: string) => {
            return sdfFonts[name];
        }),
        registerCall: vi.fn(),
        unregisterCall: vi.fn(),
        ...overrides,
    };

    return base as unknown as IGameState;
}
