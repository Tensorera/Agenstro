import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { invokeProvider, ProviderCallError } from "../../src/main/tasks/transport";

const directories: string[] = [];

afterEach(async () => {
  await Promise.all(
    directories.splice(0).map((path) => rm(path, { recursive: true, force: true })),
  );
});

function fixture(body: string) {
  return {
    executable: process.execPath,
    commandPrefix: [
      "-e",
      `
      let input = '';
      process.stdin.setEncoding('utf8');
      process.stdin.on('data', chunk => { input += chunk; });
      process.stdin.on('end', () => {
        const request = JSON.parse(input);
        const send = value => process.stdout.write(JSON.stringify({ id: request.id, ...value }) + '\\n');
        const success = text => send({ type: 'result', ok: true, value: { text } });
        ${body}
      });
    `,
      "--",
    ],
  };
}

function invocation(prompt = "fixture prompt", signal = new AbortController().signal) {
  return { root: process.cwd(), provider: "test-provider", prompt, signal };
}

describe("provider CLI transport", () => {
  it("sends long prompts over stdin through Tactus dispatch and preserves final text", async () => {
    const prompt = '```json\n{"reply":"你好"}\n```' + "x".repeat(90_000);
    const result = await invokeProvider(
      invocation(prompt),
      fixture(`
      const assert = require('node:assert/strict');
      assert.deepEqual(process.argv.slice(1), ['dispatch', '--namespace', 'provider', '--name', 'test-provider', '--root', process.cwd()]);
      assert.equal(request.api, 'agenstro.plugin/v1');
      assert.equal(request.method, 'invoke');
      assert.deepEqual(Object.keys(request.params), ['prompt']);
      success(request.params.prompt);
    `),
    );
    expect(result).toEqual({ text: prompt });
  });

  it("discards streamed events and handles UTF-8 split across chunks and a final frame without LF", async () => {
    const result = await invokeProvider(
      invocation(),
      fixture(`
      for (let index = 0; index < 10000; index++) send({ type: 'event', event: { type: 'progress', index } });
      const bytes = Buffer.from(JSON.stringify({ id: request.id, type: 'result', ok: true, value: { text: '你好🌊' } }));
      const split = bytes.indexOf(Buffer.from('你')) + 1;
      process.stdout.write(bytes.subarray(0, split));
      setTimeout(() => process.stdout.write(bytes.subarray(split)), 10);
    `),
    );
    expect(result).toEqual({ text: "你好🌊" });
  });

  it.each([
    ["wrong correlation", "send({id:'other', type:'result', ok:true, value:{text:'wrong'}});"],
    ["missing terminal", "send({type:'event',event:{type:'progress'}});"],
    ["duplicate terminal", "success('one'); success('two');"],
    ["unterminated data after terminal", "success('one'); process.stdout.write('trailing');"],
    ["event after terminal", "success('one'); send({type:'event',event:{type:'progress'}});"],
    ["invalid JSON", "process.stdout.write('not-json\\n');"],
    ["invalid UTF-8", "process.stdout.write(Buffer.from([0xff, 0x0a]));"],
    ["malformed event", "send({type:'event',event:{}});"],
    ["missing text", "send({type:'result',ok:true,value:{}});"],
    [
      "ambiguous terminal",
      "send({type:'result',ok:true,value:{text:'x'},error:{code:'x',message:'x'}});",
    ],
    ["nonzero after success", "success('one'); process.exitCode=1;"],
  ])("classifies %s as outcome unknown", async (_name, body) => {
    await expect(invokeProvider(invocation(), fixture(body))).rejects.toMatchObject({
      outcome: "outcome_unknown",
    });
  });

  it.each([0, 1])("preserves an authoritative reported failure with exit %s", async (exitCode) => {
    await expect(
      invokeProvider(
        invocation(),
        fixture(`
      send({type:'result',ok:false,error:{code:'rejected',message:'Provider rejected this request.'}});
      process.exitCode=${exitCode};
    `),
      ),
    ).rejects.toMatchObject({ outcome: "failed", message: "Provider rejected this request." });
  });

  it("preserves a reported outcome_unknown", async () => {
    await expect(
      invokeProvider(
        invocation(),
        fixture(`
      send({type:'result',ok:false,error:{code:'outcome_unknown',message:'External operation may have completed.'}});
    `),
      ),
    ).rejects.toMatchObject({ outcome: "outcome_unknown" });
  });

  it("reports spawn failure as a known failure", async () => {
    await expect(
      invokeProvider(invocation(), { executable: join(tmpdir(), "motivo-missing-tactus-binary") }),
    ).rejects.toBeInstanceOf(ProviderCallError);
    await expect(
      invokeProvider(invocation(), { executable: join(tmpdir(), "motivo-missing-tactus-binary") }),
    ).rejects.toMatchObject({ outcome: "failed" });
  });

  it("does not spawn an already cancelled invocation", async () => {
    const controller = new AbortController();
    controller.abort();
    await expect(
      invokeProvider(invocation("unused", controller.signal), fixture("success('unexpected');")),
    ).rejects.toMatchObject({
      outcome: "failed",
      message: expect.stringContaining("before dispatch"),
    });
  });

  it("waits for child close on abort and reports an unknown external outcome", async () => {
    const root = await mkdtemp(join(tmpdir(), "motivo-provider-abort-"));
    directories.push(root);
    const ready = join(root, "ready");
    const controller = new AbortController();
    const result = invokeProvider(
      invocation(ready, controller.signal),
      fixture(`
      const fs = require('node:fs');
      process.on('SIGTERM', () => setTimeout(() => {
        fs.writeFileSync(request.params.prompt + '.closed', 'closed');
        process.exit(0);
      }, 60));
      fs.writeFileSync(request.params.prompt, 'ready');
      setInterval(() => {}, 1000);
    `),
    );
    const rejected = expect(result).rejects.toMatchObject({
      outcome: "outcome_unknown",
      message: expect.stringContaining("cancelled"),
    });
    try {
      await waitForFile(ready);
      controller.abort();
      await rejected;
      if (process.platform !== "win32")
        expect(await readFile(`${ready}.closed`, "utf8")).toBe("closed");
    } finally {
      controller.abort();
    }
  });

  it("terminates a stalled invocation at the transport deadline", async () => {
    await expect(
      invokeProvider(invocation(), { ...fixture("setInterval(() => {}, 1000);"), timeoutMs: 150 }),
    ).rejects.toMatchObject({
      outcome: "outcome_unknown",
      message: expect.stringContaining("deadline"),
    });
  });

  it("escalates termination when a provider fixture ignores SIGTERM", async () => {
    await expect(
      invokeProvider(invocation(), {
        ...fixture("process.on('SIGTERM', () => {}); setInterval(() => {}, 1000);"),
        timeoutMs: 150,
      }),
    ).rejects.toMatchObject({ outcome: "outcome_unknown" });
  });

  it("rejects an oversized unterminated frame while draining the child", async () => {
    await expect(
      invokeProvider(invocation(), fixture("process.stdout.write('x'.repeat(33 * 1024 * 1024));")),
    ).rejects.toMatchObject({
      outcome: "outcome_unknown",
      message: expect.stringContaining("frame limit"),
    });
  });

  it("bounds retained stderr diagnostics", async () => {
    const result = invokeProvider(
      invocation(),
      fixture("process.stderr.write('x'.repeat(512 * 1024));"),
    );
    const error = await result.catch((failure: unknown) => failure);
    expect(error).toBeInstanceOf(ProviderCallError);
    if (!(error instanceof ProviderCallError)) throw new Error("Expected a transport error.");
    expect(error.outcome).toBe("outcome_unknown");
    expect(error.message.length).toBeLessThan(4_300);
  });
});

async function waitForFile(path: string): Promise<void> {
  const deadline = Date.now() + 2_000;
  while (Date.now() < deadline) {
    try {
      await readFile(path);
      return;
    } catch {
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
  }
  throw new Error("Provider fixture did not start.");
}
