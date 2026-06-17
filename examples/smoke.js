// Basic smoke test for quickrs
console.log("Hello from quickrs!");

// Arithmetic & operators
console.log(1 + 2 * 3);
console.log(2 ** 10);
console.log(10 % 3);
console.log(5 & 3, 5 | 2, 5 ^ 1, ~5);
console.log(1 < 2, 2 <= 2, 3 > 4, "a" < "b");
console.log(1 == "1", 1 === "1", null == undefined);

// Variables, closures, scopes
let counter = (function() {
  let n = 0;
  return function() { return ++n; };
})();
console.log(counter(), counter(), counter());

// Strings
let s = "Hello, World";
console.log(s.length, s.toUpperCase(), s.slice(0, 5), s.includes("World"));
console.log(s.split(", ").map(x => x.toUpperCase()));

// Arrays
let arr = [1, 2, 3, 4, 5];
console.log(arr.map(x => x * 2).filter(x => x > 4).reduce((a, b) => a + b, 0));
console.log([...arr, 6, 7].length);
console.log(Math.max(...arr));

// Objects & destructuring
let {a, b: renamed, c = 30} = {a: 1, b: 2};
console.log(a, renamed, c);
let [x, , z] = [10, 20, 30];
console.log(x, z);

// Classes
class Animal {
  constructor(name) { this.name = name; }
  speak() { return this.name + " makes a sound"; }
}
class Dog extends Animal {
  speak() { return this.name + " barks"; }
}
console.log(new Dog("Rex").speak());

// Template literals
let name = "world";
console.log(`Hello, ${name}! ${1 + 1}`);

// Generators
function* fib() {
  let [a, b] = [0, 1];
  while (true) {
    yield a;
    [a, b] = [b, a + b];
  }
}
let f = fib();
console.log(f.next().value, f.next().value, f.next().value, f.next().value, f.next().value);

// JSON
let json = JSON.stringify({a: 1, b: [2, 3], c: "hi"});
console.log(json);
console.log(JSON.parse(json).b[1]);

// Symbols & Map/Set
let m = new Map();
m.set("a", 1).set("b", 2);
console.log(m.get("a"), m.size);
let st = new Set([1, 2, 2, 3, 3, 3]);
console.log(st.size);

// for..of
let sum = 0;
for (let v of [1, 2, 3, 4]) sum += v;
console.log("sum:", sum);

// try/catch
try {
  throw new Error("oops");
} catch (e) {
  console.log("caught:", e.message);
}

// Promises + async/await
async function asyncMain() {
  let p = new Promise((resolve) => setTimeout(() => resolve(42), 50));
  let val = await p;
  console.log("async result:", val);
  return val;
}
asyncMain().then(v => console.log("done:", v));
