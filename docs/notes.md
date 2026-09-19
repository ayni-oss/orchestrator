Separating fast, uncertified agent feedback from isolated, certified CI gating matches Ayni's intended lifecycle. The architecture explicitly distinguishes rapid "focused verification" during active development from final "independent validation" at the release gate.

**The Two-Tiered Quality Workflow**

- **Development & Agent Loop (Uncertified):** The agent operates in an editable checkout using host mode (`--host`) or direct workspace mounts. It consumes Ayni diagnostic signals to repair code and run targeted verification commands with minimal overhead. This provides the rapid feedback loop agents need to iterate, without claiming to generate tamper-proof evidence.

- **CI & Merge Gate (Certified):** Once candidate changes are frozen and submitted via pull request, the CI pipeline runs an unscoped, isolated managed check. This enforces container isolation, disables unauthorized network access, and generates the immutable `execution.json` evidence bundle required for formal acceptance.

**How a Provider-Neutral CI Helper Fits In**

Whether built as a TypeScript GitHub Action, a GitLab CI component, or a standard shell script, managing environment images and evidence in CI follows a clean three-step flow:

1. **Identity & Cache Lookup:** The runner computes the environment identity key using the `.ayni.lock` hash, target platform, and toolchain inputs. It checks whether a matching container image digest exists in your OCI container registry (e.g., `ghcr.io/org/repo-env:<hash>`).

2. **Build & Image Export:** If no image matches, the runner executes `ayni env build`. The helper action returns the newly built image reference in its output step, allowing your workflow to push the image to your registry for subsequent runs.

3. **Managed Check Execution:** The helper runs `ayni check` through an attempt-owned container engine (such as rootless Docker-in-Docker or Podman in CI), pulling the pre-built image to skip compilation overhead. It exports `signals.json` and `execution.json` as job artifacts for automated review.

This approach allows developers and coding agents to move quickly on host execution during active work, while ensuring pull requests and main branches remain gated by reproducible, certified evidence.

---
