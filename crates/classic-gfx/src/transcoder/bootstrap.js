// classic-gfx web Basis Universal transcoder bootstrap (P1.0/R2).
//
// This script is appended after the vendored `basis_transcoder.js` (three.js
// r162, MIT/Apache — see NOTICE) and evaluated once by `basis_web.rs`.  The
// vendored UMD defines the `BASIS` Emscripten factory as a top-level `var`;
// here we instantiate it SYNCHRONOUSLY on the main thread via the Emscripten
// `instantiateWasm` hook (WebAssembly.Module + WebAssembly.Instance are
// synchronous, unlike `instantiateStreaming`), so the engine's synchronous
// `transcode`/`transcode_rgba8` load path can call straight into it.
//
// Exposed on `globalThis.__classicBasisTranscoder`:
//   initialize(wasmBytes: Uint8Array)      — instantiate + initializeBasis()
//   transcode(basisBytes: Uint8Array, format: number)
//       -> { width, height, data: Uint8Array } | null
//
// `format` is the basis_universal `transcoder_texture_format` enum (see the
// r162 `TranscoderFormat` constants — 1 = ETC2_RGBA, 3 = BC3_RGBA, 4 = BC4_R,
// 7 = BC7_M5, 13 = RGBA32).
(function () {
    var Module = null;

    function initialize(wasmBytes) {
        var config = {
            wasmBinary: wasmBytes,
            instantiateWasm: function (imports, receiveInstance) {
                var module = new WebAssembly.Module(wasmBytes);
                var instance = new WebAssembly.Instance(module, imports);
                receiveInstance(instance);
                return instance.exports;
            },
        };
        BASIS(config);
        config.initializeBasis();
        Module = config;
    }

    function transcode(basisBytes, format) {
        if (!Module) {
            return null;
        }
        var file = new Module.BasisFile(basisBytes);
        try {
            if (file.getNumImages() === 0 || file.getNumLevels(0) === 0) {
                return null;
            }
            var width = file.getImageWidth(0, 0);
            var height = file.getImageHeight(0, 0);
            if (!width || !height) {
                return null;
            }
            if (!file.startTranscoding()) {
                return null;
            }
            var hasAlpha = file.getHasAlpha();
            var size = file.getImageTranscodedSizeInBytes(0, 0, format);
            var dst = new Uint8Array(size);
            var status = file.transcodeImage(dst, 0, 0, format, 0, hasAlpha);
            if (!status) {
                return null;
            }
            return { width: width, height: height, data: dst };
        } finally {
            file.close();
            file.delete();
        }
    }

    globalThis.__classicBasisTranscoder = {
        initialize: initialize,
        transcode: transcode,
    };
})();
