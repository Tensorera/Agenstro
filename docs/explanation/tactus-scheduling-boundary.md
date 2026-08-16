# Tactus scheduling boundary

Motivo Studio manages one exact project and its durable cell history. It is
not a scheduler daemon.

`tactus loop` can continue after a host restart because cell state is stored
in SQLite and Git. After recovery, running the same loop command resumes the
existing occurrence. When the root is `complete`, another loop invocation
returns the existing completion without calling an agent.

A recurring host owns:

- trigger time and occurrence identity;
- input version freezing and idempotency keys;
- duplicate suppression and host-level retry;
- external resource refresh;
- retention, notification, and final publication.

Windows Task Scheduler may invoke such a host or the Tactus CLI. Task
Scheduler does not add occurrence semantics to Tactus.

Runtime initialization is currently a multi-step bootstrap. A host exit between
project-local configuration, SQLite initialization, helper setup, Artifact
tree initialization, and Notebook creation can leave partial `.tactus`
state. Bootstrap is non-destructive and can repair missing layout on the next
exact-directory startup, but a production occurrence host should still
journal its own claim and publication boundary.

Scheduling does not imply a temporary checkout or detached worktree. The host
selects the project root, and task-specific isolation—if required—belongs in
the main script. Legacy detached runtime roots may retain their older
one-runtime-per-occurrence policy.
