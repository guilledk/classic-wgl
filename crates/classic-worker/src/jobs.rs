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
//! - [`JobQueue::pooled`] (native only, stateless jobs): the threaded mode
//!   over several named threads sharing one command queue — the fan-out used
//!   for boot-time decode.  A panicking job loses only its own result; the
//!   worker carries on.
//! - [`JobQueue::synchronous`]: jobs run inline at `submit` and their result is
//!   immediately available to `poll` — the deterministic test/golden harness
//!   mode, and the only mode on web (where true background work uses a `Worker`
//!   instead).
//!
//! Callers submit and poll the same way in every mode, so whether work runs
//! inline is decided once, when the queue is built.  [`JobQueue::run_all`]
//! runs a batch and returns its outputs in submission order in every mode.

use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{mpsc, Arc, Barrier, Mutex};

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
    /// Wait on the barrier (shared by one flush per worker, so every worker is
    /// parked on it at once), then ack.
    Flush(mpsc::Sender<()>, Arc<Barrier>),
    Shutdown,
}

#[cfg(not(target_arch = "wasm32"))]
type Commands<J> = Arc<Mutex<mpsc::Receiver<Command<J>>>>;

enum Mode<J: Job> {
    #[cfg(not(target_arch = "wasm32"))]
    Threaded {
        tx: mpsc::Sender<Command<J>>,
        rx: mpsc::Receiver<(JobId, J::Output)>,
        workers: usize,
    },
    Synchronous(J::State),
}

/// Spawn one worker thread pulling commands off the shared queue.  With
/// `catch_panics` a panicking job loses only its own result (pool workers own
/// no meaningful state); otherwise it ends the thread, as the state it owns
/// may be left inconsistent.
#[cfg(not(target_arch = "wasm32"))]
fn spawn_worker<J: Job>(
    name: String,
    commands: Commands<J>,
    results: mpsc::Sender<(JobId, J::Output)>,
    mut state: J::State,
    catch_panics: bool,
) -> std::io::Result<()>
where
    J::State: Send,
{
    use std::panic::{catch_unwind, AssertUnwindSafe};

    crate::spawn_thread(name, move || loop {
        // The lock is released at the end of this statement, before the
        // command runs.
        let command = commands.lock().expect("job queue mutex poisoned").recv();
        match command {
            Ok(Command::Run(id, job)) => {
                let output = if catch_panics {
                    match catch_unwind(AssertUnwindSafe(|| job.run(&mut state))) {
                        Ok(output) => output,
                        Err(_) => continue,
                    }
                } else {
                    job.run(&mut state)
                };
                let _ = results.send((id, output));
            }
            Ok(Command::Update(update)) => update(&mut state),
            Ok(Command::Flush(ack, barrier)) => {
                barrier.wait();
                let _ = ack.send(());
            }
            Ok(Command::Shutdown) | Err(_) => break,
        }
    })?;
    Ok(())
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
        let (tx, commands) = mpsc::channel::<Command<J>>();
        let (results, rx) = mpsc::channel::<(JobId, J::Output)>();
        spawn_worker(name.to_owned(), Arc::new(Mutex::new(commands)), results, state, false)?;
        Ok(Self { mode: Mode::Threaded { tx, rx, workers: 1 }, results: HashMap::new() })
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
        if let Mode::Threaded { tx, workers, .. } = &self.mode {
            // One flush per worker, all parked on one barrier: a worker holding
            // a flush cannot take another, so every worker drains the commands
            // queued before the flushes and then meets at the barrier.
            let barrier = Arc::new(Barrier::new(*workers));
            let (ack_tx, ack_rx) = mpsc::channel();
            for _ in 0..*workers {
                let _ = tx.send(Command::Flush(ack_tx.clone(), Arc::clone(&barrier)));
            }
            drop(ack_tx);
            for _ in 0..*workers {
                if ack_rx.recv().is_err() {
                    break;
                }
            }
        }
    }

    /// Run `jobs` and return their outputs in submission order, regardless of
    /// which worker finishes first (the ordered fan-out).  Submits under ids
    /// `0..n`, so use it on a queue with no other outstanding results.
    ///
    /// Panics if a job panicked (its result is missing).
    pub fn run_all(&mut self, jobs: impl IntoIterator<Item = J>) -> Vec<J::Output> {
        let mut count: JobId = 0;
        for job in jobs {
            self.submit(count, job);
            count += 1;
        }
        self.join();
        (0..count).map(|id| self.poll(id).unwrap_or_else(|| panic!("job {id} panicked"))).collect()
    }
}

impl<J: Job<State = ()>> JobQueue<J> {
    /// A queue running stateless jobs on `threads` (at least one) named threads
    /// `{name}-{i}` that share one command queue.  Fails only if the OS refuses
    /// to create a thread.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn pooled(name: &str, threads: usize) -> std::io::Result<Self> {
        let workers = threads.max(1);
        let (tx, commands) = mpsc::channel::<Command<J>>();
        let (results, rx) = mpsc::channel::<(JobId, J::Output)>();
        let commands = Arc::new(Mutex::new(commands));
        for i in 0..workers {
            // On failure the already-spawned workers exit once `tx` drops.
            spawn_worker(format!("{name}-{i}"), Arc::clone(&commands), results.clone(), (), true)?;
        }
        Ok(Self { mode: Mode::Threaded { tx, rx, workers }, results: HashMap::new() })
    }
}

impl<J: Job> Drop for JobQueue<J> {
    fn drop(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Mode::Threaded { tx, workers, .. } = &self.mode {
            for _ in 0..*workers {
                let _ = tx.send(Command::Shutdown);
            }
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

    /// A stateless job: squares its input, after a delay that makes earlier
    /// jobs finish later; panics on `u64::MAX`.
    struct Square(u64);

    impl Job for Square {
        type State = ();
        type Output = u64;
        fn run(self, _: &mut ()) -> u64 {
            assert_ne!(self.0, u64::MAX, "poisoned input");
            #[cfg(not(target_arch = "wasm32"))]
            std::thread::sleep(std::time::Duration::from_millis(16u64.saturating_sub(self.0)));
            self.0 * self.0
        }
    }

    #[test]
    fn run_all_returns_outputs_in_submission_order() {
        let squares: Vec<u64> = (0..16).map(|n| n * n).collect();
        assert_eq!(JobQueue::synchronous(()).run_all((0..16).map(Square)), squares);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut pool = JobQueue::pooled("classic-jobs-pool-test", 4).unwrap();
            assert!(!pool.is_synchronous());
            assert_eq!(pool.run_all((0..16).map(Square)), squares);
            assert_eq!(pool.run_all((0..16).map(Square)), squares, "the pool is reusable");
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pooled_join_survives_a_panicking_job() {
        let mut pool = JobQueue::pooled("classic-jobs-panic-test", 3).unwrap();
        pool.submit(0, Square(u64::MAX));
        for id in 1..8 {
            pool.submit(id, Square(id));
        }
        pool.join();
        assert_eq!(pool.poll(0), None, "the panicking job has no result");
        for id in 1..8 {
            assert_eq!(pool.poll(id), Some(id * id));
        }
        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pool.run_all([Square(2), Square(u64::MAX)])
        }));
        assert!(run.is_err(), "run_all panics on a missing result");
    }
}
