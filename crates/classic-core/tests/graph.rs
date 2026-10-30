use classic_core::components::{GraphRender, GraphSeriesRef, GraphStyle};
use classic_core::graph::{
    nice_step, nice_ticks, px_to_view, view_to_px, Bucket, GraphMode, SeriesBuf,
};
use classic_core::register_all_components;

/// SplitMix64: a seeded, dependency-free generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[-1, 1)`.
    fn f32(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 23) as f32 - 1.0
    }
}

/// Monotone x (non-decreasing, with repeats) and noisy y.
fn random_series(rng: &mut Rng, n: usize) -> (Vec<f32>, Vec<f32>) {
    let mut x = Vec::with_capacity(n);
    let mut y = Vec::with_capacity(n);
    let mut t = 0.0f32;
    for _ in 0..n {
        if rng.next() & 7 != 0 {
            t += 0.25 + rng.f32().abs();
        }
        x.push(t);
        y.push(rng.f32() * 100.0);
    }
    (x, y)
}

/// Every live bucket of every level equals the brute-force min/max over
/// its surviving samples.
fn assert_pyramid_exact(buf: &SeriesBuf) {
    let first = buf.first_abs();
    let raw: Vec<[f32; 2]> = (0..buf.len()).map(|i| buf.sample(i)).collect();
    for k in 1..=buf.max_level() {
        let (start, count) = buf.live(k);
        for a in start..start + count as u64 {
            let lo = (a << k).max(first);
            let hi = ((a + 1) << k).min(buf.head());
            let want = raw[(lo - first) as usize..(hi - first) as usize]
                .iter()
                .fold(Bucket::EMPTY, |b, &[x, y]| b.merge(Bucket::point(x, y)));
            assert!(!want.is_empty());
            assert_eq!(buf.bucket(k, a), want, "level {k} bucket {a} (head {})", buf.head());
        }
    }
}

#[test]
fn from_points_pyramid_matches_brute_force() {
    let mut rng = Rng(1);
    for n in [1, 2, 3, 7, 8, 9, 100, 1000, 4096, 5000] {
        let (x, y) = random_series(&mut rng, n);
        let buf = SeriesBuf::from_points(&x, &y);
        assert_eq!(buf.capacity(), n.max(2).next_power_of_two());
        assert_eq!(buf.len(), n);
        assert_pyramid_exact(&buf);
    }
}

#[test]
fn push_pyramid_matches_brute_force_across_wraps() {
    let mut rng = Rng(2);
    for cap in [2, 4, 16, 64] {
        let mut buf = SeriesBuf::with_capacity(cap);
        let (x, y) = random_series(&mut rng, cap * 5 + 3);
        for (i, (&x, &y)) in x.iter().zip(&y).enumerate() {
            buf.push(x, y);
            assert_eq!(buf.len(), (i + 1).min(cap));
            assert_pyramid_exact(&buf);
        }
    }
}

#[test]
fn evicted_extremes_do_not_linger() {
    let mut buf = SeriesBuf::with_capacity(8);
    buf.push(0.0, 1000.0);
    for i in 1..8 {
        buf.push(i as f32, 0.0);
    }
    let top = buf.max_level();
    let (start, _) = buf.live(top);
    assert_eq!(buf.bucket(top, start).y_max, 1000.0);
    buf.push(8.0, 0.0);
    for k in 1..=top {
        let (start, count) = buf.live(k);
        for a in start..start + count as u64 {
            assert_eq!(buf.bucket(k, a).y_max, 0.0, "level {k}");
        }
    }
}

