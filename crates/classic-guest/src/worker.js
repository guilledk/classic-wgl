// classic-guest Worker: runs an untrusted ROM guest on the browser's native
// WebAssembly engine, with host imports bridged synchronously to the main
// thread over a SharedArrayBuffer + Atomics channel.  The main thread services
// host imports against the Engine and terminates this worker if a frame
// exceeds its wall-clock budget (browser Wasm has no fuel API).
//
// The SAB layout (byte offsets) must match `runtime_worker.rs` exactly.

// Control region: Int32Array, 16 slots (64 bytes).
var I_REQ_READY = 0;
var I_RESP_READY = 1;
var I_DONE = 2;
var I_GO = 3;
var I_COMMAND = 4;
var I_WASM_LEN = 5;
var I_REQ_OP = 6;
var I_REQ_STR_LEN = 7;
var I_REQ_NUM_COUNT = 8;
var I_RESP_OUT_LEN = 9;

// Numeric region: Float64Array, 16 slots (128 bytes).  F_ARG0..F_ARG11 are the
// host-import numeric args; F_DT is the frame delta; F_RET is the return value.
var F_DT = 12;
var F_RET = 13;

var NUM_OFFSET = 64; // after the 64-byte control region (8-aligned)
var STR_OFFSET = 192; // after 128 bytes of numerics
var STR_BYTES = 6144;
var OUT_OFFSET = 6336; // after the string region
var OUT_BYTES = 65536;
var WASM_OFFSET = 71872; // after the output region
var SAB_SIZE = 1048576; // 1 MiB total

var CMD_INIT = 0;
var CMD_UPDATE = 1;
var CMD_START = 2;

var flags;
var nums;
var str;
var out;
var memory = null;
var instance = null;

var OP_LOG = 0;
var OP_SPAWN = 1;
var OP_DESPAWN = 2;
var OP_HAS = 3;
var OP_NAMES = 4;
var OP_SET_POS = 9;
var OP_GET_POS = 10;
var OP_MOUSE = 11;
var OP_MOUSE_ISO = 12;
var OP_HEIGHT_AT = 13;
var OP_SET_ANIM = 14;
var OP_AGENT_SELECTED = 15;
var OP_UI_CONSUMED_CLICK = 16;
var OP_DELTA = 17;
var OP_ELAPSED = 18;
var OP_WAS_PRESSED = 19;
var OP_KEY_DOWN = 20;
var OP_WAS_KEY_PRESSED = 21;
var OP_SET_TILE = 23;
var OP_SET_HEIGHT = 24;
var OP_REBUILD_TERRAIN = 25;
var OP_REQUEST_PATH = 26;
var OP_GET_CAMERA = 27;
var OP_SET_CAMERA = 28;
var OP_PICK_AT = 29;
var OP_MOUSE_DOWN = 30;
var OP_MOUSE_RELEASED = 31;
var OP_MOUSE_WHEEL = 32;
var OP_KEY_UP = 33;
var OP_GET_LIGHT = 34;
var OP_SET_LIGHT = 35;
var OP_SPAWN_RECT = 36;
var OP_SPAWN_TEXT = 37;
var OP_SET_TEXT = 38;
var OP_UI_CONTAINER = 39;
var OP_UI_TEXT = 40;
var OP_UI_BUTTON = 41;
var OP_UI_ARRAY = 42;
var OP_UI_PADDING = 43;
var OP_UI_SPRITE = 44;
var OP_UI_ADD_CHILD = 45;
var OP_UI_ADD_TO_ROOT = 46;
var OP_UI_SET_SIZE = 47;
var OP_UI_SET_ANCHOR = 48;
var OP_UI_SET_COLOR = 49;
var OP_UI_SET_FIXED = 50;
var OP_SUBSCRIBE = 51;
var OP_POLL_EVENT = 52;
var OP_SPAWN_COLLIDER = 53;
var OP_GET_ANIM = 54;
var OP_HAS_RESOURCE = 55;
var OP_TEXTURE_SIZE = 56;
var OP_FBM_FIELD = 57;
var OP_RIDGED_FIELD = 58;
var OP_BILLOW_FIELD = 59;
var OP_TILING_FIELD = 60;
var OP_NOISE_FIELD = 61;
var OP_NOISE2D = 62;
var OP_SET_TILES = 63;
var OP_SET_HEIGHTS = 64;
var OP_SET_NAV = 65;
var OP_SET_TILESET = 66;
var OP_COMMIT_TERRAIN = 68;
var OP_ISO_TO_SCREEN = 69;
var OP_SET_GRID = 70;
var OP_START_ANIM = 71;
var OP_VEHICLE_TELEPORT = 72;
var OP_VEHICLE_GOTO = 73;
var OP_VEHICLE_STOP = 74;
var OP_VEHICLE_SPAWN = 75;
var OP_POLL_PATH = 76;
var OP_SET_SPRITE_FRAME = 77;
var OP_SET_SPRITE_COLOR = 78;
var OP_SPAWN_SPRITE_CLONE = 79;
var OP_SET_ENABLED = 80;
var OP_VEHICLE_SET_SPEED = 81;
var OP_SELECTED_NAMES = 82;
var OP_SELECTION_CLEAR = 83;
var OP_INVENTORY_DUMP = 84;
var OP_INVENTORY_ADD = 85;
var OP_INVENTORY_REMOVE = 86;
var OP_INVENTORY_TRANSFER = 87;
var OP_ITEM_DEF = 88;
var OP_SET_SPRITE_OFFSET = 89;
var OP_GET_SPRITE_FRAME = 90;
var OP_INVENTORY_CAPACITY = 91;
var OP_VEHICLE_PROBE = 92;
var OP_VEHICLE_PROBE_CLEAR = 93;

