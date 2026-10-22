#version 300 es

// Analytic stateless particles (see `classic-gfx/src/particles.rs`).
//
// One instance per ring slot; the quad corner comes from `gl_VertexID`
// (TRIANGLE_STRIP 0..3).  The particle's state at age `t = u_time - t_birth`
// is evaluated in closed form: ballistic + linear drag, an optional ground
// vortex ("swirl") capture, a smooth turbulence wobble and a ground clamp /
// bounce / die.  Dead slots collapse to a degenerate vertex.
//
// World -> screen is identical to `mesh.vert` (world metres -> camera view ->
// `(view.x, -view.y) * ppm` -> camera -> projection, depth from `view.z`).

precision highp float;

in vec4 a_p0t;   // p0.xyz, t_birth
in vec4 a_v0l;   // v0.xyz, life
in vec4 a_misc;  // seed, size, rot, ground_z

uniform mat4 world_matrix;
uniform mat4 camera_matrix;
uniform mat4 projection_matrix;
uniform vec2 depth_span;
uniform float ppm;
// Size (texels) of the target this pass draws into (half-res inside the
// particle pass).
uniform vec2 u_target_px;

// Minimum quad width in target texels (`PARTICLE_MIN_TEXELS` in
// `particles.rs`, mirrored by `particle_size_floor`).
#define PARTICLE_MIN_TEXELS 1.5

uniform vec3 ambient_color;
uniform vec3 light_direction;
uniform vec3 light_color;

uniform float u_time;
uniform vec4 u_motion;  // gravity.xyz, drag k
uniform vec4 u_turb;    // amp, freq, ramp, spin_max
uniform vec4 u_swirl0;  // slots (0 = off), kick_radius, omega, decay
uniform vec4 u_swirl1;  // lift, lift_time, growth, drift
uniform vec4 u_swirl2;  // arms, core, pinch_time, kick_jitter
uniform vec4 u_ground;  // mode (0 none, 1 clamp, 2 bounce, 3 die), restitution, offset, -
uniform vec4 u_size_t;
uniform vec4 u_size_v;
uniform vec4 u_color_t;
uniform vec4 u_color_v[4];
uniform vec4 u_render;  // mode (0 billboard, 1 streak), streak_time, depth_bias, -
uniform vec4 u_shape;   // shape, softness, additive, alpha_mul
uniform vec4 u_shade;   // lit, emissive, -, -
uniform vec3 u_tint;
uniform vec4 u_flip;    // cols, rows, frames, fps

#define MAX_LIGHTS 256

struct Light {
    vec4 pos_radius;
    vec4 color_intensity;
    vec4 dir_cone;
};

layout(std140) uniform LightBlock {
    vec4 count;
    Light lights[MAX_LIGHTS];
} u_lights;

out vec4 vColor;
out vec2 vUv;
out float vFrame;
out float vSeed;
// Lit layers: ambient + dynamic lights (per vertex), the sun colour and the
// sun direction in view space (x right, y screen-up, z toward the camera);
// the fragment shades each puff as a lumpy sphere against it.
out vec3 vAmbient;
out vec3 vSunColor;
out vec3 vSunView;

