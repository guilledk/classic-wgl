/**
 * quadtree-js
 * @version 1.2.4
 * @license MIT
 * @author Timo Hausmann
 *
 * TypeScript conversion for classic-wgl
 */

/**
 * The Quadtree uses rectangle objects for all areas ("Rect").
 * All rectangles require the properties x, y, width, height
 */
export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * Quadtree class for spatial partitioning
 */
export class Quadtree<T extends Rect = Rect> {
  maxObjects: number;
  maxLevels: number;
  level: number;
  bounds: Rect;
  objects: T[];
  nodes: Quadtree<T>[];

  /**
   * @param bounds - bounds of the node ({ x, y, width, height })
   * @param maxObjects - max objects a node can hold before splitting (default: 10)
   * @param maxLevels - total max levels inside root Quadtree (default: 4)
   * @param level - depth level, required for subnodes (default: 0)
   */
  constructor(
    bounds: Rect,
    maxObjects: number = 10,
    maxLevels: number = 4,
    level: number = 0
  ) {
    this.maxObjects = maxObjects;
    this.maxLevels = maxLevels;
    this.level = level;
    this.bounds = bounds;
    this.objects = [];
    this.nodes = [];
  }

  /**
   * Split the node into 4 subnodes
   */
  split(): void {
    const nextLevel = this.level + 1;
    const subWidth = this.bounds.width / 2;
    const subHeight = this.bounds.height / 2;
    const x = this.bounds.x;
    const y = this.bounds.y;

    // top right node
    this.nodes[0] = new Quadtree<T>(
      {
        x: x + subWidth,
        y: y,
        width: subWidth,
        height: subHeight,
      },
      this.maxObjects,
      this.maxLevels,
      nextLevel
    );

    // top left node
    this.nodes[1] = new Quadtree<T>(
      {
        x: x,
        y: y,
        width: subWidth,
        height: subHeight,
      },
      this.maxObjects,
      this.maxLevels,
      nextLevel
    );

    // bottom left node
    this.nodes[2] = new Quadtree<T>(
      {
        x: x,
        y: y + subHeight,
        width: subWidth,
        height: subHeight,
      },
      this.maxObjects,
      this.maxLevels,
      nextLevel
    );

    // bottom right node
    this.nodes[3] = new Quadtree<T>(
      {
        x: x + subWidth,
        y: y + subHeight,
        width: subWidth,
        height: subHeight,
      },
      this.maxObjects,
      this.maxLevels,
      nextLevel
    );
  }

  /**
   * Determine which node the object belongs to
   * @param pRect - bounds of the area to be checked
   * @returns an array of indexes of the intersecting subnodes (0-3 = top-right, top-left, bottom-left, bottom-right)
   */
  getIndex(pRect: Rect): number[] {
    const indexes: number[] = [];
    const verticalMidpoint = this.bounds.x + this.bounds.width / 2;
    const horizontalMidpoint = this.bounds.y + this.bounds.height / 2;

    const startIsNorth = pRect.y < horizontalMidpoint;
    const startIsWest = pRect.x < verticalMidpoint;
    const endIsEast = pRect.x + pRect.width > verticalMidpoint;
    const endIsSouth = pRect.y + pRect.height > horizontalMidpoint;

    // top-right quad
    if (startIsNorth && endIsEast) {
      indexes.push(0);
    }

    // top-left quad
    if (startIsWest && startIsNorth) {
      indexes.push(1);
    }

    // bottom-left quad
    if (startIsWest && endIsSouth) {
      indexes.push(2);
    }

    // bottom-right quad
    if (endIsEast && endIsSouth) {
      indexes.push(3);
    }

    return indexes;
  }

  /**
   * Insert the object into the node. If the node exceeds the capacity,
   * it will split and add all objects to their corresponding subnodes.
   * @param pRect - bounds of the object to be added
   */
  insert(pRect: T): void {
    let indexes: number[];

    // if we have subnodes, call insert on matching subnodes
    if (this.nodes.length) {
      indexes = this.getIndex(pRect);

      for (let i = 0; i < indexes.length; i++) {
        this.nodes[indexes[i]].insert(pRect);
      }
      return;
    }

    // otherwise, store object here
    this.objects.push(pRect);

    // max_objects reached
    if (this.objects.length > this.maxObjects && this.level < this.maxLevels) {
      // split if we don't already have subnodes
      if (!this.nodes.length) {
        this.split();
      }

      // add all objects to their corresponding subnode
      for (let i = 0; i < this.objects.length; i++) {
        indexes = this.getIndex(this.objects[i]);
        for (let k = 0; k < indexes.length; k++) {
          this.nodes[indexes[k]].insert(this.objects[i]);
        }
      }

      // clean up this node
      this.objects = [];
    }
  }

  /**
   * Return all objects that could collide with the given object
   * @param pRect - bounds of the object to be checked
   * @returns array with all detected objects
   */
  retrieve(pRect: Rect): T[] {
    const indexes = this.getIndex(pRect);
    let returnObjects = this.objects.slice();

    // if we have subnodes, retrieve their objects
    if (this.nodes.length) {
      for (let i = 0; i < indexes.length; i++) {
        returnObjects = returnObjects.concat(this.nodes[indexes[i]].retrieve(pRect));
      }
    }

    // remove duplicates
    returnObjects = returnObjects.filter((item, index) => {
      return returnObjects.indexOf(item) >= index;
    });

    return returnObjects;
  }

  /**
   * Clear the quadtree
   */
  clear(): void {
    this.objects = [];

    for (let i = 0; i < this.nodes.length; i++) {
      if (this.nodes.length) {
        this.nodes[i].clear();
      }
    }

    this.nodes = [];
  }
}

// Export for window global (backwards compatibility)
if (typeof window !== 'undefined') {
  (window as unknown as { Quadtree: typeof Quadtree }).Quadtree = Quadtree;
}
