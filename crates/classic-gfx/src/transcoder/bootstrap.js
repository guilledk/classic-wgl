// classic-gfx web Basis Universal transcoder bootstrap (P1.0/R3).
//
// Our own glue over the self-built `basis_transcoder.wasm` (see build.sh +
// NOTICE).  The wasm is a STANDALONE_WASM module, so it has no Emscripten JS
// runtime — only four import stubs (memory-growth notify + WASI fd_*, none of
// which the transcoder calls in practice) and the two exported C entry points
// from `transcoder_wrapper.cpp`:
//
//   classic_initialize()                       — init tables + persistent transcoder
//   classic_transcode(ptr, len, format, ...)   — transcode a .basis payload
//
// Exposed on `globalThis.__classicBasisTranscoder`:
//   initialize(wasmBytes: Uint8Array)      — instantiate (SYNCHRONOUS, via
//                                            WebAssembly.Module + Instance) +
//                                            classic_initialize()
//   transcode(basisBytes: Uint8Array, format: number)
//       -> { width, height, data: Uint8Array } | null
//
// `format` is the basis_universal `transcoder_texture_format` enum (1 =
// ETC2_RGBA, 3 = BC3_RGBA, 4 = BC4_R, 6 = BC7_RGBA, 13 = RGBA32,
// 20 = ETC2_EAC_R11, 21 = ETC2_EAC_RG11).
//
// This is the main-thread SYNCHRONOUS fallback; the fast path transcodes in a
// dedicated Worker (`transcoder_worker.js`, async `WebAssembly.instantiate`).
(function () {
    var memory = null;
    var malloc = null;
    var free = null;
    var tc = null;

    var imports = {
        env: { emscripten_notify_memory_growth: function () {} },
        wasi_snapshot_preview1: {
            fd_close: function () { return 0; },
            fd_write: function () { return 0; },
            fd_seek: function () { return 0; },
        },
    };

    function initialize(wasmBytes) {
        var module = new WebAssembly.Module(wasmBytes);
        var instance = new WebAssembly.Instance(module, imports);
        var exports = instance.exports;
        if (exports._initialize) {
            exports._initialize();
        }
        exports.classic_initialize();
        memory = exports.memory;
        malloc = exports.malloc;
        free = exports.free;
        tc = exports.classic_transcode;
    }

    function transcode(basisBytes, format) {
        if (!tc) {
            return null;
        }
        var inPtr = malloc(basisBytes.length);
        new Uint8Array(memory.buffer, inPtr, basisBytes.length).set(basisBytes);
        var outPtr = malloc(16);
        if (!tc(inPtr, basisBytes.length, format, outPtr, outPtr + 4, outPtr + 8, outPtr + 12)) {
            free(inPtr);
            free(outPtr);
            return null;
        }
        var v = new Uint32Array(memory.buffer, outPtr, 4);
        var width = v[0];
        var height = v[1];
        var dataPtr = v[2];
        var dataLen = v[3];
        var data = new Uint8Array(memory.buffer, dataPtr, dataLen).slice();
        free(dataPtr);
        free(inPtr);
        free(outPtr);
        return { width: width, height: height, data: data };
    }

    globalThis.__classicBasisTranscoder = {
        initialize: initialize,
        transcode: transcode,
    };
})();
