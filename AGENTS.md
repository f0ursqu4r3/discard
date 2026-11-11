# Project instructions

## Unimplemented UI actions

- Keep planned menu items and controls visible. Disable them until their behavior is implemented.
- Add a `TODO` comment beside each unimplemented action or its shared definition. State the missing behavior.
- Use the shared planned-item model for rail menus. Planned actions must have no executable handler.
- Label planned actions `Soon` in the UI. Keep `TODO` in code, not in user-facing labels.
- Allow submenus to open when they contain planned actions; disable the actions inside them.
- Do not use a no-op, sample result, or unrelated destination as an implementation.
- Distinguish planned actions from implemented actions blocked by current state. A temporary disabled state does not need a `TODO`.
- Apply this rule to all new and changed UI surfaces, including plugin-provided controls.

## Communication

Use short, direct sentences and active voice. Lead with the result or required action. Use consistent terms. Preserve essential facts and evidence.

## Web tests

Put web tests in a `__tests__` directory beside the code they test.
