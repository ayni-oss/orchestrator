import { BffError, type BrowserErrorDto, type RequestContext } from "@ayni-orchestrator/bff/server";
import { randomUUID } from "node:crypto";

const SLUG = /^[a-z0-9](?:[a-z0-9._-]{0,62}[a-z0-9])?$/u;
const GIT_REVISION = /^[0-9a-f]{40}$/u;
const SHA256 = /^[0-9a-f]{64}$/u;
const OWNER_OR_REPOSITORY = /^[A-Za-z0-9_.-]{1,100}$/u;
const BRANCH = /^(?![./])(?!.*(?:\.\.|@\{|[~^:?*\[\\]))(?!.*\/$).{1,255}$/u;

export function requestContext(locals: App.Locals): RequestContext {
  return { requestId: locals.requestId, signal: locals.requestSignal };
}

export function safeError(error: unknown, requestId: string, fallback = "the browser server could not complete the request"): BrowserErrorDto {
  if (error instanceof BffError) return error.toDto();
  return { status: 500, reason: "internal", message: fallback, requestId, retryable: false };
}

export function actionKey(): string {
  return randomUUID();
}

export function text(form: FormData, name: string, maxLength: number): string | undefined {
  const value = form.get(name);
  if (typeof value !== "string") return undefined;
  const trimmed = value.trim();
  return trimmed.length > 0 && trimmed.length <= maxLength ? trimmed : undefined;
}

export function lines(value: string, maxItems: number, maxLength: number): string[] | undefined {
  const items = value.split(/\r?\n/u).map((item) => item.trim()).filter(Boolean);
  return items.length > 0 && items.length <= maxItems && items.every((item) => item.length <= maxLength) ? items : undefined;
}

export function optionalLines(value: string | undefined, maxItems: number, maxLength: number): string[] | undefined {
  if (value === undefined || value.trim() === "") return [];
  return lines(value, maxItems, maxLength);
}

export function isSlug(value: string): boolean { return SLUG.test(value); }
export function isGitRevision(value: string): boolean { return GIT_REVISION.test(value); }
export function isSha256(value: string): boolean { return SHA256.test(value); }
export function isOwnerOrRepository(value: string): boolean { return OWNER_OR_REPOSITORY.test(value); }
export function isBranch(value: string): boolean { return BRANCH.test(value); }