var encoder = new TextEncoder();
var decoder = new TextDecoder();

function memView() {
    return new Uint8Array(memory.buffer);
}

function readStr(ptr, len) {
    return decoder.decode(memView().subarray(ptr, ptr + len));
}

function writeMem(ptr, bytes) {
    memView().set(bytes, ptr);
}

// Send one host-import call to the main thread and block for its response.
function hostCall(op, strArr, numArr) {
    var soff = 0;
    for (var i = 0; i < strArr.length; i++) {
        var b = encoder.encode(strArr[i]);
        str[soff] = b.length & 0xff;
        str[soff + 1] = (b.length >> 8) & 0xff;
        str[soff + 2] = (b.length >> 16) & 0xff;
        str[soff + 3] = (b.length >> 24) & 0xff;
        soff += 4;
        str.set(b, soff);
        soff += b.length;
    }
    Atomics.store(flags, I_REQ_STR_LEN, soff);
    for (var j = 0; j < numArr.length; j++) {
        nums[j] = numArr[j];
    }
    Atomics.store(flags, I_REQ_NUM_COUNT, numArr.length);
    Atomics.store(flags, I_REQ_OP, op);
    Atomics.store(flags, I_REQ_READY, 1);
    Atomics.notify(flags, I_REQ_READY, 1);
    Atomics.wait(flags, I_RESP_READY, 0);
    var ret = nums[F_RET];
    var outLen = Atomics.load(flags, I_RESP_OUT_LEN);
    var outBytes = out.slice(0, outLen);
    Atomics.store(flags, I_RESP_READY, 0);
    return { ret: ret, out: outBytes };
}

// Send raw guest bytes (bulk upload) to the main thread and block for the
// response.  Unlike `hostCall`, the request payload is a raw byte span, not a
// length-prefixed string stream.
function hostCallRaw(op, ptr, len, numArr) {
    str.set(memView().subarray(ptr, ptr + len), 0);
    Atomics.store(flags, I_REQ_STR_LEN, len);
    for (var j = 0; j < numArr.length; j++) {
        nums[j] = numArr[j];
    }
    Atomics.store(flags, I_REQ_NUM_COUNT, numArr.length);
    Atomics.store(flags, I_REQ_OP, op);
    Atomics.store(flags, I_REQ_READY, 1);
    Atomics.notify(flags, I_REQ_READY, 1);
    Atomics.wait(flags, I_RESP_READY, 0);
    var ret = nums[F_RET];
    var outLen = Atomics.load(flags, I_RESP_OUT_LEN);
    var outBytes = out.slice(0, outLen);
    Atomics.store(flags, I_RESP_READY, 0);
    return { ret: ret, out: outBytes };
}

