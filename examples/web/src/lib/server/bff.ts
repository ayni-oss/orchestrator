import { env } from "$env/dynamic/private";
import { OrchestratorBff } from "@ayni-orchestrator/bff/server";

let instance: OrchestratorBff | undefined;

export function getBff(): OrchestratorBff {
  instance ??= new OrchestratorBff({
    address: env.ORCHESTRATOR_GRPC_ADDRESS ?? "127.0.0.1:50051",
  });
  return instance;
}
