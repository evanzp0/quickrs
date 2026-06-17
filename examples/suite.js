// Comprehensive bug-hunt test suite for quickrs
let pass = 0, fail = 0;
function check(label, actual, expected) {
  const a = JSON.stringify(actual);
  const e = JSON.stringify(expected);
  if (a === e) { pass++; }
  else { fail++; console.log(`FAIL: ${label}\n  expected: ${e}\n  actual:   ${a}`); }
}
function checkEq(label, actual, expected) { check(label, actual, expected); }

// 1. Number formatting edge cases
check("0.1+0.2", 0.1+0.2, 0.30000000000000004);
check("1e21", 1e21, 1e21);
check("1e-7", 1e-7, 1e-7);
check("-0", 1/-Infinity, -0);
check("NaN", NaN, NaN);
check("toInt", (10.7)|0, 10);
check("mod negative", -7 % 3, -1);

// 2. String edge cases
check("charCodeAt OOB", "abc".charCodeAt(10), NaN);
check("slice neg", "hello".slice(-2), "lo");
check("substring swap", "hello".substring(3, 1), "el");
check("split limit", "a,b,c,d".split(",", 2), ["a","b"]);
check("padStart", "5".padStart(3, "0"), "005");
check("repeat 0", "x".repeat(0), "");
check("includes fromIndex", "hello world".includes("world", 0), true);
check("replace str", "aXa".replace("X", "Y"), "aYa");
check("trim", "  x  ".trim(), "x");
check("at neg", "abc".at(-1), "c");

// 3. Array edge cases
check("splice", [1,2,3,4].splice(1,2), [2,3]);
check("flat depth", [1,[2,[3,[4]]]].flat(2), [1,2,3,[4]]);
check("sort numeric", [10,2,30,1].sort((a,b)=>a-b), [1,2,10,30]);
check("reduce empty", [1].reduce((a,b)=>a+b,0), 1);
check("Array.from set", Array.from(new Set([1,1,2,3,2])), [1,2,3]);
check("Array.from mapfn", Array.from([1,2,3], x=>x*2), [2,4,6]);
check("indexOf from", [1,2,1,2].indexOf(1,1), 2);
check("keys", [...[10,20,30].keys()], [0,1,2]);
check("entries", [...[10,20].entries()], [[0,10],[1,20]]);
check("findIndex none", [1,2].findIndex(x=>x>5), -1);
check("copyWithin", [1,2,3,4,5].copyWithin(0,3), [4,5,3,4,5]);

// 4. Object
check("assign", Object.assign({}, {a:1},{b:2}), {a:1,b:2});
check("entries", Object.entries({a:1,b:2}), [["a",1],["b",2]]);
check("fromEntries", Object.fromEntries([["a",1],["b",2]]), {a:1,b:2});
check("freeze", (()=>{ const o=Object.freeze({a:1}); o.a=2; return o.a; })(), 1);
check("keys proto", Object.keys(Object.create({inherited:1})), []);

// 5. Destructuring
check("nested", (({a:{b}})=>b)({a:{b:5}}), 5);
check("default", (({x=10})=>x)({}), 10);
check("rest obj", (({a, ...r})=>r)({a:1,b:2,c:3}), {b:2,c:3});
check("swap", (([a,b])=>[b,a])([1,2]), [2,1]);

// 6. Closures / scoping
check("closure", (()=>{ let i=0; return ()=>++i; })()(), 1);
check("var hoist", (()=>{ x=5; var x; return x; })(), 5);
check("let TDZ", (()=>{ try { x; let x; return "no-throw"; } catch(e) { return "threw"; } })(), "threw");
check("block scope", (()=>{ { let x=1; } try { return x; } catch(e){ return "err"; } })(), "err");

// 7. Classes
check("class static", (()=>{ class C{ static s=42; } return C.s; })(), 42);
check("class getter", (()=>{ class C{ get x(){return 99;} } return new C().x; })(), 99);
check("class method", (()=>{ class C{ m(){return 1;} } return new C().m(); })(), 1);
check("class extends", (()=>{ class A{ f(){return "a";} } class B extends A{ f(){return super.f()+"b";} } return new B().f(); })(), "ab");
check("class instanceof", (()=>{ class A{} class B extends A{} return new B() instanceof A; })(), true);

// 8. Generators
check("gen basic", (()=>{ function* g(){yield 1;yield 2;yield 3;} return [...g()]; })(), [1,2,3]);
check("gen return", (()=>{ function* g(){yield 1;return 2;yield 3;} let it=g(); return [it.next().value, it.next().value, it.next().done]; })(), [1,2,true]);
check("gen delegate", (()=>{ function* a(){yield 1;yield 2;} function* b(){yield 0;yield* a();yield 3;} return [...b()]; })(), [0,1,2,3]);
check("gen take", (()=>{ function* nat(){let i=0;while(true)yield i++;} let r=[]; for(let v of nat()){ r.push(v); if(r.length>=3) break; } return r; })(), [0,1,2]);