function envImports() {
    return {
        log: function (ptr, len) {
            hostCall(OP_LOG, [readStr(ptr, len)], []);
        },
        spawn: function (ptr, len) {
            return hostCall(OP_SPAWN, [readStr(ptr, len)], []).ret | 0;
        },
        despawn: function (ptr, len) {
            return hostCall(OP_DESPAWN, [readStr(ptr, len)], []).ret | 0;
        },
        has: function (ptr, len) {
            return hostCall(OP_HAS, [readStr(ptr, len)], []).ret | 0;
        },
        names: function (outPtr, outCap) {
            var r = hostCall(OP_NAMES, [], [outPtr, outCap]);
            if (r.out.length > 0) writeMem(outPtr, r.out);
            return r.ret | 0;
        },
        set_pos: function (ptr, len, x, y, z) {
            return hostCall(OP_SET_POS, [readStr(ptr, len)], [x, y, z]).ret | 0;
        },
        set_sprite_frame: function (ptr, len, frame) {
            return hostCall(OP_SET_SPRITE_FRAME, [readStr(ptr, len)], [frame]).ret | 0;
        },
        get_sprite_frame: function (ptr, len) {
            return hostCall(OP_GET_SPRITE_FRAME, [readStr(ptr, len)], []).ret;
        },
        set_sprite_color: function (ptr, len, r, g, b, a) {
            return hostCall(OP_SET_SPRITE_COLOR, [readStr(ptr, len)], [r, g, b, a]).ret | 0;
        },
        set_sprite_offset: function (ptr, len, dx, dy, dz) {
            return hostCall(OP_SET_SPRITE_OFFSET, [readStr(ptr, len)], [dx, dy, dz]).ret | 0;
        },
        spawn_sprite_clone: function (tPtr, tLen, nPtr, nLen) {
            return hostCall(OP_SPAWN_SPRITE_CLONE, [readStr(tPtr, tLen), readStr(nPtr, nLen)], []).ret | 0;
        },
        set_enabled: function (ptr, len, enabled) {
            return hostCall(OP_SET_ENABLED, [readStr(ptr, len)], [enabled]).ret | 0;
        },
        get_pos: function (ptr, len, outPtr) {
            var r = hostCall(OP_GET_POS, [readStr(ptr, len)], [outPtr]);
            if (r.out.length > 0) writeMem(outPtr, r.out);
            return r.ret | 0;
        },
        mouse: function (outPtr) {
            var r = hostCall(OP_MOUSE, [], [outPtr]);
            if (r.out.length > 0) writeMem(outPtr, r.out);
            return r.ret | 0;
        },
        mouse_iso: function (outPtr) {
            var r = hostCall(OP_MOUSE_ISO, [], [outPtr]);
            if (r.out.length > 0) writeMem(outPtr, r.out);
            return r.ret | 0;
        },
        iso_to_screen: function (x, y, outPtr) {
            var r = hostCall(OP_ISO_TO_SCREEN, [], [x, y, outPtr]);
            if (r.out.length > 0) writeMem(outPtr, r.out);
            return r.ret | 0;
        },
        height_at: function (x, y) {
            return hostCall(OP_HEIGHT_AT, [], [x, y]).ret;
        },
        set_anim: function (ptr, len, animPtr, animLen) {
            return hostCall(OP_SET_ANIM, [readStr(ptr, len), readStr(animPtr, animLen)], []).ret | 0;
        },
        start_anim: function (ptr, len, animPtr, animLen, repeat) {
            return hostCall(
                OP_START_ANIM,
                [readStr(ptr, len), readStr(animPtr, animLen)],
                [repeat],
            ).ret | 0;
        },
        agent_selected: function () {
            return hostCall(OP_AGENT_SELECTED, [], []).ret | 0;
        },
        ui_consumed_click: function () {
            return hostCall(OP_UI_CONSUMED_CLICK, [], []).ret | 0;
        },
        delta: function () {
            return hostCall(OP_DELTA, [], []).ret;
        },
        elapsed: function () {
            return hostCall(OP_ELAPSED, [], []).ret;
        },
        was_pressed: function (btn) {
            return hostCall(OP_WAS_PRESSED, [], [btn]).ret | 0;
        },
        key_down: function (ptr, len) {
            return hostCall(OP_KEY_DOWN, [readStr(ptr, len)], []).ret | 0;
        },
        was_key_pressed: function (ptr, len) {
            return hostCall(OP_WAS_KEY_PRESSED, [readStr(ptr, len)], []).ret | 0;
        },
        set_tile: function (x, y, id) {
            return hostCall(OP_SET_TILE, [], [x, y, id]).ret | 0;
        },
        set_height: function (x, y, h) {
            return hostCall(OP_SET_HEIGHT, [], [x, y, h]).ret | 0;
        },
        rebuild_terrain: function () {
            return hostCall(OP_REBUILD_TERRAIN, [], []).ret | 0;
        },
        request_path: function (sx, sy, ex, ey) {
            return hostCall(OP_REQUEST_PATH, [], [sx, sy, ex, ey]).ret | 0;
        },
        poll_path: function (id, outPtr, outCap) {
            var r = hostCall(OP_POLL_PATH, [], [id, outPtr, outCap]);
            if (r.out.length > 0) writeMem(outPtr, r.out);
            return r.ret | 0;
        },
        vehicle_teleport: function (ptr, len, x, y) {
            return hostCall(OP_VEHICLE_TELEPORT, [readStr(ptr, len)], [x, y]).ret | 0;
        },
        vehicle_spawn: function (defPtr, defLen, namePtr, nameLen, x, y) {
            return hostCall(
                OP_VEHICLE_SPAWN,
                [readStr(defPtr, defLen), readStr(namePtr, nameLen)],
                [x, y],
            ).ret | 0;
        },
        vehicle_goto: function (ptr, len, tx, ty) {
            return hostCall(OP_VEHICLE_GOTO, [readStr(ptr, len)], [tx, ty]).ret | 0;
        },
        vehicle_stop: function (ptr, len) {
            return hostCall(OP_VEHICLE_STOP, [readStr(ptr, len)], []).ret | 0;
        },
        vehicle_set_speed: function (ptr, len, speed) {
            return hostCall(OP_VEHICLE_SET_SPEED, [readStr(ptr, len)], [speed]).ret | 0;
        },
        vehicle_probe: function (ptr, len, tx, ty) {
            return hostCall(OP_VEHICLE_PROBE, [readStr(ptr, len)], [tx, ty]).ret | 0;
        },
        vehicle_probe_clear: function (ptr, len) {
            return hostCall(OP_VEHICLE_PROBE_CLEAR, [readStr(ptr, len)], []).ret | 0;
        },
        selected_names: function (outPtr, outCap) {
            var r = hostCall(OP_SELECTED_NAMES, [], [outPtr, outCap]);
            if (r.out.length > 0) writeMem(outPtr, r.out);
            return r.ret | 0;
        },
        selection_clear: function () {
            return hostCall(OP_SELECTION_CLEAR, [], []).ret | 0;
        },
        inventory_dump: function (ptr, len, outPtr, outCap) {
            var r = hostCall(OP_INVENTORY_DUMP, [readStr(ptr, len)], [outPtr, outCap]);
            if (r.out.length > 0) writeMem(outPtr, r.out);
            return r.ret | 0;
        },
        inventory_capacity: function (ptr, len) {
            return hostCall(OP_INVENTORY_CAPACITY, [readStr(ptr, len)], []).ret | 0;
        },
        inventory_add: function (ptr, len, itemPtr, itemLen, n) {
            return hostCall(
                OP_INVENTORY_ADD,
                [readStr(ptr, len), readStr(itemPtr, itemLen)],
                [n],
            ).ret | 0;
        },
        inventory_remove: function (ptr, len, itemPtr, itemLen, n) {
            return hostCall(
                OP_INVENTORY_REMOVE,
                [readStr(ptr, len), readStr(itemPtr, itemLen)],
                [n],
            ).ret | 0;
        },
        inventory_transfer: function (fromPtr, fromLen, toPtr, toLen, itemPtr, itemLen, n) {
            return hostCall(
                OP_INVENTORY_TRANSFER,
                [readStr(fromPtr, fromLen), readStr(toPtr, toLen), readStr(itemPtr, itemLen)],
                [n],
            ).ret | 0;
        },
        item_def: function (ptr, len, outPtr, outCap) {
            var r = hostCall(OP_ITEM_DEF, [readStr(ptr, len)], [outPtr, outCap]);
            if (r.out.length > 0) writeMem(outPtr, r.out);
            return r.ret | 0;
        },
        get_camera: function (outPtr) {
            var r = hostCall(OP_GET_CAMERA, [], [outPtr]);
            if (r.out.length > 0) writeMem(outPtr, r.out);
            return r.ret | 0;
        },
        set_camera: function (x, y, scale) {
            return hostCall(OP_SET_CAMERA, [], [x, y, scale]).ret | 0;
        },
        set_grid: function (show) {
            return hostCall(OP_SET_GRID, [], [show]).ret | 0;
        },
        pick_at: function (x, y, outPtr, outCap) {
            var r = hostCall(OP_PICK_AT, [], [x, y, outPtr, outCap]);
            if (r.out.length > 0) writeMem(outPtr, r.out);
            return r.ret | 0;
        },
        mouse_down: function (btn) {
            return hostCall(OP_MOUSE_DOWN, [], [btn]).ret | 0;
        },
        mouse_released: function (btn) {
            return hostCall(OP_MOUSE_RELEASED, [], [btn]).ret | 0;
        },
        mouse_wheel: function () {
            return hostCall(OP_MOUSE_WHEEL, [], []).ret;
        },
        key_up: function (ptr, len) {
            return hostCall(OP_KEY_UP, [readStr(ptr, len)], []).ret | 0;
        },
        get_light: function (outPtr) {
            var r = hostCall(OP_GET_LIGHT, [], [outPtr]);
            if (r.out.length > 0) writeMem(outPtr, r.out);
            return r.ret | 0;
        },
        set_light: function (a0, a1, a2, d0, d1, d2, c0, c1, c2) {
            return hostCall(OP_SET_LIGHT, [], [a0, a1, a2, d0, d1, d2, c0, c1, c2]).ret | 0;
        },
        spawn_rect: function (namePtr, nameLen, x, y, w, h, r, g, b, a) {
            return hostCall(OP_SPAWN_RECT, [readStr(namePtr, nameLen)], [x, y, w, h, r, g, b, a]).ret | 0;
        },
        spawn_text: function (namePtr, nameLen, x, y, textPtr, textLen, scale, r, g, b, a) {
            var ret = hostCall(OP_SPAWN_TEXT, [readStr(namePtr, nameLen), readStr(textPtr, textLen)], [x, y, scale, r, g, b, a]);
            return ret.ret | 0;
        },
        set_text: function (namePtr, nameLen, textPtr, textLen) {
            return hostCall(OP_SET_TEXT, [readStr(namePtr, nameLen), readStr(textPtr, textLen)], []).ret | 0;
        },
        ui_container: function (namePtr, nameLen, w, h, r, g, b, a) {
            return hostCall(OP_UI_CONTAINER, [readStr(namePtr, nameLen)], [w, h, r, g, b, a]).ret | 0;
        },
        ui_text: function (namePtr, nameLen, textPtr, textLen, scale, maxWidth, r, g, b, a, justify) {
            var ret = hostCall(OP_UI_TEXT, [readStr(namePtr, nameLen), readStr(textPtr, textLen)], [scale, maxWidth, r, g, b, a, justify]);
            return ret.ret | 0;
        },
        ui_button: function (namePtr, nameLen, textPtr, textLen, w, h, r, g, b, a) {
            var ret = hostCall(OP_UI_BUTTON, [readStr(namePtr, nameLen), readStr(textPtr, textLen)], [w, h, r, g, b, a]);
            return ret.ret | 0;
        },
        ui_array: function (namePtr, nameLen, vertical, align, spacing, r, g, b, a) {
            return hostCall(OP_UI_ARRAY, [readStr(namePtr, nameLen)], [vertical, align, spacing, r, g, b, a]).ret | 0;
        },
        ui_padding: function (namePtr, nameLen, top, right, bottom, left, r, g, b, a) {
            var ret = hostCall(OP_UI_PADDING, [readStr(namePtr, nameLen)], [top, right, bottom, left, r, g, b, a]);
            return ret.ret | 0;
        },
        ui_sprite: function (namePtr, nameLen, texturePtr, textureLen, w, h, frame, tsx, tsy) {
            var ret = hostCall(OP_UI_SPRITE, [readStr(namePtr, nameLen), readStr(texturePtr, textureLen)], [w, h, frame, tsx, tsy]);
            return ret.ret | 0;
        },
        ui_add_child: function (parentPtr, parentLen, childPtr, childLen, selfAnchor, childAnchor) {
            var ret = hostCall(OP_UI_ADD_CHILD, [readStr(parentPtr, parentLen), readStr(childPtr, childLen)], [selfAnchor, childAnchor]);
            return ret.ret | 0;
        },
        ui_add_to_root: function (namePtr, nameLen, selfAnchor, childAnchor) {
            var ret = hostCall(OP_UI_ADD_TO_ROOT, [readStr(namePtr, nameLen)], [selfAnchor, childAnchor]);
            return ret.ret | 0;
        },
        ui_set_size: function (namePtr, nameLen, w, h) {
            return hostCall(OP_UI_SET_SIZE, [readStr(namePtr, nameLen)], [w, h]).ret | 0;
        },
        ui_set_anchor: function (namePtr, nameLen, anchor) {
            return hostCall(OP_UI_SET_ANCHOR, [readStr(namePtr, nameLen)], [anchor]).ret | 0;
        },
        ui_set_color: function (namePtr, nameLen, r, g, b, a) {
            return hostCall(OP_UI_SET_COLOR, [readStr(namePtr, nameLen)], [r, g, b, a]).ret | 0;
        },
        ui_set_fixed: function (namePtr, nameLen, fixed) {
            return hostCall(OP_UI_SET_FIXED, [readStr(namePtr, nameLen)], [fixed]).ret | 0;
        },
        subscribe: function (ptr, len) {
            return hostCall(OP_SUBSCRIBE, [readStr(ptr, len)], []).ret | 0;
        },
        poll_event: function (outPtr, outCap) {
            var r = hostCall(OP_POLL_EVENT, [], [outPtr, outCap]);
            if (r.out.length > 0) writeMem(outPtr, r.out);
            return r.ret | 0;
        },
        spawn_collider: function (namePtr, nameLen, x, y, w, h) {
            return hostCall(OP_SPAWN_COLLIDER, [readStr(namePtr, nameLen)], [x, y, w, h]).ret | 0;
        },
        get_anim: function (namePtr, nameLen, outPtr, outCap) {
            var r = hostCall(OP_GET_ANIM, [readStr(namePtr, nameLen)], [outPtr, outCap]);
            if (r.out.length > 0) writeMem(outPtr, r.out);
            return r.ret | 0;
        },
        has_resource: function (kind, ptr, len) {
            return hostCall(OP_HAS_RESOURCE, [readStr(ptr, len)], [kind]).ret | 0;
        },
        texture_size: function (ptr, len, outPtr) {
            var r = hostCall(OP_TEXTURE_SIZE, [readStr(ptr, len)], [outPtr]);
            if (r.out.length > 0) writeMem(outPtr, r.out);
            return r.ret | 0;
        },
        fbm_field: function (w, h, seedPtr, seedLen, octaves, freq, lacunarity, gain, outPtr, outCap) {
            var r = hostCall(OP_FBM_FIELD, [readStr(seedPtr, seedLen)], [w, h, octaves, freq, lacunarity, gain, outPtr, outCap]);
            if (r.out.length > 0) writeMem(outPtr, r.out);
            return r.ret | 0;
        },
        ridged_field: function (w, h, seedPtr, seedLen, octaves, freq, lacunarity, gain, warpAmp, outPtr, outCap) {
            var r = hostCall(OP_RIDGED_FIELD, [readStr(seedPtr, seedLen)], [w, h, octaves, freq, lacunarity, gain, warpAmp, outPtr, outCap]);
            if (r.out.length > 0) writeMem(outPtr, r.out);
            return r.ret | 0;
        },
        billow_field: function (w, h, seedPtr, seedLen, octaves, freq, lacunarity, gain, outPtr, outCap) {
            var r = hostCall(OP_BILLOW_FIELD, [readStr(seedPtr, seedLen)], [w, h, octaves, freq, lacunarity, gain, outPtr, outCap]);
            if (r.out.length > 0) writeMem(outPtr, r.out);
            return r.ret | 0;
        },
        tiling_field: function (w, h, seedPtr, seedLen, period, octaves, radius, outPtr, outCap) {
            var r = hostCall(OP_TILING_FIELD, [readStr(seedPtr, seedLen)], [w, h, period, octaves, radius, outPtr, outCap]);
            if (r.out.length > 0) writeMem(outPtr, r.out);
            return r.ret | 0;
        },
        noise_field: function (w, h, seedPtr, seedLen, freqX, freqY, outPtr, outCap) {
            var r = hostCall(OP_NOISE_FIELD, [readStr(seedPtr, seedLen)], [w, h, freqX, freqY, outPtr, outCap]);
            if (r.out.length > 0) writeMem(outPtr, r.out);
            return r.ret | 0;
        },
        noise2d: function (seedPtr, seedLen, x, y) {
            return hostCall(OP_NOISE2D, [readStr(seedPtr, seedLen)], [x, y]).ret;
        },
        set_tiles: function (ptr, len) {
            return hostCallRaw(OP_SET_TILES, ptr, len, []).ret | 0;
        },
        set_heights: function (ptr, len) {
            return hostCallRaw(OP_SET_HEIGHTS, ptr, len, []).ret | 0;
        },
        set_nav: function (ptr, len) {
            return hostCallRaw(OP_SET_NAV, ptr, len, []).ret | 0;
        },
        set_tileset: function (ptr, len, w, h) {
            return hostCallRaw(OP_SET_TILESET, ptr, len, [w, h]).ret | 0;
        },
        commit_terrain: function (heightScale) {
            return hostCall(OP_COMMIT_TERRAIN, [], [heightScale]).ret | 0;
        },
    };
}

