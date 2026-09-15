//! Boot drivers: how an app runs a [`BootPipeline`].
//!
//! - [`run_sync`] — boot to completion in one call (headless / golden / test).
//! - [`InterleavedBoot`] — boot on the GL thread a time-budgeted slice per
//!   frame while the loading screen renders.  The web app resolves the ROMs
//!   itself and hands the driver a pipeline ([`InterleavedBoot::start`]); the
//!   windowed desktop uses [`InterleavedBoot::threaded`], which resolves and
//!   [prepares](BootPipeline::prepare) the pipeline on a background boot thread
//!   and forwards its events to the GL thread.
//!
//! The apps keep their own run loops: a driver is polled once per frame and
//! reports when the engine is booted.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use classic_platform::InputState;
use classic_rom::{BootSink, LoadedRoms};

use super::{BootFinish, BootPipeline, BootPoll};
use crate::boot_loader::VisualBootSink;
use crate::Engine;

/// How long a frame may spend booting before it yields to rendering, so the
/// loading screen keeps animating (no single frame blocks past ~16 ms).
pub const FRAME_BUDGET: Duration = Duration::from_millis(12);

/// Boot `loaded` into a fresh engine in one call: the synchronous fast path.
pub fn run_sync(
    gl: Rc<glow::Context>,
    loaded: LoadedRoms,
    finish: Box<dyn BootFinish>,
    sink: &dyn BootSink,
) -> Engine {
    let mut engine = Engine::new();
    BootPipeline::new(loaded, Some(finish)).poll(&mut engine, &gl, sink, None);
    engine
}

/// The outcome of one [`InterleavedBoot::poll`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BootFrame {
    /// Still booting: draw the loader ([`InterleavedBoot::draw_loader`]) and
    /// poll again next frame.
    Loading,
    /// The `.basis` stage is next: await [`InterleavedBoot::upload_basis`].
    #[cfg(target_arch = "wasm32")]
    AwaitBasis,
    /// The engine is booted and the loader is gone.  Reported once.
    Booted,
    /// The boot thread failed (its `BootFailed` event was already forwarded).
    Failed(String),
}

/// The per-frame boot driver (see the module docs).
pub struct InterleavedBoot {
    loader: Option<Arc<VisualBootSink>>,
    pipeline: Option<BootPipeline>,
    #[cfg(not(target_arch = "wasm32"))]
    thread: Option<thread::BootThread>,
    booted: bool,
}

impl InterleavedBoot {
    /// A driver waiting for [`start`](Self::start), drawing `loader` (if any)
    /// meanwhile.
    pub fn new(loader: Option<Arc<VisualBootSink>>) -> Self {
        Self {
            loader,
            pipeline: None,
            #[cfg(not(target_arch = "wasm32"))]
            thread: None,
            booted: false,
        }
    }

