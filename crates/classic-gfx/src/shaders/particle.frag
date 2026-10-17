#version 300 es

// Analytic particle fragment: a procedural shape (soft disc, noise-broken
// smoke puff, hot glow) or a texture / flipbook frame, output premultiplied
// so one blend state (`ONE, ONE_MINUS_SRC_ALPHA`) serves alpha (`a`),
// additive (`0`) and hybrid layers (`u_shape.z` = additivity).
//
// Lit layers shade each puff as a lumpy sphere: a normal from the quad UV
// (a hemisphere facing the camera), perturbed by the smoke noise gradient,
// against the sun direction in view space — so overlapping puffs build up a
// billowing, self-shaded cloud instead of a flat fog.
//
// Particle textures sample LINEAR (a sampler object on the particle unit),
// each flipbook frame inset by half a texel so the filter never bleeds in a
// neighbouring frame.

precision highp float;

in vec4 vColor;
in vec2 vUv;
in float vFrame;
in float vSeed;
in vec3 vAmbient;
in vec3 vSunColor;
in vec3 vSunView;

uniform sampler2D u_tex;
// Baked tileable billow noise: r = value, g/b = x/y slope (`0.5 + s / 4`, per
// lattice cell; the tile spans 8 cells).  See `classic-gfx` `bake_noise`.
uniform sampler2D u_noise;
uniform float u_use_tex;
uniform highp vec4 u_flip;    // cols, rows, frames, fps
uniform highp vec4 u_shape;   // shape (0 soft, 1 smoke, 2 glow), softness, additive, alpha_mul
uniform highp vec4 u_render;  // mode (0 billboard, 1 streak), ...
uniform highp vec4 u_shade;   // lit, emissive, -, -

out vec4 fragColor;

// Billow noise at quad uv (`[-1, 1]`), seeded per particle: x/y = slope per
// uv unit, z = value.  One bilinear fetch of the baked tile.
vec3 billow(vec2 uv) {
    // 1.15 lattice cells per uv unit (8 cells per tile), offset per particle.
    vec2 q = uv * (1.15 / 8.0) + vec2(vSeed * 4.63, vSeed * 11.37);
    vec4 t = texture(u_noise, q);
    return vec3((t.gb - 0.5) * 4.0 * 1.15, t.r);
}

vec2 frameUv(float frame, vec2 uv) {
    float cols = u_flip.x;
    float rows = max(u_flip.y, 1.0);
    float frames = u_flip.z > 0.5 ? u_flip.z : cols * rows;
    float f = mod(floor(frame), frames);
    vec2 cell = vec2(mod(f, cols), floor(f / cols));
    // Half a texel in from the frame edge, in frame-uv units.
    vec2 inset = 0.5 * vec2(cols, rows) / vec2(textureSize(u_tex, 0));
    return (cell + clamp(uv, inset, 1.0 - inset)) / vec2(cols, rows);
}

void main(void ) {
    vec4 c = vColor;
    if (c.a <= 0.0) {
        discard;
    }
    float r2 = dot(vUv, vUv);
    // Every procedural shape is zero outside the unit disc: skip the quad
    // corners (~21% of each quad) before any shading.
    if (u_use_tex < 0.5 && r2 >= 1.0) {
        discard;
    }
    // A flattened sphere normal in view space (y up on screen: the quad's
    // +y is down) — a full hemisphere goes edge-on at the rim and draws a
    // dark outline around every puff.
    vec3 n = normalize(vec3(vUv.x * 0.6, -vUv.y * 0.6, 1.0));
    if (u_use_tex > 0.5) {
        vec2 uv = vUv * 0.5 + 0.5;
        vec4 tx;
        if (u_flip.x > 0.5) {
            vec4 a = texture(u_tex, frameUv(vFrame, uv));
            vec4 b = texture(u_tex, frameUv(vFrame + 1.0, uv));
            tx = mix(a, b, fract(vFrame));
        } else {
            tx = texture(u_tex, uv);
        }
        c *= tx;
    } else {
        float shape = u_shape.x;
        float soft = max(u_shape.y, 0.05);
        float a;
        if (shape < 0.5) {
            a = pow(clamp(1.0 - r2, 0.0, 1.0), soft);
        } else if (shape < 1.5) {
            // Erode a soft disc with billow noise into a lumpy puff, and tilt
            // the sphere normal by the noise gradient (cauliflower bumps).
            vec3 b = billow(vUv);
            float nz = b.z;
            // The old finite difference used a 0.15 uv step: slope * 0.15.
            n = normalize(n + vec3(-b.x, b.y, 0.0) * (0.15 * 3.5));
            float edge = clamp(1.0 - sqrt(r2), 0.0, 1.0);
            a = clamp(edge * (0.45 + 1.3 * nz) * 1.7 - 0.2, 0.0, 1.0);
            a = pow(a, soft);
            c.rgb *= 0.8 + 0.35 * nz;
        } else {
            float core = exp(-r2 * 10.0);
            float halo = exp(-r2 * 2.5) * 0.45;
            a = clamp(core + halo, 0.0, 1.0) * clamp(1.0 - r2, 0.0, 1.0);
            c.rgb *= 1.0 + 1.5 * core;
        }
        if (u_render.x > 0.5) {
            // Streak: brighter head (+x = newest position), fading tail.
            a *= mix(0.2, 1.0, vUv.x * 0.5 + 0.5);
        }
        c.a *= a;
    }
    if (c.a < 0.004) {
        discard;
    }
    if (u_shade.x > 0.5 && u_shade.y <= 0.0) {
        // Wrapped sun diffuse on the puff sphere: a lit side and a soft
        // shadowed side, never black (multiple scattering).
        float sun = clamp(dot(n, normalize(vSunView)) * 0.7 + 0.3, 0.0, 1.0);
        vec3 lit = vAmbient + sun * vSunColor;
        // Highlight shoulder: dust and smoke scatter light, they do not
        // glow.  Roll the brightest channel off smoothly above 0.9 toward
        // 1.3 and scale the others with it, so a strong nearby light (a
        // rocket burn) tints the cloud warm instead of clipping it to a white
        // blob that the bloom pass then blows up.
        float m = max(lit.r, max(lit.g, lit.b));
        if (m > 0.9) {
            lit *= (0.9 + 0.4 * (1.0 - exp((0.9 - m) / 0.4))) / m;
        }
        c.rgb *= lit;
    }
    fragColor = vec4(c.rgb * c.a, c.a * (1.0 - u_shape.z));
}
