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
//     footprint: [[dx, dy], …], body: [rear, front, right, left] | null,
//     pivot, pitchMax, rollMax, wheelbaseM, trackM, safeFallM, jumpCost,
//     turnCost, pose: [x, y, heading, startHeading, turnRadius, reverse,
//     reverseCost, gearCost] | null, closed: Int32Array }                — run a vehicle search
//     (a non-null `body` is a turned body rectangle and `footprint` is unused;
//     a non-null `pose` drives to that pose and `to` is unused; `closed` is
//     flat [x0, y0, x1, y1, …] tile rects blocked for this search only)
// Replies (to the main thread):
//   { type: "result", id, path: Int32Array | null }           — flat [x0,y0,…]
//      where `path` is null when no route exists.

var exports = null;
var memory = null;
var pending = [];

// Copy a typed array into a fresh wasm buffer; hand it back with `release`
// once the call that reads it returns.
function copyInto(arr) {
    var ptr = exports.alloc(arr.length * 4);
    var u8 = new Uint8Array(memory.buffer, ptr, arr.length * 4);
    u8.set(new Uint8Array(arr.buffer, arr.byteOffset, arr.length * 4));
    return ptr;
}

function release(ptr, arr) {
    exports.dealloc(ptr, arr.length * 4);
}

// Reply with a search result.  The path is copied out of wasm memory into its
// own buffer and *transferred*: posting a view of `memory.buffer` would
// structured-clone the entire wasm heap.
function postResult(id, n) {
    if (n < 0) {
        self.postMessage({ type: "result", id: id, path: null });
        return;
    }
    var path = new Int32Array(memory.buffer, exports.result_ptr(), n * 2).slice();
    self.postMessage({ type: "result", id: id, path: path }, [path.buffer]);
}

function handle(msg) {
    if (msg.type === "snapshot") {
        var ptr = copyInto(msg.data);
        exports.set_snapshot(msg.sizeX, msg.sizeY, ptr, msg.data.length);
        release(ptr, msg.data);
    } else if (msg.type === "find") {
        var n = exports.find(msg.from[0], msg.from[1], msg.to[0], msg.to[1]);
        postResult(msg.id, n);
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
        release(sp, msg.structural);
        release(hp, msg.heights);
    } else if (msg.type === "findVehicle") {
        var fp = new Int32Array(msg.footprint.length * 2);
        for (var i = 0; i < msg.footprint.length; i++) {
            fp[i * 2] = msg.footprint[i][0];
            fp[i * 2 + 1] = msg.footprint[i][1];
        }
        var fpp = copyInto(fp);
        var body = msg.body || [0, 0, 0, 0];
        // a pose goal [x, y, heading, startHeading, turnRadius, reverse,
        // reverseCost, gearCost] turns this into a pose search
        var pose = msg.pose || [0, 0, 0, 0, 0, 0, 0, 0];
        exports.set_pose_goal(msg.pose ? 1 : 0, pose[0], pose[1], pose[2], pose[3], pose[4], pose[5], pose[6], pose[7]);
        var closed = msg.closed || new Int32Array(0);
        var cp = copyInto(closed);
        exports.set_closed(cp, closed.length / 4);
        release(cp, closed);
        var n = exports.find_vehicle(
            msg.from[0],
            msg.from[1],
            msg.to[0],
            msg.to[1],
            fpp,
            msg.footprint.length,
            msg.body ? 1 : 0,
            body[0],
            body[1],
            body[2],
            body[3],
            msg.pivot ? 1 : 0,
            msg.pitchMax,
            msg.rollMax,
            msg.wheelbaseM,
            msg.trackM,
            msg.safeFallM,
            msg.jumpCost,
            msg.turnCost,
        );
        release(fpp, fp);
        postResult(msg.id, n);
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
