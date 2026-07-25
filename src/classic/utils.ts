import { mat3, mat4 } from 'gl-matrix';
import { SimplexNoise } from '/lib/simplex-noise.js';
import type {
  ShaderInfo,
  TextureManifestEntry,
  AnimationData,
  IShader,
  ITexture,
  IBuffer,
  IAnimation,
  GameBuffers,
} from './types.js';

// ============================================================================
// Utility Functions
// ============================================================================

export function getObjectValues(obj: Record<string, unknown>): unknown[] {
  const l: unknown[] = [];
  for (const key in obj) {
    l.push(obj[key]);
  }
  return l;
}

interface VideoCardInfo {
  vendor?: string;
  renderer?: string;
  error?: string;
}

export function getVideoCardInfo(gl: WebGLRenderingContext): VideoCardInfo {
  const debugInfo = gl.getExtension('WEBGL_debug_renderer_info');
  return debugInfo
    ? {
        vendor: gl.getParameter(debugInfo.UNMASKED_VENDOR_WEBGL) as string,
        renderer: gl.getParameter(debugInfo.UNMASKED_RENDERER_WEBGL) as string,
      }
    : {
        error: 'no WEBGL_debug_renderer_info',
      };
}

export async function fetchFile(
  url: string,
  config: RequestInit = {}
): Promise<string | undefined> {
  try {
    const response = await fetch(url, config);
    return await response.text();
  } catch (err) {
    console.error(err);
    return undefined;
  }
}

export async function fetchObject<T = unknown>(
  url: string,
  config: RequestInit = {}
): Promise<T | undefined> {
  try {
    const response = await fetch(url, config);
    return (await response.json()) as T;
  } catch (err) {
    console.error(err);
    return undefined;
  }
}

export async function fetchBase64Object<T = unknown>(
  url: string,
  config: RequestInit = {}
): Promise<T | undefined> {
  try {
    const response = await fetch(url, config);
    return JSON.parse(atob(await response.text())) as T;
  } catch (err) {
    console.error(err);
    return undefined;
  }
}

export function loadImage(src: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = reject;
    img.src = src;
  });
}

const loadingLabel = document.getElementById('loader');

export function setLoaderLabel(msg: string): void {
  if (loadingLabel) {
    loadingLabel.innerHTML = msg;
  }
}

export function deleteLoaderLabel(): void {
  if (loadingLabel) {
    loadingLabel.remove();
  }
}

// ============================================================================
// Shaders
// ============================================================================

export function loadShader(
  gl: WebGLRenderingContext,
  type: number,
  source: string
): WebGLShader | null {
  const shader = gl.createShader(type);
  if (!shader) {
    setLoaderLabel('Failed to create shader');
    return null;
  }

  gl.shaderSource(shader, source);
  gl.compileShader(shader);

  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    setLoaderLabel(
      'An error occurred compiling the shaders: ' + gl.getShaderInfoLog(shader)
    );
    gl.deleteShader(shader);
    return null;
  }

  return shader;
}

export function initShaderProgram(
  gl: WebGLRenderingContext,
  vsSource: string,
  fsSource: string
): WebGLProgram | null {
  const vertexShader = loadShader(gl, gl.VERTEX_SHADER, vsSource);
  const fragmentShader = loadShader(gl, gl.FRAGMENT_SHADER, fsSource);

  if (!vertexShader || !fragmentShader) {
    return null;
  }

  const shaderProgram = gl.createProgram();
  if (!shaderProgram) {
    setLoaderLabel('Failed to create shader program');
    return null;
  }

  gl.attachShader(shaderProgram, vertexShader);
  gl.attachShader(shaderProgram, fragmentShader);
  gl.linkProgram(shaderProgram);

  if (!gl.getProgramParameter(shaderProgram, gl.LINK_STATUS)) {
    setLoaderLabel(
      'Unable to initialize the shader program: ' +
        gl.getProgramInfoLog(shaderProgram)
    );
    return null;
  }

  return shaderProgram;
}

