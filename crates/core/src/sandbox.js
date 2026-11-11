// rsPostman script sandbox: implements the Postman `pm.*` scripting API on top of three
// native hooks (__input, __send, __dynamic). Evaluated before every user script.
(function () {
  "use strict";
  const S = JSON.parse(__input);
  const out = { tests: [], console: [], next: undefined, skip: false, visualizer: null };
  const pending = [];
  const timers = [];

  // ------------------------------------------------------------ helpers
  function inspect(v, depth) {
    depth = depth || 0;
    if (typeof v === "string") return depth ? JSON.stringify(v) : v;
    if (v === undefined) return "undefined";
    if (v === null) return "null";
    if (typeof v === "function") return "[Function" + (v.name ? ": " + v.name : "") + "]";
    if (v instanceof Error) return v.name + ": " + v.message;
    if (v instanceof RegExp || v instanceof Date) return String(v);
    if (typeof v === "object") {
      try {
        const s = JSON.stringify(v);
        if (s !== undefined) return s;
      } catch (e) {}
      return Object.prototype.toString.call(v);
    }
    return String(v);
  }
  const show = (v) => inspect(v, 1);
  const logger = (level) =>
    function () {
      out.console.push({ level: level, message: Array.prototype.map.call(arguments, (a) => inspect(a)).join(" ") });
    };
  globalThis.console = { log: logger("log"), info: logger("info"), warn: logger("warn"), error: logger("error"), debug: logger("debug"), trace: logger("debug") };

  function typeOf(v) {
    if (v === null) return "null";
    if (Array.isArray(v)) return "array";
    if (v instanceof RegExp) return "regexp";
    if (v instanceof Date) return "date";
    if (v instanceof Error) return "error";
    if (v instanceof Map) return "map";
    if (v instanceof Set) return "set";
    if (v instanceof Promise) return "promise";
    return typeof v;
  }

  function deepEqual(a, b) {
    if (a === b) return true;
    if (typeof a === "number" && typeof b === "number") return a !== a && b !== b;
    if (a === null || b === null || typeof a !== "object" || typeof b !== "object") return false;
    if (typeOf(a) !== typeOf(b)) return false;
    if (a instanceof Date) return a.getTime() === b.getTime();
    if (a instanceof RegExp) return String(a) === String(b);
    if (Array.isArray(a)) {
      if (a.length !== b.length) return false;
      for (let i = 0; i < a.length; i++) if (!deepEqual(a[i], b[i])) return false;
      return true;
    }
    const ka = Object.keys(a), kb = Object.keys(b);
    if (ka.length !== kb.length) return false;
    for (const k of ka) if (!Object.prototype.hasOwnProperty.call(b, k) || !deepEqual(a[k], b[k])) return false;
    return true;
  }

  function subset(obj, part, deep) {
    for (const k of Object.keys(part)) {
      if (obj === null || obj === undefined || !(k in Object(obj))) return false;
      if (deep ? !deepEqual(obj[k], part[k]) : obj[k] !== part[k]) {
        if (!(deep && typeof part[k] === "object" && part[k] !== null && typeof obj[k] === "object" && obj[k] !== null && subset(obj[k], part[k], true))) return false;
      }
    }
    return true;
  }

  function getPath(obj, path) {
    const parts = String(path).replace(/\[(\w+)\]/g, ".$1").split(".").filter((p) => p !== "");
    let cur = obj;
    for (const p of parts) {
      if (cur === null || cur === undefined || !(p in Object(cur))) return { found: false };
      cur = cur[p];
    }
    return { found: true, value: cur };
  }

  class AssertionError extends Error {
    constructor(message, expected, actual) {
      super(message);
      this.name = "AssertionError";
      this.expected = expected;
      this.actual = actual;
    }
  }

  // ------------------------------------------------------------ chai-like expect
  function Assertion(value, message) {
    this._ = { v: value, not: false, deep: false, own: false, nested: false, any: false, all: false, contains: false, length: false, msg: message };
  }
  const AP = Assertion.prototype;

  AP._assert = function (ok, positive, negative, expected, actual) {
    const st = this._;
    if (st.not ? ok : !ok) {
      const text = (st.msg ? st.msg + ": " : "") + (st.not ? negative : positive);
      throw new AssertionError(text, expected, actual);
    }
    st.not = false;
    return this;
  };
  AP._subject = function () {
    const st = this._;
    if (st.length) {
      st.length = false;
      const v = st.v;
      return v instanceof Map || v instanceof Set ? v.size : v.length;
    }
    return st.v;
  };

  function chainable(self, fn) {
    const f = function () {
      return fn.apply(self, arguments);
    };
    Object.setPrototypeOf(f, self);
    return f;
  }
  function getter(names, fn) {
    for (const n of [].concat(names)) Object.defineProperty(AP, n, { get: fn, configurable: true });
  }
  function method(names, fn) {
    for (const n of [].concat(names)) AP[n] = fn;
  }

  getter(["to", "be", "been", "is", "that", "which", "and", "has", "have", "with", "at", "of", "same", "but", "does", "still", "also"], function () {
    return this;
  });
  getter("not", function () { this._.not = !this._.not; return this; });
  getter("deep", function () { this._.deep = true; return this; });
  getter("own", function () { this._.own = true; return this; });
  getter("nested", function () { this._.nested = true; return this; });
  getter("any", function () { this._.any = true; this._.all = false; return this; });
  getter("all", function () { this._.all = true; this._.any = false; return this; });
  getter("ordered", function () { this._.ordered = true; return this; });

  getter("ok", function () { return this._assert(!!this._.v, "expected " + show(this._.v) + " to be truthy", "expected " + show(this._.v) + " to be falsy"); });
  getter("true", function () { return this._assert(this._.v === true, "expected " + show(this._.v) + " to be true", "expected " + show(this._.v) + " to be false"); });
  getter("false", function () { return this._assert(this._.v === false, "expected " + show(this._.v) + " to be false", "expected " + show(this._.v) + " to be true"); });
  getter("null", function () { return this._assert(this._.v === null, "expected " + show(this._.v) + " to be null", "expected " + show(this._.v) + " not to be null"); });
  getter("undefined", function () { return this._assert(this._.v === undefined, "expected " + show(this._.v) + " to be undefined", "expected " + show(this._.v) + " not to be undefined"); });
  getter("NaN", function () { return this._assert(this._.v !== this._.v, "expected " + show(this._.v) + " to be NaN", "expected " + show(this._.v) + " not to be NaN"); });
  getter(["exist", "exists"], function () { return this._assert(this._.v !== null && this._.v !== undefined, "expected " + show(this._.v) + " to exist", "expected " + show(this._.v) + " to not exist"); });
  getter("empty", function () {
    const v = this._.v;
    let n;
    if (typeof v === "string" || Array.isArray(v)) n = v.length;
    else if (v instanceof Map || v instanceof Set) n = v.size;
    else if (v !== null && typeof v === "object") n = Object.keys(v).length;
    else throw new AssertionError(".empty was passed non-string primitive " + show(v));
    return this._assert(n === 0, "expected " + show(v) + " to be empty", "expected " + show(v) + " not to be empty");
  });
  getter("finite", function () { return this._assert(typeof this._.v === "number" && isFinite(this._.v), "expected " + show(this._.v) + " to be a finite number", "expected " + show(this._.v) + " to not be a finite number"); });
  getter(["arguments", "Arguments"], function () { return this._assert(Object.prototype.toString.call(this._.v) === "[object Arguments]", "expected " + show(this._.v) + " to be arguments", "expected " + show(this._.v) + " to not be arguments"); });

  function isType(type) {
    const t = String(type).toLowerCase();
    const article = /^[aeiou]/.test(t) ? "an " : "a ";
    return this._assert(typeOf(this._.v) === t, "expected " + show(this._.v) + " to be " + article + t, "expected " + show(this._.v) + " not to be " + article + t);
  }
  getter(["a", "an"], function () { return chainable(this, isType); });

  method(["equal", "equals", "eq"], function (expected) {
    if (this._.deep) return this.eql(expected);
    const v = this._subject();
    return this._assert(v === expected, "expected " + show(v) + " to equal " + show(expected), "expected " + show(v) + " to not equal " + show(expected), expected, v);
  });
  method(["eql", "eqls"], function (expected) {
    const v = this._subject();
    this._.deep = false;
    return this._assert(deepEqual(v, expected), "expected " + show(v) + " to deeply equal " + show(expected), "expected " + show(v) + " to not deeply equal " + show(expected), expected, v);
  });
  function compare(names, test, words) {
    method(names, function (n) {
      const v = this._subject();
      if (typeof v !== "number" && !(v instanceof Date)) throw new AssertionError("expected " + show(v) + " to be a number or a date");
      return this._assert(test(v, n), "expected " + show(v) + " to be " + words + " " + show(n), "expected " + show(v) + " to not be " + words + " " + show(n));
    });
  }
  compare(["above", "gt", "greaterThan"], (v, n) => v > n, "above");
  compare(["least", "gte", "greaterThanOrEqual"], (v, n) => v >= n, "at least");
  compare(["below", "lt", "lessThan"], (v, n) => v < n, "below");
  compare(["most", "lte", "lessThanOrEqual"], (v, n) => v <= n, "at most");
  method("within", function (lo, hi) {
    const v = this._subject();
    return this._assert(v >= lo && v <= hi, "expected " + show(v) + " to be within " + lo + ".." + hi, "expected " + show(v) + " to not be within " + lo + ".." + hi);
  });
  method(["closeTo", "approximately"], function (expected, delta) {
    const v = this._.v;
    return this._assert(Math.abs(v - expected) <= delta, "expected " + show(v) + " to be close to " + expected + " +/- " + delta, "expected " + show(v) + " not to be close to " + expected + " +/- " + delta);
  });
  method(["instanceof", "instanceOf"], function (ctor) {
    return this._assert(this._.v instanceof ctor, "expected " + show(this._.v) + " to be an instance of " + (ctor.name || "the constructor"), "expected " + show(this._.v) + " to not be an instance of " + (ctor.name || "the constructor"));
  });

  function includes(needle) {
    const v = this._.v, deep = this._.deep;
    this._.deep = false;
    let ok;
    if (typeof v === "string") ok = v.indexOf(String(needle)) !== -1;
    else if (Array.isArray(v)) ok = deep ? v.some((x) => deepEqual(x, needle)) : v.indexOf(needle) !== -1;
    else if (v instanceof Set) ok = v.has(needle);
    else if (v instanceof Map) ok = Array.from(v.values()).indexOf(needle) !== -1;
    else if (v !== null && typeof v === "object" && needle !== null && typeof needle === "object") ok = subset(v, needle, deep);
    else throw new AssertionError("the given combination of arguments (" + typeOf(v) + " and " + typeOf(needle) + ") is invalid for this assertion");
    return this._assert(ok, "expected " + show(v) + " to include " + show(needle), "expected " + show(v) + " to not include " + show(needle));
  }
  getter(["include", "includes", "contain", "contains"], function () {
    this._.contains = true;
    return chainable(this, includes);
  });

  function lengthIs(n) {
    this._.length = false;
    const v = this._.v;
    const len = v instanceof Map || v instanceof Set ? v.size : v === null || v === undefined ? undefined : v.length;
    return this._assert(len === n, "expected " + show(v) + " to have a length of " + n + " but got " + len, "expected " + show(v) + " to not have a length of " + n);
  }
  getter("length", function () {
    this._.length = true;
    return chainable(this, lengthIs);
  });
  method("lengthOf", lengthIs);

  method(["match", "matches"], function (re) {
    return this._assert(re.test(this._.v), "expected " + show(this._.v) + " to match " + re, "expected " + show(this._.v) + " not to match " + re);
  });
  method("string", function (s) {
    return this._assert(typeof this._.v === "string" && this._.v.indexOf(s) !== -1, "expected " + show(this._.v) + " to contain " + show(s), "expected " + show(this._.v) + " to not contain " + show(s));
  });
  method("property", function (name, value) {
    const st = this._, v = st.v;
    let found, actual;
    if (st.nested) {
      const r = getPath(v, name);
      found = r.found;
      actual = r.value;
    } else {
      found = v !== null && v !== undefined && (st.own ? Object.prototype.hasOwnProperty.call(v, name) : name in Object(v));
      actual = found ? v[name] : undefined;
    }
    st.nested = st.own = false;
    if (arguments.length > 1) {
      const same = st.deep ? deepEqual(actual, value) : actual === value;
      st.deep = false;
      this._assert(found && same, "expected " + show(v) + " to have property " + show(name) + " of " + show(value) + (found ? ", but got " + show(actual) : ""), "expected " + show(v) + " to not have property " + show(name) + " of " + show(value));
    } else {
      this._assert(found, "expected " + show(v) + " to have property " + show(name), "expected " + show(v) + " to not have property " + show(name));
    }
    st.v = actual;
    return this;
  });
  method(["ownProperty", "haveOwnProperty"], function (name, value) {
    this._.own = true;
    return arguments.length > 1 ? this.property(name, value) : this.property(name);
  });
  method(["keys", "key"], function () {
    const st = this._, v = st.v;
    let keys = arguments.length === 1 && Array.isArray(arguments[0]) ? arguments[0] : arguments.length === 1 && typeof arguments[0] === "object" ? Object.keys(arguments[0]) : Array.prototype.slice.call(arguments);
    keys = keys.map(String);
    const actual = v instanceof Map ? Array.from(v.keys()).map(String) : Object.keys(Object(v));
    let ok;
    if (st.any) ok = keys.some((k) => actual.indexOf(k) !== -1);
    else {
      ok = keys.every((k) => actual.indexOf(k) !== -1);
      if (!st.contains) ok = ok && keys.length === actual.length;
    }
    st.any = st.all = st.contains = false;
    return this._assert(ok, "expected " + show(v) + " to have keys " + show(keys), "expected " + show(v) + " to not have keys " + show(keys));
  });
  method("members", function (list) {
    const st = this._, v = st.v;
    const eq = st.deep ? deepEqual : (a, b) => a === b;
    const has = (arr, x) => arr.some((y) => eq(x, y));
    let ok;
    if (st.ordered) ok = (st.contains || v.length === list.length) && list.every((x, i) => eq(x, v[i]));
    else ok = list.every((x) => has(v, x)) && (st.contains || (v.length === list.length && v.every((x) => has(list, x))));
    st.deep = st.contains = st.ordered = false;
    return this._assert(ok, "expected " + show(v) + " to have the same members as " + show(list), "expected " + show(v) + " to not have the same members as " + show(list));
  });
  method("oneOf", function (list) {
    const v = this._.v, deep = this._.deep;
    this._.deep = false;
    const ok = this._.contains ? list.some((x) => v.indexOf(x) !== -1) : deep ? list.some((x) => deepEqual(x, v)) : list.indexOf(v) !== -1;
    this._.contains = false;
    return this._assert(ok, "expected " + show(v) + " to be one of " + show(list), "expected " + show(v) + " to not be one of " + show(list));
  });
  method(["satisfy", "satisfies"], function (fn) {
    return this._assert(!!fn(this._.v), "expected " + show(this._.v) + " to satisfy the given function", "expected " + show(this._.v) + " to not satisfy the given function");
  });
  method(["throw", "throws", "Throw"], function (expected) {
    let thrown = false, err;
    try {
      this._.v();
    } catch (e) {
      thrown = true;
      err = e;
    }
    let ok = thrown;
    if (thrown && expected !== undefined) {
      const msg = err && err.message !== undefined ? err.message : String(err);
      if (typeof expected === "string") ok = msg.indexOf(expected) !== -1;
      else if (expected instanceof RegExp) ok = expected.test(msg);
      else if (typeof expected === "function") ok = err instanceof expected;
    }
    return this._assert(ok, "expected function to throw" + (expected !== undefined ? " " + show(expected) : "") + (thrown ? " but " + show(err) + " was thrown" : ""), "expected function to not throw" + (thrown ? " but " + show(err) + " was thrown" : ""));
  });
  method("fail", function (message) {
    throw new AssertionError(message || "expect.fail()");
  });
  // chai-json-schema light: required / type / properties / items / enum
  function schemaErrors(v, schema, path, errs) {
    if (!schema || typeof schema !== "object") return errs;
    const t = schema.type;
    if (t) {
      const actual = Array.isArray(v) ? "array" : v === null ? "null" : Number.isInteger(v) ? "integer" : typeof v;
      const allowed = [].concat(t);
      if (!(allowed.indexOf(actual) !== -1 || (actual === "integer" && allowed.indexOf("number") !== -1))) errs.push(path + " should be " + allowed.join("|") + " but is " + actual);
    }
    if (schema.enum && !schema.enum.some((e) => deepEqual(e, v))) errs.push(path + " should be one of " + show(schema.enum));
    if (schema.const !== undefined && !deepEqual(schema.const, v)) errs.push(path + " should be " + show(schema.const));
    if (typeof v === "number") {
      if (schema.minimum !== undefined && v < schema.minimum) errs.push(path + " should be >= " + schema.minimum);
      if (schema.maximum !== undefined && v > schema.maximum) errs.push(path + " should be <= " + schema.maximum);
    }
    if (typeof v === "string") {
      if (schema.minLength !== undefined && v.length < schema.minLength) errs.push(path + " should have at least " + schema.minLength + " characters");
      if (schema.maxLength !== undefined && v.length > schema.maxLength) errs.push(path + " should have at most " + schema.maxLength + " characters");
      if (schema.pattern && !new RegExp(schema.pattern).test(v)) errs.push(path + " should match " + schema.pattern);
    }
    if (v !== null && typeof v === "object" && !Array.isArray(v)) {
      for (const r of schema.required || []) if (!(r in v)) errs.push(path + " is missing required property " + show(r));
      for (const k of Object.keys(schema.properties || {})) if (k in v) schemaErrors(v[k], schema.properties[k], path + "." + k, errs);
      if (schema.additionalProperties === false) for (const k of Object.keys(v)) if (!schema.properties || !(k in schema.properties)) errs.push(path + " has additional property " + show(k));
    }
    if (Array.isArray(v)) {
      if (schema.minItems !== undefined && v.length < schema.minItems) errs.push(path + " should have at least " + schema.minItems + " items");
      if (schema.maxItems !== undefined && v.length > schema.maxItems) errs.push(path + " should have at most " + schema.maxItems + " items");
      if (schema.items && !Array.isArray(schema.items)) v.forEach((x, i) => schemaErrors(x, schema.items, path + "[" + i + "]", errs));
    }
    for (const sub of schema.allOf || []) schemaErrors(v, sub, path, errs);
    if (schema.anyOf && !schema.anyOf.some((sub) => schemaErrors(v, sub, path, []).length === 0)) errs.push(path + " should match at least one schema in anyOf");
    if (schema.oneOf && schema.oneOf.filter((sub) => schemaErrors(v, sub, path, []).length === 0).length !== 1) errs.push(path + " should match exactly one schema in oneOf");
    return errs;
  }
  method("jsonSchema", function (schema) {
    const errs = schemaErrors(this._.v, schema, "data", []);
    return this._assert(errs.length === 0, "expected data to satisfy schema but found: " + errs.join("; "), "expected data to not satisfy schema");
  });

  function expect(value, message) {
    return new Assertion(value, message);
  }
  expect.fail = function (message) {
    throw new AssertionError(message || "expect.fail()");
  };

  // ------------------------------------------------------------ variables
  function toStored(v) {
    if (typeof v === "string") return v;
    if (v === undefined || v === null) return "";
    if (typeof v === "object") {
      try {
        return JSON.stringify(v);
      } catch (e) {}
    }
    return String(v);
  }
  const own = (o, k) => Object.prototype.hasOwnProperty.call(o, k);

  function replaceIn(text) {
    if (typeof text !== "string") return text;
    let s = text;
    for (let depth = 0; depth < 10 && s.indexOf("{{") !== -1; depth++) {
      const next = s.replace(/\{\{\s*([^{}]+?)\s*\}\}/g, function (m, name) {
        if (name.charAt(0) === "$") {
          const d = __dynamic(name.slice(1));
          return d === "" ? m : d;
        }
        const v = variables.get(name);
        return v === undefined ? m : String(v);
      });
      if (next === s) break;
      s = next;
    }
    return s;
  }

  class VariableScope {
    constructor(map, readOnly) {
      this._m = map;
      this._ro = !!readOnly;
    }
    has(k) { return own(this._m, k); }
    get(k) { return own(this._m, k) ? this._m[k] : undefined; }
    set(k, v) { if (!this._ro) this._m[String(k)] = toStored(v); }
    unset(k) { if (!this._ro) delete this._m[k]; }
    clear() { if (!this._ro) for (const k of Object.keys(this._m)) delete this._m[k]; }
    toObject() { return Object.assign({}, this._m); }
    toJSON() { return this.toObject(); }
    replaceIn(s) { return replaceIn(s); }
  }
  const sc = S.scopes;
  const order = ["local", "data", "environment", "collection", "globals"];
  const variables = {
    has: (k) => order.some((n) => own(sc[n], k)),
    get: (k) => {
      for (const n of order) if (own(sc[n], k)) return sc[n][k];
      return undefined;
    },
    set: (k, v) => { sc.local[String(k)] = toStored(v); },
    unset: (k) => { delete sc.local[k]; },
    clear: () => { for (const k of Object.keys(sc.local)) delete sc.local[k]; },
    toObject: () => Object.assign({}, sc.globals, sc.collection, sc.environment, sc.data, sc.local),
    replaceIn: replaceIn,
  };
  const environment = new VariableScope(sc.environment);
  environment.name = S.info.environment || "";
  const globalsScope = new VariableScope(sc.globals);
  const collectionVariables = new VariableScope(sc.collection);
  const iterationData = new VariableScope(sc.data, true);

  // ------------------------------------------------------------ request / response objects
  class PropertyList {
    constructor(rows, caseInsensitive) {
      this._rows = rows;
      this._ci = !!caseInsensitive;
    }
    _same(a, b) { return this._ci ? String(a).toLowerCase() === String(b).toLowerCase() : a === b; }
    _find(k) { return this._rows.filter((r) => !r.disabled && this._same(r.key, k)).pop(); }
    get(k) { const r = this._find(k); return r ? r.value : undefined; }
    one(k) { return this._find(k); }
    has(k, v) { const r = this._find(k); return !!r && (v === undefined || r.value === v); }
    add(item) {
      if (typeof item === "string") {
        const i = item.indexOf(":");
        item = { key: item.slice(0, i).trim(), value: item.slice(i + 1).trim() };
      }
      this._rows.push({ key: String(item.key), value: toStored(item.value), disabled: !!item.disabled });
    }
    append(item) { this.add(item); }
    upsert(item) {
      const r = this._rows.find((x) => this._same(x.key, item.key));
      if (r) { r.value = toStored(item.value); r.disabled = false; } else this.add(item);
    }
    remove(k) {
      const key = typeof k === "string" ? k : k && k.key;
      for (let i = this._rows.length - 1; i >= 0; i--) if (this._same(this._rows[i].key, key)) this._rows.splice(i, 1);
    }
    clear() { this._rows.length = 0; }
    all() { return this._rows.slice(); }
    each(fn) { this._rows.forEach(fn); }
    map(fn) { return this._rows.map(fn); }
    filter(fn) { return this._rows.filter(fn); }
    find(fn) { return this._rows.find(fn); }
    count() { return this._rows.length; }
    toObject() {
      const o = {};
      for (const r of this._rows) if (!r.disabled) o[r.key] = r.value;
      return o;
    }
    toJSON() { return this._rows; }
    toString() { return this._rows.filter((r) => !r.disabled).map((r) => r.key + ": " + r.value).join("\n"); }
  }

  function makeUrl(holder) {
    const parse = () => {
      const raw = holder.url;
      const m = /^(?:([a-zA-Z][\w+.-]*):\/\/)?([^\/?#]*)([^?#]*)(?:\?([^#]*))?(?:#(.*))?$/.exec(raw) || [];
      return { protocol: m[1] || "", hostport: m[2] || "", path: m[3] || "", query: m[4] || "", hash: m[5] || "" };
    };
    const url = {
      toString: () => holder.url,
      toJSON: () => holder.url,
      getRaw: () => holder.url,
      getHost: () => parse().hostport.split(":")[0],
      getRemote: () => parse().hostport,
      getPath: () => parse().path || "/",
      getQueryString: () => parse().query,
      getPathWithQuery: () => (parse().path || "/") + (parse().query ? "?" + parse().query : ""),
      update: (u) => { holder.url = typeof u === "string" ? u : String(u); },
      addQueryParams: (params) => {
        const list = typeof params === "string" ? [params] : [].concat(params).map((p) => (typeof p === "string" ? p : p.key + "=" + p.value));
        for (const p of list) holder.url += (holder.url.indexOf("?") === -1 ? "?" : "&") + p;
      },
      removeQueryParams: (names) => {
        const drop = [].concat(names);
        const p = parse();
        const kept = p.query.split("&").filter((kv) => kv && drop.indexOf(kv.split("=")[0]) === -1);
        holder.url = (p.protocol ? p.protocol + "://" : "") + p.hostport + p.path + (kept.length ? "?" + kept.join("&") : "") + (p.hash ? "#" + p.hash : "");
      },
    };
    Object.defineProperty(url, "protocol", { get: () => parse().protocol, enumerable: true });
    Object.defineProperty(url, "host", { get: () => parse().hostport.split(":")[0].split("."), enumerable: true });
    Object.defineProperty(url, "port", { get: () => parse().hostport.split(":")[1] || "", enumerable: true });
    Object.defineProperty(url, "path", { get: () => parse().path.split("/").filter((s) => s !== ""), enumerable: true });
    Object.defineProperty(url, "query", {
      get: () => new PropertyList(parse().query.split("&").filter(Boolean).map((kv) => { const i = kv.indexOf("="); return i === -1 ? { key: kv, value: "" } : { key: kv.slice(0, i), value: kv.slice(i + 1) }; })),
      enumerable: true,
    });
    return url;
  }

  const R = S.request;
  const request = {
    id: S.info.requestId,
    name: S.info.requestName,
    headers: new PropertyList(R.headers, true),
    url: makeUrl(R),
    body: R.body
      ? {
          mode: R.body.mode,
          get raw() { return R.body.raw; },
          set raw(v) { R.body.raw = toStored(v); },
          urlencoded: new PropertyList(R.body.urlencoded || []),
          formdata: new PropertyList(R.body.formdata || []),
          graphql: R.body.graphql,
          options: R.body.options,
          toString: () => R.body.raw || "",
          toJSON: () => R.body,
          update: (b) => {
            if (typeof b === "string") { R.body.mode = "raw"; R.body.raw = b; }
            else if (b && typeof b === "object") { if (b.mode) R.body.mode = b.mode; if (b.raw !== undefined) R.body.raw = toStored(b.raw); }
          },
        }
      : undefined,
    auth: R.auth,
    addHeader: (h) => request.headers.add(h),
    removeHeader: (k) => request.headers.remove(k),
    upsertHeader: (h) => request.headers.upsert(h),
    toJSON: () => ({ method: R.method, url: R.url, header: R.headers, body: R.body }),
  };
  Object.defineProperty(request, "method", { get: () => R.method, set: (v) => { R.method = String(v).toUpperCase(); }, enumerable: true });

  const REASONS = { 200: "OK", 201: "Created", 202: "Accepted", 204: "No Content", 301: "Moved Permanently", 302: "Found", 304: "Not Modified", 400: "Bad Request", 401: "Unauthorized", 403: "Forbidden", 404: "Not Found", 405: "Method Not Allowed", 409: "Conflict", 422: "Unprocessable Entity", 429: "Too Many Requests", 500: "Internal Server Error", 502: "Bad Gateway", 503: "Service Unavailable" };

  function makeResponse(r) {
    if (!r) return undefined;
    let parsed, parsedOk = false;
    const res = {
      code: r.code,
      status: r.status || REASONS[r.code] || "",
      headers: new PropertyList(r.headers, true),
      responseTime: r.responseTime,
      responseSize: r.responseSize,
      text: () => r.body,
      json: () => {
        if (!parsedOk) {
          try {
            parsed = JSON.parse(r.body);
          } catch (e) {
            throw new Error("JSONError: response body is not valid JSON (" + e.message + ")");
          }
          parsedOk = true;
        }
        return parsed;
      },
      reason: () => r.status || REASONS[r.code] || "",
      size: () => ({ body: r.responseSize, header: 0, total: r.responseSize }),
      cookies: new PropertyList((r.cookies || []).slice()),
      toJSON: () => ({ code: r.code, status: r.status, header: r.headers, body: r.body, responseTime: r.responseTime }),
    };
    const mk = (neg) => {
      const check = (ok, pos, negMsg) => {
        if (neg ? ok : !ok) throw new AssertionError(neg ? negMsg : pos);
      };
      const statusCheck = (name, test) => ({ get: () => check(test(res.code), "expected response to have status " + name + " but got " + res.code, "expected response to not have status " + name + " but got " + res.code), enumerable: true });
      const be = {};
      Object.defineProperties(be, {
        info: statusCheck("code 1XX", (c) => c >= 100 && c < 200),
        success: statusCheck("code 2XX", (c) => c >= 200 && c < 300),
        redirection: statusCheck("code 3XX", (c) => c >= 300 && c < 400),
        clientError: statusCheck("code 4XX", (c) => c >= 400 && c < 500),
        serverError: statusCheck("code 5XX", (c) => c >= 500 && c < 600),
        error: statusCheck("code 4XX or 5XX", (c) => c >= 400 && c < 600),
        ok: statusCheck("reason OK (200)", (c) => c === 200),
        accepted: statusCheck("reason Accepted (202)", (c) => c === 202),
        badRequest: statusCheck("reason Bad Request (400)", (c) => c === 400),
        unauthorized: statusCheck("reason Unauthorized (401)", (c) => c === 401),
        unauthorised: statusCheck("reason Unauthorized (401)", (c) => c === 401),
        forbidden: statusCheck("reason Forbidden (403)", (c) => c === 403),
        notFound: statusCheck("reason Not Found (404)", (c) => c === 404),
        rateLimited: statusCheck("reason Too Many Requests (429)", (c) => c === 429),
        json: { get: () => { let ok = true; try { JSON.parse(r.body); } catch (e) { ok = false; } check(ok, "expected response body to be valid JSON", "expected response body to not be valid JSON"); }, enumerable: true },
        withBody: { get: () => check(!!r.body, "expected response to have content in body", "expected response to not have content in body"), enumerable: true },
      });
      const have = {
        status: (s) =>
          typeof s === "number"
            ? check(res.code === s, "expected response to have status code " + s + " but got " + res.code, "expected response to not have status code " + s)
            : check(res.status === s, "expected response to have status reason " + show(s) + " but got " + show(res.status), "expected response to not have status reason " + show(s)),
        header: (k, v) => {
          check(res.headers.has(k), "expected response to have header with key " + show(k), "expected response to not have header with key " + show(k));
          if (v !== undefined && !neg) check(res.headers.get(k) === v, "expected " + show(k) + " response header to be " + show(v) + " but got " + show(res.headers.get(k)), "");
        },
        body: (b) => {
          if (b === undefined) check(!!r.body, "expected response to have content in body", "expected response to not have content in body");
          else if (b instanceof RegExp) check(b.test(r.body), "expected response body to match " + b, "expected response body to not match " + b);
          else check(r.body === b, "expected response body to equal " + show(b) + " but got " + show(r.body), "expected response body to not equal " + show(b));
        },
        jsonBody: (p, v) => {
          let j, ok = true;
          try { j = JSON.parse(r.body); } catch (e) { ok = false; }
          if (!ok) return check(false, "expected response body to be valid JSON", "expected response body to not be valid JSON");
          if (p === undefined) return check(true, "", "expected response body to not be valid JSON");
          if (typeof p === "object") return check(deepEqual(j, p), "expected response body to equal " + show(p) + " but got " + show(j), "expected response body to not equal " + show(p));
          const f = getPath(j, p);
          if (v === undefined) return check(f.found, "expected " + show(j) + " in response to contain property " + show(p), "expected response to not contain property " + show(p));
          return check(f.found && deepEqual(f.value, v), "expected response body to have " + show(p) + " equal to " + show(v) + " but got " + show(f.value), "expected response body to not have " + show(p) + " equal to " + show(v));
        },
        jsonSchema: (schema) => {
          const errs = schemaErrors(res.json(), schema, "data", []);
          check(errs.length === 0, "expected response to satisfy schema but found: " + errs.join("; "), "expected response to not satisfy schema");
        },
        responseTime: undefined,
      };
      const to = { be: be, have: have };
      if (!neg) Object.defineProperty(to, "not", { get: () => mk(true), enumerable: true });
      return to;
    };
    res.to = mk(false);
    return res;
  }
  const response = makeResponse(S.response);

  // ------------------------------------------------------------ pm
  function record(name, passed, error, skipped) {
    out.tests.push({ name: String(name), passed: !!passed, error: error === undefined ? null : error, skipped: !!skipped });
  }
  function errText(e) {
    if (e && e.name === "AssertionError") return e.message;
    if (e instanceof Error) return e.name + ": " + e.message;
    return inspect(e);
  }
  function test(name, fn) {
    if (typeof fn !== "function") return record(name, true);
    try {
      let result;
      if (fn.length >= 1) {
        // callback style: test(name, function (done) { ... })
        const slot = { name: name, done: false };
        pending.push(slot);
        fn(function (err) {
          if (slot.done) return;
          slot.done = true;
          record(name, !err, err ? errText(err) : undefined);
        });
        return;
      }
      result = fn();
      if (result && typeof result.then === "function") {
        const slot = { name: name, done: false };
        pending.push(slot);
        result.then(
          () => { slot.done = true; record(name, true); },
          (e) => { slot.done = true; record(name, false, errText(e)); }
        );
        return;
      }
      record(name, true);
    } catch (e) {
      record(name, false, errText(e));
    }
  }
  test.skip = (name) => record(name, true, undefined, true);
  test.index = () => out.tests.length;

  function sendRequest(spec, callback) {
    let req;
    if (typeof spec === "string") req = { url: spec, method: "GET", headers: [] };
    else {
      const hs = [];
      const h = spec.header || spec.headers;
      if (Array.isArray(h)) for (const x of h) (typeof x === "string" ? hs.push([x.slice(0, x.indexOf(":")).trim(), x.slice(x.indexOf(":") + 1).trim()]) : x.disabled || hs.push([String(x.key), String(x.value)]));
      else if (typeof h === "string") h.split("\n").forEach((x) => x.indexOf(":") > 0 && hs.push([x.slice(0, x.indexOf(":")).trim(), x.slice(x.indexOf(":") + 1).trim()]));
      else if (h && typeof h === "object") for (const k of Object.keys(h)) hs.push([k, String(h[k])]);
      const u = spec.url && typeof spec.url === "object" ? spec.url.raw || String(spec.url) : spec.url;
      req = { url: String(u), method: String(spec.method || "GET").toUpperCase(), headers: hs };
      const b = spec.body;
      if (typeof b === "string") req.body = { mode: "raw", raw: b };
      else if (b && typeof b === "object") {
        if (b.mode === "urlencoded") req.body = { mode: "urlencoded", fields: (b.urlencoded || []).filter((x) => !x.disabled).map((x) => [String(x.key), replaceIn(String(x.value))]) };
        else if (b.mode === "formdata") req.body = { mode: "formdata", fields: (b.formdata || []).filter((x) => !x.disabled).map((x) => [String(x.key), replaceIn(String(x.value))]) };
        else if (b.mode === "graphql") req.body = { mode: "raw", raw: JSON.stringify({ query: (b.graphql || {}).query, variables: (b.graphql || {}).variables || {} }), language: "json" };
        else req.body = { mode: "raw", raw: replaceIn(typeof b.raw === "string" ? b.raw : JSON.stringify(b.raw === undefined ? "" : b.raw)), language: ((b.options || {}).raw || {}).language };
      }
    }
    req.url = replaceIn(req.url);
    req.headers = req.headers.map((h) => [replaceIn(h[0]), replaceIn(h[1])]);
    const result = JSON.parse(__send(JSON.stringify(req)));
    const err = result.error ? new Error(result.error) : null;
    const res = result.error ? undefined : makeResponse(result.response);
    if (typeof callback === "function") {
      try {
        callback(err, res);
      } catch (e) {
        record("pm.sendRequest callback", false, errText(e));
      }
      return undefined;
    }
    return err ? Promise.reject(err) : Promise.resolve(res);
  }

  const execution = {
    setNextRequest: (name) => { out.next = name === null || name === undefined ? null : String(name); },
    skipRequest: () => { out.skip = true; throw { __skip: true }; },
    location: Object.assign([S.info.collectionName, S.info.requestName].filter(Boolean), { current: S.info.requestName }),
  };

  const cookieMap = {};
  for (const c of S.cookies || []) cookieMap[c.key] = c.value;
  const cookies = {
    has: (k) => own(cookieMap, k),
    get: (k) => cookieMap[k],
    toObject: () => Object.assign({}, cookieMap),
    jar: () => ({
      get: (url, name, cb) => cb && cb(null, cookieMap[name]),
      getAll: (url, cb) => cb && cb(null, (S.cookies || []).slice()),
      set: (url, name, value, cb) => { const f = typeof value === "function" ? value : cb; f && f(new Error("pm.cookies.jar().set is not supported; use the cookie manager")); },
      unset: (url, name, cb) => cb && cb(new Error("pm.cookies.jar().unset is not supported; use the cookie manager")),
      clear: (url, cb) => cb && cb(new Error("pm.cookies.jar().clear is not supported; use the cookie manager")),
    }),
  };

  const pm = {
    info: { eventName: S.info.eventName, iteration: S.info.iteration, iterationCount: S.info.iterationCount, requestName: S.info.requestName, requestId: S.info.requestId },
    variables: variables,
    environment: environment,
    globals: globalsScope,
    collectionVariables: collectionVariables,
    iterationData: iterationData,
    request: request,
    response: response,
    cookies: cookies,
    test: test,
    expect: expect,
    sendRequest: sendRequest,
    execution: execution,
    setNextRequest: execution.setNextRequest,
    visualizer: { set: (template, data) => { out.visualizer = { template: String(template), data: data === undefined ? {} : data }; }, clear: () => { out.visualizer = null; } },
  };
  globalThis.pm = pm;
  globalThis.expect = expect;
  globalThis.AssertionError = AssertionError;

  // ------------------------------------------------------------ legacy `postman` API + globals
  globalThis.postman = {
    setNextRequest: execution.setNextRequest,
    setEnvironmentVariable: (k, v) => environment.set(k, v),
    getEnvironmentVariable: (k) => environment.get(k),
    clearEnvironmentVariable: (k) => environment.unset(k),
    clearEnvironmentVariables: () => environment.clear(),
    setGlobalVariable: (k, v) => globalsScope.set(k, v),
    getGlobalVariable: (k) => globalsScope.get(k),
    clearGlobalVariable: (k) => globalsScope.unset(k),
    clearGlobalVariables: () => globalsScope.clear(),
    getResponseHeader: (k) => (response ? response.headers.get(k) : undefined),
    getResponseCookie: (k) => (cookieMap[k] === undefined ? undefined : { name: k, value: cookieMap[k] }),
  };
  globalThis.tests = {};
  globalThis.environment = sc.environment;
  globalThis.globals = sc.globals;
  globalThis.data = sc.data;
  globalThis.iteration = S.info.iteration;
  globalThis.request = { method: R.method, url: R.url, headers: request.headers.toObject(), data: R.body ? R.body.raw : undefined, name: S.info.requestName };
  if (response) {
    globalThis.responseBody = S.response.body;
    globalThis.responseCode = { code: response.code, name: response.status, detail: "" };
    globalThis.responseTime = response.responseTime;
    globalThis.responseHeaders = response.headers.toObject();
    globalThis.responseCookies = (S.response.cookies || []).slice();
  }

  // ------------------------------------------------------------ timers, base64, require
  let timerSeq = 0;
  globalThis.setTimeout = function (fn, ms) {
    const args = Array.prototype.slice.call(arguments, 2);
    timers.push({ id: ++timerSeq, at: Number(ms) || 0, fn: fn, args: args, seq: timerSeq });
    return timerSeq;
  };
  globalThis.setImmediate = (fn) => globalThis.setTimeout(fn, 0);
  globalThis.clearTimeout = globalThis.clearImmediate = function (id) {
    const i = timers.findIndex((t) => t.id === id);
    if (i !== -1) timers.splice(i, 1);
  };
  globalThis.setInterval = function () { throw new Error("setInterval is not available in the rsPostman sandbox"); };
  globalThis.clearInterval = function () {};

  const B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  globalThis.btoa = function (input) {
    const s = String(input);
    let o = "";
    for (let i = 0; i < s.length; i += 3) {
      const a = s.charCodeAt(i), b = s.charCodeAt(i + 1), c = s.charCodeAt(i + 2);
      if (a > 255 || b > 255 || c > 255) throw new Error("btoa: string contains characters outside of the Latin1 range");
      const n = (a << 16) | ((b || 0) << 8) | (c || 0);
      o += B64[(n >> 18) & 63] + B64[(n >> 12) & 63] + (i + 1 < s.length ? B64[(n >> 6) & 63] : "=") + (i + 2 < s.length ? B64[n & 63] : "=");
    }
    return o;
  };
  globalThis.atob = function (input) {
    const s = String(input).replace(/[\s=]+/g, "");
    let o = "", bits = 0, acc = 0;
    for (const ch of s) {
      const v = B64.indexOf(ch);
      if (v === -1) throw new Error("atob: invalid character");
      acc = (acc << 6) | v;
      bits += 6;
      if (bits >= 8) {
        bits -= 8;
        o += String.fromCharCode((acc >> bits) & 255);
      }
    }
    return o;
  };

  const modules = {
    uuid: () => ({ v4: () => __dynamic("guid") }),
    chai: () => ({ expect: expect, AssertionError: AssertionError, assert: Object.assign((ok, msg) => { if (!ok) throw new AssertionError(msg || "assertion failed"); }, { equal: (a, b, m) => expect(a, m).to.equal(b), deepEqual: (a, b, m) => expect(a, m).to.eql(b), ok: (a, m) => expect(a, m).to.be.ok, isTrue: (a, m) => expect(a, m).to.be.true, isFalse: (a, m) => expect(a, m).to.be.false, fail: (m) => expect.fail(m) }) }),
    atob: () => globalThis.atob,
    btoa: () => globalThis.btoa,
  };
  globalThis.require = function (name) {
    if (own(modules, name)) return modules[name]();
    throw new Error("Cannot find module '" + name + "' (available in the rsPostman sandbox: " + Object.keys(modules).join(", ") + ")");
  };

  // ------------------------------------------------------------ hooks called from Rust
  globalThis.__skipSignal = (e) => !!(e && e.__skip);
  globalThis.__runTimers = function () {
    if (!timers.length) return false;
    timers.sort((a, b) => a.at - b.at || a.seq - b.seq);
    const t = timers.shift();
    try {
      t.fn.apply(null, t.args);
    } catch (e) {
      if (!(e && e.__skip)) out.console.push({ level: "error", message: "Uncaught error in timer: " + errText(e) });
    }
    return true;
  };
  globalThis.__finish = function () {
    for (const k of Object.keys(globalThis.tests || {})) record(k, !!globalThis.tests[k], globalThis.tests[k] ? undefined : "test flag is falsy");
    for (const slot of pending) if (!slot.done) record(slot.name, false, "asynchronous test did not complete");
    return JSON.stringify({
      scopes: sc,
      tests: out.tests,
      console: out.console,
      next: out.next === undefined ? { set: false, name: null } : { set: true, name: out.next },
      skip: out.skip,
      visualizer: out.visualizer,
      request: { method: R.method, url: R.url, headers: R.headers, body: R.body },
    });
  };
})();
