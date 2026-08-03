import game from '/classic/state.js';
import { vec3 } from 'gl-matrix';

interface ILightPreset {
    name: string;
    ambient: [number, number, number];
    direction: [number, number, number];
    color: [number, number, number];
}

function norm(x: number, y: number, z: number): [number, number, number] {
    const v = vec3.fromValues(x, y, z);
    vec3.normalize(v, v);
    return [v[0], v[1], v[2]];
}

const LIGHT_PRESETS: Record<string, ILightPreset> = {
    sunny: {
        name: 'Sunny Day',
        ambient: [0.15, 0.15, 0.2],
        direction: norm(0.453, 0.211, 0.866),
        color: [1.0, 0.95, 0.85],
    },
    cloudy: {
        name: 'Cloudy',
        ambient: [0.35, 0.35, 0.4],
        direction: norm(0.0, -0.2, 1.0),
        color: [0.7, 0.72, 0.78],
    },
    dawn: {
        name: 'Dawn / Dusk',
        ambient: [0.2, 0.15, 0.25],
        direction: norm(0.5, 0.2, 0.3),
        color: [1.0, 0.4, 0.2],
    },
    night: {
        name: 'Night',
        ambient: [0.1, 0.12, 0.25],
        direction: norm(-0.2, -0.5, 0.8),
        color: [0.3, 0.4, 0.7],
    },
};

export const PRESET_ORDER: string[] = ['sunny', 'cloudy', 'dawn', 'night'];

export function applyLightPreset(key: string): void {
    const preset = LIGHT_PRESETS[key];
    if (!preset) return;

    game.lightPreset = key;
    game.lightAmbient = [...preset.ambient] as [number, number, number];
    game.lightDir = [...preset.direction] as [number, number, number];
    game.lightColor = [...preset.color] as [number, number, number];

    const d = game.lightDir;
    game.lightAzimuth = (Math.atan2(d[0], -d[1]) * 180) / Math.PI;
    game.lightElevation = (Math.asin(d[2]) * 180) / Math.PI;
}

export function updateLightDirection(): void {
    const az = ((game.lightAzimuth ?? 0) * Math.PI) / 180;
    const el = ((game.lightElevation ?? 45) * Math.PI) / 180;
    const v = vec3.fromValues(
        Math.cos(el) * Math.sin(az),
        -Math.cos(el) * Math.cos(az),
        Math.sin(el),
    );
    vec3.normalize(v, v);
    game.lightDir = [v[0], v[1], v[2]];
}

export function initLighting(): void {
    applyLightPreset('sunny');
}

export { LIGHT_PRESETS };
