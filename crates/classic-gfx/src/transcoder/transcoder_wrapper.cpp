// classic-gfx web Basis Universal transcoder wrapper (P1.0/R3).
//
// A minimal C ABI over the Basis Universal transcoder (BinomialLLC
// basis_universal, Apache 2.0), built transcoder-only (no encoder) with
// Emscripten.  This is the *only* C++ in the web path; it exposes two symbols
// consumed by `bootstrap.js` / `transcoder_worker.js`:
//
//   int32_t classic_initialize(void)
//       Initialize the transcoder lookup tables + create the persistent
//       `basisu_transcoder` instance.  Idempotent.
//
//   int32_t classic_transcode(
//       const uint8_t* data, uint32_t data_len, uint32_t format,
//       uint32_t* out_width, uint32_t* out_height,
//       uint8_t** out_data, uint32_t* out_data_len)
//       Transcode `data` (a .basis payload) to `format` (a
//       `transcoder_texture_format` value), allocating the output buffer with
//       `malloc`.  The caller frees `*out_data` (and any input scratch) via
//       `free`.  Returns 1 on success, 0 on failure.
//
// The wrapper mirrors the native `basis-universal` crate's transcode path
// (image 0, level 0, zero row pitch/rows), so web and native transcode the
// same payload to the same bytes.

#include "basisu_transcoder.h"

#include <stdint.h>
#include <stdlib.h>

static basist::basisu_transcoder* g_transcoder = nullptr;

extern "C" {

int32_t classic_initialize(void) {
    if (g_transcoder != nullptr) {
        return 1;
    }
    basist::basisu_transcoder_init();
    g_transcoder = new basist::basisu_transcoder();
    return g_transcoder != nullptr ? 1 : 0;
}

int32_t classic_transcode(
    const uint8_t* data,
    uint32_t data_len,
    uint32_t format,
    uint32_t* out_width,
    uint32_t* out_height,
    uint8_t** out_data,
    uint32_t* out_data_len) {
    if (g_transcoder == nullptr || data == nullptr || data_len == 0) {
        return 0;
    }
    const basist::transcoder_texture_format fmt =
        static_cast<basist::transcoder_texture_format>(format);

    if (!g_transcoder->validate_header(data, data_len)) {
        return 0;
    }
    // start_transcoding decompresses the selector/endpoint codebooks for *this*
    // file, so it must be called once per payload (the transcoder instance is
    // reused across the whole batch).
    if (!g_transcoder->start_transcoding(data, data_len)) {
        return 0;
    }

    uint32_t orig_width = 0;
    uint32_t orig_height = 0;
    uint32_t total_blocks = 0;
    if (!g_transcoder->get_image_level_desc(
            data, data_len, 0, 0, orig_width, orig_height, total_blocks)) {
        return 0;
    }

    // Output buffer sizing matches the native crate's
    // `calculate_minimum_output_buffer_bytes` (row pitch / rows default to 0):
    // uncompressed formats are sized in pixels, compressed in blocks.
    const bool uncompressed = basist::basis_transcoder_format_is_uncompressed(fmt);
    const uint32_t blocks_or_pixels = uncompressed ? orig_width * orig_height : total_blocks;
    const uint32_t size = blocks_or_pixels * basist::basis_get_bytes_per_block_or_pixel(fmt);

    uint8_t* dst = static_cast<uint8_t*>(malloc(size));
    if (dst == nullptr) {
        return 0;
    }

    if (!g_transcoder->transcode_image_level(
            data,
            data_len,
            0,  // image_index
            0,  // level_index
            dst,
            blocks_or_pixels,
            fmt,
            0,  // decode_flags
            0,  // output_row_pitch_in_blocks_or_pixels
            nullptr,  // pState
            0)) {  // output_rows_in_pixels
        free(dst);
        return 0;
    }

    *out_width = orig_width;
    *out_height = orig_height;
    *out_data = dst;
    *out_data_len = size;
    return 1;
}

}  // extern "C"