    /// A driver whose pipeline comes from a background boot thread: once the
    /// first [`poll`](Self::poll) hands it the GL compressed-format caps, the
    /// thread runs `resolve`, builds the pipeline with `finish` and prepares
    /// it.  Fails only if the OS refuses to create the thread.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn threaded(
        loader: Option<Arc<VisualBootSink>>,
        resolve: impl FnOnce(&dyn BootSink) -> anyhow::Result<LoadedRoms> + Send + 'static,
        finish: Box<dyn BootFinish>,
    ) -> std::io::Result<Self> {
        let mut boot = Self::new(loader);
        boot.thread = Some(thread::BootThread::spawn(resolve, finish)?);
        Ok(boot)
    }

    /// A fresh engine with the GL layer, the embedded font and the loading
    /// screen installed, so it draws from frame 0 — before any ROM resolves.
    pub fn engine(&self, gl: Rc<glow::Context>) -> Engine {
        let mut engine = Engine::new();
        engine.init_gfx(gl);
        if let Some(loader) = &self.loader {
            loader.install(&mut engine);
        }
        engine
    }

    /// Boot `pipeline` from the next poll on.
    pub fn start(&mut self, pipeline: BootPipeline) {
        if let Some(loader) = &self.loader {
            loader.set_dag(pipeline.loaded());
        }
        self.pipeline = Some(pipeline);
    }

    /// Advance the boot by one frame's budget.
    pub fn poll(
        &mut self,
        engine: &mut Engine,
        gl: &Rc<glow::Context>,
        sink: &dyn BootSink,
    ) -> BootFrame {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(frame) = self.poll_thread(|| classic_gfx::Caps::query(gl), sink) {
            return frame;
        }
        self.poll_pipeline(engine, Some(gl), sink)
    }

    /// Forward the boot thread's events and pick up its pipeline.  `Some` is a
    /// frame to report instead of polling the pipeline.
    #[cfg(not(target_arch = "wasm32"))]
    fn poll_thread(
        &mut self,
        caps: impl FnOnce() -> classic_gfx::Caps,
        sink: &dyn BootSink,
    ) -> Option<BootFrame> {
        let thread = self.thread.as_mut()?;
        match thread.poll(caps, sink) {
            thread::Poll::Waiting => Some(BootFrame::Loading),
            thread::Poll::Ready(pipeline) => {
                self.thread = None;
                self.start(pipeline);
                None
            }
            thread::Poll::Failed(err) => {
                self.thread = None;
                Some(BootFrame::Failed(err))
            }
        }
    }

    fn poll_pipeline(
        &mut self,
        engine: &mut Engine,
        gl: Option<&Rc<glow::Context>>,
        sink: &dyn BootSink,
    ) -> BootFrame {
        let Some(pipeline) = self.pipeline.as_mut() else {
            return BootFrame::Loading;
        };
        loop {
            match pipeline.poll_with(engine, gl, sink, Some(FRAME_BUDGET)) {
                BootPoll::Pending => return BootFrame::Loading,
                #[cfg(target_arch = "wasm32")]
                BootPoll::AwaitBasis => return BootFrame::AwaitBasis,
                BootPoll::ReadyToFinish => {
                    if let Some(loader) = &self.loader {
                        loader.uninstall(engine);
                    }
                }
                BootPoll::Done => {
                    // Release the pipeline (its ROM bytes and any payloads).
                    self.pipeline = None;
                    self.booted = true;
                    return BootFrame::Booted;
                }
            }
        }
    }

    /// Transcode + upload the `.basis` sheets through the web transcoder
    /// `Worker` after a poll reported [`BootFrame::AwaitBasis`].
    #[cfg(target_arch = "wasm32")]
    pub async fn upload_basis(&mut self, engine: &mut Engine, sink: &dyn BootSink) {
        if let Some(pipeline) = self.pipeline.as_mut() {
            pipeline.upload_basis_async(engine, sink).await;
        }
    }

    /// Render the loading screen (a no-op without a loader or once booted).
    pub fn draw_loader(
        &self,
        engine: &mut Engine,
        input: &mut InputState,
        vw: f32,
        vh: f32,
        delta: f32,
    ) {
        if self.booted {
            return;
        }
        if let Some(loader) = &self.loader {
            loader.sync(engine, vw, vh);
            engine.frame(input, vw, vh, delta);
        }
    }
}

/// The background boot thread behind [`InterleavedBoot::threaded`].
#[cfg(not(target_arch = "wasm32"))]
mod thread {
    use std::sync::mpsc;

    use classic_rom::{BootEvent, BootSink, LoadedRoms};

    use super::super::{BootFinish, BootPipeline};

    enum Msg {
        Event(BootEvent),
        Ready(Box<BootPipeline>),
        Failed(String),
    }

    /// Forwards the boot thread's events to the GL thread.
    struct ChannelSink(mpsc::Sender<Msg>);

    impl BootSink for ChannelSink {
        fn on_event(&self, event: BootEvent) {
            let _ = self.0.send(Msg::Event(event));
        }
    }

    pub(super) enum Poll {
        Waiting,
        Ready(BootPipeline),
        Failed(String),
    }

    pub(super) struct BootThread {
        caps: Option<mpsc::Sender<classic_gfx::Caps>>,
        rx: mpsc::Receiver<Msg>,
    }

