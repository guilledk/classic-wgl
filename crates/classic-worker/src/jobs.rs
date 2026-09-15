//! A generic background job queue: the one place background work is run and
//! the one determinism switch.
//!
//! A [`JobQueue<J>`] owns a worker-side state (`J::State`, e.g. a pathfinder or
//! a wasm runtime) and runs submitted jobs against it, in FIFO order with any
//! state updates.  It is built in one of two modes:
//!
//! - [`JobQueue::threaded`] (native only): the state moves onto a dedicated,
//!   named thread; `submit` never blocks and results are collected by `poll`.
//!   `join` is a FIFO barrier that returns once every earlier job and update
//!   has been processed.
//! - [`JobQueue::synchronous`]: jobs run inline at `submit` and their result is
//!   immediately available to `poll` — the deterministic test/golden harness
//!   mode, and the only mode on web (where true background work uses a `Worker`
//!   instead).
//!
//! Callers submit and poll the same way in both modes, so whether work runs
//! inline is decided once, when the queue is built.

use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc;

/// Caller-chosen id correlating a submitted job with its result.
pub type JobId = u64;

/// A unit of background work run against the queue's worker-owned state.
pub trait Job: Send + 'static {
    /// The worker-owned state jobs run against.
    type State: 'static;
    /// The job's result.
    type Output: Send + 'static;

    /// Run the job.
    fn run(self, state: &mut Self::State) -> Self::Output;
}

#[cfg(not(target_arch = "wasm32"))]
type Update<S> = Box<dyn FnOnce(&mut S) + Send>;

#[cfg(not(target_arch = "wasm32"))]
enum Command<J: Job> {
    Run(JobId, J),
    Update(Update<J::State>),
    Flush(mpsc::Sender<()>),
    Shutdown,
}

enum Mode<J: Job> {
    #[cfg(not(target_arch = "wasm32"))]
    Threaded {
        tx: mpsc::Sender<Command<J>>,
        rx: mpsc::Receiver<(JobId, J::Output)>,
    },
    Synchronous(J::State),
}

/// A FIFO queue running [`Job`]s against a worker-owned state, on a background
/// thread or inline (see the module docs).
pub struct JobQueue<J: Job> {
    mode: Mode<J>,
    results: HashMap<JobId, J::Output>,
}

impl<J: Job> JobQueue<J> {
    /// A queue that runs every job inline at [`submit`](Self::submit).
    pub fn synchronous(state: J::State) -> Self {
        Self { mode: Mode::Synchronous(state), results: HashMap::new() }
    }

    /// A queue that moves `state` onto a dedicated thread named `name` and runs
    /// jobs there.  Fails only if the OS refuses to create the thread.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn threaded(name: &str, state: J::State) -> std::io::Result<Self>
    where
        J::State: Send,
    {
        let (tx, worker_rx) = mpsc::channel::<Command<J>>();
        let (worker_tx, rx) = mpsc::channel::<(JobId, J::Output)>();
        crate::spawn_thread(name, move || {
            let mut state = state;
            while let Ok(command) = worker_rx.recv() {
                match command {
                    Command::Run(id, job) => {
                        let _ = worker_tx.send((id, job.run(&mut state)));
                    }
                    Command::Update(update) => update(&mut state),
                    Command::Flush(ack) => {
                        let _ = ack.send(());
                    }
                    Command::Shutdown => break,
                }
            }
        })?;
        Ok(Self { mode: Mode::Threaded { tx, rx }, results: HashMap::new() })
    }

    /// Whether jobs run inline (the deterministic mode).
    pub fn is_synchronous(&self) -> bool {
        matches!(self.mode, Mode::Synchronous(_))
    }

    /// Submit a job under `id`.  Non-blocking when threaded; runs the job now
    /// when synchronous.
    pub fn submit(&mut self, id: JobId, job: J) {
        match &mut self.mode {
            #[cfg(not(target_arch = "wasm32"))]
            Mode::Threaded { tx, .. } => {
                let _ = tx.send(Command::Run(id, job));
            }
            Mode::Synchronous(state) => {
                let output = job.run(state);
                self.results.insert(id, output);
            }
        }
    }

    /// Mutate the worker-owned state, in FIFO order with submitted jobs (jobs
    /// submitted earlier see the old state, later ones the new).
    pub fn update(&mut self, update: impl FnOnce(&mut J::State) + Send + 'static) {
        match &mut self.mode {
            #[cfg(not(target_arch = "wasm32"))]
            Mode::Threaded { tx, .. } => {
                let _ = tx.send(Command::Update(Box::new(update)));
            }
            Mode::Synchronous(state) => update(state),
        }
    }

    /// Take a finished job's result.  `None` while it is still running (or if
    /// the id was never submitted or was already taken).
    pub fn poll(&mut self, id: JobId) -> Option<J::Output> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Mode::Threaded { rx, .. } = &self.mode {
            while let Ok((done, output)) = rx.try_recv() {
                self.results.insert(done, output);
            }
        }
        self.results.remove(&id)
    }

    /// Block until every previously submitted job and update has been processed
    /// (the frame-boundary determinism barrier).  A no-op when synchronous.
    pub fn join(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Mode::Threaded { tx, .. } = &self.mode {
            let (ack_tx, ack_rx) = mpsc::channel();
            let _ = tx.send(Command::Flush(ack_tx));
            let _ = ack_rx.recv();
        }
    }
}

impl<J: Job> Drop for JobQueue<J> {
    fn drop(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Mode::Threaded { tx, .. } = &self.mode {
            let _ = tx.send(Command::Shutdown);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Appends to a log and returns the log length at run time.
    struct Push(u32);

    impl Job for Push {
        type State = Vec<u32>;
        type Output = usize;
        fn run(self, state: &mut Vec<u32>) -> usize {
            state.push(self.0);
            state.len()
        }
    }

    fn exercise(mut queue: JobQueue<Push>) {
        queue.submit(1, Push(10));
        queue.update(|log| log.push(99));
        queue.submit(2, Push(20));
        queue.join();
        assert_eq!(queue.poll(1), Some(1));
        assert_eq!(queue.poll(2), Some(3), "the update ran between the two jobs");
        assert_eq!(queue.poll(2), None, "results are taken once");
    }

    #[test]
    fn synchronous_runs_inline_in_fifo_order() {
        let mut queue = JobQueue::synchronous(Vec::new());
        assert!(queue.is_synchronous());
        queue.submit(7, Push(1));
        assert_eq!(queue.poll(7), Some(1), "available immediately, no join needed");
        exercise(JobQueue::synchronous(Vec::new()));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn threaded_matches_synchronous() {
        let queue = JobQueue::threaded("classic-jobs-test", Vec::new()).unwrap();
        assert!(!queue.is_synchronous());
        exercise(queue);
    }
}
