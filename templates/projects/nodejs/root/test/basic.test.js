import test from "node:test";
import assert from "node:assert/strict";

import { projectName } from "../src/index.js";

test("projectName returns the repository name", () => {
  assert.equal(projectName(), "__REPO_NAME__");
});

