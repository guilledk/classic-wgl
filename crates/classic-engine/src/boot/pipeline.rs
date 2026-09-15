//! [`BootPipeline`]: the one boot sequencer every driver runs.
//!
//! A pipeline owns a resolved ROM DAG and its [`BootPlan`] and walks it through
//! [`BootStage::Uploading`] → [`BootStage::UploadingBasis`] →
//! [`BootStage::Finishing`] → [`BootStage::Done`]:
//!
//! - **Uploading** compiles the shader catalog (first poll), then drains the
//!   plan: decode + upload each texture, register metadata, load fonts,
//!   hydrate entities.
//! - **UploadingBasis** transcodes + uploads the `.basis` sheets.
//! - **Finishing** runs the [`BootFinish`] hook (the app's post-load setup:
//!   guests, editor, …) — `classic-engine` cannot depend on the app layer.
//!
//! ROM resolution happens before a pipeline exists (it produces the
//! [`LoadedRoms`] a pipeline is built from).  The CPU-only work — texture
//! decode, `.basis` transcode and [`BootFinish::prepare`] (e.g. guest module
//! compile) — runs inline by default, or ahead of time off the GL thread via
//! [`BootPipeline::prepare`] (native).
//!
//! [`BootPipeline::poll`] drives the stages in one of two modes:
//!
//! - `budget: None` — the synchronous fast path (headless / golden / tests):
//!   everything runs in one call and returns [`BootPoll::Done`].
//! - `budget: Some(_)` — interleaved with rendering: work stops once the
//!   budget is spent (at least one unit of work per call) and the driver polls
//!   again next frame.  [`BootPoll::ReadyToFinish`] is reported once before
//!   the finish hook runs, so a loading screen can be torn down first; on web
//!   the `.basis` stage is awaited through [`BootPipeline::upload_basis_async`]
//!   when a poll reports `AwaitBasis`.

use std::rc::Rc;
use std::time::Duration;

use classic_platform::BootTimer;
use classic_rom::{BootSink, LoadedRoms};

use super::BootPlan;
use crate::Engine;

/// Where a [`BootPipeline`] is (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootStage {
    /// Draining the boot plan (textures, metadata, fonts, entities).
    Uploading,
    /// Transcoding + uploading the `.basis` sheets.
    UploadingBasis,
    /// Running the [`BootFinish`] hook.
    Finishing,
    /// Boot is complete.
    Done,
}

/// The outcome of one [`BootPipeline::poll`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootPoll {
    /// The budget is spent and work remains: poll again (next frame).
    Pending,
    /// The `.basis` stage is next and must be awaited with
    /// [`BootPipeline::upload_basis_async`] (web, budgeted polls only).
    #[cfg(target_arch = "wasm32")]
    AwaitBasis,
    /// Every resource is uploaded; the next poll runs the [`BootFinish`] hook.
    /// Reported once, by budgeted polls only — tear down any loading UI now.
    ReadyToFinish,
    /// Boot is complete.
    Done,
}

/// The app's post-load boot hook.
pub trait BootFinish: Send {
    /// CPU-only preparation that may run off the GL thread, called by
    /// [`BootPipeline::prepare`] (e.g. compile guest modules ahead of time).
    /// Not called on the inline paths, where [`finish`](Self::finish) does
    /// that work itself.
    fn prepare(&mut self, _loaded: &LoadedRoms, _sink: &dyn BootSink) {}

    /// Finish boot on the GL thread once every resource is uploaded.
    fn finish(self: Box<Self>, engine: &mut Engine, loaded: &LoadedRoms, sink: &dyn BootSink);
}

/// A resolved ROM DAG's boot, driven stage by stage (see the module docs).
pub struct BootPipeline {
    loaded: LoadedRoms,
    plan: BootPlan,
    finish: Option<Box<dyn BootFinish>>,
    stage: BootStage,
    shaders_compiled: bool,
    basis_cursor: usize,
    /// `.basis` payloads transcoded by [`BootPipeline::prepare`], indexed like
    /// `plan.basis_jobs` (`None` inside = a sheet that failed to transcode).
    #[cfg(not(target_arch = "wasm32"))]
    basis_decoded: Option<Vec<Option<classic_gfx::DecodedBasis>>>,
    finish_announced: bool,
}

