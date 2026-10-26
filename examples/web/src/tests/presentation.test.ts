import { describe, expect, it } from "vitest";

import { statusTone } from "$lib/presentation.js";

describe("clean-v1 status presentation", () => {
  it("distinguishes repository, profile, task, artifact and worker states", () => {
    expect(statusTone("connected")).toBe("success");
    expect(statusTone("ready")).toBe("success");
    expect(statusTone("succeeded")).toBe("success");
    expect(statusTone("verified")).toBe("success");

    expect(statusTone("connecting")).toBe("active");
    expect(statusTone("updating")).toBe("active");
    expect(statusTone("running")).toBe("active");
    expect(statusTone("verifying")).toBe("active");

    expect(statusTone("setup_required")).toBe("warning");
    expect(statusTone("setup_pr_open")).toBe("warning");
    expect(statusTone("awaiting_environment")).toBe("warning");
    expect(statusTone("waiting_for_runtime")).toBe("warning");

    expect(statusTone("degraded")).toBe("danger");
    expect(statusTone("failed")).toBe("danger");
    expect(statusTone("deadline_exceeded")).toBe("danger");
    expect(statusTone("quality_failed")).toBe("danger");
  });

  it("keeps unknown future enum values neutral", () => {
    expect(statusTone("future_state")).toBe("neutral");
  });
});
