# Write and review documentation

Use this guide when adding or changing a page in this repository.
The content model follows the
[Diátaxis four documentation types](https://diataxis.fr/start-here/), and
`mkdocs.yml` provides the explicit
[MkDocs navigation](https://www.mkdocs.org/user-guide/writing-your-docs/#configure-pages-and-navigation).

## Choose one document type

| Type | Reader need | Required style |
| --- | --- | --- |
| Tutorial | Learn through a complete example | Ordered steps, controlled inputs, observable result |
| How-to guide | Complete one known task | Goal title, prerequisites only when required, direct steps |
| Reference | Look up an interface or constraint | Complete fields, defaults, types, errors, stable headings |
| Explanation | Understand a design or boundary | Context, reasoning, trade-offs, links to reference |

Do not mix an API field inventory into a tutorial or place operational steps
only inside an architecture page.

## Write for human and machine readers

1. Use one H1 that names the exact subject.
2. Start with two sentences stating scope and audience.
3. Use the canonical project names Clef, Tactus, Segno, and Motivo Studio.
4. Define an ambiguous term at its canonical reference page and link to that
   definition; do not create a second competing glossary entry.
5. Give each fact one canonical page. Link to it instead of copying it.
6. Put commands in a block that names the shell and state the working directory.
7. Record defaults and unsupported cases in reference pages.
8. Avoid pronouns such as “it” when two components appear in the same paragraph.
9. Mark historical material as historical and never link to it as current
   behavior.
10. Use descriptive file names. Do not encode navigation order in file names.

## Canonical subjects

| Subject | Canonical page |
| --- | --- |
| Installation, upgrade, uninstall | [Installation](../install.md) |
| First model-free tutorial | [First workflow](../getting-started.md) |
| Clef public authoring model | [Clef workflow guide](../clef.md) |
| `.tactus` layout and TOML | [Tactus workspace](../tactus-workspace.md) |
| Native provider setup/model/effort | [Provider setup](../providers.md) |
| Third-party plugin tutorial | [Plugin authoring](../plugin-authoring.md) |
| Human logs and durable transitions | [Logs and run evidence](../observability.md) |
| Backup, retention, restore | [Workspace operations](../operations.md) |
| Shared terms | [Glossary](../reference/glossary.md) |

Component README files contain package build facts and link here. Root README
contains only product identity, a copyable quick path, safety, license, and
navigation.

## Update connected pages

When a public command, field, state transition, or ownership boundary changes:

1. Update its reference page, such as the [CLI reference](../reference/cli-v0.3.md)
   or the relevant wire-protocol page.
2. Update affected tutorial or how-to steps.
3. Update an explanation only when the design or rationale changed.
4. Update `mkdocs.yml` for every added, removed, or renamed page.
5. Run the documentation build.

## Validate

From the repository root:

```powershell
python -m mkdocs build --strict
```

The build must contain every current page in navigation and must not report a
broken relative link or missing anchor.
