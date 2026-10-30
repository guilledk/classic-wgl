//! Graph core: a sample ring plus a min/max pyramid for large `(x, y)` series.
//!
//! This is the pure-data half of the graph UI element (no GL).  A
//! [`SeriesBuf`] holds up to `cap` (a power of two) `f32` samples, addressed
//! by an **absolute index** `abs` (`0` for the first sample ever pushed,
//! counting up forever).  `x` must be non-decreasing; everything below relies
//! on it (binary searches, bucket `x` spans).
//!
//! # Layout
//!
//! - **Level 0** is the raw ring: `cap` slots of `[x, y]`, sample `abs` in
//!   slot `abs & (cap - 1)`.  Once full, a `push` overwrites (evicts) the
//!   oldest sample.
//! - **Level `k`** (`1..=log2(cap)`) holds [`Bucket`]s of `2^k` aligned
//!   samples: bucket `a` covers `abs` in `[a << k, (a + 1) << k)` and lives in
//!   slot `a & (slots(k) - 1)` with `slots(k) = (2 * cap) >> k`.  The factor of
//!   two is deliberate: an unaligned live window of `cap` samples touches up
//!   to `(cap >> k) + 1` buckets (a partly evicted one at the old end and a
//!   partly filled one at the new end), so `cap >> k` slots would make them
//!   collide and drop the old end.  With `2x` slots every live bucket has its
//!   own slot (the whole pyramid is `~2 * cap` buckets).
//! - A bucket is the min/max over its **surviving** samples only: a `push`
//!   rebuilds, at each level, the bucket of the new sample and the bucket of
//!   the evicted one, each from its two children at the level below
//!   (children outside the live window count as empty).  So `push` is
//!   `O(log cap)` and an evicted extreme never lingers in a coarser level.
//!
//! # Rendering contract
//!
//! [`SeriesBuf::select`] picks the level whose bucket count over a view is
//! about the plot width in pixels and returns at most two contiguous slot
//! ranges (the ring may wrap); a renderer draws level 0 as a polyline and
//! levels `>= 1` as a min/max envelope.  [`SeriesBuf::take_dirty`] reports
//! which slots changed per level since the last call, so a GPU mirror can
//! re-upload only those ([`SeriesBuf::level_bytes`] is the upload source).

/// A min/max summary of `2^k` consecutive samples: `x0..=x1` is the span of
/// their `x`, `y_min..=y_max` the envelope of their `y`.  `#[repr(C)]` and
/// `Pod` so a level uploads verbatim (16 bytes per slot).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Bucket {
    pub x0: f32,
    pub x1: f32,
    pub y_min: f32,
    pub y_max: f32,
}

impl Bucket {
    /// The bucket of no samples (the identity of [`Bucket::merge`]).
    pub const EMPTY: Bucket = Bucket {
        x0: f32::INFINITY,
        x1: f32::NEG_INFINITY,
        y_min: f32::INFINITY,
        y_max: f32::NEG_INFINITY,
    };

    /// The bucket of a single sample.
    pub fn point(x: f32, y: f32) -> Self {
        Self { x0: x, x1: x, y_min: y, y_max: y }
    }

    /// The bucket covering both `self` and `other`.
    pub fn merge(self, other: Self) -> Self {
        Self {
            x0: self.x0.min(other.x0),
            x1: self.x1.max(other.x1),
            y_min: self.y_min.min(other.y_min),
            y_max: self.y_max.max(other.y_max),
        }
    }

    /// `true` for [`Bucket::EMPTY`] (no samples merged in).
    pub fn is_empty(&self) -> bool {
        self.x0 > self.x1
    }
}

/// How a [`Selection`] should be drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphMode {
    /// Level 0: connect the raw samples.
    Polyline,
    /// Level `>= 1`: fill each bucket's `y_min..y_max` envelope.
    Envelope,
}

