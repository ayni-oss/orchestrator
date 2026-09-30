import { credentials, Metadata, status, type ClientOptions } from "@grpc/grpc-js";
import { randomUUID } from "node:crypto";
import { readFile } from "node:fs/promises";
import {
  DevelopmentTaskServiceClient,
  DevelopmentTaskState,
  RepositoryServiceClient,
  type WatchDevelopmentTaskResponse,
} from "../index.ts";

const MAX_REQUEST_BYTES = 1024 * 1024;
const MAX_RESPONSE_BYTES = 4 * 1024 * 1024;
const UNARY_DEADLINE_MS = 30_000;
const WATCH_DEADLINE_MS = 5 * 60_000;

function required(name: string): string {
  const value = process.env[name];
  if (value === undefined || value.length === 0) throw new Error(`${name} is required`);
  return value;
}

function metadata(token: string): Metadata {
  const value = new Metadata();
  value.set("authorization", `Bearer ${token}`);
  value.set("x-request-id", randomUUID());
  return value;
}

function deadline(milliseconds: number): Date {
  return new Date(Date.now() + milliseconds);
}

function reconnectDelay(attempt: number): number {
  const ceiling = Math.min(250 * (2 ** attempt), 2_000);
  return Math.floor((ceiling / 2) + (Math.random() * ceiling / 2));
}

async function sleep(milliseconds: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, milliseconds));
}

async function main(): Promise<void> {
  const endpoint = required("ORCHESTRATOR_GRPC_ENDPOINT");
  const tlsName = required("ORCHESTRATOR_GRPC_TLS_NAME");
  const [ca, tokenValue] = await Promise.all([
    readFile(required("ORCHESTRATOR_GRPC_CA_PATH")),
    readFile(required("ORCHESTRATOR_GRPC_TOKEN_PATH"), "utf8"),
  ]);
  const token = tokenValue.trim();
  if (
    token.length < 32
    || token.length > 512
    || [...token].some((character) => {
      const code = character.charCodeAt(0);
      return code < 0x21 || code > 0x7e;
    })
  ) throw new Error("credential file is invalid");

  const options: Partial<ClientOptions> = {
    "grpc.max_send_message_length": MAX_REQUEST_BYTES,
    "grpc.max_receive_message_length": MAX_RESPONSE_BYTES,
    "grpc.ssl_target_name_override": tlsName,
    "grpc.default_authority": tlsName,
  };
  const channelCredentials = credentials.createSsl(ca);
  const repositories = new RepositoryServiceClient(endpoint, channelCredentials, options);
  const tasks = new DevelopmentTaskServiceClient(endpoint, channelCredentials, options);

  try {
    const page = await new Promise<string[]>((resolve, reject) => {
      repositories.listRepositoryConnections(
        { page: { pageSize: 50, pageToken: "" } },
        metadata(token),
        { deadline: deadline(UNARY_DEADLINE_MS) },
        (error, response) => error === null
          ? resolve(response.repositoryConnections.map((repository) => repository.id))
          : reject(error),
      );
    });
    console.log(JSON.stringify({ repositoryConnections: page }));

    const taskId = process.env.ORCHESTRATOR_TASK_ID;
    if (taskId !== undefined) await watchTask(tasks, taskId, token);
  } finally {
    repositories.close();
    tasks.close();
  }
}

function terminal(state: DevelopmentTaskState | undefined): boolean {
  return state === DevelopmentTaskState.DEVELOPMENT_TASK_STATE_SUCCEEDED
    || state === DevelopmentTaskState.DEVELOPMENT_TASK_STATE_FAILED
    || state === DevelopmentTaskState.DEVELOPMENT_TASK_STATE_CANCELLED
    || state === DevelopmentTaskState.DEVELOPMENT_TASK_STATE_DEADLINE_EXCEEDED
    || state === DevelopmentTaskState.DEVELOPMENT_TASK_STATE_UNSUPPORTED;
}

async function watchTask(
  tasks: DevelopmentTaskServiceClient,
  taskId: string,
  token: string,
): Promise<void> {
  let cursor: string | undefined;
  for (let attempt = 0; attempt < 3; attempt += 1) {
    const completed = await new Promise<boolean>((resolve, reject) => {
      let lastState: DevelopmentTaskState | undefined;
      const stream = tasks.watchDevelopmentTask(
        { developmentTaskId: taskId, ...(cursor === undefined ? {} : { afterCursor: cursor }) },
        metadata(token),
        { deadline: deadline(WATCH_DEADLINE_MS) },
      );
      stream.on("data", (update: WatchDevelopmentTaskResponse) => {
        cursor = update.cursor;
        if (update.update?.$case === "developmentTask") lastState = update.update.developmentTask.state;
        console.log(JSON.stringify({ taskId, cursor, state: lastState }));
      });
      stream.on("end", () => resolve(terminal(lastState)));
      stream.on("error", (error: { code?: number }) => {
        if (error.code === status.UNAVAILABLE || error.code === status.DEADLINE_EXCEEDED) resolve(false);
        else reject(error);
      });
    });
    if (completed) return;
    if (attempt < 2) await sleep(reconnectDelay(attempt));
  }
  throw new Error("task watch did not complete after bounded reconnects");
}

main().catch((error: unknown) => {
  const code = typeof error === "object" && error !== null && "code" in error
    ? String(error.code)
    : "client_failure";
  console.error(`custom-client request failed (${code})`);
  process.exitCode = 1;
});
