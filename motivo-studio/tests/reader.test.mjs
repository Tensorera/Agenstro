import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { JSDOM } from "jsdom";

const template = await readFile(new URL("../report-template.html", import.meta.url), "utf8");
const content =
  '<header class="motivo-header"><h1>复盘：导入卡顿</h1></header><main><article><p>先前判断与实际观察不同。</p><details><summary>实验 S01</summary><pre>原始输出</pre></details><details><summary>调查 S02</summary><p>来源</p></details></article></main>';

test("a file report remains readable without JavaScript or external resources", () => {
  const dom = new JSDOM(template.replace("<!-- MOTIVO_CONTENT -->", content), {
    url: "file:///tmp/report.html",
  });
  assert.equal(dom.window.document.querySelector("h1").textContent, "复盘：导入卡顿");
  assert.match(dom.window.document.querySelector("main").textContent, /先前判断与实际观察不同/);
  assert.equal(
    dom.window.document.querySelectorAll("script[src],link[href],iframe,img[src],form").length,
    0,
  );
  assert.equal(dom.window.document.querySelector(".reader-actions").hidden, true);
  dom.window.close();
});

test("reader controls expand evidence without an execution endpoint", () => {
  const dom = new JSDOM(template.replace("<!-- MOTIVO_CONTENT -->", content), {
    url: "file:///tmp/report.html",
    runScripts: "dangerously",
  });
  const document = dom.window.document;
  assert.equal(document.querySelector(".reader-actions").hidden, false);
  const button = document.getElementById("expand-evidence");
  button.click();
  assert.ok([...document.querySelectorAll("details")].every((detail) => detail.open));
  assert.equal(button.getAttribute("aria-expanded"), "true");
  button.click();
  assert.ok([...document.querySelectorAll("details")].every((detail) => !detail.open));
  assert.equal(button.textContent, "展开证据");
  assert.equal(document.querySelectorAll("button").length, 2);
  dom.window.close();
});