export class Shader implements IShader {
  gl: WebGLRenderingContext;
  name: string;
  vertexSrc: string;
  fragmentSrc: string;
  attributes: string[];
  uniforms: string[];
  vertexCode: string = '';
  fragmentCode: string = '';
  program: WebGLProgram | null = null;
  attr: Record<string, number> = {};
  unif: Record<string, WebGLUniformLocation | null> = {};

  constructor(
    gl: WebGLRenderingContext,
    name: string,
    vertexSrc: string,
    fragmentSrc: string,
    attributes: string[],
    uniforms: string[]
  ) {
    this.gl = gl;
    this.name = name;
    this.vertexSrc = vertexSrc;
    this.fragmentSrc = fragmentSrc;
    this.attributes = attributes;
    this.uniforms = uniforms;
  }

  async fetchCode(): Promise<void> {
    const vertexCode = await fetchFile(this.vertexSrc);
    const fragmentCode = await fetchFile(this.fragmentSrc);
    this.vertexCode = vertexCode ?? '';
    this.fragmentCode = fragmentCode ?? '';
  }

  compile(): void {
    console.log('Compiling', this.name, '...');
    this.program = initShaderProgram(this.gl, this.vertexCode, this.fragmentCode);

    if (!this.program) {
      throw new Error(`Failed to compile shader: ${this.name}`);
    }

    this.attr = {};
    for (const attr of this.attributes) {
      this.attr[attr] = this.gl.getAttribLocation(this.program, attr);
    }

    this.unif = {};
    for (const unif of this.uniforms) {
      this.unif[unif] = this.gl.getUniformLocation(this.program, unif);
    }
  }

  bind(): void {
    this.gl.useProgram(this.program);
  }

  unbind(): void {
    this.gl.useProgram(null);
  }
}

export async function initShaders(
  gl: WebGLRenderingContext,
  shaderManifest: ShaderInfo[]
): Promise<Record<string, Shader>> {
  const shaders: Record<string, Shader> = {};

  for (const shaderInfo of shaderManifest) {
    const name = shaderInfo.name;

    shaders[name] = new Shader(
      gl,
      name,
      shaderInfo.vertex,
      shaderInfo.fragment,
      shaderInfo.attr,
      shaderInfo.unif
    );

    await shaders[name].fetchCode();
    shaders[name].compile();
  }

  return shaders;
}

// ============================================================================
// Buffers
// ============================================================================

type TypedArrayConstructor =
  | Float32ArrayConstructor
  | Uint16ArrayConstructor
  | Int16ArrayConstructor
  | Uint8ArrayConstructor
  | Int8ArrayConstructor;

export class Buffer implements IBuffer {
  gl: WebGLRenderingContext;
  buffer: WebGLBuffer | null;
  type: number;
  data: number[];
  array: Float32Array | Uint16Array | Int16Array | Uint8Array | Int8Array;
  usage: number;

  constructor(
    gl: WebGLRenderingContext,
    type: number,
    data: number[],
    dataType: TypedArrayConstructor,
    usage: number
  ) {
    this.gl = gl;
    this.buffer = gl.createBuffer();
    this.type = type;
    this.data = data;
    this.array = new dataType(data);
    this.usage = usage;

    gl.bindBuffer(type, this.buffer);
    gl.bufferData(type, this.array, usage);
    gl.bindBuffer(type, null);
  }

  bind(): void {
    this.gl.bindBuffer(this.type, this.buffer);
  }

  unbind(): void {
    this.gl.bindBuffer(this.type, null);
  }
}

