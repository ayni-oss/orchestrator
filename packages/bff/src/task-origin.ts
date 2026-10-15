import type {TaskOrigin} from "@ayni-orchestrator/grpc-bindings/server";
import type {TaskOriginDto} from "./dto.js";
import {BffMappingError} from "./mapping.js";

export function mapTaskOrigin(value:TaskOrigin):TaskOriginDto {
  if(!value.projectId||!value.conversationId||!value.planId||!value.proposalId||!value.researchRunId||!/^[1-9][0-9]{0,19}$/u.test(value.planVersion))throw new BffMappingError();
  return {projectId:value.projectId,conversationId:value.conversationId,planId:value.planId,planVersion:value.planVersion,proposalId:value.proposalId,researchRunId:value.researchRunId};
}
