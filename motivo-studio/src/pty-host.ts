import { runPtyHost } from "./pty/host";
import { loadNodePty } from "./pty/node-pty-loader";

const parentPort = process.parentPort;
if (!parentPort) {
  throw new Error("Motivo PTY host requires an Electron utility-process parent port.");
}

const nodePtyRoot = process.argv[2];
if (!nodePtyRoot) {
  throw new Error("Motivo PTY host requires a locked node-pty package root.");
}

const stop = runPtyHost(parentPort, loadNodePty(nodePtyRoot), () => process.exit(0));
const exit = () => {
  stop();
  process.exit(0);
};
process.once("disconnect", exit);
process.once("SIGTERM", exit);
