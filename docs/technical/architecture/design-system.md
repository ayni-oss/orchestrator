# Ayni Orchestrator UI design system

Use the existing shadcn-svelte components and theme tokens. Product components
compose these primitives; they do not introduce a separate visual framework.

## Information cards

Use `InformationCard` for cards that display repository, onboarding, runtime,
or configuration information. The repository detail page is the first screen
using this pattern. Apply it to other information cards as those screens are
updated.

Every card requires a visible title. Use a flexible header row without fixed
column widths. Keep the title on the left, taking the available space. Group
the optional status badge and vertical three-dot menu together at the far
right, aligned vertically. Omit absent controls without reserving empty space.
Align the opened menu to the right edge of its trigger.

Use the optional `titleAccessory` snippet for a compact icon immediately after
the title. Repository cards show the Ayni logo there only when Ayni is enabled;
do not repeat that indicator as a table row.

Repository list cards use one native link around the entire card, with a
visible keyboard focus ring and hover feedback. Keep their title plain and
omit the action menu. Do not nest links, buttons, or other interactive controls
inside a linked card.

Runtime cards use `InformationCard`'s `navigationHref` to create a full-card
native link with keyboard focus. Keep the title plain and omit the action
menu. Supporting body links remain separate, accessible controls above the
card link.

On runtime detail pages, the Current runtime identity card contains readiness,
preparation, certificate, source identity, and reconciliation data. Sync and
Rebuild belong in its menu; preserve pending-state and rebuild availability
guards. Environment cards use the same pattern for verification status, image
identity, and verification data. Keep findings and failures within the relevant
card, omit repeated identity values, and format timestamps as readable dates.

Long titles wrap within the available space. Give each menu an accessible name containing
the card title. Use the shadcn dropdown menu for keyboard navigation, focus
handling, dismissal, and disabled actions.

Avoid card descriptions. Put concrete information in the body, including
explanations that were previously descriptions. Do not repeat the header status
as a body row or duplicate runtime readiness in repository information.

Use `InformationTable` for body data. Despite its retained component name, it
renders a semantic definition list: muted, regular-weight labels and foreground
values. Label/value pairs sit side by side when their content fits. Otherwise
the value moves below its label automatically, based on the card's available
width and the content's natural width. Long values, including URLs and hashes,
wrap within the card instead of requiring horizontal scrolling.

Keep compact spacing, a minimum row height of 2rem, and no row dividers or hover
backgrounds. Rows may grow when stacked or wrapped. Use monospace text for
commits, fingerprints, and image references. Links remain keyboard accessible
and inherit the theme.

Actions such as Edit, Details, Analyze repository, and Retry analysis belong in
the menu. Selecting an action opens its existing editor or submits its existing
form; the menu does not bypass validation or change resource-version checks.
Cards without available actions omit the menu button.

All dropdown menus size their width to the longest label. Action, checkbox,
radio, and submenu rows share a 2rem height and keep labels on one line.
Constrain menus to the viewport; exceptionally long labels can scroll
horizontally instead of wrapping into taller rows.

```svelte
<InformationCard
  title="Ayni managed"
  status="ready"
  actions={[{ label: "Edit", onSelect: openEditor }]}
>
  <InformationTable rows={[
    { label: "Certificate", value: certificateName },
    { label: "Image destination", value: imageRepository },
  ]} />
</InformationCard>
```

## Section titles and responsive layout

Place section headings outside cards. Use `text-lg font-semibold` and a flex
row with the heading on the left and secondary information, such as live-update
status, on the right. Allow that row to wrap at narrow widths.

Related information cards may appear side by side on wide screens. Stack them
on narrow screens, keeping the same card header and body structure.

## Danger zone

Keep destructive configuration actions in a Danger zone at the bottom of the
detail page. Use the same information-card structure with a destructive border.
Explain what is removed, retained, and blocks removal as table rows.

Confirmation inputs and their final submit button remain in a form below the
data. Omit the three-dot menu on these cards: the explicit destructive button
is the only removal action and requires repository-name confirmation.

## Icons and status

Use Lucide icons for interface actions and the shared theme-aware Ayni SVG for
Ayni-specific items and the Ayni Orchestrator header. The application, documentation
site and repository README share the Ayni mark and the full product name
**Ayni Orchestrator**. Ayni by itself refers to the CLI or its runtime configuration. Do not add
decorative arrows to navigation buttons. Keep dropdown indicators and explicit
external-link icons where they communicate behavior.

Use `SidebarStatus` for sidebar configuration indicators. It shares colors,
accessible descriptions and tooltips. Leaf links place their status in the
far-right slot used by group chevrons; only expandable groups render a separate
chevron slot. A check means
ready, a warning means attention is required or status is unavailable, and an X
means configuration or verified access is missing. Never infer readiness from a failed
metadata request.

Under Config, Source control and Container registries are peers. Ayni opens a configuration overview with a Certificates card; certificate
management is reached from that card rather than a nested sidebar entry. Source control retains its connection health check. Container
registries show a check when an enabled, ready connection has passed both push
and pull tests, a warning when only pull access is verified, and an X when no
enabled connection has verified access. Cleanup warnings do not change this
permission indicator. Certificates show a check when a certificate exists and
an X when none exists. The Ayni sidebar link shows certificate presence, independently of registry permissions.
Unavailable metadata shows a warning instead of claiming configuration is missing.


## Notifications

Mount one shadcn Sonner Toaster in the root layout through
`ApplicationNotifications`. It follows the application theme. Use the shared
`notify` API for transient client events such as clipboard results and runtime
state changes. Never include credentials or raw server responses in a toast.

Action success and safe server errors are handled centrally from `page.form`;
do not repeat notification markup or state in pages and dialogs. Consecutive
requests may have the same text and must each notify, while repeated rendering
of one result must not. Page-load errors notify once per route and unchanged
failure; recovery permits a later failure to notify again. Polling must not
flood the toast stack. Errors remain until dismissed; other toasts expire.

Keep field validation beside inputs, and persistent readiness, failure reasons,
and repair guidance beside the affected resource. These are page content,
not transient notifications. Full-page route errors retain the error page.
