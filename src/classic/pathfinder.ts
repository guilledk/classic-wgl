/**
 * Pathfinding Web Worker
 *
 * Pathfinding Message Protocol:
 *
 * request: {
 *     op: '{initmap|updatemap|findpath}',
 *     args: {
 *         (if initmap)
 *         name: "{map name}",
 *         size: vec2,
 *         data: [2d array of bools (walkable info)]
 *
 *         (if updatemap)
 *         name: "{map name}",
 *         corner: vec2,
 *         size: vec2,
 *         data: [2d array of bools (walkable info)]
 *
 *         (if findpath)
 *         name: "{map name}",
 *         from: vec2,
 *         to: vec2
 *     },
 *     id: {int id}
 * }
 *
 * response: { id: {int id}, data: {result} }
 */

type Vec2 = [number, number];

interface MapData {
  size: Vec2;
  data: number[];
}

interface InitMapArgs {
  name: string;
  size: Vec2;
  data: number[];
}

interface UpdateMapArgs {
  name: string;
  corner: Vec2;
  size: Vec2;
  data: number[];
}

interface FindPathArgs {
  name: string;
  from: Vec2;
  to: Vec2;
}

interface WorkerMessage {
  op: 'initmap' | 'updatemap' | 'findpath';
  args: InitMapArgs | UpdateMapArgs | FindPathArgs;
  id: number;
}

interface WorkerResponse {
  id: number;
  data: string | Vec2[] | null;
}

// Vector utilities
function vecAdd(a: Vec2, b: Vec2): Vec2 {
  return [a[0] + b[0], a[1] + b[1]];
}

function vecFloor(v: Vec2): void {
  v[0] = Math.floor(v[0]);
  v[1] = Math.floor(v[1]);
}

function vecDistance(a: Vec2, b: Vec2): number {
  const deltaX = a[0] - b[0];
  const deltaY = a[1] - b[1];
  return deltaX * deltaX + deltaY * deltaY;
}

function vecIsEqual(a: Vec2, b: Vec2): boolean {
  return a[0] === b[0] && a[1] === b[1];
}

// Data Structures

const TOP_INDEX = 0;
const getParentIndex = (i: number): number => ((i + 1) >>> 1) - 1;
const getLeftIndex = (i: number): number => (i << 1) + 1;
const getRightIndex = (i: number): number => (i + 1) << 1;

class PriorityQueue<T> {
  private _heap: T[];
  private _comparator: (a: T, b: T) => boolean;

  constructor(comparator: (a: T, b: T) => boolean = (a, b) => a > b) {
    this._heap = [];
    this._comparator = comparator;
  }

  size(): number {
    return this._heap.length;
  }

  isEmpty(): boolean {
    return this.size() === 0;
  }

  peek(): T {
    return this._heap[TOP_INDEX];
  }

  push(...values: T[]): number {
    values.forEach((value) => {
      this._heap.push(value);
      this._siftUp();
    });
    return this.size();
  }

  pop(): T {
    const poppedValue = this.peek();
    const bottom = this.size() - 1;
    if (bottom > TOP_INDEX) {
      this._swap(TOP_INDEX, bottom);
    }
    this._heap.pop();
    this._siftDown();
    return poppedValue;
  }

  replace(value: T): T {
    const replacedValue = this.peek();
    this._heap[TOP_INDEX] = value;
    this._siftDown();
    return replacedValue;
  }

  private _greater(i: number, j: number): boolean {
    return this._comparator(this._heap[i], this._heap[j]);
  }

  private _swap(i: number, j: number): void {
    [this._heap[i], this._heap[j]] = [this._heap[j], this._heap[i]];
  }

  private _siftUp(): void {
    let node = this.size() - 1;
    while (node > TOP_INDEX && this._greater(node, getParentIndex(node))) {
      this._swap(node, getParentIndex(node));
      node = getParentIndex(node);
    }
  }

  private _siftDown(): void {
    let node = TOP_INDEX;
    while (
      (getLeftIndex(node) < this.size() && this._greater(getLeftIndex(node), node)) ||
      (getRightIndex(node) < this.size() && this._greater(getRightIndex(node), node))
    ) {
      const maxChild =
        getRightIndex(node) < this.size() && this._greater(getRightIndex(node), getLeftIndex(node))
          ? getRightIndex(node)
          : getLeftIndex(node);
      this._swap(node, maxChild);
      node = maxChild;
    }
  }
}

