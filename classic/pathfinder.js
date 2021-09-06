
/*
 *  Pathfinding Message Protocol
 *
 *  request: {
 *      op: '{initmap|updatemap|findpath}',
 *      args: {
 *          (if initmap)
 *          name: "{map name}",
 *          size: vec2,
 *          data: [2d array of bools (walkable info)]
 *
 *          (if updatemap)
 *          name: "{map name}",
 *          corner: vec2,
 *          size: vec2,
 *          data: [2d array of bools (walkable info)]
 *
 *          (if findpath)
 *          name: "{map name}",
 *          from: vec2,
 *          to: vec2
 *      },
 *      id: {int id}
 *  }
 *
 *  response: { id: {int id}, status: {ok} }
 *
 */

function vecAdd(a, b) {
    return [a[0] + b[0], a[1] + b[1]];
}

function vecDistance(a, b) {
    const deltaX = a[0] - b[0];
    const deltaY = a[1] - b[1];
    return (deltaX * deltaX) + (deltaY * deltaY);
}

function vecIsEqual(a, b) {
    return (a[0] == b[0]) && (a[1] == b[1]);
}

/*
 *
 *  Data Structures
 *
 */

const top = 0;
const parent = i => ((i + 1) >>> 1) - 1;
const left = i => (i << 1) + 1;
const right = i => (i + 1) << 1;

class PriorityQueue {
    constructor(comparator = (a, b) => a > b) {
        this._heap = [];
        this._dict = {};
        this._comparator = comparator;
    }
    size() {
        return this._heap.length;
    }
    isEmpty() {
        return this.size() == 0;
    }
    peek() {
        return this._heap[top];
    }
    push(...values) {
        values.forEach(value => {
            this._heap.push(value);
            this._siftUp();
        });
        return this.size();
    }
    pop() {
        const poppedValue = this.peek();
        const bottom = this.size() - 1;
        if (bottom > top) {
        this._swap(top, bottom);
        }
        this._heap.pop();
        this._siftDown();
        return poppedValue;
    }
    replace(value) {
        const replacedValue = this.peek();
        this._heap[top] = value;
        this._siftDown();
        return replacedValue;
    }
    _greater(i, j) {
        return this._comparator(this._heap[i], this._heap[j]);
    }
    _swap(i, j) {
        [this._heap[i], this._heap[j]] = [this._heap[j], this._heap[i]];
    }
    _siftUp() {
        let node = this.size() - 1;
        while (node > top && this._greater(node, parent(node))) {
        this._swap(node, parent(node));
        node = parent(node);
        }
    }
    _siftDown() {
        let node = top;
        while (
        (left(node) < this.size() && this._greater(left(node), node)) ||
        (right(node) < this.size() && this._greater(right(node), node))
        ) {
        let maxChild = (right(node) < this.size() && this._greater(right(node), left(node))) ? right(node) : left(node);
        this._swap(node, maxChild);
        node = maxChild;
        }
    }
}

class Map2D {
    constructor(size, def) {
        this._map = new Array(size[0] * size[1]).fill(def);
        this.size = size;
    }

    flattenIndex(pos) {
        return pos[0] + (pos[1] * this.size[0]);
    }

    get(pos) {
        return this._map[this.flattenIndex(pos)];
    }

    set(pos, value) {
        this._map[this.flattenIndex(pos)] = value;
    }
};


var maps = {};

function flattenIndex(size, x, y) {
    return x + (y * size[0]);
}

function aStarPath(mapName, from, to) {
    const map = maps[mapName];
    let gCosts = new Map2D(map.size, Number.MAX_SAFE_INTEGER);
    let fCosts = new Map2D(map.size, Number.MAX_SAFE_INTEGER);
    let inOpen = new Map2D(map.size, false); 
    let cameFrom = new Map2D(map.size, false);

    const neighbours = [
        [-1, -1],
        [ 0, -1],
        [ 1, -1],
        [-1,  0],
        [ 1,  0],
        [-1,  1],
        [ 0,  1],
        [ 1,  1]
    ];

    function isWalkable(node) {
        return map.data[flattenIndex(map.size, node[0], node[1])];
    }

    function reconstructPath(node) {
        let current = node;
        let path = new Array();
        do {
            current = cameFrom.get(current);
            path.unshift(current);
        } while(!vecIsEqual(current, from) && current != false);

        return path;
    }

    gCosts.set(from, 0);
    fCosts.set(from, vecDistance(from, to));

    const fCostOrder = (a, b) => fCosts.get(a) < fCosts.get(b);

    const open = new PriorityQueue(fCostOrder);
    open.push(from);

    while (!open.isEmpty()) {
        const current = open.pop();
        inOpen.set(current, false);

        if (vecIsEqual(current, to))
            return reconstructPath(current);
        
        for (let neighbour of neighbours) {
            neighbour = vecAdd(neighbour, current);
            if (neighbour[0] < 0 ||
                neighbour[0] > map.size[0] ||
                neighbour[1] < 0 ||
                neighbour[1] > map.size[1] ||
                !isWalkable(neighbour))
                continue;

            const score = gCosts.get(current)
                + vecDistance(current, neighbour);

            if (score < gCosts.get(neighbour)) {
                cameFrom.set(neighbour, current);
                gCosts.set(neighbour, score);
                fCosts.set(
                    neighbour,
                    score + vecDistance(neighbour, to));
               
                if (!inOpen.get(neighbour)) {
                    open.push(neighbour);
                    inOpen.set(neighbour, true);
                }
            }
        }
    }

    return null;
}

onmessage = function(e) {

    const msg = e.data;

    switch (msg.op) {

        case "initmap": {
            maps[msg.args.name] = {
                size: msg.args.size,
                data: msg.args.data
            };

            postMessage({ id: msg.id, data: "ok" });
            break;
        }

        case "updatemap": {
            const map = maps[msg.args.name];
            const corner = msg.args.corner;

            for (var y = 0; y < msg.args.size[1]; y++)
                for (var x = 0; x < msg.args.size[0]; x++)
                    map.data[
                        flattenIndex(
                            map.size,
                            corner[0] + x, corner[1] + y)
                    ] = msg.args.data[
                        flattenIndex(msg.args.size, x, y)];

            postMessage({ id: msg.id, data: "ok" });
            break;
        }

        case "findpath": {
            const map = msg.args.name;
            const from = msg.args.from;
            const to = msg.args.to;

            postMessage({ id: msg.id, data: aStarPath(map, from, to) });
            break;
        }

    }

}
