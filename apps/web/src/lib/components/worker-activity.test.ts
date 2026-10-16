import {render} from "svelte/server";
import {describe, expect, it} from "vitest";
import WorkerActivity from "./worker-activity.svelte";

describe("worker diagnostics", () => {
  it("renders errors and interaction details as escaped text with explicit retention limits", () => {
    const {body} = render(WorkerActivity, {props:{attemptNumber:1,events:[{
      sequence:12,message:"Environment lock validation failed",failed:true,details:"<script>alert(1)</script>\ncargo test",truncated:true,
    }]}});
    expect(body).toContain("Environment lock validation failed — failed");
    expect(body).toContain("View interaction details");
    expect(body).toContain("&lt;script>");
    expect(body).not.toContain("<script>");
    expect(body).toContain("Showing recent activity");
    expect(body).toContain("Details truncated");
    expect(body).toContain("Private reasoning is unavailable");
  });
  it("keeps historical content-free observations readable", () => {
    const {body} = render(WorkerActivity, {props:{attemptNumber:1,events:[{sequence:1,message:"Worker completed",failed:false}]}});
    expect(body).toContain("Worker completed");
    expect(body).not.toContain("View interaction details");
    expect(body).not.toContain("Showing recent activity");
  });
});
