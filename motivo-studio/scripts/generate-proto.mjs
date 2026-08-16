import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";

const root = resolve(import.meta.dirname, "..");
const repositoryRoot = resolve(root, "..");
const protoRoot = resolve(repositoryRoot, "proto");
const output = resolve(root, "src", "generated");
const protoc = resolve(root, "node_modules", "grpc-tools", "bin", "protoc.js");
const plugin = resolve(
  root,
  "node_modules",
  ".bin",
  process.platform === "win32" ? "protoc-gen-ts_proto.cmd" : "protoc-gen-ts_proto",
);
const files = [
  "agentro/common/v1/capability.proto",
  "agentro/common/v1/error.proto",
  "agentro/common/v1/pagination.proto",
  "agentro/common/v1/resource.proto",
  "agentro/execution/v1/run_service.proto",
  "agentro/schedule/v1/schedule_service.proto",
  "agentro/system/v1/system_service.proto",
  "agentro/workflow/v1/workflow_service.proto",
  "agentro/workspace/v1/workspace_service.proto",
];

rmSync(output, { recursive: true, force: true });
mkdirSync(output, { recursive: true });

const result = spawnSync(
  process.execPath,
  [
    protoc,
    `--plugin=protoc-gen-ts_proto=${plugin}`,
    `--ts_proto_out=${output}`,
    "--ts_proto_opt=env=node,esModuleInterop=true,forceLong=bigint,useExactTypes=true,useOptionals=messages,useDate=string,oneof=unions-value,outputServices=grpc-js,outputJsonMethods=false",
    `--proto_path=${protoRoot}`,
    `--proto_path=${resolve(root, "node_modules", "grpc-tools", "bin")}`,
    ...files.map((file) => resolve(protoRoot, file)),
  ],
  { cwd: root, stdio: "inherit" },
);

if (result.error) {
  throw result.error;
}
if (result.status !== 0) {
  throw new Error(`Protobuf generation failed with status ${String(result.status)}.`);
}

// ts-proto uses `any` only for internal enum and generic bootstrap casts. Keep
// the checked-in generated surface compatible with the repository's no-any gate.
const generatedFiles = [
  "google/protobuf/timestamp.ts",
  "agentro/common/v1/capability.ts",
  "agentro/common/v1/error.ts",
  "agentro/common/v1/pagination.ts",
  "agentro/common/v1/resource.ts",
  "agentro/execution/v1/run_service.ts",
  "agentro/schedule/v1/schedule_service.ts",
  "agentro/system/v1/system_service.ts",
  "agentro/workflow/v1/workflow_service.ts",
  "agentro/workspace/v1/workspace_service.ts",
];
for (const file of generatedFiles) {
  const path = resolve(output, file);
  const source = readFileSync(path, "utf8")
    .replaceAll("({} as any)", "({} as never)")
    .replaceAll("reader.int32() as any", "reader.int32() as never");
  writeFileSync(path, source, "utf8");
}