// --- BEGIN SHARED LIGHTING (byte-identical in sheet.frag / iso_tilemap.frag /
// --- mesh.frag / particle.vert; pinned by `lit_shaders_share_the_lighting_block`) ---
//
// `p` and `l.pos_radius.xyz` are both **metric world space** (+Z up, metres),
// so `length` is a true distance and `dot(n, L)` a true cosine.  They previously
// lived in the isometric screen space, which compresses y by 2x; every point
// light was therefore an ellipsoid evaluated as if it were a sphere.
vec3 evaluateLight(Light l, vec3 n, vec3 p) {
    vec3 toLight = l.pos_radius.xyz - p;
    float dist = length(toLight);
    float radius = l.pos_radius.w;
    // Smooth windowed falloff: `w(d)^2 / (1 + d^2)`, `w = saturate(1 - d^2)`,
    // `d = dist / radius`.  Softer than the previous `w = saturate(1 - d^4)` /
    // `1 + 8 d^2` form, whose quartic window + 8x inverse-square term made the
    // light read as a hot core with a sharp cutoff at roughly a tenth of the
    // radius (nearly invisible for a low light like the rocket's flame, whose
    // Lambertian grazing angle already shrinks the ground pool).  The quadratic
    // window + unit inverse-square term keeps a bounded, C0 edge while letting
    // the light actually span the authored `radius`.
    float attenuation = 1.0;
    if (radius > 0.0) {
        float d = dist / radius;
        float d2 = d * d;
        // Radius early-out: at `d2 >= 1` the window is exactly 0, so the
        // light contributes exactly 0 and skipping the cone/Lambert work
        // changes no pixel.  The test reuses the falloff's own `d2` (not a
        // separate `dot(toLight, toLight) >= radius * radius`, which rounds
        // differently at the edge), so it is bit-exact.
        if (d2 >= 1.0) {
            return vec3(0.0);
        }
        float window = clamp(1.0 - d2, 0.0, 1.0);
        attenuation = window * window / (1.0 + d2);
    }
    vec3 L = toLight / max(dist, 0.0001);
    float cone = 1.0;
    if (l.dir_cone.w > 0.0) {
        // `dir_cone.xyz` is the beam direction (light -> target) and `w` the
        // half-angle of the full-intensity cone; the edge softens out to 1.25x.
        float cosTheta = dot(-L, normalize(l.dir_cone.xyz));
        cone = smoothstep(cos(l.dir_cone.w * 1.25), cos(l.dir_cone.w), cosTheta);
    }
    float diff = max(dot(n, L), 0.0);
    return attenuation * cone * diff * l.color_intensity.rgb * l.color_intensity.a;
}

// Every active light, in UBO order.  The screen-tile culled variant is
// `evaluateLightsTiled` (the LIGHT GRID block of the lit fragment shaders);
// it falls back to this loop wherever it has no list.
vec3 evaluateLights(vec3 n, vec3 p) {
    vec3 acc = vec3(0.0);
    int cnt = int(u_lights.count.x + 0.5);
    for (int i = 0; i < MAX_LIGHTS; i++) {
        if (i >= cnt) {
            break;
        }
        acc += evaluateLight(u_lights.lights[i], n, p);
    }
    return acc;
}
// --- END SHARED LIGHTING ---

float gSeed;
float gDead;

float hash1(float n) {
    return fract(sin(gSeed * 157.31 + n * 12.9898) * 43758.5453);
}

// Ballistic + linear drag: dv/dt = g - k v.
vec3 dragPos(vec3 p0, vec3 v0, float t) {
    vec3 g = u_motion.xyz;
    float k = u_motion.w;
    if (k > 1e-3) {
        float e = (1.0 - exp(-k * t)) / k;
        return p0 + (v0 - g / k) * e + (g / k) * t;
    }
    return p0 + v0 * t + 0.5 * g * t * t;
}

vec3 dragVel(vec3 v0, float t) {
    vec3 g = u_motion.xyz;
    float k = u_motion.w;
    if (k > 1e-3) {
        return g / k + (v0 - g / k) * exp(-k * t);
    }
    return v0 + g * t;
}

// Age at which the particle comes down to `zg` (-1 = never).  The drag-free
// root refined by Newton steps; mirrors `classic_core::effects::land_time`.
float landTime(vec3 p0, vec3 v0, float zg) {
    float gz = u_motion.z;
    if (gz >= 0.0) {
        return -1.0;
    }
    float c = p0.z - zg;
    float disc = v0.z * v0.z - 2.0 * gz * c;
    if (disc < 0.0) {
        return -1.0;
    }
    float t = (-v0.z - sqrt(disc)) / gz;
    if (u_motion.w > 1e-3) {
        for (int i = 0; i < 4; i++) {
            float z = dragPos(p0, v0, t).z - zg;
            float v = dragVel(v0, t).z;
            if (abs(v) < 1e-4) {
                break;
            }
            t -= z / v;
        }
    }
    return t >= 0.0 ? t : -1.0;
}

