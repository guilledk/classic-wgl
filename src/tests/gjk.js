import { GJK } from "/lib/gjk.js";
import { vec3 } from "/lib/gl-matrix/index.js";


function assert(bool, msg) {
    if (!bool)
        throw "Assertion failed: " + msg;
}


function versorRad(rad) {
    return vec3.fromValues(Math.cos(rad), -Math.sin(rad), 0);
}

const topLeft = [0, 0, 0];
const topRight = [1, 0, 0];
const botRight = [1, 1, 0];
const botLeft = [0, 1, 0];
const rectVerts = [topLeft, topRight, botRight, botLeft];


class Shape {
    constructor(pos, verts) {
        this.pos = pos;
        this.verts = verts;
    }

    support(dir) {
        let res = vec3.clone(this.verts[0]);
        let d = vec3.dot(dir, res);
        for (const vert of this.verts) {
            let c = vec3.dot(dir, vert);
            if (c > d) {
                d = c;
                res = vec3.clone(vert);
            }
        }
        vec3.add(res, res, this.pos);
        return res;
    }
};


function testShapeSupport() {

    let rect = new Shape(
        [0, 0, 0], rectVerts);

    let angle = Math.PI / 4;

    assert(vec3.equals(rect.support(versorRad(angle)), topRight), "wrong support for topRight dir");
    assert(vec3.equals(rect.support(versorRad(angle * 3)), topLeft), "wrong support for topLeft dir");
    assert(vec3.equals(rect.support(versorRad(angle * 5)), botLeft), "wrong support for botLeft dir");
    assert(vec3.equals(rect.support(versorRad(angle * 7)), botRight), "wrong support for botRight dir");

    return "PASSED"
}


function testGJKCollision_true() {
    
    const shape1 = new Shape(
        [0, 0, 0], rectVerts);

    const shape2 = new Shape(
        [-0.5, -0.5, 0], rectVerts);

    const res = GJK(shape1, shape2);

    assert(res == null, "failed GJK Shape collision check");
    
    return "PASSED"
}


function runTest(testFn, resultElement) {
    try {
        testFn();
        resultElement.innerHTML = "<p style=\"color:green;\">PASSED</p>"
    } catch(error) {
        console.error(error);
        resultElement.innerHTML = "<p style=\"color:red;\">FAILED</p>";
    }
}


runTest(testShapeSupport, document.getElementById("label_test_support"));
runTest(testGJKCollision_true, document.getElementById("label_test_gjk"));

