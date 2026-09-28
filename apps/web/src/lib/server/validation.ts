const CANONICAL_IDENTIFIER = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u;
const RESOURCE_VERSION = /^(?:0|[1-9][0-9]{0,19})$/u;
const MAX_UINT64 = 18_446_744_073_709_551_615n;

export function isCanonicalIdentifier(value: string): boolean {
  return CANONICAL_IDENTIFIER.test(value);
}

export function isResourceVersion(value: string): boolean {
  return RESOURCE_VERSION.test(value) && BigInt(value) <= MAX_UINT64;
}
