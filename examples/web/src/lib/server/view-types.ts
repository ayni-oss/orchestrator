import type {AgentCapabilitiesDto,AgentConfigurationDto,BrowserErrorDto,DevelopmentTaskDto,OrchestratorBff,ProjectConversationDto,ProjectDto,ProviderModelDto,ProviderModelPageDto,RepositoryConnectionDto,RuntimeProfileDto,WorkspaceDto} from "@ayni-orchestrator/bff/server";
import type {ProjectView} from "../project-navigation.js";
export type ConnectionChoice=Pick<Awaited<ReturnType<OrchestratorBff["listConnections"]>>[number],"id"|"displayName">;
export type ModelCatalogues=Record<string,ProviderModelPageDto>;
export type RepositorySummary=Awaited<ReturnType<OrchestratorBff["listRepositorySummaries"]>>["summaries"][number];
/** Feature projection assembled by routes; no generated route types in components. */
export interface ProjectViewData {
  project:ProjectDto|undefined; workspace:WorkspaceDto|undefined;
  conversation:ProjectConversationDto|undefined; conversations:ProjectConversationDto[];
  conversationError:BrowserErrorDto|undefined;
  agentConfiguration:AgentConfigurationDto|undefined;capabilities:AgentCapabilitiesDto|undefined;configurationError:BrowserErrorDto|undefined;
  view:ProjectView;planVersion:string;idempotencyKey:string;taskKey:string;requestId:string;
  repositories:RepositoryConnectionDto[];profiles:RuntimeProfileDto[];tasks:DevelopmentTaskDto[];
  taskProgress:Record<string,Awaited<ReturnType<OrchestratorBff["taskProgress"]>>>;
  llmConnections:ConnectionChoice[];connectionModels:ModelCatalogues;models:ProviderModelDto[];provider:string;
}

export type {AgentCapabilitiesDto,AgentConfigurationDto,AgentProfileSettingsDto,ProjectAgentDefaultsDto,DevelopmentTaskDto,RuntimeProfileDto,RepositoryConnectionDto} from "@ayni-orchestrator/bff/server";