#[test]
fn push_built_equals_from_points_built() {
    let mut rng = Rng(3);
    let cap = 256;
    // Pushing a whole number of capacities leaves the survivors aligned at
    // every level, so the two pyramids bucket identically.
    for wraps in 1..=3 {
        let (x, y) = random_series(&mut rng, cap * wraps);
        let mut pushed = SeriesBuf::with_capacity(cap);
        for (&x, &y) in x.iter().zip(&y) {
            pushed.push(x, y);
        }
        let tail = cap * (wraps - 1);
        let built = SeriesBuf::from_points(&x[tail..], &y[tail..]);
        assert_eq!(built.capacity(), pushed.capacity());
        assert_eq!(built.len(), pushed.len());
        for i in 0..cap {
            assert_eq!(built.sample(i), pushed.sample(i));
        }
        for k in 1..=built.max_level() {
            let (bs, bc) = built.live(k);
            let (ps, pc) = pushed.live(k);
            assert_eq!(bc, pc);
            for j in 0..bc as u64 {
                assert_eq!(built.bucket(k, bs + j), pushed.bucket(k, ps + j), "level {k}");
            }
        }
        for (v0, v1, w) in [(0.0, 1e9, 100.0), (x[tail] as f64, x[tail + 10] as f64, 300.0)] {
            let (b, p) = (built.select(v0, v1, w), pushed.select(v0, v1, w));
            assert_eq!((b.level, b.mode, b.count), (p.level, p.mode, p.count));
            assert_eq!(built.y_range(v0, v1, w), pushed.y_range(v0, v1, w));
        }
    }
    // A partial fill is the same as from_points at equal capacity.
    let (x, y) = random_series(&mut rng, 100);
    let mut pushed = SeriesBuf::with_capacity(128);
    for (&x, &y) in x.iter().zip(&y) {
        pushed.push(x, y);
    }
    let built = SeriesBuf::from_points(&x, &y);
    for k in 1..=built.max_level() {
        assert_eq!(built.level(k), pushed.level(k), "level {k}");
    }
}

/// Uniform x = 0, 1, 2, ... for predictable view maths.
fn uniform(n: usize) -> SeriesBuf {
    let x: Vec<f32> = (0..n).map(|i| i as f32).collect();
    let y: Vec<f32> = (0..n).map(|i| ((i * 7919) % 1000) as f32).collect();
    SeriesBuf::from_points(&x, &y)
}

#[test]
fn select_full_range_is_envelope_of_about_plot_width() {
    let buf = uniform(1 << 20);
    for w in [1.0, 100.0, 640.0, 1000.0, 3840.0] {
        let sel = buf.select(0.0, (1 << 20) as f64, w);
        assert_eq!(sel.mode, GraphMode::Envelope);
        assert!(sel.level >= 1);
        let w = w as usize;
        assert!(sel.count <= w + 2 && sel.count * 2 >= w, "w {w}: {} buckets", sel.count);
        assert_eq!(sel.slot_ranges().map(|r| r.1).sum::<usize>(), sel.count);
        let (y0, y1) = buf.y_range(0.0, (1 << 20) as f64, w as f32).unwrap();
        assert_eq!((y0, y1), (0.0, 999.0));
    }
}

#[test]
fn select_polyline_when_zoomed_in() {
    let buf = uniform(1 << 20);
    // ~1 px of data: 10 samples on a 1000 px plot.
    let sel = buf.select(5000.0, 5009.0, 1000.0);
    assert_eq!((sel.level, sel.mode), (0, GraphMode::Polyline));
    assert_eq!((sel.start, sel.count), (4999, 12));
    // Just under the threshold stays a polyline, at it becomes an envelope.
    let sel = buf.select(0.0, 1997.0, 1000.0);
    assert_eq!((sel.mode, sel.count), (GraphMode::Polyline, 1999));
    let sel = buf.select(0.0, 1998.0, 1000.0);
    assert_eq!((sel.mode, sel.level), (GraphMode::Envelope, 1));
    // Sub-sample zoom: the view falls between two samples.
    let sel = buf.select(10.25, 10.75, 1000.0);
    assert_eq!((sel.mode, sel.start, sel.count), (GraphMode::Polyline, 10, 2));
    assert_eq!(buf.y_range(10.25, 10.75, 1000.0), {
        let [a, b] = [buf.sample(10)[1], buf.sample(11)[1]];
        Some((a.min(b), a.max(b)))
    });
    // Outside the data.
    assert!(buf.select(-10.0, -1.0, 100.0).is_empty());
    assert!(buf.select(2e6, 3e6, 100.0).is_empty());
    assert!(buf.select(5.0, 1.0, 100.0).is_empty());
    assert!(SeriesBuf::with_capacity(8).select(0.0, 1.0, 100.0).is_empty());
}

#[test]
fn select_is_deterministic_and_bounded_at_every_zoom() {
    let buf = uniform(1 << 16);
    let mut span = 0.5;
    while span < 1e6 {
        for w in [64.0, 512.0] {
            let sel = buf.select(1000.0, 1000.0 + span, w);
            assert_eq!(sel, buf.select(1000.0, 1000.0 + span, w));
            assert!(sel.count <= 2 * w as usize + 2, "span {span}: {sel:?}");
        }
        span *= 1.7;
    }
}

