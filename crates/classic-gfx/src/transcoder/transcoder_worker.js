// classic-gfx web Basis Universal transcoder worker (P1.0/R3).
//
// Runs the self-built `basis_transcoder.wasm` (STANDALONE_WASM — no Emscripten
// JS runtime) in a dedicated Worker, so the CPU transcode happens off the main
// thread.  The wasm is instantiated asynchronously (`WebAssembly.instantiate`)
// on `init`; any `transcode` messages that arrive first are queued.
//
// Messages (from the main thread):
//   { type: "init", wasm: Uint8Array }                         — instantiate wasm
//   { type: "transcode", id, bytes: Uint8Array, format: number } — transcode
// Replies (to the main thread):
//   { type: "result", id, ok: boolean,
//     width: number, height: number, data: Uint8Array }        — `data` present iff ok
//
// `format` is the basis_universal `transcoder_texture_format` enum — see
// bootstrap.js for the values.

var memory = null;
var malloc = null;
var free = null;
var tc = null;
var pending = [];

var imports = {
    env: { emscripten_notify_memory_growth: function () {} },
    wasi_snapshot_preview1: {
        fd_close: function () { return 0; },
        fd_write: function () { return 0; },
        fd_seek: function () { return 0; },
    },
};

function init(wasmBytes) {
    WebAssembly.instantiate(wasmBytes, imports).then(function (r) {
        var exports = r.instance.exports;
        if (exports._initialize) {
            exports._initialize();
        }
        exports.classic_initialize();
        memory = exports.memory;
        malloc = exports.malloc;
        free = exports.free;
        tc = exports.classic_transcode;
        while (pending.length) {
            handle(pending.shift());
        }
    });
}

function handle(msg) {
    var inPtr = malloc(msg.bytes.length);
    new Uint8Array(memory.buffer, inPtr, msg.bytes.length).set(msg.bytes);
    var outPtr = malloc(16);
    var ok = tc(inPtr, msg.bytes.length, msg.format, outPtr, outPtr + 4, outPtr + 8, outPtr + 12);
    var result = { type: "result", id: msg.id, ok: !!ok };
    if (ok) {
        var v = new Uint32Array(memory.buffer, outPtr, 4);
        result.width = v[0];
        result.height = v[1];
        result.data = new Uint8Array(memory.buffer, v[2], v[3]).slice();
        free(v[2]);
    }
    free(inPtr);
    free(outPtr);
    self.postMessage(result);
}

self.onmessage = function (e) {
    var msg = e.data;
    if (msg.type === "init") {
        init(msg.wasm);
        return;
    }
    if (tc === null) {
        pending.push(msg);
        return;
    }
    handle(msg);
};
