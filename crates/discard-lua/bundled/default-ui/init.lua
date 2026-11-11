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
    { "text", "# " .. state.channel, weight = "bold", variant = "mono" },
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

-- One row per agent. `agent` sits on the row so `color = "agent"` resolves to that agent's identity.
local function agent_row(agent)
  local status = agent.status
  local avatar = { "avatar", name = agent.name, agent = agent.id }
  if status ~= "working" then
    avatar.status = STATUS_COLOR[status] or "border"
  end
  local row = {
    "row", gap = 12, align = "center", agent = agent.id,
    avatar,
    {
      "column", grow = true,
      { "text", agent.name, color = "agent", weight = "semibold", size = "xs" },
      {
        "row", gap = 4, align = "center",
        { "text", (STATUS_LABEL[status] or status) .. " · " .. agent.provider, size = "xs", color = "muted_foreground" },
      },
    },
  }
  if status == "working" then
    row[#row + 1] = { "spinner" }
  end
  return row
end

discard.slot("member_list", function(state)
  local agents = state.agents
  if agents == nil or #agents == 0 then
    agents = { state.agent }
  end
  -- The app draws the card frame around this slot, so the slot draws only its content.
  local column = {
    "column", gap = 16,
    { "text", "Agents", variant = "label" },
  }
  for _, agent in ipairs(agents) do
    column[#column + 1] = agent_row(agent)
  end
  return column
end)