// Ballistic motion against the ground plane at `zg`.
vec3 groundedPos(vec3 p0, vec3 v0, float t, float zg, float mode) {
    if (mode < 0.5) {
        return dragPos(p0, v0, t);
    }
    if (mode > 1.5 && mode < 2.5) {
        // One bounce, then clamp.
        float tl = landTime(p0, v0, zg);
        if (tl >= 0.0 && t > tl) {
            vec3 pl = dragPos(p0, v0, tl);
            vec3 vl = dragVel(v0, tl);
            vec3 vb = vec3(vl.xy * 0.7, -vl.z * u_ground.y);
            vec3 p = dragPos(vec3(pl.xy, zg), vb, t - tl);
            p.z = max(p.z, zg);
            return p;
        }
    }
    if (mode > 2.5) {
        float tl = landTime(p0, v0, zg);
        if (tl >= 0.0 && t > tl) {
            gDead = 1.0;
        }
    }
    vec3 p = dragPos(p0, v0, t);
    p.z = max(p.z, zg);
    return p;
}

// Ground vortex capture: the particle runs outward along the ground until it
// reaches the kick radius, then blends into a rising helix around one of
// `slots` vortices spaced around the emitter axis (alternate slots
// counter-rotate).  Continuous at the kick: the helix is blended in over the
// pinch time from the particle's own radial path.
vec3 swirlPos(vec3 p0, vec3 v0, float t, float zg) {
    vec3 pre = groundedPos(p0, v0, t, zg, 1.0);
    float sp = length(v0.xy);
    float k = u_motion.w;
    float R = u_swirl0.y * (1.0 + u_swirl2.w * (hash1(3.0) * 2.0 - 1.0));
    float tk = -1.0;
    if (k > 1e-3) {
        float q = 1.0 - R * k / max(sp, 1e-4);
        if (q > 0.0) {
            tk = -log(q) / k;
        }
    } else if (sp > 1e-4) {
        tk = R / sp;
    }
    if (tk < 0.0 || t <= tk) {
        return pre;
    }
    float tau = t - tk;
    float slots = u_swirl0.x;
    float slotW = 6.2831853 / slots;
    float ang = atan(v0.y, v0.x);
    float slot = floor(ang / slotW + 0.5);
    float sj = fract(sin(slot * 78.233 + 1.7) * 43758.5453);
    float angC = (slot + (sj - 0.5) * 0.5) * slotW;
    vec2 dirC = vec2(cos(angC), sin(angC));
    float sgn = mod(slot, 2.0) < 0.5 ? 1.0 : -1.0;
    vec2 c = p0.xy + dirC * (u_swirl0.y + u_swirl1.w * (1.0 - exp(-tau * 0.5)));

    float arms = max(u_swirl2.x, 1.0);
    float arm = floor(hash1(5.0) * arms);
    float theta0 = angC + 3.14159265 + arm * 6.2831853 / arms + (hash1(6.0) - 0.5) * 0.5;
    float d = max(u_swirl0.w, 1e-3);
    float theta = theta0 + sgn * u_swirl0.z * (1.0 - exp(-d * tau)) / d;

    float hr = mix(0.35, 1.0, hash1(7.0));
    float rise = u_swirl1.x * hr * (1.0 - exp(-tau / max(u_swirl1.y, 0.05)));
    float r0 = max(length(pre.xy - c), 0.05);
    float rc = u_swirl2.y * (0.6 + 0.8 * hash1(8.0));
    float pinch = max(u_swirl2.z, 0.01);
    float r = mix(r0, rc, smoothstep(0.0, pinch * 2.0, tau)) + u_swirl1.z * rise;
    vec3 orbit = vec3(c + r * vec2(cos(theta), sin(theta)), zg + rise);
    return mix(pre, orbit, smoothstep(0.0, pinch, tau));
}