impl BootPipeline {
    /// Plan the boot of `loaded`, finishing with `finish` (if any).
    pub fn new(loaded: LoadedRoms, finish: Option<Box<dyn BootFinish>>) -> Self {
        let plan = Engine::begin_boot(&loaded);
        Self {
            loaded,
            plan,
            finish,
            stage: BootStage::Uploading,
            shaders_compiled: false,
            basis_cursor: 0,
            #[cfg(not(target_arch = "wasm32"))]
            basis_decoded: None,
            finish_announced: false,
        }
    }

    /// The ROM DAG being booted.
    pub fn loaded(&self) -> &LoadedRoms {
        &self.loaded
    }

    /// The current stage.
    pub fn stage(&self) -> BootStage {
        self.stage
    }

    /// The boot plan (e.g. for progress reporting).
    pub fn plan(&self) -> &BootPlan {
        &self.plan
    }

    /// Run the CPU-only boot work now, off the GL thread (native): decode every
    /// texture and transcode every `.basis` sheet for `caps` across the loader
    /// threads, then [`BootFinish::prepare`].  Later polls only upload.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn prepare(&mut self, caps: classic_gfx::Caps, sink: &dyn BootSink) {
        super::decode_plan(&mut self.plan, sink);
        self.basis_decoded = Some(super::decode_basis_jobs(&self.plan.basis_jobs, caps, sink));
        if let Some(finish) = self.finish.as_mut() {
            finish.prepare(&self.loaded, sink);
        }
    }

    /// Advance the boot on the GL thread, within `budget` (`None` = run to
    /// completion).  See the module docs for the two modes.
    pub fn poll(
        &mut self,
        engine: &mut Engine,
        gl: &Rc<glow::Context>,
        sink: &dyn BootSink,
        budget: Option<Duration>,
    ) -> BootPoll {
        self.poll_with(engine, Some(gl), sink, budget)
    }

    /// [`poll`](Self::poll) with an optional GL context: without one, shader
    /// compile is skipped and uploads find no GL layer (the GL-free hydration
    /// used by tests).
    pub(crate) fn poll_with(
        &mut self,
        engine: &mut Engine,
        gl: Option<&Rc<glow::Context>>,
        sink: &dyn BootSink,
        budget: Option<Duration>,
    ) -> BootPoll {
        let timer = BootTimer::start();
        let spent = || budget.is_some_and(|budget| timer.elapsed_duration() >= budget);
        loop {
            match self.stage {
                BootStage::Uploading => {
                    if !self.shaders_compiled {
                        self.shaders_compiled = true;
                        if let (Some(gl), Some(root)) = (gl, self.loaded.root_rom()) {
                            engine.ensure_gfx(gl.clone(), &root.manifest, sink);
                        }
                    }
                    if budget.is_none() {
                        engine.boot_step(&mut self.plan, &self.loaded, sink, usize::MAX);
                    }
                    while !self.plan.is_done() {
                        engine.boot_step(&mut self.plan, &self.loaded, sink, 1);
                        if spent() {
                            break;
                        }
                    }
                    if !self.plan.is_done() {
                        return BootPoll::Pending;
                    }
                    self.stage = BootStage::UploadingBasis;
                    if spent() {
                        return BootPoll::Pending;
                    }
                }
                BootStage::UploadingBasis => {
                    let total = self.plan.basis_jobs.len();
                    #[cfg(target_arch = "wasm32")]
                    if budget.is_some() && self.basis_cursor < total {
                        return BootPoll::AwaitBasis;
                    }
                    while self.basis_cursor < total {
                        let job = &self.plan.basis_jobs[self.basis_cursor];
                        #[cfg(not(target_arch = "wasm32"))]
                        match self.basis_decoded.as_mut() {
                            // Taken, so each payload is freed once uploaded.
                            Some(decoded) => {
                                let payload = decoded[self.basis_cursor].take();
                                engine.upload_basis_decoded(job, payload.as_ref(), sink)
                            }
                            None => engine.upload_basis(job, sink),
                        }
                        #[cfg(target_arch = "wasm32")]
                        engine.upload_basis(job, sink);
                        self.basis_cursor += 1;
                        if spent() {
                            break;
                        }
                    }
                    if self.basis_cursor < total {
                        return BootPoll::Pending;
                    }
                    self.stage = BootStage::Finishing;
                }
                BootStage::Finishing => {
                    if budget.is_some() && !self.finish_announced {
                        self.finish_announced = true;
                        return BootPoll::ReadyToFinish;
                    }
                    if let Some(finish) = self.finish.take() {
                        finish.finish(engine, &self.loaded, sink);
                    }
                    self.stage = BootStage::Done;
                }
                BootStage::Done => return BootPoll::Done,
            }
        }
    }

    /// Transcode + upload the remaining `.basis` sheets through the web
    /// transcoder `Worker` (awaited), after a poll reported
    /// [`BootPoll::AwaitBasis`].  The next poll moves on to finishing.
    #[cfg(target_arch = "wasm32")]
    pub async fn upload_basis_async(&mut self, engine: &mut Engine, sink: &dyn BootSink) {
        if self.stage != BootStage::UploadingBasis {
            return;
        }
        engine.upload_basis_async(&self.plan.basis_jobs[self.basis_cursor..], sink).await;
        self.basis_cursor = self.plan.basis_jobs.len();
        self.stage = BootStage::Finishing;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn loaded() -> LoadedRoms {
        crate::tests::two_rom_dag()
    }

    /// Records the hook calls it receives.
    struct Recorder(Arc<Mutex<Vec<String>>>);

    impl BootFinish for Recorder {
        fn prepare(&mut self, loaded: &LoadedRoms, _: &dyn BootSink) {
            self.0.lock().unwrap().push(format!("prepare {}", loaded.root));
        }
        fn finish(self: Box<Self>, engine: &mut Engine, loaded: &LoadedRoms, _: &dyn BootSink) {
            let names = engine.name_order.len();
            self.0.lock().unwrap().push(format!("finish {} after {names} entities", loaded.root));
        }
    }

    type Log = Arc<Mutex<Vec<String>>>;

    fn recorder() -> (Log, Option<Box<dyn BootFinish>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        (log.clone(), Some(Box::new(Recorder(log))))
    }

    #[test]
    fn synchronous_poll_runs_every_stage_and_the_finish_hook() {
        let (log, finish) = recorder();
        let mut engine = Engine::new_for_test();
        let mut boot = BootPipeline::new(loaded(), finish);
        let sink = classic_rom::NullBootSink;
        assert_eq!(boot.poll_with(&mut engine, None, &sink, None), BootPoll::Done);
        assert_eq!(boot.stage(), BootStage::Done);
        assert!(engine.names.contains_key("scene::rocket"));
        assert_eq!(*log.lock().unwrap(), ["finish scene after 2 entities"]);
        assert_eq!(boot.poll_with(&mut engine, None, &sink, None), BootPoll::Done, "idempotent");
    }

    #[test]
    fn budgeted_polls_match_the_synchronous_boot() {
        let sink = classic_rom::NullBootSink;
        let mut full = Engine::new_for_test();
        BootPipeline::new(loaded(), None).poll_with(&mut full, None, &sink, None);

        // A zero budget does one unit of work per poll.
        let (log, finish) = recorder();
        let mut stepwise = Engine::new_for_test();
        let mut boot = BootPipeline::new(loaded(), finish);
        let total = boot.plan().total_steps();
        let mut polls = Vec::new();
        loop {
            let poll = boot.poll_with(&mut stepwise, None, &sink, Some(Duration::ZERO));
            polls.push(poll);
            if poll == BootPoll::Done {
                break;
            }
        }
        assert_eq!(full.name_order, stepwise.name_order);
        assert_eq!(polls.iter().filter(|p| **p == BootPoll::Pending).count(), total);
        assert_eq!(polls.iter().filter(|p| **p == BootPoll::ReadyToFinish).count(), 1);
        assert_eq!(polls[polls.len() - 2], BootPoll::ReadyToFinish, "announced right before");
        assert_eq!(*log.lock().unwrap(), ["finish scene after 2 entities"]);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn prepare_consumes_the_cpu_work_and_prepares_the_hook() {
        let (log, finish) = recorder();
        let mut boot = BootPipeline::new(loaded(), finish);
        let caps = classic_gfx::Caps { bptc: false, rgtc: false, s3tc: false, etc2: false };
        boot.prepare(caps, &classic_rom::NullBootSink);
        assert!(boot
            .plan()
            .steps
            .iter()
            .all(|s| !matches!(s, super::super::BootStep::Decode { .. })));
        assert_eq!(*log.lock().unwrap(), ["prepare scene"]);

        let mut engine = Engine::new_for_test();
        boot.poll_with(&mut engine, None, &classic_rom::NullBootSink, None);
        assert_eq!(log.lock().unwrap().len(), 2);
    }

    #[test]
    fn the_pipeline_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<BootPipeline>();
    }
}
