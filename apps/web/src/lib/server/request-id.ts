import { randomUUID } from "node:crypto";

const SAFE_REQUEST_ID = /^[A-Za-z0-9._-]{1,64}$/u;

export function acceptedRequestId(value: string | null): string {
  return value !== null && SAFE_REQUEST_ID.test(value) ? value : randomUUID();
}