vec3 wobble(float t) {
    float f = u_turb.y;
    vec3 ph = vec3(hash1(11.0), hash1(12.0), hash1(13.0)) * 6.2831853;
    return vec3(
        sin(f * t + ph.x) + 0.5 * sin(f * 2.3 * t + ph.y),
        sin(f * 1.3 * t + ph.y) + 0.5 * sin(f * 2.9 * t + ph.z),
        0.5 * (sin(f * 1.7 * t + ph.z) + 0.5 * sin(f * 3.1 * t + ph.x)));
}

vec3 particlePos(float t) {
    vec3 p0 = a_p0t.xyz;
    vec3 v0 = a_v0l.xyz;
    float zg = a_misc.w + u_ground.z;
    vec3 p = u_swirl0.x > 0.5 ? swirlPos(p0, v0, t, zg) : groundedPos(p0, v0, t, zg, u_ground.x);
    if (u_turb.x > 0.0) {
        float ramp = u_turb.z > 0.0 ? clamp(t / u_turb.z, 0.0, 1.0) : 1.0;
        p += u_turb.x * ramp * (wobble(t) - wobble(0.0));
        if (u_ground.x > 0.5) {
            p.z = max(p.z, zg);
        }
    }
    return p;
}

float curve4(vec4 ts, vec4 vs, float x) {
    if (x <= ts.x) return vs.x;
    if (x <= ts.y) return mix(vs.x, vs.y, (x - ts.x) / max(ts.y - ts.x, 1e-5));
    if (x <= ts.z) return mix(vs.y, vs.z, (x - ts.y) / max(ts.z - ts.y, 1e-5));
    if (x <= ts.w) return mix(vs.z, vs.w, (x - ts.z) / max(ts.w - ts.z, 1e-5));
    return vs.w;
}

vec4 colorCurve(float x) {
    vec4 ts = u_color_t;
    if (x <= ts.x) return u_color_v[0];
    if (x <= ts.y) return mix(u_color_v[0], u_color_v[1], (x - ts.x) / max(ts.y - ts.x, 1e-5));
    if (x <= ts.z) return mix(u_color_v[1], u_color_v[2], (x - ts.y) / max(ts.z - ts.y, 1e-5));
    if (x <= ts.w) return mix(u_color_v[2], u_color_v[3], (x - ts.z) / max(ts.w - ts.z, 1e-5));
    return u_color_v[3];
}

vec2 toScreen(vec3 p, out float viewZ) {
    vec4 view = world_matrix * vec4(p, 1.0);
    viewZ = view.z;
    return vec2(view.x, -view.y) * ppm;
}

