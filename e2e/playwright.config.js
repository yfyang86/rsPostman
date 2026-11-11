const { defineConfig } = require("./pw");

module.exports = defineConfig({
  testDir: ".",
  testMatch: /.*\.spec\.js$/,
  timeout: 45000,
  expect: { timeout: 8000 },
  fullyParallel: false,
  workers: process.env.CI ? 2 : 3,
  retries: 0,
  reporter: [["list"]],
  use: {
    viewport: { width: 1360, height: 860 },
    trace: "off",
    screenshot: "only-on-failure",
    actionTimeout: 8000,
  },
  outputDir: "test-results",
});
