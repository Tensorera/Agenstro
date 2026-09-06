# Motivo Studio

Motivo Studio is an offline, read-only HTML report for methods run by the user's
existing coding agent. It has no model selector, task loop, server, Electron
process, login, or execution controls.

Initialize a workspace with `tactus init`, open your coding agent in that folder,
and ask it to use the Motivo skill. Open `.tactus/motivo/index.html` in a browser
for the overview, or a method's `report.html` for its evidence and findings.
The calling agent stays responsible for choosing and continuing work.

Reports contain the content and styles needed to read them without a network
connection. Browser refresh loads a newer snapshot; an old page is not a live
subscription. The document shows when its observations were recorded. Closing
the browser does not stop the Haskell script.

`report-template.html` is installed into
`.tactus/skills/motivo/assets/report-template.html`. The Motivo Haskell helper
replaces its single `<!-- MOTIVO_CONTENT -->` marker with escaped, pre-rendered
content. JavaScript only expands evidence or reloads the current document;
core content remains readable without it. No adjacent JSON fetch is needed.

The old Electron task client and its installers were removed. Existing
`.motivo/tasks` data is historical and is not automatically rewritten or resumed.
The Tactus control/session APIs remain available independently.

## Development

Node.js >=22.12 is needed for these development checks, not for reading a report:

```sh
npm --prefix motivo-studio ci
npm --prefix motivo-studio run format:check
npm --prefix motivo-studio run lint
npm --prefix motivo-studio test
npm --prefix motivo-studio run build
```

The build copies the standalone template to `Build/motivo/observer`.
Haskell methods and their tests live in `../motivo`; no TypeScript task runtime
or native browser authority is included in the report.

See the [Motivo guide](../docs/motivo-studio.md) for methods, records, experiments,
and the [installation guide](../docs/install.md) for tool setup.

Licensed with Agenstro under [GNU AGPL v3.0 only](../LICENSE).
