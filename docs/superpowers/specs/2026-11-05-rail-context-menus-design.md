# Rail context menus

Approved direction: adapt Discord server actions to Discard projects. Keep all mapped options visible; disable missing behavior and track it with TODO comments. This rule applies throughout the project.

Every project gets its own menu, including inactive projects. Opening a menu does not select a project. Copy and Reveal use its saved identity. Project Settings and New Channel select it through the existing guarded flow. Remove requires confirmation and retains files, history, and memory.

Include read-all, agent creation, timed mute, notification modes and filters, hide muted channels, project settings, permissions, appearance, channel creation, categories, scheduled tasks, grouping/order, removal, and copy information. Unsupported actions remain disabled. Submenu containers remain inspectable. Do not invent saved notification or grouping state.

Use existing Reka context menu primitives for focus, keyboard navigation, submenus, and collision handling. Keep plugin entries after built-in actions. Retain current Channel terminology.

Approaches: expanding the Rust action enum for frontend navigation would couple it to dialogs and pages. A dedicated project menu can reuse existing backend commands and the existing plugin menu contract. Choose the dedicated menu; leave other context menus unchanged.

Persistent DMs do not exist yet. Do not create DM actions. Rail grouping now exists (schema 15); see `docs/project-channel-status.md`. Group actions of the project menu are real. Group read markers remain planned.
