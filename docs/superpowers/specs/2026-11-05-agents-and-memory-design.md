# Agents and memory

Approved in chat on 2026-11-05. This is a new application; data migrations and backward compatibility are not required.

## Agents

- Each agent has a stable ID, a generated editable name, an optional avatar, an optional personality, and a configured provider and model.
- Use connected providers. Multiple agents may use the same model.
- Personality defines tone, working style, and role. Keep it separate from learned memory. Agents cannot edit their own personality.
- Apply edits without restarting. Apply runtime changes after an active response finishes.
- Archive preserves conversations and memory and supports restoration. Permanent deletion separately offers choices about deleting conversations and private memory. Keep shared global and project memory.

## Conversations

- Each agent has a default direct chat and may have additional conversations, including multiple conversations in one project.
- A conversation has one fixed context: no project or exactly one project. Switching projects opens a separate conversation.
- Load only the current conversation's transcript and applicable saved memory. Do not automatically load other conversations' transcripts.
- A model change preserves the conversation, personality, and memory. Mark the change in the transcript. Never fall back to another model silently; offer retry and model selection.
- Summarize older messages when the model cannot fit the transcript. Preserve original messages and show that a summary is in use.

## Memory domains

| Domain | Visibility |
| --- | --- |
| Global | All agents in every context |
| Agent | One agent across projects and outside projects |
| Project | All agents in one project |
| Agent-project | One agent in one project |

Enforce boundaries in the application, not only in prompts. General chats read global and agent memory. Project chats also read their project and matching agent-project memory.

Current user instructions take priority over memory. Memory precedence is agent-project, project, agent, global. Keep conflicting entries inspectable.

## Capture and updates

- Automatically capture durable facts, preferences, decisions, and ongoing commitments. Exclude temporary details, guesses, and secrets such as passwords and tokens.
- Automatic writes go to agent memory in general chats and agent-project memory in project chats. Shared global/project writes require a user instruction or approval.
- Each memory is an editable item with a domain, source, creation time, and update time. Show automatic saves beside the response with Undo.
- Explicit user corrections may replace a memory. Inferred contradictions require clarification. Keep previous values in change history.
- Refresh applicable memory before every response, including existing conversations. Select relevant items and always include applicable saved instructions and preferences.
- Forget removes a saved item from retrieval and prevents immediate automatic recapture. It does not erase the source chat messages. Offer a fresh conversation when the user wants to stop using that history.

## Controls

Conversations show applicable memory and recent saves. Settings shows all four domains. Users can inspect, edit, delete, and change the domain of each item.

## First release

Implement agent creation and editing, direct and project conversations, and memory management. Defer group chats, unsolicited messages, and external actions such as editing files or sending messages. Keep visible planned controls disabled with `Soon` and a nearby `TODO` comment. Use the shared planned-item model for rail menus.
