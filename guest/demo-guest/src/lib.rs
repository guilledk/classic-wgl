#![no_std]

//! ROM guest for the `demo` scene, compiled to `.wasm` and run by the host
//! against the `classic-guest` SDK.  Host imports are declared under the
//! `env` module; the `update` export is invoked once per frame.

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

/// Called once per frame with the frame delta in seconds.
#[no_mangle]
pub extern "C" fn update(_dt: f64) {}
