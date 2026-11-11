# Scripting reference

Pre-request and test scripts are JavaScript (ES2022-level) executed by an embedded engine.
Scripts can be attached to a collection, a folder or a request and run outermost first:

```text
collection pre-request → folder pre-request → request pre-request → HTTP request
→ collection tests → folder tests → request tests
```

Top-level `await` and `return` are allowed. Each script runs in a fresh context; share data
between scripts and requests through variables.

## `pm`

| API | Notes |
|-----|-------|
| `pm.test(name, fn)` | Records a pass/fail result. `fn` may be `async` or take a `done` callback. `pm.test.skip(name)` |
| `pm.expect(value)` | Chai-style BDD assertions, see below. `pm.expect.fail(msg)` |
| `pm.response` | `code`, `status`, `headers` (`get`, `has`, `toObject`, `each`), `responseTime`, `responseSize`, `text()`, `json()`, `cookies`; `to.have.status(200 \| "OK")`, `to.have.header(k[, v])`, `to.have.body(str \| regexp)`, `to.have.jsonBody([path[, value]])`, `to.have.jsonSchema(schema)`, `to.be.ok / success / error / clientError / serverError / json / notFound / unauthorized / forbidden / badRequest / accepted / rateLimited / redirection / info / withBody`, and `to.not.…` |
| `pm.request` | `method`, `url` (`toString()`, `getHost()`, `getPath()`, `getQueryString()`, `query`, `addQueryParams()`, `removeQueryParams()`, `update()`), `headers` (`add`, `upsert`, `remove`, `get`, `has`), `body` (`mode`, `raw`, `urlencoded`, `formdata`, `update()`). Changes made in a **pre-request** script are sent |
| `pm.variables` | Merged view (local → data → environment → collection → globals): `get`, `has`, `set` (local), `unset`, `replaceIn("{{x}}")`, `toObject` |
| `pm.environment`, `pm.globals`, `pm.collectionVariables` | `get`, `set`, `unset`, `has`, `clear`, `toObject`, `replaceIn`; changes are saved to the workspace. `pm.environment.name` |
| `pm.iterationData` | Read-only row of the runner's data file |
| `pm.info` | `eventName`, `iteration`, `iterationCount`, `requestName`, `requestId` |
| `pm.sendRequest(urlOrObject[, callback])` | Callback `(err, response)` or returns a promise. Object form: `{ url, method, header, body: { mode: "raw" \| "urlencoded" \| "formdata" \| "graphql", … } }` |
| `pm.execution.setNextRequest(name \| null)` | Runner flow control; `null` stops the run. `pm.execution.skipRequest()` in a pre-request script skips the request |
| `pm.cookies` | Cookies that apply to the request URL: `get`, `has`, `toObject` |
| `pm.visualizer.set(template, data)` | Stored with the result (not rendered) |

Values written to variables are stored as strings; objects are JSON-encoded
(`JSON.parse(pm.environment.get("obj"))` reads them back).

## `pm.expect`

Chains: `to be been is that which and has have with at of same but does still also`.
Flags: `not deep own nested any all ordered`.

```js
pm.expect(json.id).to.equal(42);            // equal / equals / eq, eql (deep)
pm.expect(json).to.have.property("name", "Ann");
pm.expect(json).to.have.nested.property("profile.age").that.is.above(18);
pm.expect(json.tags).to.be.an("array").that.includes("a").and.has.lengthOf(2);
pm.expect(json).to.include({ id: 42 });     // deep.include for nested values
pm.expect(json).to.have.all.keys("id", "name");   // any.keys, include.keys
pm.expect(list).to.have.members([1, 2]);    // deep / ordered / include members
pm.expect(n).to.be.within(1, 10);           // above, below, least, most, closeTo
pm.expect(s).to.match(/^A/).and.have.string("nn");
pm.expect(v).to.be.oneOf([200, 201]);
pm.expect(fn).to.throw(TypeError);          // or a message substring / RegExp
pm.expect(json).to.be.jsonSchema(schema);   // type, required, properties, items, enum, const,
                                            // min/max(Length|Items), pattern, allOf/anyOf/oneOf,
                                            // additionalProperties: false
// also: ok true false null undefined NaN exist empty finite instanceOf satisfy
```

## Globals

`console.log / info / warn / error / debug`, `setTimeout`, `clearTimeout`, `atob`, `btoa`,
`require("uuid")`, `require("chai")`, and the legacy Postman API: `postman.setNextRequest`,
`postman.setEnvironmentVariable` / `getEnvironmentVariable` / `clearEnvironmentVariable`,
`postman.setGlobalVariable` / `getGlobalVariable` / `clearGlobalVariable`,
`postman.getResponseHeader`, `tests["name"] = boolean`, `responseBody`, `responseCode`,
`responseHeaders`, `responseTime`, `environment`, `globals`, `data`, `iteration`, `request`.

## Differences from Postman

* `pm.sendRequest` runs synchronously: callbacks are invoked before the call returns and
  promises are already settled.
* No Node.js or browser APIs, no `require()` of npm packages, no `setInterval`.
* Scripts are stopped after 5 million loop iterations or very deep recursion.
* An uncaught error in a pre-request script aborts the request; in a test script it is
  reported as a script error next to the test results.
