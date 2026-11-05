# Rail Context Menus Implementation Plan

**Goal:** Give every project an accurate context menu and track planned UI actions globally.

**Architecture:** A typed project-menu model separates executable actions from planned entries. A recursive Vue renderer uses existing context-menu primitives. App handlers reuse project switching, creation, settings, and removal.

**Tech Stack:** Vue 3, TypeScript, Reka UI, existing Tauri commands.

**Spec:** ../specs/2026-11-05-rail-context-menus-design.md

## Global constraints

Keep planned options visible and disabled with TODO comments. Preserve project IDs, background-agent guards, removal confirmation, and plugin entries.

## Review focus

- Inactive project actions must use the clicked project.
- Disabled actions must never dispatch commands.
- Opening menus must not switch projects.
- Failed switching must not open a dialog for the wrong project.
- Nested menus must stay usable within the viewport.

## Tasks

- [x] Add the global rule and TODO markers to existing placeholders.
- [x] Add a project menu model and recursive renderer, including all mapped options.
- [x] Connect every rail project to targeted actions and removal confirmation.
- [x] Test inactive targets, disabled actions, submenu rendering, and plugin preservation.
- [x] Run frontend checks and tests; inspect the final diff and update documentation.
