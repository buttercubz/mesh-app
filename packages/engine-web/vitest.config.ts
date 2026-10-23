import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    include: ["test/**/*.test.ts"],
    restoreMocks: true,
    // The simulated convoy runs real engines: a minute of simulated time takes a while.
    testTimeout: 120_000,
  },
});
