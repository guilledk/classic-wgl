//! Shared spawn helpers for background execution: named native threads and
//! web `Worker`s built from inline JS source.

/// Spawn a named native thread (the name shows up in debuggers, profilers and
/// panic messages).  Fails only if the OS refuses to create the thread.
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn_thread<F, T>(
    name: impl Into<String>,
    f: F,
) -> std::io::Result<std::thread::JoinHandle<T>>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    std::thread::Builder::new().name(name.into()).spawn(f)
}

/// Spawn a web `Worker` running `js_source` (built from an inline `Blob`, so no
/// separate script needs to be served).
///
/// When `on_message` is given it is installed as the worker's `onmessage`
/// handler and called with each message's `data`.  The handler lives as long
/// as the page (its `Closure` is intentionally leaked, like the worker itself).
#[cfg(target_arch = "wasm32")]
pub fn spawn_web_worker(
    js_source: &str,
    on_message: Option<Box<dyn FnMut(wasm_bindgen::JsValue)>>,
) -> Result<web_sys::Worker, wasm_bindgen::JsValue> {
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::{JsCast, JsValue};

    let blob_parts = js_sys::Array::of1(&JsValue::from_str(js_source));
    let blob = web_sys::Blob::new_with_str_sequence(blob_parts.as_ref())?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)?;
    let worker = web_sys::Worker::new(&url)?;

    if let Some(mut on_message) = on_message {
        // Uses `JsValue` for the event so no `MessageEvent` web-sys feature is
        // required.
        let onmessage = Closure::wrap(Box::new(move |event: JsValue| {
            let data =
                js_sys::Reflect::get(&event, &JsValue::from_str("data")).unwrap_or(JsValue::NULL);
            on_message(data);
        }) as Box<dyn FnMut(JsValue)>);
        worker.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        onmessage.forget();
    }

    Ok(worker)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn spawn_thread_names_the_thread_and_returns_its_value() {
        let handle =
            spawn_thread("classic-spawn-test", || std::thread::current().name().map(str::to_owned))
                .unwrap();
        assert_eq!(handle.join().unwrap().as_deref(), Some("classic-spawn-test"));
    }
}
