# Visual implementation plan

**Goal:** Apply the Design Lab layout to the existing Vue desktop app before adding new backend contracts.

**Architecture:** Keep the existing snapshot, history, commands, safe rendering, and protected decisions. Compose the workspace from small Vue components. Keep all existing Ghostty tokens and derive new surfaces from those tokens.

**Stack:** Vue 3, Tailwind v4, shadcn-vue/Reka, Lucide, Tauri.

**Reference:** [Design specification](README.md). The user's instruction to preserve the existing palette scheme takes precedence over the reference color values.

## Constraints

- Do not change Rust contracts or replace real data with fixtures.
- Keep provider/model selection in agent settings.
- Keep protected notices reachable in every workspace view.
- Keep existing branch tree behavior, context menus, Send/Stop, archive guards, and read tracking.
- Preserve existing token names, Ghostty palettes, plugin radius, font/spacing scale, and protected colors.
- Use local assets, semantic Tailwind colors, Lucide, and existing shadcn components.
- Leave generated output, local browser checks, and dependency caches out of Git.

## Tasks

- [x] **1. Primitives and geometry.** Add official Tabs, Sheet, Empty, InputGroup, Sonner, and MessageScroller. Add derived workspace surfaces without changing the palette bridge. Implement the shared 64px header line, 36px toolbar, 6px gutters, rounded panels, and three-dot resize handles.
- [x] **2. Workspace navigation.** Recompose `App.vue`, `ServerRail.vue`, `ChannelList.vue`, and `ChannelHeader.vue`. Add shared user footer, project navigation, DM entry, and context cards. Preserve saved channel width by adding the rail for display and subtracting it when saving. Respect the backend's current 180–400px persisted bounds. Keep unsupported actions disabled or route to existing settings.
- [x] **3. Conversation and secondary views.** Restyle `MessageRow.vue`, `Composer.vue`, and typed blocks. Adopt MessageScroller with stable row IDs, read tracking, stream following, and a jump control. Add view tabs and real-data/empty-state screens. Add named UI regions and a developer inspector. Use Sonner for transient feedback; retain protected decision surfaces.
- [x] **4. Verification and handoff.** Run relevant tests, typecheck, lint, and production build. Check desktop, compact layout, light/dark palettes, panel resizing, scrolling, and protected controls in a browser. Record remaining backend work.

## Review focus

- Saved panel widths must not shrink on reload or on a responsive layout change.
- A palette change must recolor new surfaces and preserve protected colors.
- Switching to DMs must not expose project history as a direct message or send to a project channel.
- Browsing history must not be interrupted by streamed output.
- Pending approvals must remain visible when another view or a plugin slot is active.

## Validation

Use the existing Vitest suites for theme, composer, layout, messages, protected surfaces, settings, and bridge behavior. Add behavioral tests for width conversion and new navigation boundaries. Use browser measurements for alignment and scrolling rather than tests that copy CSS class strings.

## Status

The visual phase is implemented. It does not add project memberships, direct-message persistence, agent creation, run records, or merge-request APIs. Those views use real available data or explicit unavailable/empty states.

### Implementation notes

- The existing 14 palette tokens and `applyTheme` bridge are unchanged. Workspace and card colors derive from those tokens. Ghostty palettes, protected colors, diff colors, font scale, spacing scale, and plugin radius still apply.
- The shell has aligned 64px headers, separate rounded panels, shared user footer, full-height/half-height/unread rail indicators, and keyboard/pointer resize handles.
- Saved sizes retain the native channel-only format. The rail width is added for display and subtracted before saving. Layout changes do not write preferences until a resize finishes.
- Drafts and transcript positions stay in frontend memory by channel when navigating. They are not persisted across app restarts.
- MessageScroller uses eager item layout in this app to avoid estimated-height shifts when returning to a conversation. Its upstream anchor tracking is adapted so a saved streaming reply cannot jump to old history. Pointer scrolling releases automatic following.
- Protected decisions remain outside plugin-owned content and remain available in secondary views. Existing plugin member-list replacement behavior is preserved.
- Sonner uses the existing palette variables and its packaged stylesheet. No remote fonts or new palette system were introduced.
- `.gitignore` excludes frontend dependencies, builds, test artifacts, local browser checks, and local environment overrides. Source, lockfile, design documents, and reference images remain eligible for Git.

### Verification

- Frontend unit tests cover existing behavior, panel width conversion, navigation isolation, draft retention, and IME input.
- Browser checks cover header alignment, shared footer width, rail overflow/pill position, pointer and keyboard resize behavior, draft retention across panel changes, transcript position on navigation, streaming completion, palette switching, compact sheets, and UI inspection.
- The production frontend build and lint checks pass. The build reports large syntax-highlighting chunks; this work does not change the existing Shiki language loading strategy.
- Native backend behavior remains unchanged. Browser checks use test-only bridge responses; no mock conversation data ships in the app.
