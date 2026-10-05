import { z } from "zod/v3";
export const projectSchema = z.object({
  idempotencyKey: z.string().uuid(),
  displayName: z.string().trim().min(1, "Enter a project name.").refine(value => new TextEncoder().encode(value).length <= 256 && !/[\u0000-\u001f\u007f]/u.test(value), "Use at most 256 bytes without control characters."),
  repositoryConnectionIds: z.array(z.string().regex(/^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u)).min(1, "Select at least one repository.").max(100).refine(values => new Set(values).size === values.length, "Select each repository once."),
});