#[test]
fn select_splits_wrapped_ring_into_two_ranges() {
    let cap = 64;
    let mut buf = SeriesBuf::with_capacity(cap);
    for i in 0..(cap + cap / 2) {
        buf.push(i as f32, i as f32);
    }
    // Survivors are abs 32..96: slots 32..64 then 0..32 on level 0.
    let sel = buf.select(0.0, 1e9, 1000.0);
    assert_eq!((sel.mode, sel.start, sel.count), (GraphMode::Polyline, 32, 64));
    assert_eq!(sel.ranges, [(32, 32), (0, 32)]);
    let pts: Vec<_> =
        sel.slot_ranges().flat_map(|(s, c)| buf.points()[s..s + c].to_vec()).collect();
    assert_eq!(pts, (32..96).map(|i| [i as f32; 2]).collect::<Vec<_>>());
    // Envelope over the wrap (level 3 has 16 slots; buckets 4..12).
    let sel = buf.select(0.0, 1e9, 8.0);
    assert_eq!(sel.mode, GraphMode::Envelope);
    assert_eq!((sel.level, sel.start, sel.count), (3, 4, 8));
    let slots = buf.slots(3);
    assert_eq!(slots, 16);
    assert_eq!(sel.ranges, [(4, 8), (0, 0)]);
    // Push until the level-3 window wraps its 16 slots: survivors are abs
    // 196..260, buckets 24..=32 (both ends partial), slots 8..16 then 0.
    for i in 96..260 {
        buf.push(i as f32, i as f32);
    }
    let sel = buf.select(0.0, 1e9, 8.0);
    assert_eq!((sel.level, sel.start, sel.count), (3, 24, 9));
    assert_eq!(sel.ranges, [(8, 8), (0, 1)]);
    assert_eq!((sel.start, sel.count), buf.live(3));
    let b: Vec<Bucket> =
        sel.slot_ranges().flat_map(|(s, c)| buf.level(3)[s..s + c].to_vec()).collect();
    let want: Vec<Bucket> = (24..33).map(|a| buf.bucket(3, a)).collect();
    assert_eq!(b, want);
    assert_eq!((b[0].x0, b[0].x1), (196.0, 199.0));
    assert_eq!((b[8].x0, b[8].x1), (256.0, 259.0));
}

#[test]
fn take_dirty_reports_changed_slots() {
    let mut buf = uniform(100);
    let d = buf.take_dirty();
    assert_eq!(d.len(), buf.max_level() as usize + 1);
    for l in &d {
        assert_eq!(l.ranges, vec![(0, buf.slots(l.level))]);
    }
    assert!(buf.take_dirty().is_empty());

    let mut buf = SeriesBuf::with_capacity(16);
    for i in 0..16 {
        buf.push(i as f32, 0.0);
    }
    buf.take_dirty();
    buf.push(16.0, 1.0); // abs 16, evicts abs 0
    let d = buf.take_dirty();
    assert_eq!(d[0].level, 0);
    assert_eq!(d[0].ranges, vec![(0, 1)]);
    // Level 1 (16 slots): new bucket 8 + evicted bucket 0.
    assert_eq!(d[1].ranges, vec![(8, 1), (0, 1)]);
    // Top level 4 (2 slots): bucket 1 (new) + bucket 0 (evicted).
    assert_eq!(d[4].ranges, vec![(1, 1), (0, 1)]);
    // A burst longer than a level's slot count marks it fully dirty.
    for i in 17..60 {
        buf.push(i as f32, 0.0);
    }
    let d = buf.take_dirty();
    assert_eq!(d[0].ranges, vec![(0, 16)]);
}

#[test]
fn level_bytes_match_slot_stride() {
    let buf = uniform(1000);
    assert_eq!(buf.level_bytes(0).len(), buf.slots(0) * SeriesBuf::slot_stride(0));
    for k in 1..=buf.max_level() {
        assert_eq!(buf.level_bytes(k).len(), buf.slots(k) * SeriesBuf::slot_stride(k));
    }
}