/// What to draw for a view: `count` consecutive entries of `level`, starting
/// at absolute index `start` (a sample index on level 0, a bucket index
/// above), stored in the slot `ranges` (`(first_slot, count)`, the second
/// one non-empty only when the ring wraps).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub level: u32,
    pub mode: GraphMode,
    pub start: u64,
    pub count: usize,
    pub ranges: [(usize, usize); 2],
}

impl Selection {
    fn empty() -> Self {
        Self { level: 0, mode: GraphMode::Polyline, start: 0, count: 0, ranges: [(0, 0); 2] }
    }

    /// `true` when nothing is in view.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The non-empty slot ranges, oldest first.
    pub fn slot_ranges(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.ranges.iter().copied().filter(|r| r.1 > 0)
    }
}

/// The slots of one level that changed since the last
/// [`SeriesBuf::take_dirty`], as `(first_slot, count)` ranges.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirtyLevel {
    pub level: u32,
    pub ranges: Vec<(usize, usize)>,
}

/// Dirty absolute-index hulls of one level: the growing head end and the
/// evicting tail end are tracked apart so a steady stream stays two short
/// spans instead of one hull over the whole window.
#[derive(Clone, Copy, Debug, Default)]
struct LevelDirtySpans {
    full: bool,
    head: Option<(u64, u64)>,
    tail: Option<(u64, u64)>,
}

fn widen(span: &mut Option<(u64, u64)>, abs: u64) {
    *span = Some(match *span {
        None => (abs, abs + 1),
        Some((lo, hi)) => (lo.min(abs), hi.max(abs + 1)),
    });
}

/// A power-of-two ring of `(x, y)` samples with a min/max pyramid; see the
/// module docs for the layout.
#[derive(Clone, Debug)]
pub struct SeriesBuf {
    log2: u32,
    /// Absolute index of the next sample (= samples ever pushed).
    head: u64,
    len: usize,
    points: Vec<[f32; 2]>,
    /// `levels[k - 1]` is level `k`.
    levels: Vec<Vec<Bucket>>,
    /// `dirty[k]` is level `k` (0 = the raw ring).
    dirty: Vec<LevelDirtySpans>,
}

impl SeriesBuf {
    /// An empty streaming buffer holding the newest `capacity` samples
    /// (rounded up to a power of two, at least 2).
    pub fn with_capacity(capacity: usize) -> Self {
        let log2 = capacity.max(2).next_power_of_two().trailing_zeros();
        let cap = 1usize << log2;
        Self {
            log2,
            head: 0,
            len: 0,
            points: vec![[0.0; 2]; cap],
            levels: (1..=log2).map(|k| vec![Bucket::EMPTY; (2 * cap) >> k]).collect(),
            dirty: vec![LevelDirtySpans::default(); log2 as usize + 1],
        }
    }

    /// A static buffer of exactly these samples (capacity = the next power of
    /// two `>= x.len()`, so nothing is evicted).  `x` must be non-decreasing.
    /// Every level starts dirty.
    pub fn from_points(x: &[f32], y: &[f32]) -> Self {
        assert_eq!(x.len(), y.len(), "SeriesBuf::from_points: x/y length mismatch");
        debug_assert!(x.windows(2).all(|w| w[0] <= w[1]), "x must be non-decreasing");
        let mut buf = Self::with_capacity(x.len());
        for (slot, (&x, &y)) in buf.points.iter_mut().zip(x.iter().zip(y)) {
            *slot = [x, y];
        }
        buf.head = x.len() as u64;
        buf.len = x.len();
        for k in 1..=buf.log2 {
            let (start, count) = buf.live(k);
            for a in start..start + count as u64 {
                buf.rebuild(k, a);
            }
        }
        for d in &mut buf.dirty {
            d.full = true;
        }
        buf
    }

