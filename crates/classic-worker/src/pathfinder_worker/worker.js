// classic-worker pathfinder worker (web): runs A* over a shared nav snapshot.
//
// A faithful mirror of `classic_core::pathfinder::find_path` (itself a port of
// the retired TypeScript `pathfinder.ts`), so native and web agree on routes.
// Messages (from the main thread):
//   { type: "snapshot", sizeX, sizeY, data: Int32Array }  — replace the grid
//   { type: "find", id, from: [x, y], to: [x, y] }         — run a search
// Replies (to the main thread):
//   { type: "result", id, path: Int32Array | null }        — flat [x0,y0,…]
//      where `path` is null when no route exists.

var nav = null;
var SQRT2 = Math.SQRT2;

// Max-heap keyed by `before` (higher priority pops first), mirroring the Rust
// `BinaryHeap<Key>` ordering: lowest f first, then largest (x, then y).
function before(a, b) {
    if (a.f !== b.f) return a.f < b.f;
    if (a.x !== b.x) return a.x > b.x;
    return a.y > b.y;
}

function makeHeap() {
    var items = [];
    return {
        push: function (item) {
            items.push(item);
            var i = items.length - 1;
            while (i > 0) {
                var p = (i - 1) >> 1;
                if (before(items[i], items[p])) {
                    var t = items[i];
                    items[i] = items[p];
                    items[p] = t;
                    i = p;
                } else {
                    break;
                }
            }
        },
        pop: function () {
            if (items.length === 0) return null;
            var top = items[0];
            var last = items.pop();
            if (items.length > 0) {
                items[0] = last;
                var i = 0;
                for (;;) {
                    var l = i * 2 + 1;
                    var r = l + 1;
                    var best = i;
                    if (l < items.length && before(items[l], items[best])) best = l;
                    if (r < items.length && before(items[r], items[best])) best = r;
                    if (best === i) break;
                    var t = items[i];
                    items[i] = items[best];
                    items[best] = t;
                    i = best;
                }
            }
            return top;
        },
    };
}

function heuristic(ax, ay, bx, by) {
    var dx = Math.abs(ax - bx);
    var dy = Math.abs(ay - by);
    return dx + dy + (SQRT2 - 2) * Math.min(dx, dy);
}

function findPath(sizeX, sizeY, data, fromX, fromY, toX, toY) {
    fromX = Math.max(0, Math.min(sizeX - 1, fromX));
    fromY = Math.max(0, Math.min(sizeY - 1, fromY));
    toX = Math.max(0, Math.min(sizeX - 1, toX));
    toY = Math.max(0, Math.min(sizeY - 1, toY));

    function flatten(x, y) {
        return x + y * sizeX;
    }

    if (fromX === toX && fromY === toY) {
        return [fromX, fromY, toX, toY];
    }

    var total = sizeX * sizeY;
    var INF = Infinity;
    var gCost = new Float64Array(total).fill(INF);
    var fCost = new Float64Array(total).fill(INF);
    var cameFrom = new Int32Array(total).fill(-1);
    var inOpen = new Uint8Array(total);

    var fromIdx = flatten(fromX, fromY);
    gCost[fromIdx] = 0;
    fCost[fromIdx] = heuristic(fromX, fromY, toX, toY);

    var open = makeHeap();
    open.push({ f: fCost[fromIdx], x: fromX, y: fromY });

    var neighbours = [
        [-1, -1], [0, -1], [1, -1],
        [-1, 0], [1, 0],
        [-1, 1], [0, 1], [1, 1],
    ];

    var toIdx = flatten(toX, toY);

    for (;;) {
        var current = open.pop();
        if (current === null) return null;
        var cx = current.x;
        var cy = current.y;
        var curIdx = flatten(cx, cy);

        if (cx === toX && cy === toY) {
            var path = [toX, toY];
            var cur = toIdx;
            while (cur !== fromIdx) {
                var prev = cameFrom[cur];
                if (prev < 0) break;
                path.push(prev % sizeX, Math.floor(prev / sizeX));
                cur = prev;
            }
            path.reverse();
            return path;
        }

        inOpen[curIdx] = 0;

        for (var n = 0; n < 8; n++) {
            var nx = cx + neighbours[n][0];
            var ny = cy + neighbours[n][1];
            if (nx < 0 || nx >= sizeX || ny < 0 || ny >= sizeY) continue;
            var nIdx = flatten(nx, ny);
            if (data[nIdx] === 0) continue;

            var stepCost = neighbours[n][0] !== 0 && neighbours[n][1] !== 0 ? SQRT2 : 1;
            var tentativeG = gCost[curIdx] + stepCost;

            if (tentativeG < gCost[nIdx]) {
                cameFrom[nIdx] = curIdx;
                gCost[nIdx] = tentativeG;
                fCost[nIdx] = tentativeG + heuristic(nx, ny, toX, toY);
                if (!inOpen[nIdx]) {
                    open.push({ f: fCost[nIdx], x: nx, y: ny });
                    inOpen[nIdx] = 1;
                }
            }
        }
    }
}

self.onmessage = function (e) {
    var msg = e.data;
    if (msg.type === "snapshot") {
        nav = { sizeX: msg.sizeX, sizeY: msg.sizeY, data: msg.data };
    } else if (msg.type === "find") {
        if (nav === null) {
            self.postMessage({ type: "result", id: msg.id, path: null });
            return;
        }
        var path = findPath(
            nav.sizeX,
            nav.sizeY,
            nav.data,
            msg.from[0],
            msg.from[1],
            msg.to[0],
            msg.to[1],
        );
        self.postMessage({ type: "result", id: msg.id, path: path });
    }
};