    impl BootThread {
        pub(super) fn spawn(
            resolve: impl FnOnce(&dyn BootSink) -> anyhow::Result<LoadedRoms> + Send + 'static,
            finish: Box<dyn BootFinish>,
        ) -> std::io::Result<Self> {
            // Basis transcode needs the GL compressed-format caps, which only
            // exist once the GL context does: the thread waits for them.
            let (caps_tx, caps_rx) = mpsc::channel::<classic_gfx::Caps>();
            let (tx, rx) = mpsc::channel();
            classic_worker::spawn_thread("classic-boot", move || {
                let Ok(caps) = caps_rx.recv() else { return };
                let sink = ChannelSink(tx.clone());
                match resolve(&sink) {
                    Ok(loaded) => {
                        let mut pipeline = BootPipeline::new(loaded, Some(finish));
                        pipeline.prepare(caps, &sink);
                        let _ = tx.send(Msg::Ready(Box::new(pipeline)));
                    }
                    Err(err) => {
                        let error = format!("{err:#}");
                        sink.on_event(BootEvent::BootFailed {
                            phase: "resolve",
                            error: error.clone(),
                        });
                        let _ = tx.send(Msg::Failed(error));
                    }
                }
            })?;
            Ok(Self { caps: Some(caps_tx), rx })
        }

        /// Hand over the caps (first call), forward pending events to `sink`,
        /// and report the pipeline once it is prepared.
        pub(super) fn poll(
            &mut self,
            caps: impl FnOnce() -> classic_gfx::Caps,
            sink: &dyn BootSink,
        ) -> Poll {
            if let Some(tx) = self.caps.take() {
                let _ = tx.send(caps());
            }
            loop {
                match self.rx.try_recv() {
                    Ok(Msg::Event(event)) => sink.on_event(event),
                    Ok(Msg::Ready(pipeline)) => return Poll::Ready(*pipeline),
                    Ok(Msg::Failed(err)) => return Poll::Failed(err),
                    Err(mpsc::TryRecvError::Empty) => return Poll::Waiting,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        return Poll::Failed("boot thread exited without a pipeline".into())
                    }
                }
            }
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use classic_rom::BootEvent;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<BootEvent>>);

    impl BootSink for Recorder {
        fn on_event(&self, event: BootEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    struct Finish(Arc<Mutex<Vec<&'static str>>>);

    impl BootFinish for Finish {
        fn prepare(&mut self, _: &LoadedRoms, _: &dyn BootSink) {
            self.0.lock().unwrap().push("prepare");
        }
        fn finish(self: Box<Self>, _: &mut Engine, _: &LoadedRoms, _: &dyn BootSink) {
            self.0.lock().unwrap().push("finish");
        }
    }

    const CAPS: classic_gfx::Caps =
        classic_gfx::Caps { bptc: false, rgtc: false, s3tc: false, etc2: false };

    /// Poll a threaded driver GL-free until it stops loading.
    fn drive(boot: &mut InterleavedBoot, engine: &mut Engine, sink: &Recorder) -> BootFrame {
        for _ in 0..2000 {
            let frame = match boot.poll_thread(|| CAPS, sink) {
                Some(frame) => frame,
                None => boot.poll_pipeline(engine, None, sink),
            };
            if frame != BootFrame::Loading {
                return frame;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("boot never finished");
    }

    #[test]
    fn threaded_boot_prepares_off_thread_and_finishes_on_the_gl_thread() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let finish = Box::new(Finish(calls.clone()));
        let mut boot = InterleavedBoot::threaded(
            None,
            |_: &dyn BootSink| Ok(crate::tests::two_rom_dag()),
            finish,
        )
        .unwrap();
        let sink = Recorder::default();
        let mut engine = Engine::new_for_test();

        assert_eq!(drive(&mut boot, &mut engine, &sink), BootFrame::Booted);
        assert_eq!(*calls.lock().unwrap(), ["prepare", "finish"]);
        assert!(engine.names.contains_key("scene::rocket"));
        let spawned = sink
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, BootEvent::StateSpawned { .. }))
            .count();
        assert_eq!(spawned, 2, "GL-thread events reach the app sink");
    }

    #[test]
    fn threaded_boot_reports_a_resolve_failure() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut boot = InterleavedBoot::threaded(
            None,
            |_: &dyn BootSink| Err(anyhow::anyhow!("no such rom")),
            Box::new(Finish(calls.clone())),
        )
        .unwrap();
        let sink = Recorder::default();
        let mut engine = Engine::new_for_test();

        assert_eq!(drive(&mut boot, &mut engine, &sink), BootFrame::Failed("no such rom".into()));
        assert!(calls.lock().unwrap().is_empty());
        assert!(sink
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, BootEvent::BootFailed { phase: "resolve", .. })));
    }
}