    /// Append a sample, evicting the oldest one when full.  `x` must be `>=`
    /// the previous sample's `x`.  `O(log cap)`.
    pub fn push(&mut self, x: f32, y: f32) {
        debug_assert!(
            self.len == 0 || self.sample(self.len - 1)[0] <= x,
            "SeriesBuf::push: x must be non-decreasing"
        );
        let cap = self.capacity() as u64;
        let h = self.head;
        let evict = self.len as u64 == cap;
        self.points[(h & (cap - 1)) as usize] = [x, y];
        self.head += 1;
        if !evict {
            self.len += 1;
        }
        // Level 0: the evicted sample shared the new sample's slot.
        widen(&mut self.dirty[0].head, h);
        for k in 1..=self.log2 {
            self.rebuild(k, h >> k);
            widen(&mut self.dirty[k as usize].head, h >> k);
            if evict {
                let e = (h - cap) >> k;
                self.rebuild(k, e);
                widen(&mut self.dirty[k as usize].tail, e);
            }
        }
    }

    /// Ring capacity (a power of two).
    pub fn capacity(&self) -> usize {
        1 << self.log2
    }

    /// Surviving samples.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Samples ever pushed = the absolute index of the next one.
    pub fn head(&self) -> u64 {
        self.head
    }

    /// Absolute index of the oldest surviving sample.
    pub fn first_abs(&self) -> u64 {
        self.head - self.len as u64
    }

    /// The coarsest pyramid level, `log2(capacity)`; levels are `0..=max_level`.
    pub fn max_level(&self) -> u32 {
        self.log2
    }

    /// Slot count of `level`: `capacity` for level 0, `(2 * capacity) >> k`
    /// above.
    pub fn slots(&self, level: u32) -> usize {
        assert!(level <= self.log2, "level {level} > max_level {}", self.log2);
        if level == 0 {
            self.capacity()
        } else {
            (2 * self.capacity()) >> level
        }
    }

    /// Bytes per slot of `level` (8 for `[x, y]` samples, 16 for buckets).
    pub fn slot_stride(level: u32) -> usize {
        if level == 0 {
            std::mem::size_of::<[f32; 2]>()
        } else {
            std::mem::size_of::<Bucket>()
        }
    }

    /// The raw level-0 ring, indexed by slot.
    pub fn points(&self) -> &[[f32; 2]] {
        &self.points
    }

    /// The buckets of `level` (`1..=max_level`), indexed by slot.  Slots
    /// outside [`SeriesBuf::live`] hold stale or empty buckets.
    pub fn level(&self, level: u32) -> &[Bucket] {
        assert!((1..=self.log2).contains(&level), "level {level} not in 1..={}", self.log2);
        &self.levels[level as usize - 1]
    }

    /// `level`'s slots as bytes (the GPU upload source); slice it by
    /// `slot * slot_stride(level)`.
    pub fn level_bytes(&self, level: u32) -> &[u8] {
        if level == 0 {
            bytemuck::cast_slice(&self.points)
        } else {
            bytemuck::cast_slice(self.level(level))
        }
    }

    /// The `i`-th surviving sample, oldest first.
    pub fn sample(&self, i: usize) -> [f32; 2] {
        debug_assert!(i < self.len);
        self.points[((self.first_abs() + i as u64) & (self.capacity() as u64 - 1)) as usize]
    }

    /// The bucket at absolute bucket index `abs` of `level` (`>= 1`); only
    /// meaningful inside [`SeriesBuf::live`].
    pub fn bucket(&self, level: u32, abs: u64) -> Bucket {
        let slots = self.slots(level) as u64;
        self.level(level)[(abs & (slots - 1)) as usize]
    }

    /// `(first_x, last_x)` of the surviving samples.
    pub fn x_range(&self) -> Option<(f32, f32)> {
        (self.len > 0).then(|| (self.sample(0)[0], self.sample(self.len - 1)[0]))
    }

    /// The live entries of `level` as `(start_abs, count)`: every entry with
    /// at least one surviving sample.
    pub fn live(&self, level: u32) -> (u64, usize) {
        if self.len == 0 {
            return (self.head >> level, 0);
        }
        let start = self.first_abs() >> level;
        let end = ((self.head - 1) >> level) + 1;
        (start, (end - start) as usize)
    }

