# Glossary

Use these terms consistently across code and documentation.

| Term | Definition |
| --- | --- |
| Agent | A native Codex, Claude Code, or OpenCode process used to edit a project or, on compatibility CLI paths, return a composition decision |
| Artifact | A project-relative file or directory tracked in Tactus history, or a declared Clef task value where context requires |
| Attempt | One Tactus execution try within a phase |
| Cell | The immutable Tactus record for one attempt after execution starts |
| Compose | Persist Python source as the next runtime attempt; compatibility commands may first ask an agent for that source |
| Contract | A Clef `DomainContract` describing I/O, effects, and verification |
| Clef Profile | Clef SDK TOML runtime configuration; unrelated to Studio preferences |
| Clef SDK | The `clef_sdk` Python distribution for contract-driven static DAGs |
| Occurrence | One externally scheduled task instance; the scheduling host owns its identity |
| Tactus project | The exact directory whose project-local state is under `.tactus` |
| Tactus Runtime | The `tactus-runtime` distribution and its compose/run state machine |
| Phase | An Tactus task stage completed by one successful attempt |
| Run | Execute the current Tactus draft cell in a fresh Jupyter kernel |
| Runtime root | Compatibility term for a Tactus project or a legacy detached runtime root |
| Studio | The React + TypeScript Windows editor for `.tactus/main_script.py`, Monaco project editing, an xterm.js PowerShell/Bash terminal, and runtime evidence |
| Workflow memory | Bounded structured state returned by the agent and stored by Tactus |
| Workflow plan | A Clef `WorkflowPlan` containing a static task DAG |

The word **workflow** alone may refer to either project. Pages must use
**Clef workflow plan** or **Tactus runtime** when the distinction affects
behavior.