// 9. Iterators
check("Symbol.iterator", (()=>{ let o={[Symbol.iterator](){let i=0;return{next:()=>({value:i++,done:i>3})}}}; return [...o]; })(), [0,1,2]);
check("Map iter", (()=>{ let m=new Map([["a",1],["b",2]]); return [...m]; })(), [["a",1],["b",2]]);
check("Set iter", (()=>{ let s=new Set([1,2,3]); return [...s]; })(), [1,2,3]);
check("String iter", (()=>{ return [..."abc"]; })(), ["a","b","c"]);

// 10. Promises (sync checks only)
check("Promise.resolve", (()=>{ let r; Promise.resolve(5).then(v=>r=v); return r; })(), undefined);

// 11. JSON
check("JSON.stringify num", JSON.stringify(42), "42");
check("JSON.stringify str", JSON.stringify("hi"), '"hi"');
check("JSON.stringify null", JSON.stringify(null), "null");
check("JSON.stringify bool", JSON.stringify(true), "true");
check("JSON.stringify arr", JSON.stringify([1,2,3]), "[1,2,3]");
check("JSON.stringify nested", JSON.stringify({a:[1,{b:2}]}), '{"a":[1,{"b":2}]}');
check("JSON.parse", JSON.parse('{"a":1,"b":[2,3]}'), {a:1,b:[2,3]});
check("JSON.parse null", JSON.parse("null"), null);

// 12. Math
check("Math.max", Math.max(1,2,3), 3);
check("Math.min", Math.min(1,2,3), 1);
check("Math.floor", Math.floor(2.7), 2);
check("Math.abs", Math.abs(-5), 5);
check("Math.pow", Math.pow(2,10), 1024);
check("Math.sqrt", Math.sqrt(16), 4);
check("Math.max spread", Math.max(...[1,5,3]), 5);
check("Math.round half", Math.round(0.5), 1);
check("Math.round neg half", Math.round(-0.5), 0);

// 13. Number methods
check("toFixed", (3.14159).toFixed(2), "3.14");
check("toString radix", (255).toString(16), "ff");
check("Number.isInteger", Number.isInteger(5), true);
check("Number.isInteger", Number.isInteger(5.5), false);
check("parseInt hex", parseInt("0xff"), 255);
check("parseInt radix", parseInt("10", 2), 2);
check("parseFloat", parseFloat("3.14abc"), 3.14);

// 14. Globals
check("parseInt", parseInt("42"), 42);
check("isNaN", isNaN(NaN), true);
check("isFinite", isFinite(5), true);
check("isFinite", isFinite(Infinity), false);
check("encodeURI", encodeURI("a b"), "a%20b");
check("encodeURIComponent", encodeURIComponent("a/b"), "a%2Fb");

// 15. typeof / instanceof
check("typeof", [typeof 1, typeof "x", typeof true, typeof undefined, typeof null, typeof {}, typeof function(){}], ["number","string","boolean","undefined","object","object","function"]);
check("instanceof", (()=>{ class A{}; let a=new A(); return a instanceof A; })(), true);

// 16. Spread / rest
check("spread call", ((...a)=>a)(1, ...[2,3], 4), [1,2,3,4]);
check("spread obj", {...{a:1}, b:2}, {a:1,b:2});
check("spread arr", [...[1,2], ...[3,4]], [1,2,3,4]);

// 17. Optional chaining & nullish
check("opt chain", null?.x, undefined);
check("opt chain call", null?.(), undefined);
check("opt chain method", null?.toString(), undefined);
check("nullish", null ?? "default", "default");
check("nullish 0", 0 ?? "default", 0);

// 18. Tagged templates
check("tagged", ((s, ...v)=>s.join("|")+":"+v.join(","))`a${1}b${2}c`, "a|b|c:1,2");

// 19. Logical assignment
check("||=", (()=>{ let x; x ||= 5; return x; })(), 5);
check("&&=", (()=>{ let x = 3; x &&= 7; return x; })(), 7);
check("??=", (()=>{ let x; x ??= 9; return x; })(), 9);

// 20. try/catch/finally
check("finally runs", (()=>{ let r; try { r = 1; } finally { r = 2; } return r; })(), 2);
check("catch no param", (()=>{ try { throw 1; } catch { return "ok"; } })(), "ok");

console.log(`\n=== ${pass} passed, ${fail} failed ===`);
