import { paletteSlots } from './agents/agent-icon'
import type { RailGroupOption } from './rail-layout'

export type ProjectMenuAction =
  | 'settings'
  | 'open-window'
  | 'remove'
  | 'reveal'
  | 'copy-name'
  | 'copy-path'
  | 'copy-id'
/** Rail actions of a project. `group-join` carries the group ID as `arg`. The rail applies them. */
export type RailMenuAction = 'group-none' | 'group-join' | 'group-new' | 'move-first' | 'move-last'
/** Actions of a group menu. `group-color` carries the color as `arg`, or none for the default. */
export type GroupMenuAction = 'group-rename' | 'group-color' | 'group-toggle' | 'group-ungroup'
export type MenuAction = ProjectMenuAction | RailMenuAction | GroupMenuAction

export type PlannedMenuEntry =
  | { kind: 'planned'; label: string; control?: 'checkbox' }
  | { kind: 'planned-radio'; labels: string[] }

export type ProjectMenuEntry =
  | { kind: 'separator' }
  | { kind: 'submenu'; label: string; children: ProjectMenuEntry[] }
  | {
      kind: 'action'
      label: string
      action: MenuAction
      /** Passed with the action, for example the target group. */
      arg?: string
      disabled?: boolean
      destructive?: boolean
      /** A theme color slot, drawn as a dot before the label. */
      swatch?: string
    }
  | PlannedMenuEntry

const separator: ProjectMenuEntry = { kind: 'separator' }
export const planned = (
  label: string,
  control?: 'checkbox',
): Extract<PlannedMenuEntry, { kind: 'planned' }> => ({
  kind: 'planned',
  label,
  control,
})

/** The rail groups a project can join, and the group that holds it now. */
export interface ProjectRailContext {
  groups: RailGroupOption[]
  groupId: string | null
}

export function projectMenu(
  busy: boolean,
  rail: ProjectRailContext = { groups: [], groupId: null },
): ProjectMenuEntry[] {
  return [
    // TODO: Persist read markers for all channels in the target project.
    planned('Mark Project as Read'),
    separator,
    // TODO: Create and assign agents to the target project.
    planned('Add Agent…'),
    separator,
    {
      kind: 'submenu',
      label: 'Mute Project',
      // TODO: Persist mute deadlines and expire them without stopping agents.
      children: [
        'For 15 Minutes',
        'For 1 Hour',
        'For 3 Hours',
        'For 8 Hours',
        'For 24 Hours',
        'Until I turn it back on',
      ].map((label) => planned(label)),
    },
    {
      kind: 'submenu',
      label: 'Notifications',
      children: [
        // TODO: Implement project notification modes while preserving approval badges.
        { kind: 'planned-radio', labels: ['All Activity', 'Needs Attention', 'None'] },
        separator,
        // TODO: Filter project-wide notification broadcasts.
        planned('Suppress Broadcasts', 'checkbox'),
        // TODO: Filter notifications for agent mentions.
        planned('Suppress Agent Mentions', 'checkbox'),
        // TODO: Filter summary notifications.
        planned('Suppress Summaries', 'checkbox'),
        // TODO: Mute notifications from scheduled tasks.
        planned('Mute Scheduled Tasks', 'checkbox'),
        separator,
        // TODO: Add per-project desktop notification delivery preferences.
        planned('Desktop Notifications', 'checkbox'),
      ],
    },
    // TODO: Filter muted channels out of the channel list without hiding pending approvals.
    planned('Hide Muted Channels', 'checkbox'),
    separator,
    { kind: 'action', label: 'Project Settings…', action: 'settings', disabled: busy },
    // TODO: Define project-scoped permissions without changing another agent's policy.
    planned('Permissions & Approvals…'),
    // TODO: Persist project display names, icons, and colors separately from disk paths.
    planned('Project Appearance…'),
    separator,
    // TODO: Create branch-bound project channels and select their canonical channel.
    planned('New Channel…'),
    // TODO: Add categories within a project and persist channel membership.
    planned('New Category…'),
    // TODO: Create scheduled agent tasks with their own execution and notification model.
    planned('Schedule Task…'),
    separator,
    { kind: 'action', label: 'Open in New Window', action: 'open-window' },
    { kind: 'action', label: 'Reveal in Finder', action: 'reveal' },
    {
      kind: 'submenu',
      label: 'Move to…',
      children: [
        {
          kind: 'submenu',
          label: 'Group',
          children: [
            // A project with no group has nothing to leave. The state blocks the action; it is not planned.
            {
              kind: 'action',
              label: 'No Group',
              action: 'group-none',
              disabled: rail.groupId === null,
            },
            ...(rail.groups.length ? [separator] : []),
            ...rail.groups.map((group): ProjectMenuEntry => ({
              kind: 'action',
              label: group.label,
              action: 'group-join',
              arg: group.id,
              disabled: group.id === rail.groupId,
            })),
            separator,
            { kind: 'action', label: 'New Group…', action: 'group-new' },
          ],
        },
        separator,
        { kind: 'action', label: 'First in Group', action: 'move-first' },
        { kind: 'action', label: 'Last in Group', action: 'move-last' },
      ],
    },
    separator,
    {
      kind: 'action',
      label: 'Remove Project…',
      action: 'remove',
      disabled: busy,
      destructive: true,
    },
    separator,
    {
      kind: 'submenu',
      label: 'Copy',
      children: [
        { kind: 'action', label: 'Project Name', action: 'copy-name' },
        { kind: 'action', label: 'Project Path', action: 'copy-path' },
        { kind: 'action', label: 'Project ID', action: 'copy-id' },
        // TODO: Support project icons before exposing an icon image link.
        planned('Project Icon Image Link'),
      ],
    },
  ]
}

export function groupMenu(collapsed: boolean): ProjectMenuEntry[] {
  return [
    // TODO: Persist read markers for all channels of the projects in the group.
    planned('Mark Group as Read'),
    separator,
    { kind: 'action', label: 'Rename Group…', action: 'group-rename' },
    {
      kind: 'submenu',
      label: 'Group Color',
      children: [
        { kind: 'action', label: 'Default', action: 'group-color' },
        separator,
        ...paletteSlots.map(({ slot, label }): ProjectMenuEntry => ({
          kind: 'action',
          label,
          action: 'group-color',
          arg: `ansi-${slot}`,
          swatch: `ansi-${slot}`,
        })),
      ],
    },
    separator,
    { kind: 'action', label: collapsed ? 'Expand' : 'Collapse', action: 'group-toggle' },
    { kind: 'action', label: 'Ungroup', action: 'group-ungroup' },
  ]
}
