import type {ProjectWorkerUpdate} from "@ayni-orchestrator/grpc-bindings/server";
import type {ProjectWorkerUpdateDto} from "./dto.js";
import {BffMappingError,mapAttemptState,mapWorkerProgress,mapWorkerResult,mapPullRequest} from "./mapping.js";
import {mapTaskOrigin} from "./task-origin.js";
import {safeText} from "./onboarding.js";

export function mapWorkerUpdate(value:ProjectWorkerUpdate,projectId:string,conversationId:string):ProjectWorkerUpdateDto {
  if(!value.origin||value.origin.projectId!==projectId||value.origin.conversationId!==conversationId)throw new BffMappingError();
  return {
    origin:mapTaskOrigin(value.origin),taskId:value.taskId,attemptId:value.attemptId,attemptNumber:value.attemptNumber,
    resourceVersion:value.resourceVersion,state:mapAttemptState(value.state),publicationOnly:value.publicationOnly,
    requestedInput:value.requestedInput?safeText(value.requestedInput,1024):undefined,
    progress:value.progress?mapWorkerProgress(value.progress):undefined,result:value.result?mapWorkerResult(value.result):undefined,
    pullRequest:value.pullRequest?mapPullRequest(value.pullRequest):undefined,failure:safeText(value.failure,512),updatedAt:value.updatedAt?.toISOString(),
  };
}
