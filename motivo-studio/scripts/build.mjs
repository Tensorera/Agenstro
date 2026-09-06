import { mkdir, readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const template = await readFile(new URL("../report-template.html", import.meta.url), "utf8");
if (template.split("<!-- MOTIVO_CONTENT -->").length !== 2) {
  throw new Error("The report template must contain exactly one content slot.");
}
const output = new URL("../../Build/motivo/observer/", import.meta.url);
await mkdir(output, { recursive: true });
await writeFile(new URL("report-template.html", output), template);
console.log(`Offline report template: ${fileURLToPath(output)}report-template.html`);