#[test]
fn nice_ticks_are_125_steps_covering_range() {
    let mut rng = Rng(4);
    let mut cases = vec![(0.0, 1.0, 5), (-3.7, 12.2, 5), (1e6, 1e6 + 3.0, 4), (0.001, 0.0042, 6)];
    for _ in 0..200 {
        let a = rng.f32() as f64 * 10f64.powi((rng.next() % 12) as i32 - 6);
        let b = a + rng.f32().abs() as f64 * 10f64.powi((rng.next() % 12) as i32 - 6) + 1e-9;
        cases.push((a, b, 1 + (rng.next() % 10) as u32));
    }
    for (min, max, target) in cases {
        let ticks = nice_ticks(min, max, target);
        let step = nice_step((max - min) / target as f64);
        // f64 rounding of `k * step` at large `|k|`.
        let tol = step * 1e-6 + max.abs().max(min.abs()) * 1e-14;
        let mant = step / 10f64.powf(step.log10().floor());
        assert!([1.0, 2.0, 5.0, 10.0].iter().any(|m| (mant - m).abs() < 1e-6), "{step}");
        assert!(!ticks.is_empty() || step > max - min, "{min}..{max}");
        assert!(ticks.len() as u32 <= target + 1, "{min}..{max} /{target}: {ticks:?}");
        for t in &ticks {
            assert!(*t >= min - tol && *t <= max + tol);
            assert!(((t / step).round() - t / step).abs() < 1e-6);
        }
        for w in ticks.windows(2) {
            assert!(((w[1] - w[0]) - step).abs() <= tol, "{min}..{max}: {w:?} step {step}");
        }
        if let (Some(first), Some(last)) = (ticks.first(), ticks.last()) {
            assert!(first - min < step + tol && max - last < step + tol);
        }
    }
    assert_eq!(nice_ticks(0.0, 10.0, 5), vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0]);
    assert_eq!(nice_ticks(3.0, 3.0, 5), vec![3.0]);
    assert!(nice_ticks(f64::NAN, 1.0, 5).is_empty());
}

#[test]
fn view_to_px_round_trips() {
    for (v0, v1, p0, p1) in [(0.0, 1.0, 0.0, 640.0), (-5.0, 1e6, 480.0, 20.0)] {
        for v in [v0, v1, (v0 + v1) * 0.37, v1 * 2.0] {
            let px = view_to_px(v, v0, v1, p0, p1);
            assert!((px_to_view(px, v0, v1, p0, p1) - v).abs() <= 1e-9 * (1.0 + v.abs()));
        }
        assert_eq!(view_to_px(v0, v0, v1, p0, p1), p0);
        assert_eq!(view_to_px(v1, v0, v1, p0, p1), p1);
    }
}

#[test]
fn graph_component_dumps_names_only() {
    register_all_components();
    let g = GraphRender {
        series: vec![GraphSeriesRef::new("perf.frame_ms", GraphStyle::default())],
        ..Default::default()
    };
    let mut world = hecs::World::new();
    let e = world.spawn((g.clone(),));
    let regs = classic_core::registry::ordered_regs();
    let reg = regs.iter().find(|r| r.name == "Graph").unwrap();
    let val = reg.dump_value(&world, e).unwrap();
    assert_eq!(val["type"], "Graph");
    assert_eq!(val["series"][0]["data"], "perf.frame_ms");
    let back: GraphRender = serde_json::from_value(val.clone()).unwrap();
    assert_eq!(back, g);

    // Spawn from a minimal JSON body: everything else defaults.
    let mut b = hecs::EntityBuilder::new();
    (classic_core::registry::lookup("Graph").unwrap())(
        &mut b,
        serde_json::json!({ "series": [{ "data": "s" }] }),
    )
    .unwrap();
    let e = world.spawn(b.build());
    let g = world.get::<&GraphRender>(e).unwrap();
    assert_eq!(g.series[0].style, GraphStyle::default());
    assert!(g.view.auto_y && g.axes.show);
}

/// Release timings (report, don't gate):
/// `cargo test --release -p classic-core --test graph -- --ignored --nocapture`
#[test]
#[ignore]
fn timings_1m() {
    let n = 1 << 20;
    let x: Vec<f32> = (0..n).map(|i| i as f32).collect();
    let y: Vec<f32> = (0..n).map(|i| (i as f32 * 0.001).sin()).collect();
    let t = std::time::Instant::now();
    let buf = SeriesBuf::from_points(&x, &y);
    println!("from_points 1M: {:?}", t.elapsed());
    std::hint::black_box(&buf);
    let mut buf = SeriesBuf::with_capacity(n);
    let t = std::time::Instant::now();
    for i in 0..n {
        buf.push(x[i], y[i]);
    }
    println!("1M push (fill): {:?}", t.elapsed());
    let t = std::time::Instant::now();
    for i in 0..n {
        buf.push(x[i] + n as f32, y[i]);
    }
    println!("1M push (evicting): {:?}", t.elapsed());
    let t = std::time::Instant::now();
    let mut acc = 0;
    for i in 0..1000 {
        acc += buf.select(i as f64, 2e6 - i as f64, 1920.0).count;
    }
    println!("1000 select: {:?} ({acc})", t.elapsed());
}
