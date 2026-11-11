export type ProjectMenuAction =
  | 'settings'
  | 'remove'
  | 'reveal'
  | 'copy-name'
  | 'copy-path'
  | 'copy-id'

export type PlannedMenuEntry =
  | { kind: 'planned'; label: string; control?: 'checkbox' }
  | { kind: 'planned-radio'; labels: string[] }

export type ProjectMenuEntry =
  | { kind: 'separator' }
  | { kind: 'submenu'; label: string; children: ProjectMenuEntry[] }
  | {
      kind: 'action'
      label: string
      action: ProjectMenuAction
      disabled?: boolean
      destructive?: boolean
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

export function projectMenu(busy: boolean): ProjectMenuEntry[] {
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
    { kind: 'action', label: 'Reveal in Finder', action: 'reveal' },
    {
      kind: 'submenu',
      label: 'Move to…',
      children: [
        {
          kind: 'submenu',
          label: 'Group',
          children: [
            // TODO: Persist rail groups and move projects without changing disk folders.
            planned('No Group'),
            planned('New Group…'),
          ],
        },
        separator,
        // TODO: Persist project order within rail groups and the ungrouped rail.
        planned('First in Group'),
        planned('Last in Group'),
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
