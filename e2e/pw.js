// Resolve the Playwright test runner whether it is installed as `@playwright/test`
// (npm install in this folder) or only as the bundled `playwright` package.
let pw;
try {
  pw = require("@playwright/test");
} catch (e) {
  pw = require("playwright/test");
}
module.exports = pw;
