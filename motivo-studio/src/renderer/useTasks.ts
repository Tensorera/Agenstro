import { useEffect, useRef, useState } from "react";
import type { TaskDocument, TaskSummary } from "../shared/task-contracts";
import { errorMessage } from "./format";

const POLL_MS = 1500;

export function useTasks(
  workspaceHandle: string | undefined,
  externalBusy: boolean,
  onError: (message: string | null) => void,
) {
  const [tasks, setTasks] = useState<TaskSummary[]>([]);
  const [loadedHandle, setLoadedHandle] = useState<string | undefined>(undefined);
  const [task, setTask] = useState<TaskDocument | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [operating, setOperating] = useState(false);
  const handleRef = useRef<string | undefined>(undefined);
  const selectedRef = useRef<string | null>(null);
  const listRequest = useRef(0);
  const currentRequest = useRef(0);
  const mutationRequest = useRef(0);
  const operationRef = useRef(false);
  const runningRef = useRef(false);

  useEffect(() => {
    handleRef.current = workspaceHandle;
    selectedRef.current = null;
    listRequest.current += 1;
    currentRequest.current += 1;
    mutationRequest.current += 1;
    operationRef.current = false;
    runningRef.current = false;
    if (!workspaceHandle) return;

    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let lastError: string | undefined;
    async function poll(): Promise<void> {
      let isCurrent = () => !disposed;
      try {
        if (operationRef.current || !workspaceHandle) return;
        const listToken = ++listRequest.current;
        isCurrent = () => !disposed && listToken === listRequest.current;
        const list = await window.motivo.tasks.list({ workspaceHandle });
        if (disposed || listToken !== listRequest.current) return;
        setLoadedHandle(workspaceHandle);
        setTasks(list);
        setOperating(false);
        runningRef.current = list.some((item) => item.status === "running");
        if (!selectedRef.current || !list.some((item) => item.id === selectedRef.current)) {
          selectedRef.current = list[0]?.id ?? null;
          setSelectedId(selectedRef.current);
          setTask(null);
          setLoading(Boolean(selectedRef.current));
        }
        const taskId = selectedRef.current;
        if (!taskId) return;
        const currentToken = ++currentRequest.current;
        isCurrent = () =>
          !disposed && currentToken === currentRequest.current && selectedRef.current === taskId;
        const document = await window.motivo.tasks.current({ workspaceHandle, taskId });
        if (disposed || currentToken !== currentRequest.current || selectedRef.current !== taskId) {
          return;
        }
        setTask((previous) =>
          previous?.id === document.id && previous.revision > document.revision
            ? previous
            : document,
        );
        lastError = undefined;
      } catch (caught) {
        if (isCurrent()) {
          const message = errorMessage(caught);
          if (message !== lastError) onError(message);
          lastError = message;
        }
      } finally {
        if (!disposed) {
          if (isCurrent()) setLoading(false);
          timer = setTimeout(() => void poll(), POLL_MS);
        }
      }
    }
    void poll();
    return () => {
      disposed = true;
      if (timer) clearTimeout(timer);
      listRequest.current += 1;
      currentRequest.current += 1;
      mutationRequest.current += 1;
    };
  }, [workspaceHandle, onError]);

  function select(taskId: string): void {
    if (taskId === selectedRef.current || operationRef.current || !workspaceHandle) return;
    selectedRef.current = taskId;
    setSelectedId(taskId);
    setTask(null);
    setLoading(true);
    const token = ++currentRequest.current;
    void window.motivo.tasks.current({ workspaceHandle, taskId }).then(
      (document) => {
        if (handleRef.current !== workspaceHandle || currentRequest.current !== token) return;
        setTask(document);
        setLoading(false);
      },
      (caught: unknown) => {
        if (handleRef.current !== workspaceHandle || currentRequest.current !== token) return;
        onError(errorMessage(caught));
        setLoading(false);
      },
    );
  }

  async function mutate(
    action: "create" | "continue" | "pause",
    input: {
      goal?: string;
      constraints?: string;
      provider?: string;
      maxCalls?: number;
      note?: string;
    },
  ): Promise<boolean> {
    if (!workspaceHandle || operationRef.current || externalBusy) return false;
    if (action !== "pause" && runningRef.current) return false;
    const taskId = selectedRef.current;
    if (action !== "create" && !taskId) return false;
    const token = ++mutationRequest.current;
    listRequest.current += 1;
    currentRequest.current += 1;
    operationRef.current = true;
    setOperating(true);
    onError(null);
    try {
      let document: TaskDocument;
      if (action === "create") {
        if (!input.goal || !input.provider) return false;
        document = await window.motivo.tasks.create({
          workspaceHandle,
          goal: input.goal,
          constraints: input.constraints ?? "",
          provider: input.provider,
        });
      } else {
        if (!taskId) return false;
        document =
          action === "pause"
            ? await window.motivo.tasks.pause({ workspaceHandle, taskId })
            : await window.motivo.tasks.continue({
                workspaceHandle,
                taskId,
                maxCalls: input.maxCalls ?? 4,
                ...(input.note ? { note: input.note } : {}),
              });
      }
      if (handleRef.current !== workspaceHandle || token !== mutationRequest.current) return false;
      selectedRef.current = document.id;
      setSelectedId(document.id);
      setTask(document);
      setTasks((previous) => [document, ...previous.filter((item) => item.id !== document.id)]);
      runningRef.current = document.status === "running";
      return true;
    } catch (caught) {
      if (handleRef.current === workspaceHandle && token === mutationRequest.current) {
        onError(errorMessage(caught));
      }
      return false;
    } finally {
      if (handleRef.current === workspaceHandle && token === mutationRequest.current) {
        operationRef.current = false;
        setOperating(false);
        setLoading(false);
      }
    }
  }

  const currentWorkspace = loadedHandle === workspaceHandle;
  const visibleTasks = currentWorkspace ? tasks : [];
  const visibleTask = currentWorkspace && task?.id === selectedId ? task : null;
  const running =
    visibleTasks.some((item) => item.status === "running") || visibleTask?.status === "running";
  return {
    tasks: visibleTasks,
    task: visibleTask,
    selectedId: currentWorkspace ? selectedId : null,
    loading: Boolean(workspaceHandle) && (!currentWorkspace || loading),
    operating: currentWorkspace && operating,
    running,
    operationRef,
    runningRef,
    select,
    mutate,
  };
}
