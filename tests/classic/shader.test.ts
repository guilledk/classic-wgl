import { describe, it, expect, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { initShaderProgram, Shader } from '/classic/utils.js';

const shaderDir = path.resolve(process.cwd(), 'src/shaders');

const shaderFiles = [
    'direct.vert',
    'direct_tex.vert',
    'iso_tilemap.vert',
    'iso_tilemap.frag',
    'image.frag',
    'image_colorized.frag',
    'sheet.frag',
    'solid.frag',
];

function readShader(name: string): string {
    return readFileSync(path.join(shaderDir, name), 'utf-8');
}

type ShaderGL = WebGLRenderingContext & {
    shaderSources: string[];
    _compileShouldFail: boolean;
    _linkShouldFail: boolean;
};

function createMockShaderGL(overrides: Partial<ShaderGL> = {}): ShaderGL {
    let nextShaderId = 1;
    const shaderSources: string[] = [];
    const state = { compileShouldFail: false, linkShouldFail: false };

    const gl: any = {
        VERTEX_SHADER: 0x8b31,
        FRAGMENT_SHADER: 0x8b30,
        COMPILE_STATUS: 0x8b81,
        LINK_STATUS: 0x8b82,

        get _compileShouldFail(): boolean {
            return state.compileShouldFail;
        },
        set _compileShouldFail(v: boolean) {
            state.compileShouldFail = v;
        },
        get _linkShouldFail(): boolean {
            return state.linkShouldFail;
        },
        set _linkShouldFail(v: boolean) {
            state.linkShouldFail = v;
        },

        createShader: vi.fn(() => nextShaderId++),
        shaderSource: vi.fn((_shader: number, source: string) => {
            shaderSources.push(source);
        }),
        compileShader: vi.fn(),
        getShaderParameter: vi.fn((_shader: number, pname: number) => {
            if (pname === 0x8b81) return !state.compileShouldFail;
            return true;
        }),
        getShaderInfoLog: vi.fn(() => ''),
        deleteShader: vi.fn(),

        createProgram: vi.fn(() => ({})),
        attachShader: vi.fn(),
        linkProgram: vi.fn(),
        getProgramParameter: vi.fn((_program: unknown, pname: number) => {
            if (pname === 0x8b82) return !state.linkShouldFail;
            return true;
        }),
        getProgramInfoLog: vi.fn(() => ''),
        getAttribLocation: vi.fn(() => 0),
        getUniformLocation: vi.fn(() => ({})),
        useProgram: vi.fn(),

        shaderSources,

        ...overrides,
    };

    return gl as ShaderGL;
}

// ============================================================================
// GLSL100 validity checks on actual shader source files
// ============================================================================

describe('shader source validity', () => {
    for (const file of shaderFiles) {
        it(`${file} is valid GLSL 100 (no GLSL 300 only identifiers)`, () => {
            const source = readShader(file);

            expect(source.length).toBeGreaterThan(0);
            expect(source).not.toMatch(/gl_VertexID/);
            expect(source).not.toMatch(/gl_InstanceID/);
            expect(source).not.toMatch(/#version 300/);
        });
    }

    it('direct_tex.vert declares isoDepthCorners uniform', () => {
        const source = readShader('direct_tex.vert');

        expect(source).toContain('uniform vec4 isoDepthCorners');
    });
});

// ============================================================================
// initShaderProgram tests
// ============================================================================

describe('initShaderProgram', () => {
    it('returns a program with valid source', () => {
        const gl = createMockShaderGL() as unknown as WebGLRenderingContext;

        const vs = `
            attribute vec4 vertexPos;
            void main(void) { gl_Position = vertexPos; }
        `;
        const fs = `
            precision mediump float;
            void main(void) { gl_FragColor = vec4(1.0); }
        `;

        const program = initShaderProgram(gl, vs, fs);
        expect(program).not.toBeNull();
        expect(gl.createShader).toHaveBeenCalledTimes(2);
        expect(gl.createProgram).toHaveBeenCalledTimes(1);
        expect(gl.attachShader).toHaveBeenCalledTimes(2);
        expect(gl.linkProgram).toHaveBeenCalledTimes(1);
    });

    it('returns null when the vertex shader fails to compile', () => {
        const gl = createMockShaderGL() as ShaderGL;
        gl._compileShouldFail = true;

        const program = initShaderProgram(gl as unknown as WebGLRenderingContext, '', '');
        expect(program).toBeNull();
    });

    it('returns null when the fragment shader fails to compile', () => {
        const gl = createMockShaderGL() as ShaderGL;
        const inner = gl as unknown as ShaderGL;

        const compileParamCalls: number[] = [];
        (inner.getShaderParameter as ReturnType<typeof vi.fn>).mockImplementation(
            (_shader: number, pname: number) => {
                compileParamCalls.push(pname);
                return compileParamCalls.length > 1;
            },
        );

        const program = initShaderProgram(inner as unknown as WebGLRenderingContext, '', '');
        expect(program).toBeNull();
        expect(compileParamCalls.length).toBeGreaterThan(1);
    });

    it('returns null when program fails to link', () => {
        const gl = createMockShaderGL() as ShaderGL;
        gl._linkShouldFail = true;

        const vs = `
            attribute vec4 vertexPos;
            void main(void) { gl_Position = vertexPos; }
        `;
        const fs = `
            precision mediump float;
            void main(void) { gl_FragColor = vec4(1.0); }
        `;

        const program = initShaderProgram(gl as unknown as WebGLRenderingContext, vs, fs);
        expect(program).toBeNull();
    });
});

// ============================================================================
// Shader class tests (compile, attr/unif mapping)
// ============================================================================

describe('Shader class', () => {
    it('maps attributes from the manifest', () => {
        const gl = createMockShaderGL() as unknown as WebGLRenderingContext;
        const shader = new Shader(
            gl,
            'test',
            'test.vert',
            'test.frag',
            ['vertexPos', 'texCoord'],
            ['modelMatrix', 'color'],
        );
        shader.vertexCode =
            'attribute vec4 vertexPos; void main(void) { gl_Position = vertexPos; }';
        shader.fragmentCode =
            'precision mediump float; void main(void) { gl_FragColor = vec4(1.0); }';
        shader.compile();

        expect(shader.attr.vertexPos).toBe(0);
        expect(shader.attr.texCoord).toBe(0);
        expect(shader.unif.modelMatrix).not.toBeNull();
        expect(shader.unif.color).not.toBeNull();
    });

    it('throws when compilation fails', () => {
        const gl = createMockShaderGL() as ShaderGL;
        gl._linkShouldFail = true;
        const shader = new Shader(
            gl as unknown as WebGLRenderingContext,
            'fail',
            'fail.vert',
            'fail.frag',
            [],
            [],
        );
        shader.vertexCode =
            'attribute vec4 vertexPos; void main(void) { gl_Position = vertexPos; }';
        shader.fragmentCode =
            'precision mediump float; void main(void) { gl_FragColor = vec4(1.0); }';

        expect(() => shader.compile()).toThrow('Failed to compile shader: fail');
    });
});