    /// Map `count` consecutive entries of `level` from absolute index
    /// `start` onto at most two `(first_slot, count)` slot ranges.
    pub fn slot_ranges(&self, level: u32, start: u64, count: usize) -> [(usize, usize); 2] {
        let slots = self.slots(level);
        assert!(count <= slots, "{count} entries exceed the {slots} slots of level {level}");
        let s = (start & (slots as u64 - 1)) as usize;
        if s + count <= slots {
            [(s, count), (0, 0)]
        } else {
            [(s, slots - s), (0, count - (slots - s))]
        }
    }

    /// Choose what to draw for the x view `[view_x0, view_x1]` on a plot
    /// `plot_w_px` pixels wide.  Raw points in view (plus one neighbour on
    /// each side, so the line reaches the edges) `< 2 * plot_w` draw as a
    /// level-0 [`GraphMode::Polyline`]; otherwise the smallest level `k >= 1`
    /// whose bucket count is `<= plot_w` draws as an [`GraphMode::Envelope`]
    /// (`plot_w / 2 ..= plot_w + 2` buckets).  Deterministic; `O(log n)`.
    pub fn select(&self, view_x0: f64, view_x1: f64, plot_w_px: f32) -> Selection {
        if self.len == 0 || view_x0.is_nan() || view_x1.is_nan() || view_x0 > view_x1 {
            return Selection::empty();
        }
        let lo = self.partition_point(|x| x < view_x0);
        let hi = self.partition_point(|x| x <= view_x1);
        if lo == self.len || hi == 0 {
            return Selection::empty();
        }
        let i0 = lo.saturating_sub(1);
        let i1 = (hi + 1).min(self.len);
        let n = i1 - i0;
        let w = (plot_w_px.max(1.0).ceil() as usize).max(1);
        let a0 = self.first_abs() + i0 as u64;
        if n < 2 * w {
            return Selection {
                level: 0,
                mode: GraphMode::Polyline,
                start: a0,
                count: n,
                ranges: self.slot_ranges(0, a0, n),
            };
        }
        let mut k = 1;
        while k < self.log2 && (n >> k) > w {
            k += 1;
        }
        let a1 = a0 + n as u64 - 1;
        let start = a0 >> k;
        let count = ((a1 >> k) - start + 1) as usize;
        Selection {
            level: k,
            mode: GraphMode::Envelope,
            start,
            count,
            ranges: self.slot_ranges(k, start, count),
        }
    }

    /// Auto-Y: the `(y_min, y_max)` of what [`SeriesBuf::select`] draws for
    /// this view.  On level 0 only samples inside the view count (the edge
    /// neighbours do when nothing else is in view, i.e. sub-sample zoom).
    pub fn y_range(&self, view_x0: f64, view_x1: f64, plot_w_px: f32) -> Option<(f32, f32)> {
        let sel = self.select(view_x0, view_x1, plot_w_px);
        if sel.is_empty() {
            return None;
        }
        let mut acc = Bucket::EMPTY;
        match sel.mode {
            GraphMode::Polyline => {
                let i0 = (sel.start - self.first_abs()) as usize;
                let samples = (i0..i0 + sel.count).map(|i| self.sample(i));
                let mut inside = Bucket::EMPTY;
                for [x, y] in samples {
                    let b = Bucket::point(x, y);
                    acc = acc.merge(b);
                    if (view_x0..=view_x1).contains(&(x as f64)) {
                        inside = inside.merge(b);
                    }
                }
                if !inside.is_empty() {
                    acc = inside;
                }
            }
            GraphMode::Envelope => {
                for a in sel.start..sel.start + sel.count as u64 {
                    acc = acc.merge(self.bucket(sel.level, a));
                }
            }
        }
        (acc.y_min <= acc.y_max).then_some((acc.y_min, acc.y_max))
    }

