import type { RuntimeReadinessSummary } from "@ayni-orchestrator/grpc-bindings/server";
import { BffMappingError } from "./mapping.js";

/** Preserve unavailable/partial status; never infer readiness from a missing field. */
export function mapSummaryReadiness(value: RuntimeReadinessSummary | undefined) {
  if (!value || !Number.isInteger(value.operationalProfiles) || !Number.isInteger(value.readyProfiles)
    || value.operationalProfiles < 0 || value.readyProfiles < 0 || value.readyProfiles > value.operationalProfiles
    || (value.ready && (!value.complete || value.operationalProfiles === 0 || value.readyProfiles !== value.operationalProfiles))) {
    throw new BffMappingError();
  }
  return {complete:value.complete,ready:value.ready,operationalProfiles:value.operationalProfiles,readyProfiles:value.readyProfiles};
}
