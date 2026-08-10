//! # Skill: `classic-physics`
//!
//! **Read `.claude/skills/classic-physics/SKILL.md` before working on this module.**
//!
//! A* pathfinding over a 2D grid of walkable/blocked cells.
//!
//! Port of the algorithm from `src/classic/pathfinder.ts` (web worker),
//! running in-thread.  The nav mesh is a flat `[i32]` where `1` = walkable,
//! `0` = blocked.

use std::collections::BinaryHeap;

/// Ordered float for priority queue keys.
#[derive(Copy, Clone, PartialEq)]
struct Key {
    cost: f32,
    cell: (i32, i32),
}

impl Eq for Key {}
impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Key {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reverse: lower cost = higher priority.
        other.cost.total_cmp(&self.cost).then_with(|| self.cell.cmp(&other.cell))
    }
}

/// A single cell coordinate on the nav grid.
pub type GridCell = (i32, i32);

/// Run 8-directional A* from `from` to `to` on a grid of `size_x × size_y`
/// cells.  `nav_data[i]` must be 1 (walkable) or 0 (blocked).
///
/// Returns the full path (including `from` and `to`) as vec of integer
/// cell coordinates, or `None` if no path exists.
pub fn find_path(
    nav_data: &[i32],
    size_x: i32,
    size_y: i32,
    mut from: GridCell,
    mut to: GridCell,
) -> Option<Vec<GridCell>> {
    // Clamp targets to valid range.
    from.0 = from.0.clamp(0, size_x - 1);
    from.1 = from.1.clamp(0, size_y - 1);
    to.0 = to.0.clamp(0, size_x - 1);
    to.1 = to.1.clamp(0, size_y - 1);

    let flatten = |x: i32, y: i32| -> usize { (x + y * size_x) as usize };

    if from == to {
        return Some(vec![from, to]);
    }

    let total = (size_x * size_y) as usize;
    let inf: f32 = f32::MAX;

    let mut g_cost = vec![inf; total];
    let mut f_cost = vec![inf; total];
    let mut came_from: Vec<Option<usize>> = vec![None; total];
    let mut in_open = vec![false; total];

    let from_idx = flatten(from.0, from.1);

    g_cost[from_idx] = 0.0;
    f_cost[from_idx] = heuristic(from, to);

    let mut open = BinaryHeap::new();
    open.push(Key { cost: f_cost[from_idx], cell: from });

    let neighbours: [(i32, i32); 8] =
        [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];

    while let Some(Key { cell: current, .. }) = open.pop() {
        let cur_idx = flatten(current.0, current.1);

        if current == to {
            return Some(reconstruct_path(&came_from, &g_cost, &f_cost, from, to, size_x));
        }

        in_open[cur_idx] = false;

        for &(dx, dy) in &neighbours {
            let nx = current.0 + dx;
            let ny = current.1 + dy;

            if nx < 0 || nx >= size_x || ny < 0 || ny >= size_y {
                continue;
            }
            let n_idx = flatten(nx, ny);
            if nav_data[n_idx] == 0 {
                continue;
            }

            let step_cost = if dx != 0 && dy != 0 { (2.0_f32).sqrt() } else { 1.0 };
            let tentative_g = g_cost[cur_idx].min(inf) + step_cost;

            if tentative_g < g_cost[n_idx] {
                came_from[n_idx] = Some(cur_idx);
                g_cost[n_idx] = tentative_g;
                f_cost[n_idx] = tentative_g + heuristic((nx, ny), to);

                if !in_open[n_idx] {
                    open.push(Key { cost: f_cost[n_idx], cell: (nx, ny) });
                    in_open[n_idx] = true;
                }
            }
        }
    }

    None
}

/// Chebyshev-approximation heuristic (admissible for 8-directional grid).
/// Returns the estimate of cost from `a` to `b`.
fn heuristic(a: GridCell, b: GridCell) -> f32 {
    let dx = (a.0 - b.0).abs() as f32;
    let dy = (a.1 - b.1).abs() as f32;
    dx + dy + (2.0_f32.sqrt() - 2.0) * dx.min(dy)
}

/// Walk backwards from `to` through `came_from` to rebuild the path.
fn reconstruct_path(
    came_from: &[Option<usize>],
    _g_cost: &[f32],
    _f_cost: &[f32],
    from: GridCell,
    to: GridCell,
    size_x: i32,
) -> Vec<GridCell> {
    let flatten = |x: i32, y: i32| -> usize { (x + y * size_x) as usize };
    let from_idx = flatten(from.0, from.1);
    let to_idx = flatten(to.0, to.1);

    let mut path = vec![to];
    let mut cur = to_idx;
    while cur != from_idx {
        match came_from[cur] {
            Some(prev) => {
                let px = (prev % size_x as usize) as i32;
                let py = (prev / size_x as usize) as i32;
                path.push((px, py));
                cur = prev;
            }
            None => break,
        }
    }
    path.reverse();
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simple_grid() -> Vec<i32> {
        // 5x5 all walkable
        vec![1; 25]
    }

    #[test]
    fn straight_line_path() {
        let grid = simple_grid();
        let path = find_path(&grid, 5, 5, (0, 0), (4, 0)).expect("path");
        assert_eq!(path.len(), 5);
        assert_eq!(path[0], (0, 0));
        assert_eq!(path[path.len() - 1], (4, 0));
    }

    #[test]
    fn diagonal_path() {
        let grid = simple_grid();
        let path = find_path(&grid, 5, 5, (0, 0), (4, 4)).expect("path");
        assert_eq!(path[0], (0, 0));
        assert_eq!(path[path.len() - 1], (4, 4));
        // Should take diagonal (straight-line) route.
        for (i, &(x, y)) in path.iter().enumerate() {
            assert_eq!(x, y, "diagonal path step {i} should be on diagonal");
        }
    }

    #[test]
    fn blocked_no_path() {
        // 3x3 with a wall in the middle column
        let mut grid = vec![1_i32; 9];
        grid[1] = 0; // (1,0)
        grid[4] = 0; // (1,1)
        grid[7] = 0; // (1,2)
        let path = find_path(&grid, 3, 3, (0, 0), (2, 0));
        assert!(path.is_none(), "should be no path through wall");
    }

    #[test]
    fn navigate_around_wall() {
        // 3x3 with a single blocked cell at center
        let mut grid = vec![1_i32; 9];
        grid[4] = 0; // (1,1) blocked
        let path = find_path(&grid, 3, 3, (0, 0), (2, 2)).expect("path");
        assert_eq!(path[0], (0, 0));
        assert_eq!(path[path.len() - 1], (2, 2));
        // Path must go around center.
        for &(x, y) in &path {
            assert!(!(x == 1 && y == 1), "path must not pass through blocked cell");
        }
    }

    #[test]
    fn start_equals_goal() {
        let grid = simple_grid();
        let path = find_path(&grid, 5, 5, (2, 2), (2, 2)).expect("path");
        assert_eq!(path, vec![(2, 2), (2, 2)]);
    }
}
