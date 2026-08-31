---
title: Author a local Agenstro plugin
status: alpha
owners: [protocol]
last_verified: 2026-08-17
applies_to: "agenstro.plugin/v1"
platforms: [windows, ubuntu]
---

# Author a local Agenstro plugin

An Agenstro plugin is a one-shot executable that reads one strict JSON request
from stdin, writes zero or more JSONL events, writes exactly one terminal frame,
and exits. This tutorial builds a small typed calculator in Rust and registers
it as a generic plugin.

## Choose the correct boundary

The wire protocol is shared, but the registry communicates intent:

| Registry | Use |
| --- | --- |
| `[providers]` | Prompt-in/final-text-out coding-agent adapters |
| `[effects]` | Named external operations and optional invocation observers |
| `[plugins]` | Any capability that is neither provider-shaped nor effect convenience |

Segno trigger and state backends also use the same process envelope with the
additional methods defined by the [Segno plugin wire](reference/segno-plugin-wire-v1.md).

## Wire lifecycle

Tactus writes one request document:

```json
{"api":"agenstro.plugin/v1","id":"request-1","method":"add","params":{"left":19,"right":23}}
```

The plugin can emit an event:

```json
{"type":"event","id":"request-1","event":{"type":"calculator.started"}}
```

It must then emit exactly one terminal result:

```json
{"type":"result","id":"request-1","ok":true,"value":{"sum":42}}
```

A known domain failure is structured:

```json
{"type":"result","id":"request-1","ok":false,"error":{"code":"invalid_operands","message":"left and right must be integers"}}
```

Every stdout line is protocol data. Send human diagnostics to stderr, keep
them bounded, and never print banners or debug objects to stdout.

## Reuse the strict Rust implementation

Inside this repository, a Rust plugin can reuse Tactus's tested request decoder
and wire types instead of implementing duplicate-key, number-domain, and
terminal rules again.

Create a binary crate and add local dependencies:

```toml
[dependencies]
serde_json = "=1.0.149"
tactus-runtime = { path = "D:/src/Agenstro/tactus-runtime" }
```

Use this minimal `src/main.rs`:

```rust
use std::io::{self, Read, Write};

use serde_json::{Map, json};
use tactus_runtime::{
    JsonField, PluginEvent, PluginFailure, PluginFrame, decode_request,
};

fn write_frame(
    output: &mut impl Write,
    frame: &PluginFrame,
) -> Result<(), Box<dyn std::error::Error>> {
    serde_json::to_writer(&mut *output, frame)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = Vec::new();
    io::stdin().read_to_end(&mut input)?;
    let request = decode_request(&input)?;

    let stdout = io::stdout();
    let mut output = stdout.lock();
    write_frame(
        &mut output,
        &PluginFrame::Event {
            id: request.id.clone(),
            event: PluginEvent {
                kind: "calculator.started".to_owned(),
                payload: Map::new(),
            },
        },
    )?;

    let terminal = if request.method == "add" {
        match (
            request.params.get("left").and_then(|value| value.as_i64()),
            request.params.get("right").and_then(|value| value.as_i64()),
        ) {
            (Some(left), Some(right)) => PluginFrame::Result {
                id: request.id,
                ok: true,
                value: JsonField::Present(json!({ "sum": left + right })),
                error: JsonField::Missing,
            },
            _ => PluginFrame::Result {
                id: request.id,
                ok: false,
                value: JsonField::Missing,
                error: JsonField::Present(PluginFailure {
                    code: "invalid_operands".to_owned(),
                    message: "left and right must be integers".to_owned(),
                    details: None,
                }),
            },
        }
    } else {
        PluginFrame::Result {
            id: request.id,
            ok: false,
            value: JsonField::Missing,
            error: JsonField::Present(PluginFailure {
                code: "method_not_found".to_owned(),
                message: "supported method: add".to_owned(),
                details: None,
            }),
        }
    };

    write_frame(&mut output, &terminal)?;
    Ok(())
}
```

This example uses the current source package, which is not published to
crates.io. A standalone third-party implementation may use TypeScript, C#,
Haskell, or another language, but its JSON decoder must enforce the same strict
contract rather than relying on a permissive default `JSON.parse` equivalent.

## Register the executable

Build the plugin and add an argv array to `.tactus/tactus.toml`:

```toml
[plugins.calculator]
command = ["D:/work/calculator/target/release/calculator-plugin.exe"]

[plugins.calculator.options]
```

Use forward slashes or properly quoted TOML strings for Windows paths. Tactus
uses argv arrays with `shell: false`; do not combine an executable and all
arguments into one shell command string.

## Test through Tactus

Call the plugin directly:

```powershell
tactus doctor
tactus plugin-call calculator add --namespace plugin `
  --params '{"left":19,"right":23}' --json
```

Then use it from Clef:

```haskell
{-# LANGUAGE DeriveAnyClass #-}
{-# LANGUAGE DeriveGeneric #-}
{-# LANGUAGE OverloadedStrings #-}

import Clef
import Data.Aeson (FromJSON, ToJSON)
import GHC.Generics (Generic)

data Add = Add { left :: Int, right :: Int }
  deriving (Generic, ToJSON)

data Sum = Sum { sum :: Int }
  deriving (Generic, FromJSON)

calculator :: Plugin Add Sum
calculator = jsonPlugin "calculator" "add"

workflow :: Workflow Sum
workflow = call calculator (Add 19 23)
```

## Required protocol behavior

A conforming implementation must:

- decode UTF-8 strictly;
- reject duplicate keys recursively;
- accept only the documented finite JSON number domain;
- require `api = agenstro.plugin/v1`;
- require a non-empty method and object params;
- copy the exact request ID to every frame;
- emit LF-delimited JSON objects;
- emit at most the configured number and size of frames;
- emit exactly one terminal result;
- emit no frames after the terminal; and
- make `ok`, `value`, and `error` structurally consistent.

Tactus treats malformed output as a protocol failure. If the process may have
performed an external action before losing the terminal, the caller can receive
`OutcomeUnknown`.

## Event design

Events are observational and open-ended. Use stable dotted types such as
`calculator.started` or `index.progress`, keep payloads bounded, and assume
low-priority progress may be aggregated or dropped under pressure. Never rely
on event delivery as the authoritative operation result.

Provider thinking and raw native output should be summarized before it enters
the protocol. Durable transitions use the separate state/trigger/guard/state
model in [Logs and run evidence](observability.md).

## Discovery methods

Plugins should normally implement:

- `describe`: implementation version, methods/operations, open option schema,
  and observed capabilities;
- `smoke`: an offline health check by default; and
- their domain methods.

Do not make `smoke` contact a paid or state-changing service unless a separate
explicit `live` parameter authorizes it.

## Security checklist

- Treat `params`, workspace files, and environment variables as untrusted data.
- Never invoke a shell merely to join argv.
- Constrain file operations to the declared workspace when that is part of the
  plugin contract.
- Bound time, output lines, total bytes, and child processes.
- Keep credentials out of result details and stderr.
- Classify a lost external outcome honestly instead of guessing failure.
- Test cancellation and descendant cleanup on Windows and Unix.

The normative frame grammar, numeric rules, limits, and adapter behavior are in
[Local plugin protocol v1](reference/plugin-protocol-v1.md).