export function initBuffers(gl: WebGLRenderingContext): GameBuffers {
  // Verts
  const vertBuffer = new Buffer(
    gl,
    gl.ARRAY_BUFFER,
    [0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    Float32Array,
    gl.STATIC_DRAW
  );

  // Indices
  const indexBuffer = new Buffer(
    gl,
    gl.ELEMENT_ARRAY_BUFFER,
    [0, 1, 2, 1, 2, 3],
    Uint16Array,
    gl.STATIC_DRAW
  );

  // UVs
  const texCoordBuffer = new Buffer(
    gl,
    gl.ARRAY_BUFFER,
    [0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0],
    Float32Array,
    gl.STATIC_DRAW
  );

  return {
    quad: {
      verts: vertBuffer,
      indices: indexBuffer,
      uvs: texCoordBuffer,
    },
  };
}

// ============================================================================
// Textures
// ============================================================================

export async function loadTexture(
  gl: WebGLRenderingContext,
  url: string
): Promise<[WebGLTexture | null, HTMLImageElement]> {
  const texture = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, texture);

  const image = await loadImage(url);

  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, image);

  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);

  return [texture, image];
}

export class Texture implements ITexture {
  gl: WebGLRenderingContext;
  name: string;
  src: string;
  texture: WebGLTexture | null = null;
  image!: HTMLImageElement;

  constructor(gl: WebGLRenderingContext, name: string, src: string) {
    this.gl = gl;
    this.name = name;
    this.src = src;
  }

  async load(): Promise<void> {
    const [tex, img] = await loadTexture(this.gl, this.src);
    this.texture = tex;
    this.image = img;
  }

  bind(texCore: number): void {
    this.gl.activeTexture(texCore);
    this.gl.bindTexture(this.gl.TEXTURE_2D, this.texture);
  }
}

export async function initTextures(
  gl: WebGLRenderingContext,
  textureManifest: TextureManifestEntry[]
): Promise<Record<string, Texture>> {
  const textures: Record<string, Texture> = {};

  for (const tex of textureManifest) {
    textures[tex.name] = new Texture(gl, tex.name, tex.src);
    await textures[tex.name].load();
  }

  return textures;
}

// ============================================================================
// Animations
// ============================================================================

export class Animation implements IAnimation {
  name: string;
  src: string;
  rate: number;
  sequence: number[];

  constructor(name: string, src: string, rate: number, sequence: number[]) {
    this.name = name;
    this.src = src;
    this.rate = rate;
    this.sequence = sequence;
  }
}

export function initAnimations(
  animationManifest: AnimationData[]
): Record<string, Animation> {
  const animations: Record<string, Animation> = {};

  for (const anim of animationManifest) {
    animations[anim.name] = new Animation(
      anim.name,
      anim.src,
      anim.rate,
      anim.sequence
    );
  }

  return animations;
}

// ============================================================================
// Isometric Tools
// ============================================================================

const _cartesianToIso3 = mat3.create();
mat3.rotate(_cartesianToIso3, _cartesianToIso3, Math.PI / 4);
mat3.scale(_cartesianToIso3, _cartesianToIso3, [1, 2]);

export const cartesianToIso3 = _cartesianToIso3;

const _isoToCartesian3 = mat3.create();
mat3.invert(_isoToCartesian3, _cartesianToIso3);

export const isoToCartesian3 = _isoToCartesian3;

const _cartesianToIso4 = mat4.create();
mat4.rotateZ(_cartesianToIso4, _cartesianToIso4, Math.PI / 4);
mat4.scale(_cartesianToIso4, _cartesianToIso4, [1, 2, 1]);

export const cartesianToIso4 = _cartesianToIso4;

const _isoToCartesian4 = mat4.create();
mat4.invert(_isoToCartesian4, _cartesianToIso4);

export const isoToCartesian4 = _isoToCartesian4;

// ============================================================================
// Simplex Noise
// ============================================================================

const factor = 50.0;

export const noiseGen = new SimplexNoise();

export function getNoiseRange(
  x: number,
  y: number,
  from: number,
  to: number
): number {
  return ((noiseGen.noise2D(x / factor, y / factor) + 1) / 2) * (to - from) + from;
}

// ============================================================================
// Math
// ============================================================================

export function degreeToRadian(deg: number): number {
  return deg * (Math.PI / 180);
}

export function radianToDegree(rad: number): number {
  return rad * (180 / Math.PI);
}
