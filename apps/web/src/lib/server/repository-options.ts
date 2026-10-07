import type { BrowserErrorDto, GithubInstallationOptionDto, GithubRepositoryOptionDto } from "@ayni-orchestrator/bff/server";

export interface RepositoryOptions {
  installations: GithubInstallationOptionDto[];
  selectedAppName: string;
  selectedInstallationId: string;
  repositories: GithubRepositoryOptionDto[];
  error?: BrowserErrorDto;
}
