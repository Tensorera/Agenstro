import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import packageMetadata from "../../package.json";
import {
  ArtifactKind,
  EffectKind,
  PromptRole,
  WorkflowDefinition,
} from "../../src/generated/agentro/workflow/v1/workflow_service";

const bindingSchema = z
  .object({ output_name: z.string().min(1), source_task_id: z.string().min(1) })
  .strict();
const workflowSchema = z
  .object({
    id: z.string().min(1),
    outputs: z.array(z.object({ name: z.string().min(1), source: bindingSchema }).strict()),
    policy: z
      .object({
        fail_fast: z.boolean(),
        max_concurrency: z.number().int().positive(),
        max_fan_out: z.number().int().positive(),
      })
      .strict(),
    required_capabilities: z.array(z.string().min(1)),
    schema_version: z.literal("clef.workflow/v2"),
    tasks: z.array(
      z
        .object({
          domain_function: z.string().min(1),
          effects: z.array(
            z
              .object({
                kind: z.enum(["read", "create", "modify", "move", "delete", "shell", "network"]),
                path_glob: z.string().min(1).nullable(),
              })
              .strict(),
          ),
          effort: z.enum(["xhigh", "high", "medium", "low"]).nullable(),
          id: z.string().min(1),
          inputs: z.array(z.unknown()).max(0),
          outputs: z.array(
            z
              .object({
                description: z.string().min(1),
                kind: z.enum(["file", "directory", "text", "json"]),
                name: z.string().min(1),
                path: z.string().min(1).nullable(),
                required: z.boolean(),
              })
              .strict(),
          ),
          preferred_capabilities: z.array(z.string().min(1)),
          prompts: z.array(
            z
              .object({
                content: z.string().min(1),
                name: z.string().min(1).nullable(),
                priority: z.number().int(),
                role: z.enum(["policy", "context", "instruction", "repair"]),
              })
              .strict(),
          ),
          required_capabilities: z.array(z.string().min(1)),
        })
        .strict(),
    ),
  })
  .strict();
const fixtureSchema = z
  .object({
    expected: z.unknown(),
    fixture_version: z.literal("agentro.cross-language-fixture/v1"),
    products: z.array(z.unknown()),
    protocol: z
      .object({
        api_major: z.literal(1),
        api_minor: z.literal(0),
        workflow_proto: z.literal("agentro.workflow.v1.WorkflowDefinition"),
      })
      .strict(),
    release_version: z.string().min(1),
    workflow: workflowSchema,
  })
  .strict();

describe("shared alpha workflow DTO", () => {
  it("decodes the Python-generated fixture with the root Protobuf DTO", () => {
    const fixturePath = resolve(
      import.meta.dirname,
      "../../../fixtures/cross-language/alpha-workflow.json",
    );
    const fixture = fixtureSchema.parse(JSON.parse(readFileSync(fixturePath, "utf8")) as unknown);
    const workflow = WorkflowDefinition.fromPartial({
      schemaVersion: fixture.workflow.schema_version,
      id: fixture.workflow.id,
      tasks: fixture.workflow.tasks.map((task) => ({
        id: task.id,
        domainFunction: task.domain_function,
        prompts: task.prompts.map((prompt) => ({
          role: promptRole(prompt.role),
          content: prompt.content,
          name: prompt.name ?? undefined,
          priority: prompt.priority,
        })),
        inputs: [],
        outputs: task.outputs.map((output) => ({
          name: output.name,
          description: output.description,
          kind: artifactKind(output.kind),
          path: output.path ?? undefined,
          required: output.required,
        })),
        effects: task.effects.map((effect) => ({
          kind: effectKind(effect.kind),
          pathGlob: effect.path_glob ?? undefined,
        })),
        requiredCapabilities: task.required_capabilities,
        preferredCapabilities: task.preferred_capabilities,
        effort: undefined,
      })),
      outputs: fixture.workflow.outputs.map((output) => ({
        name: output.name,
        source: {
          sourceTaskId: output.source.source_task_id,
          outputName: output.source.output_name,
        },
      })),
      policy: {
        maxConcurrency: fixture.workflow.policy.max_concurrency,
        failFast: fixture.workflow.policy.fail_fast,
        maxFanOut: fixture.workflow.policy.max_fan_out,
      },
      requiredCapabilities: fixture.workflow.required_capabilities,
    });

    const decoded = WorkflowDefinition.decode(WorkflowDefinition.encode(workflow).finish());
    expect(decoded).toEqual(workflow);
    expect(decoded.tasks[0]?.prompts[0]?.content).toBe("print('alpha vertical slice')\n");
    expect(fixture.release_version).toBe(packageMetadata.version);
  });
});

function promptRole(value: "policy" | "context" | "instruction" | "repair"): PromptRole {
  switch (value) {
    case "policy":
      return PromptRole.PROMPT_ROLE_POLICY;
    case "context":
      return PromptRole.PROMPT_ROLE_CONTEXT;
    case "instruction":
      return PromptRole.PROMPT_ROLE_INSTRUCTION;
    case "repair":
      return PromptRole.PROMPT_ROLE_REPAIR;
  }
}

function artifactKind(value: "file" | "directory" | "text" | "json"): ArtifactKind {
  switch (value) {
    case "file":
      return ArtifactKind.ARTIFACT_KIND_FILE;
    case "directory":
      return ArtifactKind.ARTIFACT_KIND_DIRECTORY;
    case "text":
      return ArtifactKind.ARTIFACT_KIND_TEXT;
    case "json":
      return ArtifactKind.ARTIFACT_KIND_JSON;
  }
}

function effectKind(
  value: "read" | "create" | "modify" | "move" | "delete" | "shell" | "network",
): EffectKind {
  switch (value) {
    case "read":
      return EffectKind.EFFECT_KIND_READ;
    case "create":
      return EffectKind.EFFECT_KIND_CREATE;
    case "modify":
      return EffectKind.EFFECT_KIND_MODIFY;
    case "move":
      return EffectKind.EFFECT_KIND_MOVE;
    case "delete":
      return EffectKind.EFFECT_KIND_DELETE;
    case "shell":
      return EffectKind.EFFECT_KIND_SHELL;
    case "network":
      return EffectKind.EFFECT_KIND_NETWORK;
  }
}
