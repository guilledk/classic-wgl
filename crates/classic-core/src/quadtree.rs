//! Generic spatial quadtree.
//!
//! Port of `src/lib/quadtree.ts`.

use crate::types::Rect;

/// A Quadtree node for spatial partitioning of objects that can provide a `Rect` bounding box.
pub struct Quadtree<T: RectBounds> {
    max_objects: usize,
    max_levels: usize,
    level: usize,
    bounds: Rect,
    pub objects: Vec<T>,
    pub nodes: Vec<Quadtree<T>>,
}

/// Trait for objects that can be stored in the quadtree.
pub trait RectBounds {
    fn rect(&self) -> Rect;
}

impl RectBounds for Rect {
    fn rect(&self) -> Rect {
        *self
    }
}

impl<T: RectBounds> Quadtree<T> {
    /// Create a new quadtree node.
    ///
    /// * `bounds` — the spatial bounds of this node
    /// * `max_objects` — max objects before splitting (default: 10)
    /// * `max_levels` — max depth (default: 4)
    pub fn new(bounds: Rect, max_objects: usize, max_levels: usize) -> Self {
        Self { max_objects, max_levels, level: 0, bounds, objects: Vec::new(), nodes: Vec::new() }
    }

    fn new_child(&self, bounds: Rect, level: usize) -> Self {
        Self {
            max_objects: self.max_objects,
            max_levels: self.max_levels,
            level,
            bounds,
            objects: Vec::new(),
            nodes: Vec::new(),
        }
    }

    /// Split this node into 4 sub-nodes.
    fn split(&mut self) {
        let next_level = self.level + 1;
        let sub_w = self.bounds.width / 2.0;
        let sub_h = self.bounds.height / 2.0;
        let x = self.bounds.x;
        let y = self.bounds.y;

        // 0: top-right
        self.nodes.push(self.new_child(Rect::new(x + sub_w, y, sub_w, sub_h), next_level));
        // 1: top-left
        self.nodes.push(self.new_child(Rect::new(x, y, sub_w, sub_h), next_level));
        // 2: bottom-left
        self.nodes.push(self.new_child(Rect::new(x, y + sub_h, sub_w, sub_h), next_level));
        // 3: bottom-right
        self.nodes.push(self.new_child(Rect::new(x + sub_w, y + sub_h, sub_w, sub_h), next_level));
    }

    /// Determine which sub-node indices a rectangle intersects.
    pub fn get_index(&self, r: &Rect) -> Vec<usize> {
        let mut idx = Vec::new();
        let v_mid = self.bounds.x + self.bounds.width / 2.0;
        let h_mid = self.bounds.y + self.bounds.height / 2.0;

        let start_north = r.y < h_mid;
        let start_west = r.x < v_mid;
        let end_east = r.x + r.width > v_mid;
        let end_south = r.y + r.height > h_mid;

        // top-right (0)
        if start_north && end_east {
            idx.push(0);
        }
        // top-left (1)
        if start_west && start_north {
            idx.push(1);
        }
        // bottom-left (2)
        if start_west && end_south {
            idx.push(2);
        }
        // bottom-right (3)
        if end_east && end_south {
            idx.push(3);
        }

        idx
    }

    /// Insert an object into the quadtree.
    pub fn insert(&mut self, obj: T) {
        // If we have subnodes, delegate to the one(s) that fully contain the object.
        if !self.nodes.is_empty() {
            let idxs = self.get_index(&obj.rect());
            if idxs.len() == 1 {
                self.nodes[idxs[0]].insert(obj);
            } else {
                // Straddles multiple subnodes — keep in this level.
                self.objects.push(obj);
            }
            return;
        }

        self.objects.push(obj);

        if self.objects.len() > self.max_objects && self.level < self.max_levels {
            if self.nodes.is_empty() {
                self.split();
            }
            let objs = std::mem::take(&mut self.objects);
            for obj in objs {
                let idxs = self.get_index(&obj.rect());
                if idxs.len() == 1 {
                    self.nodes[idxs[0]].insert(obj);
                } else {
                    // Straddler — stays at this level.
                    self.objects.push(obj);
                }
            }
        }
    }

    /// Retrieve all objects that could intersect with the given rectangle.
    pub fn retrieve(&self, r: &Rect) -> Vec<&T> {
        let idxs = self.get_index(r);
        let mut result: Vec<&T> = self.objects.iter().collect();

        if !self.nodes.is_empty() {
            for &i in &idxs {
                result.extend(self.nodes[i].retrieve(r));
            }
        }

        // No dedup needed — each object lives in exactly one node.
        result
    }

    /// Clear all objects and sub-nodes.
    pub fn clear(&mut self) {
        self.objects.clear();
        for node in &mut self.nodes {
            node.clear();
        }
        self.nodes.clear();
    }
}
