-- Discard's default UI. Replace any slot by registering the same slot name in your own plugin.

local STATUS_LABEL = {
  working = "Working",
  awaiting_approval = "Waiting for approval",
  idle = "Idle",
  offline = "Offline",
}

local STATUS_COLOR = {
  working = "success",
  awaiting_approval = "warning",
  idle = "muted_foreground",
  offline = "border",
}

discard.slot("channel_header", function(state)
  local children = {
    "row", gap = 12, align = "center", padding_x = 16, padding_y = 12,
    { "text", "# " .. state.channel, weight = "bold" },
    { "spacer" },
  }
  if state.pending_approvals > 0 then
    children[#children + 1] = {
      "row", gap = 8, align = "center", color = "warning",
      { "badge", count = state.pending_approvals },
      { "text", "Needs approval", color = "warning" },
    }
  end
  return children
end)

discard.slot("member_list", function(state)
  local agent = state.agent
  local status = agent.status
  return {
    "column", gap = 8, padding = 12, grow = true, height_full = true, border_left = true,
    bg = "sidebar", color = "sidebar_foreground",
    { "text", "AGENTS", size = "xs", color = "muted_foreground" },
    {
      "row", gap = 8, align = "center",
      { "avatar", name = agent.name, size = "small", status = STATUS_COLOR[status] or "border" },
      {
        "column",
        { "text", agent.name },
        {
          "row", gap = 4, align = "center",
          { "text", (STATUS_LABEL[status] or status) .. " · " .. agent.provider, size = "xs", color = "muted_foreground" },
        },
      },
    },
  }
end)
