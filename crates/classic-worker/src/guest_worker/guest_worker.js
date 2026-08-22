// classic-worker guest worker (web): instantiates a ROM's background guest
// wasm module (Tier 3) and runs its pure entry points off the render thread.
//
// The reduced import surface surfaced here is `env.task_arg` / `env.task_return`
// only — the two imports the shipped `lunar-worker` guest uses.  Its heavy
// noise/field/kernel computation is compiled *into* the wasm, so nothing else
// needs to cross the Worker boundary.  A worker guest that imports the wider
// reduced surface (or any engine-mutating import) fails to instantiate with an
// "unknown import" error rather than running against a silently-wrong host.
//
// Messages (from the main thread):
//   { type: "init", wasm: Uint8Array }                 — instantiate the wasm
//   { type: "run", id, entry, arg: Uint8Array }        — run a named export
// Replies (to the main thread):
//   { type: "result", id, result: Uint8Array }         — task_return payload
//   { type: "error", id, message }                     — trapped/panicked

var exports = null;
var memory = null;
var pending = [];
var currentArg = null;
var currentResult = null;

function imports() {
    return {
        task_arg: function (outPtr, outCap) {
            var n = currentArg ? currentArg.length : 0;
            if (n > outCap) {
                return -1;
            }
            if (n > 0) {
                new Uint8Array(memory.buffer, outPtr, n).set(currentArg);
            }
            return n;
        },
        task_return: function (ptr, len) {
            currentResult = new Uint8Array(memory.buffer, ptr, len).slice();
        },
    };
}

function handle(msg) {
    currentArg = msg.arg;
    currentResult = null;
    try {
        exports[msg.entry]();
        self.postMessage({
            type: "result",
            id: msg.id,
            result: currentResult || new Uint8Array(0),
        });
    } catch (err) {
        var message = err && err.message ? err.message : String(err);
        self.postMessage({ type: "error", id: msg.id, message: message });
    }
}

self.onmessage = function (e) {
    var msg = e.data;
    if (msg.type === "init") {
        WebAssembly.instantiate(msg.wasm, { env: imports() }).then(function (r) {
            exports = r.instance.exports;
            memory = exports.memory;
            while (pending.length) {
                handle(pending.shift());
            }
        });
        return;
    }
    if (exports === null) {
        pending.push(msg);
        return;
    }
    handle(msg);
};