    /// The slots changed since the last call, per level (only levels with
    /// changes); resets the tracking.  `from_points` starts fully dirty.
    pub fn take_dirty(&mut self) -> Vec<DirtyLevel> {
        let mut out = Vec::new();
        for level in 0..=self.log2 {
            let d = std::mem::take(&mut self.dirty[level as usize]);
            let slots = self.slots(level);
            let mut ranges = Vec::new();
            let mut full = d.full;
            for (lo, hi) in [d.head, d.tail].into_iter().flatten() {
                let count = (hi - lo) as usize;
                if count >= slots {
                    full = true;
                    break;
                }
                ranges.extend(self.slot_ranges(level, lo, count).into_iter().filter(|r| r.1 > 0));
            }
            if full {
                ranges = vec![(0, slots)];
            }
            if !ranges.is_empty() {
                out.push(DirtyLevel { level, ranges });
            }
        }
        out
    }

    /// First surviving index `i` (oldest first) where `pred(x_i)` is false;
    /// `pred` must be true on a prefix (x is monotone).
    fn partition_point(&self, pred: impl Fn(f64) -> bool) -> usize {
        let (mut lo, mut hi) = (0, self.len);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if pred(self.sample(mid)[0] as f64) {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }

    /// `true` when entry `abs` of `level` has a surviving sample.
    fn is_live(&self, level: u32, abs: u64) -> bool {
        (abs << level) < self.head && ((abs + 1) << level) > self.first_abs()
    }

    fn child(&self, level: u32, abs: u64) -> Bucket {
        if !self.is_live(level, abs) {
            Bucket::EMPTY
        } else if level == 0 {
            let [x, y] = self.points[(abs & (self.capacity() as u64 - 1)) as usize];
            Bucket::point(x, y)
        } else {
            self.bucket(level, abs)
        }
    }

    /// Recompute bucket `abs` of `level` from its surviving children.
    fn rebuild(&mut self, level: u32, abs: u64) {
        let b = self.child(level - 1, 2 * abs).merge(self.child(level - 1, 2 * abs + 1));
        let slots = self.slots(level) as u64;
        self.levels[level as usize - 1][(abs & (slots - 1)) as usize] = b;
    }
}

/// Ticks on `1/2/5 × 10^n` steps inside `[min, max]`, about `target` of
/// them (at most `target + 1`), each an integer multiple of the step.
pub fn nice_ticks(min: f64, max: f64, target: u32) -> Vec<f64> {
    if !(min.is_finite() && max.is_finite()) {
        return Vec::new();
    }
    let (min, max) = if max < min { (max, min) } else { (min, max) };
    if max == min {
        return vec![min];
    }
    let step = nice_step((max - min) / target.max(1) as f64);
    let k0 = (min / step - 1e-9).ceil() as i64;
    let k1 = (max / step + 1e-9).floor() as i64;
    (k0..=k1).map(|k| k as f64 * step).collect()
}

/// The smallest `1/2/5 × 10^n` step `>= raw` (`raw > 0`).
pub fn nice_step(raw: f64) -> f64 {
    let e = 10f64.powf(raw.log10().floor());
    let f = raw / e;
    let m = if f <= 1.0 + 1e-9 {
        1.0
    } else if f <= 2.0 + 1e-9 {
        2.0
    } else if f <= 5.0 + 1e-9 {
        5.0
    } else {
        10.0
    };
    m * e
}

/// Map a view coordinate `v` in `[v_min, v_max]` linearly onto pixels
/// `[px_min, px_max]` (pass `px_min > px_max` for a downward y axis).
pub fn view_to_px(v: f64, v_min: f64, v_max: f64, px_min: f64, px_max: f64) -> f64 {
    if v_max == v_min {
        return px_min;
    }
    px_min + (v - v_min) * (px_max - px_min) / (v_max - v_min)
}

/// The inverse of [`view_to_px`].
pub fn px_to_view(px: f64, v_min: f64, v_max: f64, px_min: f64, px_max: f64) -> f64 {
    if px_max == px_min {
        return v_min;
    }
    v_min + (px - px_min) * (v_max - v_min) / (px_max - px_min)
}
