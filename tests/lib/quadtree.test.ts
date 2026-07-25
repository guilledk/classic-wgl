import { describe, it, expect } from 'vitest';
import { Quadtree, type Rect } from '/lib/quadtree.js';

function rect(x: number, y: number, width: number, height: number): Rect {
  return { x, y, width, height };
}

describe('Quadtree', () => {
  it('retrieves objects from the matching quadrant only, once split', () => {
    // maxObjects=1 forces a split on the second insert, so retrieve()
    // will actually narrow down by quadrant instead of returning everything.
    const qt = new Quadtree(rect(0, 0, 100, 100), 1, 4);
    const a = rect(10, 10, 5, 5); // top-left quadrant
    const b = rect(90, 90, 5, 5); // bottom-right quadrant

    qt.insert(a);
    qt.insert(b);

    const nearA = qt.retrieve(rect(0, 0, 20, 20));
    expect(nearA).toContain(a);
    expect(nearA).not.toContain(b);
  });

  it('returns every object when the tree has not split (broad-phase only, no split)', () => {
    const qt = new Quadtree(rect(0, 0, 100, 100));
    const a = rect(10, 10, 5, 5);
    const b = rect(90, 90, 5, 5);

    qt.insert(a);
    qt.insert(b);

    // Below maxObjects, so no split occurred: retrieve() returns all
    // objects in the (single) node regardless of query rect.
    const nearA = qt.retrieve(rect(0, 0, 20, 20));
    expect(nearA).toContain(a);
    expect(nearA).toContain(b);
  });

  it('splits into 4 subnodes once maxObjects is exceeded', () => {
    const qt = new Quadtree(rect(0, 0, 100, 100), 2, 4);

    qt.insert(rect(1, 1, 1, 1));
    qt.insert(rect(2, 2, 1, 1));
    expect(qt.nodes.length).toBe(0);

    qt.insert(rect(3, 3, 1, 1));
    expect(qt.nodes.length).toBe(4);
  });

  it('does not split past maxLevels', () => {
    const qt = new Quadtree(rect(0, 0, 100, 100), 1, 0);

    qt.insert(rect(1, 1, 1, 1));
    qt.insert(rect(2, 2, 1, 1));

    // maxLevels is 0, so the root should never split even when over capacity
    expect(qt.nodes.length).toBe(0);
    expect(qt.objects.length).toBe(2);
  });

  it('classifies straddling rects into multiple quadrants', () => {
    const qt = new Quadtree(rect(0, 0, 100, 100));
    // A rect that straddles the vertical midpoint (x=50) in the top half
    const indexes = qt.getIndex(rect(40, 0, 20, 10));
    expect(indexes.sort()).toEqual([0, 1]); // top-right and top-left
  });

  it('deduplicates objects returned across overlapping subnodes', () => {
    const qt = new Quadtree(rect(0, 0, 100, 100), 1, 4);
    const shared = rect(45, 45, 10, 10); // straddles all 4 quadrants
    const other1 = rect(1, 1, 1, 1);
    const other2 = rect(2, 2, 1, 1);

    qt.insert(shared);
    qt.insert(other1);
    qt.insert(other2); // triggers split, shared gets inserted into multiple nodes

    const results = qt.retrieve(rect(0, 0, 100, 100));
    const occurrences = results.filter((r) => r === shared).length;
    expect(occurrences).toBe(1);
  });

  it('clear() resets objects and subnodes', () => {
    const qt = new Quadtree(rect(0, 0, 100, 100), 1, 4);
    qt.insert(rect(1, 1, 1, 1));
    qt.insert(rect(2, 2, 1, 1));
    expect(qt.nodes.length).toBe(4);

    qt.clear();
    expect(qt.nodes.length).toBe(0);
    expect(qt.objects.length).toBe(0);
  });
});
