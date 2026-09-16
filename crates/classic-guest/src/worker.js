// classic-guest Worker: runs an untrusted ROM guest on the browser's native
// WebAssembly engine, with host imports bridged synchronously to the main
// thread over a SharedArrayBuffer + Atomics channel.  The main thread services
// host imports against the Engine and terminates this worker if a call
// exceeds its wall-clock budget (browser Wasm has no fuel API).
//
// The import stubs are built from the descriptor the main thread sends
// (generated from the ABI table): op code = index.  Each call sends every wasm
// argument as a number plus one length-prefixed blob per guest-memory
// parameter; requests and responses stream through BUF in chunks.
//
// The SAB layout (offsets and flag indices) must match `runtime_worker.rs`.

var FLAG_SLOTS = 16;
var NUM_OFFSET = 128;
var NUM_SLOTS = 40;
var BUF_OFFSET = NUM_OFFSET + NUM_SLOTS * 8;

var I_REQ_READY = 0;
var I_RESP_READY = 1;
var I_DONE = 2;
var I_GO = 3;
var I_COMMAND = 4;
var I_FAULT = 5;
var I_REQ_OP = 6;
var I_REQ_NUM_COUNT = 7;
var I_MSG = 8;
var I_CHUNK_LEN = 9;
var I_TOTAL_LEN = 10;
var I_READY = 11;

var F_DT = 32;
var F_RET = 33;

var CMD_INIT = 0;
var CMD_UPDATE = 1;
var CMD_START = 2;

var MSG_REQUEST = 0;
var MSG_RESPONSE_ACK = 1;

var flags;
var nums;
var buf;
var memory = null;
var instance = null;

function memView() {
    return new Uint8Array(memory.buffer);
}

// Hand the current message to the main thread and block until it responds.
function post(msg) {
    Atomics.store(flags, I_MSG, msg);
    Atomics.store(flags, I_REQ_READY, 1);
    Atomics.notify(flags, I_REQ_READY, 1);
    Atomics.wait(flags, I_RESP_READY, 0);
    Atomics.store(flags, I_RESP_READY, 0);
}

// One host-import call: stream the request, then collect the response.
function hostCall(op, numArr, payload) {
    for (var j = 0; j < numArr.length; j++) {
        nums[j] = numArr[j];
    }
    Atomics.store(flags, I_REQ_NUM_COUNT, numArr.length);
    Atomics.store(flags, I_REQ_OP, op);

    var off = 0;
    do {
        var n = Math.min(buf.length, payload.length - off);
        buf.set(payload.subarray(off, off + n), 0);
        Atomics.store(flags, I_CHUNK_LEN, n);
        Atomics.store(flags, I_TOTAL_LEN, payload.length);
        off += n;
        post(MSG_REQUEST);
    } while (off < payload.length);

    var ret = nums[F_RET];
    var total = Atomics.load(flags, I_TOTAL_LEN);
    var out = new Uint8Array(total);
    var got = 0;
    for (;;) {
        var chunk = Atomics.load(flags, I_CHUNK_LEN);
        out.set(buf.subarray(0, chunk), got);
        got += chunk;
        if (got >= total || chunk === 0) break;
        post(MSG_RESPONSE_ACK);
    }
    return { ret: ret, out: out };
}

// Build one import stub from a descriptor entry `[name, shape, outParams, returnsValue]`.
function makeImport(op, shape, outParams, returnsValue) {
    return function () {
        var args = arguments;
        var numArr = [];
        var blobs = [];
        var size = 0;
        var a = 0;
        for (var i = 0; i < shape.length; i++) {
            if (shape.charCodeAt(i) === 110 /* 'n' */) {
                numArr.push(args[a++]);
            } else {
                var ptr = Math.max(args[a++] | 0, 0);
                var len = Math.max(args[a++] | 0, 0);
                numArr.push(ptr, len);
                var blob = memView().slice(ptr, ptr + len);
                blobs.push(blob);
                size += 4 + blob.length;
            }
        }
        var outPtr = outParams > 0 ? args[a] : 0;
        for (var k = 0; k < outParams; k++) {
            numArr.push(args[a++]);
        }

        var payload = new Uint8Array(size);
        var view = new DataView(payload.buffer);
        var p = 0;
        for (var b = 0; b < blobs.length; b++) {
            view.setUint32(p, blobs[b].length, true);
            payload.set(blobs[b], p + 4);
            p += 4 + blobs[b].length;
        }

        var r = hostCall(op, numArr, payload);
        if (r.out.length > 0) memView().set(r.out, outPtr);
        return returnsValue ? r.ret : undefined;
    };
}

function envImports(descriptor) {
    var env = {};
    for (var op = 0; op < descriptor.length; op++) {
        var d = descriptor[op];
        env[d[0]] = makeImport(op, d[1], d[2], d[3]);
    }
    return env;
}

// Report a guest trap / link error to the main thread (message in BUF).
function fault(e) {
    var bytes = new TextEncoder().encode(String(e && e.stack ? e.stack : e));
    var n = Math.min(bytes.length, buf.length);
    buf.set(bytes.subarray(0, n), 0);
    Atomics.store(flags, I_CHUNK_LEN, n);
    Atomics.store(flags, I_FAULT, 1);
}

self.onmessage = function (e) {
    var sab = e.data.sab;
    flags = new Int32Array(sab, 0, FLAG_SLOTS);
    nums = new Float64Array(sab, NUM_OFFSET, NUM_SLOTS);
    buf = new Uint8Array(sab, BUF_OFFSET, sab.byteLength - BUF_OFFSET);

    var init = null;
    var update = null;
    var start = null;
    var linkError = null;
    try {
        var module = new WebAssembly.Module(e.data.wasm);
        instance = new WebAssembly.Instance(module, { env: envImports(JSON.parse(e.data.imports)) });
        memory = instance.exports.memory;
        init = instance.exports.init;
        update = instance.exports.update;
        start = instance.exports.start;
    } catch (err) {
        linkError = err;
    }
    // Booted: the main thread may now run guest entry points (a link error is
    // reported by the first call).
    Atomics.store(flags, I_READY, 1);

    while (true) {
        Atomics.wait(flags, I_GO, 0);
        Atomics.store(flags, I_GO, 0);
        Atomics.store(flags, I_FAULT, 0);
        if (linkError !== null) {
            // A module that failed to compile/link fails every call.
            fault(linkError);
        } else {
            var cmd = Atomics.load(flags, I_COMMAND);
            try {
                if (cmd === CMD_INIT && init) {
                    init();
                } else if (cmd === CMD_UPDATE && update) {
                    update(nums[F_DT]);
                } else if (cmd === CMD_START && start) {
                    start();
                }
            } catch (err) {
                // A guest trap fails this call only, as on the native backends.
                fault(err);
            }
        }
        Atomics.store(flags, I_DONE, 1);
        Atomics.notify(flags, I_DONE, 1);
    }
};
