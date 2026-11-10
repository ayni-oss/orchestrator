# Product design manual

This manual records the shared interface conventions for the web application.
Apply them to new screens and when changing existing screens. The implementation
lives in `apps/web/src/lib/components`; use those components rather than copying
page markup.

## Titles, badges and actions

Use `TitleRow` for a title that can have a badge or actions. `PageHeader` and
`InformationCard` compose it for page and card headings.

```text
title  badge                                  icons
```

The badge belongs beside the title. Actions align at the right edge. Both are
optional. Allow the title and badge to wrap together at narrow widths while
keeping icon actions reachable. Keep semantic heading levels independent of the
layout; a badge is not part of a heading's accessible name.

Use `StatusBadge` for state, with a meaningful text label and tone. Do not rely on
color alone. In compact activity rows, label each badge, such as Content,
Managed, Cleanup or Status. Put supporting information below the status row;
inspection entries put their commit above the labeled status. Time sits below
the info and detail icons on the right.

## Icon actions

Info, refresh and overflow actions use the shared Button's `ghost` variant and
an icon size. Their resting appearance has no visible border or filled surface.
Keep the hover treatment and keyboard focus ring. Give every icon button a
specific accessible name, and hide its decorative SVG from assistive technology.

Every info (i) icon uses `InfoTooltip` to show data on hover or keyboard focus.
Use the same compact definition list everywhere: muted labels above wrapping
values, monospace identifiers, a small popover with an end-aligned anchor and
consistent padding. Pass rows into the shared component instead of supplying
page-specific tooltip markup. Clicking an info icon never opens a dialog. Use an ellipsis or a labeled action
for full-detail dialogs. Refresh
reads the latest state; it does not retry a build or repeat a mutation. Disable a
refresh action while its request is running, retain the existing content, and
animate the refresh icon only when reduced motion is not requested.

## Loading and refresh

Render everything already known: headings, tabs, field labels, explanatory copy,
section boundaries and controls. Place skeletons only where data is pending.
Do not replace the whole page or a populated section with a loading sentence.
Server-rendered workspace and project metadata arrives with its page; keep the
existing route visible while server navigation resolves.

Match skeletons to the final content's spacing and dimensions. Use the shared
Skeleton, InformationTable loading mode, FieldsSkeleton and ActivitySkeleton.
Use a stable row height and reserve pagination space so moving between cached
history pages does not move the following section. Unknown collection lengths,
wrapped content and conditional server results cannot have exact dimensions
before they are known; avoid inventing counts or states to fill that gap.

Keep confirmed data visible during refresh and show loading state on the action
or region. A failed read must end its loading state and expose retry guidance;
an empty result is different from a failed request. Keep forms disabled until
the data needed to safely submit them is available. Skeletons are decorative;
use `aria-busy` and a concise screen-reader status where needed.

Read independent data concurrently. Opening collapsed history fetches its first
batch; returning to cached pages reuses the records. Repository histories request
10 records at a time, show 5 per page, and prefetch the next batch on the last
cached page. Cache lifetime is the repository page, with invalidation for refresh
and relevant mutations.

## Repository presentation

The repository header shows its source connection badge beside its title and
shows source integration details on hover or focus of the info icon. Visibility and branch are
non-interactive secondary-style labels, right aligned, with eye/eye-off and
branch icons. They must not look disabled or enter the tab order.

Status is the first tab; ayni, with its mark, is second. Status starts with the
current preparation row, without a redundant visible heading. Previous
preparations and inspection history are collapsed by default. Preserve full
identifiers and retained evidence in the entry's extended details.

For integration setup, put overall readiness beside the section title. Name each
requirement (source control, runtime registry and signing key) with a checkmark
when ready or an attention icon when incomplete. Put read-only connection details
in the shared info hover; keep the registry and signer selectors visible.

The lock-check step keeps the branch selector as its main control and preserves
the detailed report below it. Run the check when the branch changes. Show check
status beside the title and put a ghost refresh icon at the right to recheck. Keep the last report visible during
a recheck; a changed branch or source revision requires a fresh check before
showing a passing current status.

The review step uses the shared branch label and copyable code block for its
reviewed commit. Keep build-attempt and replacement controls visible, disabling
them when build permission is off; removing permission also clears replacement
permission. This keeps the form stable without granting extra authority.

## Verification

Exercise delayed responses as well as fast responses. Check that known page
structure is visible before data arrives, fields cannot submit prematurely,
refresh keeps existing rows visible, and failures leave a usable retry path.
Compare bounding boxes for known sections before and after loading at compact
and wide viewport sizes. Check keyboard access, dark mode and reduced motion
when changing shared controls. Fixture tests establish UI behavior; they do not
qualify provider access or Kubernetes execution.