self.onmessage = function (e) {
    var sab = e.data;
    flags = new Int32Array(sab, 0, 16);
    nums = new Float64Array(sab, NUM_OFFSET, 16);
    str = new Uint8Array(sab, STR_OFFSET, STR_BYTES);
    out = new Uint8Array(sab, OUT_OFFSET, OUT_BYTES);

    var wasmLen = Atomics.load(flags, I_WASM_LEN);
    var wasmRegion = new Uint8Array(sab, WASM_OFFSET, SAB_SIZE - WASM_OFFSET);
    var module = new WebAssembly.Module(wasmRegion.slice(0, wasmLen));

    instance = new WebAssembly.Instance(module, { env: envImports() });
    memory = instance.exports.memory;

    var init = instance.exports.init;
    var update = instance.exports.update;
    var start = instance.exports.start;

    while (true) {
        Atomics.wait(flags, I_GO, 0);
        Atomics.store(flags, I_GO, 0);
        var cmd = Atomics.load(flags, I_COMMAND);
        if (cmd === CMD_INIT && init) {
            init();
        } else if (cmd === CMD_UPDATE && update) {
            update(nums[F_DT]);
        } else if (cmd === CMD_START && start) {
            start();
        }
        Atomics.store(flags, I_DONE, 1);
        Atomics.notify(flags, I_DONE, 1);
    }
};