void main(void ) {
    gSeed = a_misc.x;
    gDead = 0.0;
    vSeed = a_misc.x;
    float life = a_v0l.w;
    float t = u_time - a_p0t.w;
    int vid = gl_VertexID;
    vec2 corner = vec2(float(vid & 1) * 2.0 - 1.0, float((vid >> 1) & 1) * 2.0 - 1.0);
    vUv = corner;
    vFrame = 0.0;
    if (life <= 0.0 || t < 0.0 || t > life) {
        gl_Position = vec4(2.0, 2.0, 2.0, 1.0);
        vColor = vec4(0.0);
        return;
    }
    float lf = t / life;
    vec3 p = particlePos(t);
    if (gDead > 0.5) {
        gl_Position = vec4(2.0, 2.0, 2.0, 1.0);
        vColor = vec4(0.0);
        return;
    }
    float size = a_misc.y * curve4(u_size_t, u_size_v, lf);
    float half_px = 0.5 * size * ppm;

    // Size floor: a quad under ~a texel wide hits no pixel centre and
    // vanishes; widen it to PARTICLE_MIN_TEXELS and dim it by the area
    // ratio (`cover`) so its average light is kept at any zoom.
    mat4 pc = projection_matrix * camera_matrix;
    float texPerPx = 0.5 * u_target_px.x * abs(pc[0][0]);
    float minHalf = texPerPx > 0.0 ? 0.5 * PARTICLE_MIN_TEXELS / texPerPx : 0.0;
    float cover = 1.0;

    float viewZ;
    vec2 sc = toScreen(p, viewZ);
    vec2 off;
    if (u_render.x > 0.5) {
        // Streak: stretch from the position `streak_time` ago to now.
        float z1;
        vec2 sc1 = toScreen(particlePos(max(t - u_render.y, 0.0)), z1);
        vec2 axis = sc - sc1;
        float len = length(axis);
        vec2 dir = len > 1e-3 ? axis / len : vec2(1.0, 0.0);
        vec2 perp = vec2(-dir.y, dir.x);
        float hp = max(half_px, minHalf);
        cover = ((0.5 * len + half_px) * half_px) / max((0.5 * len + hp) * hp, 1e-12);
        half_px = hp;
        float halfL = 0.5 * len + half_px;
        sc = 0.5 * (sc + sc1);
        off = dir * corner.x * halfL + perp * corner.y * half_px;
    } else {
        float hp = max(half_px, minHalf);
        cover = (half_px * half_px) / max(hp * hp, 1e-12);
        half_px = hp;
        float spin = (hash1(9.0) * 2.0 - 1.0) * u_turb.w;
        float a = a_misc.z + spin * t;
        float ca = cos(a);
        float sa = sin(a);
        off = vec2(ca * corner.x - sa * corner.y, sa * corner.x + ca * corner.y) * half_px;
    }
    vec4 clipPos = projection_matrix * camera_matrix * vec4(sc + off, 0.0, 1.0);
    // Treat the quad as a sphere: depth at its camera-facing surface, so a
    // ground-hugging puff is not half-buried in the terrain it sits on.
    highp float z = viewZ + size * u_render.z;
    highp float isoDepth = (depth_span.x - z) / (depth_span.x - depth_span.y);
    clipPos.z = isoDepth * 2.0 - 1.0;
    gl_Position = clipPos;

    vec4 col = colorCurve(lf);
    col.rgb *= u_tint;
    col.a *= u_shape.w * cover;
    vAmbient = vec3(1.0);
    vSunColor = vec3(0.0);
    vSunView = vec3(0.0, 0.0, 1.0);
    if (u_shade.y > 0.0) {
        col.rgb *= u_shade.y;
    } else if (u_shade.x > 0.5) {
        // Camera basis (rows of the view matrix), all in world space.
        vec3 right = vec3(world_matrix[0][0], world_matrix[1][0], world_matrix[2][0]);
        vec3 up = vec3(world_matrix[0][1], world_matrix[1][1], world_matrix[2][1]);
        vec3 back = vec3(world_matrix[0][2], world_matrix[1][2], world_matrix[2][2]);
        // Dynamic lights per vertex with an up-biased normal bulged toward
        // the camera and out at the corners (screen y points down, so the
        // +y corner is world-down): a puff over the burn light glows on its
        // underside.
        vec3 n = normalize(vec3(0.0, 0.0, 1.0) + back * 0.55
            + (right * corner.x - up * corner.y) * 0.7);
        vec3 pw = p + (right * corner.x - up * corner.y) * (0.5 * size);
        vAmbient = ambient_color + evaluateLights(n, pw);
        vSunColor = light_color;
        vSunView = vec3(dot(right, light_direction), dot(up, light_direction),
            dot(back, light_direction));
    }
    vColor = col;

    if (u_flip.x > 0.5) {
        float frames = u_flip.z > 0.5 ? u_flip.z : u_flip.x * u_flip.y;
        if (u_flip.w > 0.0) {
            vFrame = floor(hash1(10.0) * frames) + t * u_flip.w;
        } else {
            vFrame = lf * (frames - 1.0);
        }
    }
}
