import { render } from "svelte/server";
import { describe, expect, it } from "vitest";
import StatusBadge from "./status-badge.svelte";

describe("StatusBadge", () => {
  it("renders a human-readable status without relying on color alone", () => {
    const output = render(StatusBadge, { props: { status: "waiting_for_input", tone: "warning" } });
    expect(output.body).toContain("Waiting for input");
    expect(output.body).toContain("amber");
  });
});