class Map2D<T> {
  private _map: T[];
  size: Vec2;

  constructor(size: Vec2, def: T) {
    this._map = new Array(size[0] * size[1]).fill(def);
    this.size = size;
  }

  flattenIndex(pos: Vec2): number {
    return pos[0] + pos[1] * this.size[0];
  }

  get(pos: Vec2): T {
    return this._map[this.flattenIndex(pos)];
  }

  set(pos: Vec2, value: T): void {
    this._map[this.flattenIndex(pos)] = value;
  }
}

const maps: Record<string, MapData> = {};

function flattenIndex(size: Vec2, x: number, y: number): number {
  return x + y * size[0];
}

function aStarPath(mapName: string, from: Vec2, to: Vec2): Vec2[] | null {
  const map = maps[mapName];
  const gCosts = new Map2D<number>(map.size, Number.MAX_SAFE_INTEGER);
  const fCosts = new Map2D<number>(map.size, Number.MAX_SAFE_INTEGER);
  const inOpen = new Map2D<boolean>(map.size, false);
  const cameFrom = new Map2D<Vec2 | false>(map.size, false);

  vecFloor(from);
  vecFloor(to);

  const neighbours: Vec2[] = [
    [-1, -1],
    [0, -1],
    [1, -1],
    [-1, 0],
    [1, 0],
    [-1, 1],
    [0, 1],
    [1, 1],
  ];

  function isWalkable(node: Vec2): boolean {
    return !!map.data[flattenIndex(map.size, node[0], node[1])];
  }

  function reconstructPath(node: Vec2): Vec2[] {
    let current: Vec2 | false = node;
    const path: Vec2[] = [];
    do {
      current = cameFrom.get(current as Vec2);
      if (current !== false) {
        path.unshift(current);
      }
    } while (current !== false && !vecIsEqual(current, from));

    return path;
  }

  gCosts.set(from, 0);
  fCosts.set(from, vecDistance(from, to));

  const fCostOrder = (a: Vec2, b: Vec2): boolean =>
    fCosts.get(a) < fCosts.get(b);

  const open = new PriorityQueue<Vec2>(fCostOrder);
  open.push(from);

  while (!open.isEmpty()) {
    const current = open.pop();
    inOpen.set(current, false);

    if (vecIsEqual(current, to)) {
      return reconstructPath(current);
    }

    for (const neighbourOffset of neighbours) {
      const neighbour = vecAdd(neighbourOffset, current);
      if (
        neighbour[0] < 0 ||
        neighbour[0] > map.size[0] ||
        neighbour[1] < 0 ||
        neighbour[1] > map.size[1] ||
        !isWalkable(neighbour)
      ) {
        continue;
      }

      const score = gCosts.get(current) + vecDistance(current, neighbour);

      if (score < gCosts.get(neighbour)) {
        cameFrom.set(neighbour, current);
        gCosts.set(neighbour, score);
        fCosts.set(neighbour, score + vecDistance(neighbour, to));

        if (!inOpen.get(neighbour)) {
          open.push(neighbour);
          inOpen.set(neighbour, true);
        }
      }
    }
  }

  return null;
}

// Worker message handler
onmessage = function (e: MessageEvent<WorkerMessage>): void {
  const msg = e.data;

  switch (msg.op) {
    case 'initmap': {
      const args = msg.args as InitMapArgs;
      maps[args.name] = {
        size: args.size,
        data: args.data,
      };

      console.log(
        "Init nav mesh '" + args.name + "' of size " + args.size
      );

      const response: WorkerResponse = { id: msg.id, data: 'ok' };
      postMessage(response);
      break;
    }

    case 'updatemap': {
      const args = msg.args as UpdateMapArgs;
      const map = maps[args.name];
      const corner = args.corner;

      for (let y = 0; y < args.size[1]; y++) {
        for (let x = 0; x < args.size[0]; x++) {
          map.data[flattenIndex(map.size, corner[0] + x, corner[1] + y)] =
            args.data[flattenIndex(args.size, x, y)];
        }
      }

      const response: WorkerResponse = { id: msg.id, data: 'ok' };
      postMessage(response);
      break;
    }

    case 'findpath': {
      const args = msg.args as FindPathArgs;
      const result = aStarPath(args.name, args.from, args.to);

      const response: WorkerResponse = { id: msg.id, data: result };
      postMessage(response);
      break;
    }
  }
};
