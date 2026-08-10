use classic_core::quadtree::Quadtree;
use classic_core::Rect;

fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::new(x, y, w, h)
}

#[test]
fn retrieves_from_matching_quadrant_after_split() {
    let mut qt = Quadtree::new(r(0.0, 0.0, 100.0, 100.0), 1, 4);
    let a = r(10.0, 10.0, 5.0, 5.0);
    let b = r(90.0, 90.0, 5.0, 5.0);

    qt.insert(a);
    qt.insert(b);

    let near_a = qt.retrieve(&r(0.0, 0.0, 20.0, 20.0));
    assert!(near_a.iter().any(|o| o.x == a.x && o.y == a.y));
    assert!(!near_a.iter().any(|o| o.x == b.x && o.y == b.y));
}

#[test]
fn returns_all_when_not_split() {
    let mut qt = Quadtree::new(r(0.0, 0.0, 100.0, 100.0), 10, 4);
    let a = r(10.0, 10.0, 5.0, 5.0);
    let b = r(90.0, 90.0, 5.0, 5.0);

    qt.insert(a);
    qt.insert(b);

    let near_a = qt.retrieve(&r(0.0, 0.0, 20.0, 20.0));
    assert!(near_a.iter().any(|o| o.x == a.x));
    assert!(near_a.iter().any(|o| o.x == b.x));
}

#[test]
fn splits_at_max_objects() {
    let mut qt = Quadtree::new(r(0.0, 0.0, 100.0, 100.0), 2, 4);

    qt.insert(r(1.0, 1.0, 1.0, 1.0));
    qt.insert(r(2.0, 2.0, 1.0, 1.0));
    assert!(qt.nodes.is_empty());

    qt.insert(r(3.0, 3.0, 1.0, 1.0));
    assert_eq!(qt.nodes.len(), 4);
}

#[test]
fn does_not_split_past_max_levels() {
    let mut qt = Quadtree::new(r(0.0, 0.0, 100.0, 100.0), 1, 0);

    qt.insert(r(1.0, 1.0, 1.0, 1.0));
    qt.insert(r(2.0, 2.0, 1.0, 1.0));

    assert!(qt.nodes.is_empty());
    assert_eq!(qt.objects.len(), 2);
}

#[test]
fn straddling_rects_span_multiple_quadrants() {
    let qt = Quadtree::<Rect>::new(r(0.0, 0.0, 100.0, 100.0), 10, 4);
    let mut indexes = qt.get_index(&r(40.0, 0.0, 20.0, 10.0));
    indexes.sort();
    assert_eq!(indexes, vec![0, 1]);
}

#[test]
fn deduplicates_across_subnodes() {
    let mut qt = Quadtree::new(r(0.0, 0.0, 100.0, 100.0), 1, 4);
    let shared = r(45.0, 45.0, 10.0, 10.0); // straddles all 4 quadrants
    let other1 = r(1.0, 1.0, 1.0, 1.0);
    let other2 = r(2.0, 2.0, 1.0, 1.0);

    qt.insert(shared);
    qt.insert(other1);
    qt.insert(other2);

    let results = qt.retrieve(&r(0.0, 0.0, 100.0, 100.0));
    let count = results.iter().filter(|o| o.x == shared.x && o.y == shared.y).count();
    assert_eq!(count, 1);
}

#[test]
fn clear_resets_everything() {
    let mut qt = Quadtree::new(r(0.0, 0.0, 100.0, 100.0), 1, 4);
    qt.insert(r(1.0, 1.0, 1.0, 1.0));
    qt.insert(r(2.0, 2.0, 1.0, 1.0));
    assert_eq!(qt.nodes.len(), 4);

    qt.clear();
    assert!(qt.nodes.is_empty());
    assert!(qt.objects.is_empty());
}
