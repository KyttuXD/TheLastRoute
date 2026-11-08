import test from "node:test";
import assert from "node:assert/strict";
import {
  checkGithubSourceRevision,
  githubSourceWatchEnabled,
} from "./github_source_watch.mjs";

test("detects when GitHub main moved past the deployed commit", async () => {
  const local = "a".repeat(40);
  const remote = "b".repeat(40);
  const result = await checkGithubSourceRevision({
    execFileImpl: async (command, args) => {
      assert.equal(command, "git");
      assert.deepEqual(args, ["rev-parse", "HEAD"]);
      return { stdout: `${local}\n` };
    },
    fetchImpl: async (_url, options) => {
      assert.equal(options.headers.authorization, "Bearer secret");
      return new Response(JSON.stringify({ sha: remote }));
    },
    token: "secret",
  });
  assert.deepEqual(result, { changed: true, local, remote });
});

test("source restart watching follows the server PAT unless explicitly disabled", () => {
  assert.equal(githubSourceWatchEnabled({ ACCESS_TOKEN: "secret" }), true);
  assert.equal(githubSourceWatchEnabled({ LASTROUTE_RESTART_ON_GITHUB_UPDATE: "true" }), true);
  assert.equal(githubSourceWatchEnabled({
    ACCESS_TOKEN: "secret",
    LASTROUTE_RESTART_ON_GITHUB_UPDATE: "false",
  }), false);
  assert.equal(githubSourceWatchEnabled({}), false);
});
