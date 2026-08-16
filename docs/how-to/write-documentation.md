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
3. Use the canonical project names Clef SDK, Tactus Runtime, and
   Studio.
4. Define ambiguous terms in the [glossary](../reference/glossary.md).
5. Give each fact one canonical page. Link to it instead of copying it.
6. Put commands in a block that names the shell and state the working directory.
7. Record defaults and unsupported cases in reference pages.
8. Avoid pronouns such as “it” when two components appear in the same paragraph.
9. Mark historical material as historical and never link to it as current
   behavior.
10. Use descriptive file names. Do not encode navigation order in file names.

## Update connected pages

When a public command, field, state transition, or ownership boundary changes:

1. Update its reference page.
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
