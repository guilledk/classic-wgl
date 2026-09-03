// classic-worker pathfinder worker (web): instantiates the compiled
// `pathfinder.wasm` (the same Rust pathfinder the native worker thread runs)
// and forwards messages.  No pathfinding algorithm lives here.
//
// Messages (from the main thread):
//   { type: "init", wasm: Uint8Array }                        — instantiate wasm
//   { type: "snapshot", sizeX, sizeY, data: Int32Array }      — replace nav grid
//   { type: "find", id, from: [x, y], to: [x, y] }            — run a search
//   { type: "vehicleSnapshot", sizeX, sizeY, structural: Int32Array,
//     heights: Float32Array, tileM }                          — replace vehicle nav
//   { type: "findVehicle", id, from: [x, y], to: [x, y],
//     footprint: [[dx, dy], …], pitchMax, rollMax, wheelbaseM, trackM,
//     safeFallM, jumpCost, turnCost }                          — run a vehicle search
// Replies (to the main thread):
//   { type: "result", id, path: Int32Array | null }           — flat [x0,y0,…]
//      where `path` is null when no route exists.

var exports = null;
var memory = null;
var pending = [];

function copyInto(arr) {
    var ptr = exports.alloc(arr.length * 4);
    var u8 = new Uint8Array(memory.buffer, ptr, arr.length * 4);
    u8.set(new Uint8Array(arr.buffer, arr.byteOffset, arr.length * 4));
    return ptr;
}

function readResult(n) {
    return new Int32Array(memory.buffer, exports.result_ptr(), n * 2);
}

function handle(msg) {
    if (msg.type === "snapshot") {
        var ptr = copyInto(msg.data);
        exports.set_snapshot(msg.sizeX, msg.sizeY, ptr, msg.data.length);
    } else if (msg.type === "find") {
        var n = exports.find(msg.from[0], msg.from[1], msg.to[0], msg.to[1]);
        self.postMessage({ type: "result", id: msg.id, path: n < 0 ? null : readResult(n) });
    } else if (msg.type === "vehicleSnapshot") {
        var sp = copyInto(msg.structural);
        var hp = copyInto(msg.heights);
        exports.set_vehicle_snapshot(
            msg.sizeX,
            msg.sizeY,
            sp,
            msg.structural.length,
            hp,
            msg.heights.length,
            msg.tileM,
        );
    } else if (msg.type === "findVehicle") {
        var fp = new Int32Array(msg.footprint.length * 2);
        for (var i = 0; i < msg.footprint.length; i++) {
            fp[i * 2] = msg.footprint[i][0];
            fp[i * 2 + 1] = msg.footprint[i][1];
        }
        var fpp = copyInto(fp);
        var n = exports.find_vehicle(
            msg.from[0],
            msg.from[1],
            msg.to[0],
            msg.to[1],
            fpp,
            msg.footprint.length,
            msg.pitchMax,
            msg.rollMax,
            msg.wheelbaseM,
            msg.trackM,
            msg.safeFallM,
            msg.jumpCost,
            msg.turnCost,
        );
        self.postMessage({ type: "result", id: msg.id, path: n < 0 ? null : readResult(n) });
    }
}

self.onmessage = function (e) {
    var msg = e.data;
    if (msg.type === "init") {
        WebAssembly.instantiate(msg.wasm, {}).then(function (r) {
            exports = r.instance.exports;
            memory = exports.memory;
            while (pending.length) handle(pending.shift());
        });
        return;
    }
    if (exports === null) {
        pending.push(msg);
        return;
    }
    handle(msg);
};
