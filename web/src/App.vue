<script setup lang="ts">
import {
  computed,
  nextTick,
  onMounted,
  onUnmounted,
  reactive,
  ref,
  watch,
} from 'vue';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { useAppearanceSettings } from '@/bridge/appearance';
import { isTauri } from '@tauri-apps/api/core';
import { useMediaQuery } from '@vueuse/core';
import { ScanEye, GitPullRequest, ListChecks } from '@lucide/vue';
import { toast } from 'vue-sonner';
import { useStore } from './bridge/store';
import { useWebShortcuts } from './bridge/shortcuts';
import {
  setMemberListVisible,
  setPanels,
  toggleBranch,
  sendMessage,
  cancelTurn,
  startThread,
  openThread,
  closeThread,
  selectThread,
  leaveThread,
  runAction,
  switchChannel,
  openBranchChannel,
  chooseProjectFolder,
  getProjects,
  getRailLayout,
  moveRailProject,
  groupRailProjects,
  createRailGroup,
  moveRailGroup,
  renameRailGroup,
  setRailGroupColor,
  setRailGroupCollapsed,
  ungroupRailGroup,
  getProjectChannels,
  getConversations,
  selectProject,
  removeProject,
  revealProject,
  restoreConversation,
  openAgentChat,
  type ProjectEntry,
  type RailLayoutDto,
  type HistoryMatch,
} from './bridge/api';
import {
  createReply,
  joinReply,
  parseMessageSource,
  type MessageSource,
  type PendingReply,
} from './components/reply';
import type {
  AttachmentDto,
  MessageDto,
  ProjectChannelDto,
} from './bridge/types';
import ChannelList from './components/ChannelList.vue';
import ChannelHeader from './components/ChannelHeader.vue';
import MessageList from './components/MessageList.vue';
import Composer from './components/Composer.vue';
import TerminalPanel from './components/terminal/TerminalPanel.vue';
import {
  terminalAvailability,
  useTerminalVisibility,
} from './components/terminal/terminal-state';
import { useActionShortcut } from './bridge/terminal';
import { unescapeSlash } from './components/slash';
import Notices from './components/Notices.vue';
import ApprovalDock from './components/ApprovalDock.vue';
import ChangesView from './components/changes/ChangesView.vue';
import ToolCallBlock from './components/ToolCallBlock.vue';
import BrailleSpinner from './components/BrailleSpinner.vue';
import StatusLine from './components/StatusLine.vue';
import { channelBranch } from './components/workspace/channel-branch';
import { useAgentIdentity } from './components/agents/useAgentIdentity';
import { agentIdentity, mixedAgentMark } from './components/agents/agent-icon';
import { activeTheme } from './bridge/theme';
import { changeEntries, thinkingLine } from './components/messages';
import { selectedChannelApprovals } from './components/threads';
import {
  navigationWidth,
  panelWidths,
  savePanelWidths,
} from './components/layout';
import PluginElement from './plugin/PluginElement.vue';
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from './components/ui/resizable';
import { Tabs, TabsList, TabsTrigger } from './components/ui/tabs';
import {
  Sheet,
  SheetContent,
  SheetTitle,
  SheetDescription,
} from './components/ui/sheet';
import {
  Empty,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
  EmptyDescription,
} from './components/ui/empty';
import { Button } from './components/ui/button';
import { Badge } from './components/ui/badge';
import { Toaster } from './components/ui/sonner';
import NavigationPanel from './components/workspace/NavigationPanel.vue';
import DirectMessageSidebar from './components/workspace/DirectMessageSidebar.vue';
import ContextPanel from './components/workspace/ContextPanel.vue';
import ThreadPanel from './components/workspace/ThreadPanel.vue';
import ThreadActionDialogs from './components/workspace/ThreadActionDialogs.vue';
import WorkspacePageView from './components/workspace/WorkspacePage.vue';
import UiInspector from './components/workspace/UiInspector.vue';
import FriendsList from './components/agents/FriendsList.vue';
import AddAgentDialog from './components/agents/AddAgentDialog.vue';
import AgentHistorySearch from './components/agents/AgentHistorySearch.vue';
import AgentProfileDialog from './components/agents/AgentProfileDialog.vue';
import {
  agentAvatars,
  useAgentAvatar,
} from './components/agents/useAgentAvatar';
import type { WorkspacePage } from './components/workspace/types';
import type { ProjectMenuAction } from './components/project-menu';
import type { RailChange } from './components/rail-layout';
import type { TranscriptPosition } from './components/workspace/transcript';
// Match the macOS overlay in tauri.conf.json; browser previews keep their normal inset.
const integratedTitlebar = isTauri() && /mac/i.test(navigator.platform);
const store = useStore();
const mainPanel = ref<HTMLElement | null>(null);
const agentAvatar = useAgentAvatar(() => store.snapshot?.agentId);
const activeAgent = computed(
  () =>
    store.snapshot?.responder ?? {
      agentId: store.snapshot?.agentId ?? '',
      name: store.snapshot?.agent ?? '',
      avatar: agentAvatar.value,
    }
);
const activeAgentIdentity = useAgentIdentity(
  () => activeAgent.value.agentId,
  () => activeAgent.value.avatar
);
// The agents named in the thinking line of the open channel. The spinner color is a mix of their colors.
const thinkingAgents = computed(() =>
  store.snapshot?.turnRunning ? [activeAgent.value] : []
);
const thinkingColor = computed(() =>
  mixedAgentMark(thinkingAgents.value, activeTheme.value?.tokens ?? {})
);
const workingChannels = computed(() =>
  (store.snapshot?.channels ?? []).map((channel) =>
    channel.id === store.snapshot?.channelId && store.snapshot?.turnRunning
      ? {
          ...channel,
          agentId: activeAgent.value.agentId,
          agent: activeAgent.value.name,
          running: true,
        }
      : channel
  )
);
const projects = ref<ProjectEntry[]>([]);
const railLayout = ref<RailLayoutDto | null>(null);
const projectChannels = ref<ProjectChannelDto[]>([]);
const statusBranch = computed(() =>
  store.snapshot ? channelBranch(store.snapshot, projectChannels.value) : null
);
const workingAgents = computed(() => {
  const snapshot = store.snapshot;
  if (!snapshot) return [];
  const agents = new Map<
    string,
    { id: string; name: string; avatar: string | null }
  >();
  for (const channel of workingChannels.value) {
    if (channel.running)
      agents.set(channel.agentId, {
        id: channel.agentId,
        name: channel.agent,
        avatar: agentAvatars.value[channel.agentId] ?? null,
      });
  }
  if (snapshot.turnRunning)
    agents.set(activeAgent.value.agentId, {
      id: activeAgent.value.agentId,
      name: activeAgent.value.name,
      avatar: activeAgent.value.avatar,
    });
  return [...agents.values()];
});
let projectChannelRequest = 0;
// The project that owns `projectChannels`. A refresh keeps the list of the same project on screen.
let projectChannelsProject: string | null = null;
async function refreshProjectChannels(projectId: string | null) {
  const request = ++projectChannelRequest;
  if (projectId !== projectChannelsProject) {
    // Never show the channels of another project while the new list loads.
    projectChannels.value = [];
    projectChannelsProject = projectId;
  }
  if (!projectId) return;
  try {
    const channels = await getProjectChannels(projectId);
    if (
      request === projectChannelRequest &&
      store.snapshot?.projectId === projectId
    )
      projectChannels.value = channels;
  } catch (error) {
    if (request === projectChannelRequest)
      toast.error('Could not load project channels', {
        description: String(error),
      });
  }
}
async function refreshProjectContext() {
  await store.refresh();
  await refreshProjects();
  await refreshProjectChannels(store.snapshot?.projectId || null);
}
async function loadRailLayout() {
  try {
    return (await getRailLayout()) ?? null;
  } catch (error) {
    toast.error('Could not load the project rail', {
      description: String(error),
    });
    return null;
  }
}
async function refreshProjects() {
  const [list, layout] = await Promise.all([getProjects(), loadRailLayout()]);
  projects.value = list;
  railLayout.value = layout;
}
watch(
  () => store.snapshot?.projectId,
  (projectId) => void refreshProjectChannels(projectId || null),
  { immediate: true }
);
let stopProjectCatalogue: UnlistenFn | undefined;
let stopProjectProfiles: UnlistenFn | undefined;
onMounted(async () => {
  try {
    const stop = await listen(
      'discard://conversations',
      () => void refreshProjectChannels(store.snapshot?.projectId || null)
    );
    if (disposed) stop();
    else stopProjectCatalogue = stop;
    const stopProfiles = await listen('discard://projects', () => {
      void refreshProjects().catch((error) => toast.error(String(error)));
    });
    if (disposed) stopProfiles();
    else stopProjectProfiles = stopProfiles;
  } catch (error) {
    toast.error(String(error));
  }
});
watch(
  () => store.snapshot?.projectId,
  () => {
    void refreshProjects().catch((error) => toast.error(String(error)));
  },
  { immediate: true }
);
useWebShortcuts();
const { visible: terminalVisible, toggle: toggleTerminalVisible } =
  useTerminalVisibility();
const terminalShortcut = useActionShortcut('toggle_terminal');
const composer = ref<InstanceType<typeof Composer> | null>(null);
const drafts = reactive<Record<string, string>>({});
// The reply that waits for each composer. It uses the keys of `drafts`.
const replies = reactive<Record<string, PendingReply>>({});
const attachmentDrafts = reactive<
  Record<string, import('./bridge/types').AttachmentDto[]>
>({});
const transcriptPositions = reactive<Record<string, TranscriptPosition>>({});
const page = ref<WorkspacePage>('conversation');
const terminalAccess = computed(() =>
  terminalAvailability(
    store.snapshot ? { ...store.snapshot, page: page.value } : null
  )
);
const terminalShown = computed(
  () => terminalVisible.value && terminalAccess.value.available
);
function toggleTerminal() {
  if (terminalAccess.value.available) toggleTerminalVisible();
  else toast(terminalAccess.value.reason);
}
const lastDirectMessageKey = 'discard:last-direct-message';
let lastDirectMessage: string | null = null;
try {
  lastDirectMessage = localStorage.getItem(lastDirectMessageKey);
} catch {
  // Keep navigation usable when browser storage is unavailable.
}
function rememberDirectMessage(id: string | null) {
  lastDirectMessage = id;
  try {
    if (id) localStorage.setItem(lastDirectMessageKey, id);
    else localStorage.removeItem(lastDirectMessageKey);
  } catch {
    // The last DM still remains available for this session.
  }
}
watch(
  () =>
    [
      page.value,
      store.snapshot?.channelId,
      store.snapshot?.activeProjectId,
    ] as const,
  ([currentPage, channelId, projectId]) => {
    if (currentPage === 'conversation' && channelId && projectId === null)
      rememberDirectMessage(channelId);
  },
  { immediate: true }
);
const addAgentOpen = ref(false),
  profileId = ref<string | null>(null),
  profileBusy = ref(false);
const conversationTab = ref('conversation');
const sidebarMode = ref<'profile' | 'search'>('profile');
const historyQuery = ref(''),
  historyRevision = ref(0),
  profileRequest = ref(0);
const searchAgent = ref<{ id: string; name: string } | null>(null);
const messageJump = ref<{
  channelId: string;
  messageId: string;
  request: number;
} | null>(null);
let jumpRequest = 0;
/** A thread panel that has no thread yet: the anchor and a composer. The first send creates the thread. */
const draftThread = ref<{ parentChannelId: string; anchor: MessageDto } | null>(
  null
);
const threadPanel = ref<InstanceType<typeof ThreadPanel> | null>(null);
const threadJump = ref<{ messageId: string; request: number } | null>(null);

const navigationPage = computed<WorkspacePage>(() =>
  page.value === 'conversation' && !store.snapshot?.activeProjectId
    ? 'direct-messages'
    : page.value
);
let stopConversationOpened: UnlistenFn | undefined;
let stopAgentProfile: UnlistenFn | undefined;
let stopToggleTerminal: UnlistenFn | undefined;
let disposed = false;
onMounted(async () => {
  try {
    const stop = await listen('discard://conversation-opened', () =>
      navigate('conversation')
    );
    if (disposed) stop();
    else stopConversationOpened = stop;
    const stopProfile = await listen<string>(
      'discard://agent-profile',
      (event) => {
        if (profileBusy.value) return;
        if (event.payload) profileId.value = event.payload;
        else navigate('direct-messages');
      }
    );
    if (disposed) stopProfile();
    else stopAgentProfile = stopProfile;
  } catch (error) {
    toast.error(String(error));
  }
});
onMounted(async () => {
  try {
    const stop = await listen('discard://toggle-terminal', toggleTerminal);
    if (disposed) stop();
    else stopToggleTerminal = stop;
  } catch (error) {
    toast.error(String(error));
  }
});
onUnmounted(() => {
  disposed = true;
  stopToggleTerminal?.();
  stopConversationOpened?.();
  stopAgentProfile?.();
  stopProjectCatalogue?.();
  stopProjectProfiles?.();
});
const switching = ref(false),
  sending = ref(false);
const inspector = ref(false);
const isWide = useMediaQuery('(min-width: 1280px)');
const isCompact = useMediaQuery('(max-width: 639px)');
const navigationOpen = ref(false),
  contextOpen = ref(false);
const showContextPanel = computed(
  () =>
    isWide.value &&
    page.value === 'conversation' &&
    !!store.snapshot?.channelId &&
    !!store.snapshot?.memberListVisible
);
// The thread panel takes the right panel slot and the right sheet while it is open.
const threadActive = computed(
  () => !!draftThread.value || !!store.snapshot?.openThread
);
const threadVisible = computed(
  () =>
    page.value === 'conversation' &&
    !!store.snapshot?.channelId &&
    threadActive.value
);
const showThreadPanel = computed(() => isWide.value && threadVisible.value);
const threadSheetOpen = computed(() => !isWide.value && threadVisible.value);
const showRightPanel = computed(
  () => showContextPanel.value || showThreadPanel.value
);
// Draft text and attachments of the thread composer, kept apart from the channel composer.
const threadKey = computed(() =>
  draftThread.value
    ? `thread-draft:${draftThread.value.anchor.id}`
    : (store.snapshot?.openThread?.channelId ?? '')
);
const openThreadView = computed(() =>
  draftThread.value ? null : store.snapshot?.openThread
);
const threadParentName = computed(() =>
  store.snapshot?.activeProjectId
    ? store.snapshot.channel
    : (store.snapshot?.agent ?? '')
);
const threadPanelProps = computed(() => {
  const snapshot = store.snapshot;
  const open = openThreadView.value ?? null;
  return {
    thread: open,
    history: store.threadHistory ?? null,
    draftAnchor: draftThread.value?.anchor,
    parentChannelId:
      draftThread.value?.parentChannelId ??
      open?.parentChannelId ??
      snapshot?.channelId ??
      '',
    parentName: threadParentName.value,
    directMessage: !snapshot?.activeProjectId,
    projectId: snapshot?.activeProjectId,
    agentAvatar: open ? (agentAvatars.value[open.agentId] ?? null) : null,
    composerDisabled: threadStartDisabled.value,
    busy: switching.value || sending.value,
    replyDisabled: threadReplyDisabled.value,
    reply: replies[threadKey.value],
    draft: drafts[threadKey.value],
    attachments: attachmentDrafts[threadKey.value] ?? [],
    jumpTo: threadJump.value ?? undefined,
  };
});
const archivedLegacy = computed(
  () =>
    projectChannels.value.find(
      (channel) => channel.id === store.snapshot?.channelId
    )?.kind === 'archived_legacy'
);
// A general or text channel has no branch. Its agent works in the main checkout.
const branchless = computed(() => {
  const kind = projectChannels.value.find(
    (channel) => channel.id === store.snapshot?.channelId
  )?.kind;
  return kind === 'general' || kind === 'text';
});
const appearance = useAppearanceSettings((error) => {
  store.error = error;
});
const railWidth = computed(
  () =>
    (appearance.value.miniProjectRail ? 48 : 72) *
    (store.snapshot?.theme.spacingScale ?? 1) *
    (store.snapshot?.theme.fontScale ?? 1)
);
const currentLayout = ref<number[]>([]);
const heading = computed(() =>
  page.value === 'conversation'
    ? store.snapshot?.channelId
      ? store.snapshot.activeProjectId
        ? store.snapshot.channel
        : store.snapshot.agent
      : 'Agents'
    : page.value === 'direct-messages'
      ? 'Agents'
      : {
          overview: 'Project overview',
          agents: 'Project agents',
          memory: 'Project memory',
          artifacts: 'Artifacts',
          'project-settings': 'Project settings',
        }[page.value]
);
const changes = computed(() => changeEntries(store.history));
const showChangesTab = computed(
  () => !branchless.value || changes.value.length > 0
);
watch(showChangesTab, (shown) => {
  if (!shown && conversationTab.value === 'changes')
    conversationTab.value = 'conversation';
});
const tools = computed(() => {
  const rows = [
    ...store.history.map((message) => ({
      blocks: message.blocks,
      author: message.author,
      running: false,
    })),
    {
      blocks: store.snapshot?.liveBlocks ?? [],
      author: store.snapshot?.responder,
      running: !!store.snapshot?.turnRunning,
    },
  ];
  return rows.flatMap((row) =>
    row.blocks
      .filter((block) => block.type === 'tool_call')
      .map((block) => ({
        ...block,
        running: row.running,
        identity: agentIdentity(
          row.author ? row.author.avatar : agentAvatar.value,
          row.author?.agentId ?? store.snapshot?.agentId ?? '',
          activeTheme.value?.tokens ?? {}
        ),
      }))
  );
});
function navigate(value: WorkspacePage) {
  page.value = value;
  if (value !== 'conversation') contextOpen.value = false;
  navigationOpen.value = false;
  if (value === 'conversation') conversationTab.value = 'conversation';
}
async function navigateFromSidebar(value: WorkspacePage) {
  if (value !== 'direct-messages') {
    navigate(value);
    return;
  }
  if (switching.value) return;
  const snapshot = store.snapshot;
  if (snapshot?.channelId && snapshot.activeProjectId === null) {
    navigate('conversation');
    return;
  }
  if (!lastDirectMessage) {
    navigate('direct-messages');
    return;
  }
  switching.value = true;
  try {
    const conversations = await getConversations();
    if (disposed) return;
    const previous = conversations.find(
      (conversation) =>
        conversation.id === lastDirectMessage && conversation.projectId === null
    );
    if (!previous) {
      rememberDirectMessage(null);
      navigate('direct-messages');
      return;
    }
    await switchChannel(previous.id);
    await store.refresh();
    if (!disposed) navigate('conversation');
  } catch (error) {
    toast.error('Could not open direct message', {
      description: String(error),
    });
  } finally {
    switching.value = false;
  }
}
function openProviderConnections() {
  addAgentOpen.value = false;
  void action('open_provider_settings');
}
async function action(id: string) {
  try {
    if (id === 'add_agent') {
      addAgentOpen.value = true;
      return;
    }
    if (id === 'show_friends') {
      navigate('direct-messages');
      return;
    }
    if (id === 'open_agent_settings' || id.startsWith('view_agent_profile:')) {
      if (profileBusy.value) return;
      const agentId = id.startsWith('view_agent_profile:')
        ? id.slice('view_agent_profile:'.length)
        : store.snapshot?.agentId;
      if (agentId) profileId.value = agentId;
      else navigate('direct-messages');
      return;
    }
    if (id.startsWith('message_agent:')) {
      if (switching.value) return;
      switching.value = true;
      try {
        const channelId = await openAgentChat(
          id.slice('message_agent:'.length)
        );
        await switchChannel(channelId);
        await store.refresh();
        profileId.value = null;
        navigate('conversation');
      } finally {
        switching.value = false;
      }
      return;
    }
    if (id === 'import_project' || id === 'change_project_folder') {
      await chooseProjectFolder(id === 'change_project_folder');
      await store.refresh();
      await refreshProjects();
      return;
    }
    if (id.startsWith('select_project:')) {
      if (switching.value) return;
      switching.value = true;
      try {
        const channelId = await selectProject(
          id.slice('select_project:'.length)
        );
        await store.refresh();
        await refreshProjectChannels(store.snapshot?.projectId || null);
        if (channelId) navigate('conversation');
      } finally {
        switching.value = false;
      }
      return;
    }
    if (id === 'remove_project' && store.snapshot) {
      await removeProject(store.snapshot.projectId);
      await store.refresh();
      await refreshProjects();
      navigate('conversation');
      return;
    }
    await runAction(id);
  } catch (error) {
    toast.error('Could not complete action', { description: String(error) });
  }
}
async function projectAction(id: ProjectMenuAction, project: ProjectEntry) {
  if (switching.value) return;
  switching.value = true;
  try {
    if (id === 'copy-name' || id === 'copy-path' || id === 'copy-id') {
      await navigator.clipboard.writeText(
        id === 'copy-name'
          ? project.name
          : id === 'copy-path'
            ? project.path
            : project.id
      );
    } else if (id === 'reveal') {
      await revealProject(project.path);
    } else if (id === 'remove') {
      const wasSelected = store.snapshot?.projectId === project.id;
      await removeProject(project.id);
      await store.refresh();
      await refreshProjects();
      if (wasSelected) navigate('conversation');
    } else {
      if (store.snapshot?.projectId !== project.id) {
        await selectProject(project.id);
        await store.refresh();
        await refreshProjectChannels(store.snapshot?.projectId || null);
      }
      if (store.snapshot?.projectId !== project.id)
        throw new Error('Project selection did not complete');
      if (id === 'settings') navigate('project-settings');
    }
  } catch (error) {
    toast.error('Could not complete project action', {
      description: String(error),
    });
  } finally {
    switching.value = false;
  }
}
// The backend emits `discard://projects` after each change, which reloads the rail.
async function railChange(change: RailChange) {
  try {
    if (change.type === 'move-project')
      await moveRailProject(change.projectId, change.groupId, change.index);
    else if (change.type === 'group-projects')
      await groupRailProjects(change.draggedId, change.targetId);
    else if (change.type === 'create-group')
      change.created?.(await createRailGroup(change.projectId));
    else if (change.type === 'move-group')
      await moveRailGroup(change.groupId, change.index);
    else if (change.type === 'rename-group')
      await renameRailGroup(change.groupId, change.name);
    else if (change.type === 'color-group')
      await setRailGroupColor(change.groupId, change.color);
    else if (change.type === 'collapse-group')
      await setRailGroupCollapsed(change.groupId, change.collapsed);
    else await ungroupRailGroup(change.groupId);
  } catch (error) {
    toast.error('Could not update the project rail', {
      description: String(error),
    });
  }
}
async function searchHistory(query: string) {
  const snapshot = store.snapshot;
  if (!snapshot?.agentId || snapshot.activeProjectId || !query.trim()) return;
  await closeThreadPanel();
  searchAgent.value = { id: snapshot.agentId, name: snapshot.agent };
  historyQuery.value = query.trim();
  historyRevision.value++;
  sidebarMode.value = 'search';
  try {
    if (isWide.value) await setMemberListVisible(true);
    else contextOpen.value = true;
  } catch (error) {
    toast.error(String(error));
  }
}
const missingSourceNotice = 'Original message is not available.';
async function selectHistoryMatch(
  match: Pick<HistoryMatch, 'channelId' | 'messageId'>,
  missing?: string
) {
  if (switching.value) return;
  switching.value = true;
  try {
    if (store.snapshot?.channelId !== match.channelId)
      await switchChannel(match.channelId);
    await store.refresh();
    await nextTick();
    // A reply link can point to a message that is gone. Open the channel, skip the jump, and say so.
    const found =
      !missing || store.history.some((item) => item.id === match.messageId);
    if (found)
      messageJump.value = {
        channelId: match.channelId,
        messageId: match.messageId,
        request: ++jumpRequest,
      };
    navigate('conversation');
    if (!isWide.value) contextOpen.value = false;
    if (!found && missing) toast.error(missing);
  } catch (error) {
    if (missing) toast.error(missing, { description: String(error) });
    else toast.error('Could not open message', { description: String(error) });
  } finally {
    switching.value = false;
  }
}
async function toggleMembers() {
  profileRequest.value++;
  if (threadActive.value) {
    // The thread panel holds the slot. Closing it restores the context panel when that was shown.
    await closeThreadPanel();
    if (
      isWide.value &&
      store.snapshot?.memberListVisible &&
      sidebarMode.value === 'profile'
    )
      return;
  }
  if (
    sidebarMode.value === 'search' &&
    (showContextPanel.value || contextOpen.value)
  ) {
    sidebarMode.value = 'profile';
    return;
  }
  sidebarMode.value = 'profile';
  if (!isWide.value) {
    contextOpen.value = !contextOpen.value;
    return;
  }
  try {
    await setMemberListVisible(!store.snapshot!.memberListVisible);
  } catch {
    toast.error('Could not change panel visibility');
  }
}
async function savePanels() {
  await nextTick();
  const snapshot = store.snapshot;
  if (
    !snapshot ||
    isCompact.value ||
    !savePanelWidths(showRightPanel.value, currentLayout.value.length)
  )
    return;
  // The thread panel has its own size. Only the context panel size is saved.
  const [channel, member] = panelWidths(
    showThreadPanel.value
      ? currentLayout.value.slice(0, 2)
      : currentLayout.value,
    railWidth.value,
    snapshot.panels.memberList
  );
  try {
    await setPanels(channel, member);
  } catch {
    toast.error('Could not save panel sizes');
  }
}
function resizeFinished(dragging: boolean) {
  if (!dragging) void savePanels();
}
const replyDisabled = computed(
  () =>
    !store.snapshot?.channelId ||
    store.snapshot.composerDisabled ||
    store.snapshot.archived ||
    switching.value ||
    sending.value ||
    store.snapshot.archiveBusy ||
    !!store.snapshot.banners.archive
);
function reply(message: MessageDto) {
  const snapshot = store.snapshot;
  if (!snapshot || replyDisabled.value) return;
  const source = store.history.find((item) => item.id === message.id);
  if (!source) return;
  const pending = createReply(source, snapshot.channelId, snapshot.agent);
  if (!pending) return;
  replies[snapshot.channelId] = pending;
  void composer.value?.focusEnd?.();
}
function cancelReply(key: string) {
  delete replies[key];
}
function jumpToSource(source: MessageSource) {
  const thread = store.snapshot?.openThread;
  if (source.channelId === thread?.channelId) {
    // A reply to a thread message stays in the thread panel.
    const history = store.threadHistory;
    if (
      history &&
      !history.messages.some((item) => item.id === source.messageId)
    ) {
      toast.error(missingSourceNotice);
      return;
    }
    threadJump.value = { messageId: source.messageId, request: ++jumpRequest };
    return;
  }
  conversationTab.value = 'conversation';
  void selectHistoryMatch(source, missingSourceNotice);
}
function openMessageSource(event: MouseEvent) {
  const anchor =
    event.target instanceof Element ? event.target.closest('a') : null;
  const source = parseMessageSource(anchor?.getAttribute('href') ?? '');
  if (!source) return;
  event.preventDefault();
  jumpToSource(source);
}
async function send(
  text: string,
  attachments: import('./bridge/types').AttachmentDto[] = []
) {
  const id = store.snapshot?.channelId;
  if (!id || switching.value || sending.value) return;
  sending.value = true;
  const pending = replies[id];
  try {
    await sendMessage(
      id,
      pending
        ? joinReply(pending.prefix, unescapeSlash(text))
        : unescapeSlash(text),
      attachments.map((file) => file.id)
    );
    // `text` is the typed text. The draft never holds the reply prefix.
    if (drafts[id]?.trim() === text) drafts[id] = '';
    if (replies[id] === pending) delete replies[id];
    const sentIds = new Set(attachments.map((file) => file.id));
    attachmentDrafts[id] = (attachmentDrafts[id] ?? []).filter(
      (file) => !sentIds.has(file.id)
    );
  } catch (error) {
    toast.error('Message was not sent', { description: String(error) });
  } finally {
    sending.value = false;
  }
}
const threadStartDisabled = computed(
  () =>
    !store.snapshot?.channelId ||
    store.snapshot.composerDisabled ||
    store.snapshot.archived
);
const threadReplyDisabled = computed(() => {
  const open = openThreadView.value;
  // A sub-agent thread has no composer to take a reply. Its reply goes to the channel composer.
  if (open?.kind === 'sub_agent') return replyDisabled.value;
  return (
    !threadActive.value ||
    (open ? open.composerDisabled : threadStartDisabled.value) ||
    switching.value ||
    sending.value
  );
});
function startThreadDraft(message: MessageDto) {
  const snapshot = store.snapshot;
  if (
    !snapshot ||
    threadStartDisabled.value ||
    message.thread ||
    message.postedFrom
  )
    return;
  draftThread.value = { parentChannelId: snapshot.channelId, anchor: message };
  // One thread panel at a time: the open thread closes while the draft shows.
  if (snapshot.openThread) void closeThreadPanel(true);
}
async function openThreadById(id: string) {
  draftThread.value = null;
  try {
    await openThread(id);
    await store.refresh();
  } catch (error) {
    toast.error('Could not open thread', { description: String(error) });
  }
}
async function closeThreadPanel(keepDraft = false) {
  if (!keepDraft && draftThread.value) {
    delete drafts[threadKey.value];
    delete replies[threadKey.value];
    delete attachmentDrafts[threadKey.value];
    draftThread.value = null;
  }
  if (!store.snapshot?.openThread) return;
  try {
    await closeThread();
    await store.refresh();
  } catch (error) {
    toast.error('Could not close thread', { description: String(error) });
  }
}
async function openThreadFromBar(id: string) {
  if (switching.value) return;
  switching.value = true;
  draftThread.value = null;
  try {
    await selectThread(id);
    await store.refresh();
    await refreshProjectChannels(store.snapshot?.projectId || null);
    navigate('conversation');
  } catch (error) {
    toast.error('Could not open thread', { description: String(error) });
  } finally {
    switching.value = false;
  }
}
async function leaveThreadFromBar(id: string) {
  try {
    await leaveThread(id);
    await store.refresh();
    await refreshProjectChannels(store.snapshot?.projectId || null);
  } catch (error) {
    toast.error('Could not leave thread', { description: String(error) });
  }
}
async function sendThread(
  text: string,
  attachments: AttachmentDto[],
  alsoPost: boolean
) {
  if (switching.value || sending.value) return;
  const draft = draftThread.value;
  const open = draft ? null : store.snapshot?.openThread;
  const key = threadKey.value;
  if (!draft && !open) return;
  sending.value = true;
  const pending = replies[key];
  try {
    const ids = attachments.map((file) => file.id);
    const typed = unescapeSlash(text);
    const body = pending ? joinReply(pending.prefix, typed) : typed;
    if (draft) {
      // No thread exists before this call. It creates the thread and opens it.
      await startThread(
        draft.parentChannelId,
        draft.anchor.id,
        body,
        ids,
        alsoPost
      );
      await store.refresh();
      if (draftThread.value === draft) draftThread.value = null;
    } else if (open) await sendMessage(open.channelId, body, ids, alsoPost);
    // `text` is the typed text. The draft never holds the reply prefix.
    if (drafts[key]?.trim() === text) drafts[key] = '';
    if (replies[key] === pending) delete replies[key];
    const sentIds = new Set(attachments.map((file) => file.id));
    attachmentDrafts[key] = (attachmentDrafts[key] ?? []).filter(
      (file) => !sentIds.has(file.id)
    );
  } catch (error) {
    toast.error('Message was not sent', { description: String(error) });
  } finally {
    sending.value = false;
  }
}
async function stopThread() {
  const open = store.snapshot?.openThread;
  if (!open) return;
  try {
    await cancelTurn(open.channelId);
  } catch {
    toast.error('Could not stop the agent');
  }
}
function replyInThread(message: MessageDto) {
  const snapshot = store.snapshot;
  if (!snapshot || threadReplyDisabled.value) return;
  const open = openThreadView.value;
  const history = store.threadHistory;
  const anchor = draftThread.value?.anchor ?? history?.anchor;
  // The anchor lives in the parent channel; every other thread message lives in the thread.
  const [source, channelId] =
    anchor?.id === message.id
      ? [anchor, draftThread.value?.parentChannelId ?? history?.parentChannelId]
      : [
          history?.messages.find((item) => item.id === message.id),
          open?.channelId,
        ];
  if (!source || !channelId) return;
  const pending = createReply(source, channelId, open?.agent ?? snapshot.agent);
  if (!pending) return;
  if (open?.kind === 'sub_agent') {
    replies[snapshot.channelId] = pending;
    void composer.value?.focusEnd?.();
    return;
  }
  replies[threadKey.value] = pending;
  void threadPanel.value?.focusComposer?.();
}
async function copyUnsavedReply() {
  const text =
    store.snapshot?.liveBlocks
      .map((block) => ('text' in block ? block.text : ''))
      .filter(Boolean)
      .join('\n\n') ?? '';
  try {
    await navigator.clipboard.writeText(text);
    toast.success('Reply copied');
  } catch (cause) {
    toast.error(String(cause));
  }
}
async function restoreCurrentConversation() {
  const id = store.snapshot?.channelId;
  if (!id) return;
  try {
    await restoreConversation(id);
    await store.refresh();
  } catch (cause) {
    toast.error(String(cause));
  }
}
async function retry() {
  const last = [...store.history]
    .reverse()
    .find((message) => message.role === 'user');
  const text = last?.blocks
    .map((block) => ('text' in block ? block.text : ''))
    .filter(Boolean)
    .join('\n\n');
  if (text) await send(text);
}
/** The agent request that a decision is working on. The card disables its buttons meanwhile. */
const decidingRequest = ref<string | null>(null);
async function decideAgentRequest(requestId: string, decision: 'add' | 'deny') {
  if (decidingRequest.value || switching.value) return;
  decidingRequest.value = requestId;
  try {
    await store.decideAgentRequest(requestId, decision);
    // A decision can continue the requesting agent. Load the snapshot again.
    await store.refresh();
  } catch (error) {
    toast.error('Could not decide request', { description: String(error) });
  } finally {
    decidingRequest.value = null;
  }
}
async function stop() {
  if (store.snapshot)
    try {
      await cancelTurn(store.snapshot.channelId);
    } catch {
      toast.error('Could not stop the agent');
    }
}
async function selectChannel(id: string) {
  if (switching.value) return;
  switching.value = true;
  try {
    await switchChannel(id);
    await store.refresh();
    navigate('conversation');
  } catch (error) {
    toast.error('Could not open channel', { description: String(error) });
  } finally {
    switching.value = false;
  }
}
async function inspectChannel(id: string) {
  if (switching.value) return;
  switching.value = true;
  try {
    await switchChannel(id);
    await store.refresh();
    if (disposed) return;
    sidebarMode.value = 'profile';
    navigate('conversation');
    if (isWide.value) {
      if (!store.snapshot?.memberListVisible) await setMemberListVisible(true);
    } else {
      contextOpen.value = true;
    }
  } catch (error) {
    toast.error('Could not open archived channel', {
      description: String(error),
    });
  } finally {
    switching.value = false;
  }
}
async function openBranch(key: string) {
  if (switching.value) return;
  switching.value = true;
  try {
    const projectId = store.snapshot?.projectId;
    if (
      projectId &&
      !projectChannels.value.some(
        (channel) => channel.kind === 'general' && !channel.archived
      )
    ) {
      await selectProject(projectId);
    }
    await openBranchChannel(key);
    await store.refresh();
    await refreshProjectChannels(store.snapshot?.projectId || null);
    navigate('conversation');
  } catch (error) {
    toast.error('Could not open branch', { description: String(error) });
  } finally {
    switching.value = false;
  }
}
async function showApprovals() {
  const snapshot = store.snapshot;
  const channels = snapshot?.channels ?? [];
  const threads = snapshot?.threadApprovals ?? [];
  // The selected channel and its threads come first. Then the other channels, then the other threads.
  const channel =
    channels.find(
      (entry) => entry.id === snapshot?.channelId && entry.pendingApprovals
    ) ?? undefined;
  const ownThread = threads.find(
    (entry) => entry.parentChannelId === snapshot?.channelId
  );
  const otherChannel = channels.find((entry) => entry.pendingApprovals);
  const anyThread = threads[0];
  const target: { channelId: string } | { threadId: string } | null = channel
    ? { channelId: channel.id }
    : ownThread
      ? { threadId: ownThread.threadId }
      : otherChannel
        ? { channelId: otherChannel.id }
        : anyThread
          ? { threadId: anyThread.threadId }
          : null;
  if (!target) {
    toast.info('No pending approvals');
    return;
  }
  if ('threadId' in target) await openThreadFromBar(target.threadId);
  else await selectChannel(target.channelId);
  await nextTick();
  document
    .querySelector<HTMLElement>('[data-ui-id="approval-card"]')
    ?.scrollIntoView({ block: 'nearest' });
}
watch([isWide, isCompact], () => {
  navigationOpen.value = false;
  contextOpen.value = false;
  currentLayout.value = [];
});
watch(
  () => store.snapshot?.openThread?.channelId,
  (id) => {
    threadJump.value = null;
    // An opened thread replaces the draft.
    if (id) draftThread.value = null;
  }
);
watch(
  () => store.snapshot?.channelId,
  () => {
    draftThread.value = null;
    conversationTab.value = 'conversation';
    messageJump.value = null;
    if (
      searchAgent.value?.id !== store.snapshot?.agentId ||
      store.snapshot?.activeProjectId
    )
      sidebarMode.value = 'profile';
  }
);
</script>
<template>
  <main
    data-testid="app-root"
    data-ui-id="app-shell"
    class="flex h-dvh flex-col overflow-hidden bg-canvas text-sm text-foreground"
  >
    <header
      data-ui-id="app-toolbar"
      data-tauri-drag-region
      class="flex h-9 shrink-0 select-none items-center gap-2 pr-3"
      :class="integratedTitlebar ? 'pl-[88px]' : 'pl-3'"
    >
      <span class="pointer-events-none text-xs font-semibold">Discard</span>
      <span
        class="pointer-events-none mx-auto hidden truncate text-xs text-muted-foreground sm:block"
      >
        {{ store.snapshot?.project }}
        <template v-if="store.snapshot">— agent workspace</template>
      </span>
      <span
        v-if="store.snapshot?.turnRunning"
        :style="{ '--agent-color': activeAgentIdentity.mark }"
        class="pointer-events-none ml-auto flex items-center gap-1.5 text-xs text-muted-foreground"
      >
        <BrailleSpinner />
        Working
      </span>
      <Button
        data-ui-id="ui-inspector-toggle"
        :variant="inspector ? 'secondary' : 'ghost'"
        size="icon-xs"
        class="ml-auto sm:ml-0"
        aria-label="Toggle UI names"
        :aria-pressed="inspector"
        @click="inspector = !inspector"
      >
        <ScanEye />
      </Button>
    </header>
    <div
      v-if="store.snapshot"
      class="flex min-h-0 flex-1 gap-1.5 px-1.5 pb-1.5"
    >
      <NavigationPanel
        @project-action="projectAction"
        @rail-change="railChange"
        :projects="projects"
        :rail-layout="railLayout"
        v-if="isCompact"
        class="shrink-0"
        compact
        :snapshot="{ ...store.snapshot, channels: workingChannels }"
        :project-channels="projectChannels"
        :page="navigationPage"
        @navigate="navigateFromSidebar"
        @action="action"
        @toggle-branch="toggleBranch"
        :busy="switching"
        @select-channel="selectChannel"
        @inspect-channel="inspectChannel"
        @open-branch="openBranch"
        @approvals="showApprovals"
        @select-thread="openThreadFromBar"
        @leave-thread="leaveThreadFromBar"
      />
      <ResizablePanelGroup
        direction="horizontal"
        :keyboard-resize-by="1"
        @layout="currentLayout = $event"
      >
        <template v-if="!isCompact">
          <ResizablePanel
            :order="1"
            :default-size="
              navigationWidth(store.snapshot.panels.channelList, railWidth)
            "
            size-unit="px"
            :min-size="180 + railWidth"
            :max-size="400 + railWidth"
          >
            <NavigationPanel
              @project-action="projectAction"
              @rail-change="railChange"
              :projects="projects"
              :rail-layout="railLayout"
              :snapshot="store.snapshot"
              :project-channels="projectChannels"
              :page="navigationPage"
              @navigate="navigateFromSidebar"
              @action="action"
              @toggle-branch="toggleBranch"
              :busy="switching"
              @select-channel="selectChannel"
              @inspect-channel="inspectChannel"
              @open-branch="openBranch"
              @approvals="showApprovals"
              @select-thread="openThreadFromBar"
              @leave-thread="leaveThreadFromBar"
            />
          </ResizablePanel>
          <ResizableHandle
            data-ui-id="navigation-resizer"
            aria-label="Resize navigation"
            with-handle
            @dragging="resizeFinished"
            @keyup="savePanels"
          />
        </template>
        <!-- The chat takes window size changes; sidebars keep their pixel widths. -->
        <ResizablePanel :order="2" :min-size="0" size-unit="%">
          <section
            ref="mainPanel"
            @click="openMessageSource"
            data-ui-id="main-panel"
            class="relative flex h-full min-w-0 flex-col overflow-hidden rounded-xl border border-border/60 bg-workspace"
          >
            <ChannelHeader
              :conversation="
                page === 'conversation' && !!store.snapshot.channelId
              "
              :channel="heading || ''"
              :subtitle="
                store.snapshot.activeProjectId && page !== 'direct-messages'
                  ? projects.find(
                      (p) => p.id === store.snapshot?.activeProjectId
                    )?.name
                  : undefined
              "
              :direct-message="
                page === 'direct-messages' ||
                (page === 'conversation' && !store.snapshot.activeProjectId)
              "
              :compact="isCompact"
              :agent-id="store.snapshot.agentId"
              :sidebar-available="
                page === 'conversation' && !!store.snapshot.channelId
              "
              :agent-name="
                page === 'conversation' && store.snapshot.agentId
                  ? store.snapshot.agent
                  : undefined
              "
              @search="searchHistory"
              :threads="store.snapshot.channelThreads ?? []"
              @open-thread="openThreadById"
              :member-list-visible="
                (showContextPanel || contextOpen) &&
                sidebarMode === 'profile' &&
                !threadActive
              "
              @toggle-members="toggleMembers"
              :terminal-visible="terminalShown"
              :terminal-disabled-reason="
                terminalAccess.available ? '' : terminalAccess.reason
              "
              :terminal-shortcut="terminalShortcut"
              @toggle-terminal="toggleTerminal"
              @toggle-navigation="navigationOpen = true"
            >
              <PluginElement
                v-if="
                  page === 'conversation' &&
                  store.snapshot.activeProjectId &&
                  store.snapshot.slots.channel_header
                "
                :element="store.snapshot.slots.channel_header.element"
                :plugin="store.snapshot.slots.channel_header.plugin"
              />
            </ChannelHeader>
            <template
              v-if="page === 'conversation' && store.snapshot.channelId"
            >
              <Tabs
                v-if="store.snapshot.activeProjectId"
                v-model="conversationTab"
                data-ui-id="conversation-tabs"
                class="flex h-11 shrink-0 flex-row items-center justify-start overflow-x-auto border-b border-border/60 px-3 sm:px-5"
              >
                <TabsList variant="line" class="*:after:bg-primary">
                  <TabsTrigger value="conversation">Conversation</TabsTrigger>
                  <TabsTrigger v-if="showChangesTab" value="changes">
                    Changes
                    <Badge
                      v-if="changes.length"
                      variant="secondary"
                      class="font-mono"
                    >
                      {{ changes.length }}
                    </Badge>
                  </TabsTrigger>
                  <TabsTrigger value="run">Run</TabsTrigger>
                  <!-- TODO: Implement merge review. -->
                  <TabsTrigger value="review" disabled
                    >Merge review · Soon</TabsTrigger
                  >
                </TabsList>
                <span
                  v-if="store.snapshot.turnRunning"
                  data-ui-id="channel-activity"
                  class="ml-auto flex shrink-0 items-center gap-1.5 pl-4 font-mono text-xs text-muted-foreground"
                  :style="{ '--agent-color': activeAgentIdentity.mark }"
                >
                  <BrailleSpinner />
                  {{ activeAgent.name }} is working
                </span>
              </Tabs>
              <MessageList
                :agent-id="store.snapshot.agentId"
                :conversation-id="store.snapshot.channelId"
                :project-id="store.snapshot.activeProjectId"
                :jump-to="
                  messageJump?.channelId === store.snapshot.channelId
                    ? messageJump
                    : undefined
                "
                :memory-revision="store.snapshot.memoryRev"
                :position="transcriptPositions[store.snapshot.channelId]"
                @save-position="
                  (id, position) => (transcriptPositions[id] = position)
                "
                v-if="
                  !store.snapshot.activeProjectId ||
                  conversationTab === 'conversation'
                "
                :key="store.snapshot.channelId"
                :messages="store.history"
                :reply-disabled="replyDisabled"
                :thread-disabled="threadStartDisabled"
                @reply="reply"
                @start-thread="startThreadDraft"
                @open-thread="openThreadById"
                @decide-agent-request="decideAgentRequest"
                :live-sub-agent-threads="
                  store.snapshot.liveSubAgentThreads ?? []
                "
                :agent-requests="store.agentRequests"
                :agent-request-busy="decidingRequest"
                :live="store.snapshot.liveBlocks"
                :live-author="store.snapshot.responder"
                :running="store.snapshot.turnRunning"
                :new-anchor-id="store.snapshot.newAnchorId"
                :agent="store.snapshot.agent"
                :agent-avatar="agentAvatar"
                :prompts="store.snapshot.prompts"
              />
              <ChangesView
                v-else-if="conversationTab === 'changes'"
                :channel-id="store.snapshot.channelId"
                :history="store.history"
                :turn-running="!!store.snapshot.turnRunning"
                :branchless="branchless"
              />
              <div
                v-else-if="conversationTab === 'run'"
                data-ui-id="run-view"
                class="min-h-0 flex-1 overflow-auto overscroll-contain p-5"
              >
                <div v-if="tools.length" class="flex flex-col gap-1">
                  <h2 class="mb-3 text-lg font-semibold">Tool activity</h2>
                  <!-- A dense log. The bar keeps the agent color, because no message row names the agent here. -->
                  <ToolCallBlock
                    class="my-0! border-agent!"
                    v-for="(tool, index) in tools"
                    :key="index"
                    :name="tool.name"
                    :input="tool.input"
                    :output="tool.output"
                    :is-error="tool.is_error"
                    :running="tool.running"
                    :style="{ '--agent-color': tool.identity.mark }"
                  />
                </div>
                <Empty v-else>
                  <EmptyHeader>
                    <EmptyMedia variant="icon"><ListChecks /></EmptyMedia>
                    <EmptyTitle>No tool activity</EmptyTitle>
                    <EmptyDescription>
                      Tool calls from this conversation appear here.
                    </EmptyDescription>
                  </EmptyHeader>
                </Empty>
              </div>
              <div
                v-else
                data-ui-id="merge-review-view"
                class="min-h-0 flex-1 overflow-auto overscroll-contain"
              >
                <Empty>
                  <EmptyHeader>
                    <EmptyMedia variant="icon"><GitPullRequest /></EmptyMedia>
                    <EmptyTitle>No merge review</EmptyTitle>
                    <EmptyDescription>
                      Merge reviews are not available in this workspace yet.
                    </EmptyDescription>
                  </EmptyHeader>
                </Empty>
              </div>
            </template>
            <FriendsList
              v-else-if="
                page === 'direct-messages' ||
                (page === 'conversation' && !store.snapshot.channelId)
              "
              @action="action"
            />
            <WorkspacePageView
              v-else
              :page="page"
              :snapshot="store.snapshot"
              :messages="store.history"
              :project="
                projects.find((entry) => entry.id === store.snapshot?.projectId)
              "
              :project-channels="projectChannels"
              @project-changed="refreshProjectContext"
              @action="action"
              @conversation="navigate('conversation')"
            />
            <div
              data-ui-id="protected-notices"
              class="max-h-64 shrink-0 overflow-auto overscroll-contain"
            >
              <ApprovalDock
                :channel-id="store.snapshot.channelId"
                :prompts="store.snapshot.prompts"
              />
              <div
                v-if="
                  store.snapshot.notice &&
                  !store.snapshot.turnRunning &&
                  store.snapshot.channelId
                "
                class="flex flex-wrap gap-2 px-5 py-2"
              >
                <Button
                  v-if="store.snapshot.liveBlocks.length"
                  variant="outline"
                  @click="copyUnsavedReply"
                >
                  Copy reply before sending again
                </Button>
                <Button
                  v-if="!store.snapshot.composerDisabled"
                  variant="outline"
                  @click="retry"
                >
                  Retry last message
                </Button>
                <Button
                  variant="outline"
                  @click="action('open_agent_settings')"
                >
                  Choose model
                </Button>
              </div>
              <Button
                v-if="store.snapshot.archived && !archivedLegacy"
                class="mx-5 my-2"
                variant="outline"
                @click="restoreCurrentConversation"
              >
                Restore conversation
              </Button>
              <Button
                v-if="store.snapshot.archived && archivedLegacy"
                class="mx-5 my-2"
                variant="outline"
                @click="inspectChannel(store.snapshot.channelId)"
              >
                View channel details
              </Button>
              <p
                v-if="
                  store.snapshot.composerDisabled && store.snapshot.channelId
                "
                class="px-5 py-2 text-xs text-muted-foreground"
              >
                <template v-if="archivedLegacy">
                  Archived project history. Choose its details to select it as
                  the canonical channel.
                </template>
                <template v-else>
                  Read only. You can restore archived agents from their profile.
                  Deleted agents cannot send messages.
                </template>
              </p>
              <Notices
                :banners="store.snapshot.banners"
                :notice="store.snapshot.notice"
                :archived="store.snapshot.archived"
              />
            </div>
            <template
              v-if="page === 'conversation' && store.snapshot.channelId"
            >
              <p
                v-if="thinkingAgents.length"
                data-ui-id="thinking-line"
                class="flex items-center gap-2 px-5 py-1 text-xs text-muted-foreground"
              >
                <BrailleSpinner :color="thinkingColor" />
                {{
                  thinkingLine(
                    true,
                    thinkingAgents.map((agent) => agent.name)
                  )
                }}
              </p>
              <Composer
                ref="composer"
                :key="store.snapshot.channelId"
                :channel-id="store.snapshot.channelId"
                :drop-target="mainPanel"
                v-model:draft="drafts[store.snapshot.channelId]"
                :attachments="attachmentDrafts[store.snapshot.channelId] ?? []"
                @update:attachments="
                  attachmentDrafts[store.snapshot.channelId] = $event
                "
                :channel="store.snapshot.channel"
                :direct-message="!store.snapshot.activeProjectId"
                :project-id="store.snapshot.activeProjectId"
                :agent="store.snapshot.agent"
                :archived="
                  store.snapshot.composerDisabled || store.snapshot.archived
                "
                :busy="
                  switching ||
                  sending ||
                  store.snapshot.archiveBusy ||
                  !!store.snapshot.banners.archive
                "
                :running="store.snapshot.turnRunning"
                :reply="replies[store.snapshot.channelId]"
                @cancel-reply="cancelReply(store.snapshot.channelId)"
                @send="send"
                @stop="stop"
              />
            </template>
            <TerminalPanel
              v-if="terminalShown && store.snapshot.channelId"
              :channel-id="store.snapshot.channelId"
              :toggle-shortcut="terminalShortcut"
              @hide="terminalVisible = false"
            />
          </section>
        </ResizablePanel>
        <template v-if="showRightPanel">
          <ResizableHandle
            :data-ui-id="showThreadPanel ? 'thread-resizer' : 'context-resizer'"
            :aria-label="showThreadPanel ? 'Resize thread' : 'Resize context'"
            with-handle
            @dragging="resizeFinished"
            @keyup="savePanels"
          />
          <!-- The thread panel and the context panel have separate size ranges. -->
          <ResizablePanel
            :key="showThreadPanel ? 'thread' : 'context'"
            :order="3"
            :default-size="
              showThreadPanel
                ? 420
                : Math.max(240, store.snapshot.panels.memberList)
            "
            size-unit="px"
            :min-size="showThreadPanel ? 360 : 240"
            :max-size="showThreadPanel ? 640 : 400"
          >
            <div
              class="h-full overflow-hidden rounded-xl border border-border/60"
            >
              <div
                v-if="showThreadPanel"
                class="h-full"
                @click="openMessageSource"
              >
                <ThreadPanel
                  :parent-agent-id="store.snapshot?.agentId"
                  :parent-agent="store.snapshot?.agent"
                  ref="threadPanel"
                  v-bind="threadPanelProps"
                  @update:draft="drafts[threadKey] = $event"
                  @update:attachments="attachmentDrafts[threadKey] = $event"
                  @close="closeThreadPanel()"
                  @send="sendThread"
                  @stop="stopThread"
                  @reply="replyInThread"
                  @cancel-reply="cancelReply(threadKey)"
                />
              </div>
              <AgentHistorySearch
                v-else-if="sidebarMode === 'search' && searchAgent"
                :agent-id="searchAgent.id"
                :agent-name="searchAgent.name"
                :query="historyQuery"
                :revision="historyRevision"
                :busy="switching"
                @select="selectHistoryMatch"
              />
              <ContextPanel
                v-else
                :agent-avatar="agentAvatar"
                :profile-request="profileRequest"
                :direct-message="
                  page === 'direct-messages' ||
                  (page === 'conversation' && !store.snapshot.activeProjectId)
                "
                :snapshot="store.snapshot"
                :project-channels="projectChannels"
                @action="action"
                @main-agent-changed="refreshProjectContext"
                @canonical-changed="refreshProjectContext"
                @approvals="navigate('conversation')"
              />
            </div>
          </ResizablePanel>
        </template>
      </ResizablePanelGroup>
      <Sheet v-model:open="navigationOpen">
        <SheetContent side="left" class="gap-0" :show-close-button="false">
          <SheetTitle class="sr-only">Workspace navigation</SheetTitle>
          <SheetDescription class="sr-only"
            >Projects, channels, and agents</SheetDescription
          >
          <Button
            variant="ghost"
            class="shrink-0"
            @click="navigationOpen = false"
          >
            Close navigation
          </Button>
          <DirectMessageSidebar
            :channels="workingChannels"
            v-if="navigationPage === 'direct-messages'"
            class="min-h-0 flex-1"
            :channel-id="store.snapshot.channelId"
            :busy="switching"
            :open-thread-id="store.snapshot.openThread?.channelId"
            @select-channel="selectChannel"
            @action="action"
            @select-thread="openThreadFromBar"
            @leave-thread="leaveThreadFromBar"
          />
          <ChannelList
            v-else
            :channels="workingChannels"
            :project-channels="projectChannels"
            :channel-id="store.snapshot.channelId"
            class="min-h-0 flex-1"
            :page="navigationPage"
            :project="store.snapshot.project"
            :project-id="store.snapshot.projectId"
            :active-project-id="store.snapshot.activeProjectId"
            :project-path="store.snapshot.projectPath"
            :project-display-path="store.snapshot.projectDisplayPath"
            :project-avatar="
              projects.find((entry) => entry.id === store.snapshot?.projectId)
                ?.avatar
            "
            :agent="store.snapshot.agent"
            :channel="store.snapshot.channel"
            :branches="store.snapshot.branches"
            :repositories="store.snapshot.repositories"
            :repositories-loading="store.snapshot.repositoriesLoading"
            :unread="store.snapshot.unread"
            :pending-approvals="store.snapshot.pendingApprovals"
            :target="{ type: 'channel', id: store.snapshot.channelId }"
            :open-thread-id="store.snapshot.openThread?.channelId"
            @select-thread="openThreadFromBar"
            @leave-thread="leaveThreadFromBar"
            @navigate="navigateFromSidebar"
            @action="action"
            @toggle-branch="toggleBranch"
            :busy="switching"
            @select-channel="selectChannel"
            @inspect-channel="inspectChannel"
            @open-branch="openBranch"
            @approvals="showApprovals"
          />
        </SheetContent>
      </Sheet>
      <Sheet
        :open="threadSheetOpen"
        @update:open="(open) => !open && closeThreadPanel()"
      >
        <SheetContent
          side="right"
          class="gap-0 data-[side=right]:sm:max-w-md"
          :show-close-button="false"
        >
          <SheetTitle class="sr-only">Thread</SheetTitle>
          <SheetDescription class="sr-only"
            >A side conversation about one message</SheetDescription
          >
          <div class="min-h-0 flex-1" @click="openMessageSource">
            <ThreadPanel
              :parent-agent-id="store.snapshot?.agentId"
              :parent-agent="store.snapshot?.agent"
              v-if="threadSheetOpen"
              ref="threadPanel"
              v-bind="threadPanelProps"
              @update:draft="drafts[threadKey] = $event"
              @update:attachments="attachmentDrafts[threadKey] = $event"
              @close="closeThreadPanel()"
              @send="sendThread"
              @stop="stopThread"
              @reply="replyInThread"
              @cancel-reply="cancelReply(threadKey)"
            />
          </div>
        </SheetContent>
      </Sheet>
      <Sheet
        :open="contextOpen && !threadActive"
        @update:open="contextOpen = $event"
      >
        <SheetContent side="right" class="gap-0" :show-close-button="false">
          <SheetTitle class="sr-only">Workspace context</SheetTitle>
          <SheetDescription class="sr-only"
            >Channel context, members, and reviews</SheetDescription
          >
          <Button variant="ghost" class="shrink-0" @click="contextOpen = false">
            Close context
          </Button>
          <AgentHistorySearch
            v-if="sidebarMode === 'search' && searchAgent"
            :agent-id="searchAgent.id"
            :agent-name="searchAgent.name"
            :query="historyQuery"
            :revision="historyRevision"
            :busy="switching"
            @select="selectHistoryMatch"
          />
          <ContextPanel
            v-else
            :agent-avatar="agentAvatar"
            :profile-request="profileRequest"
            class="min-h-0 flex-1"
            :direct-message="
              page === 'direct-messages' ||
              (page === 'conversation' && !store.snapshot.activeProjectId)
            "
            :snapshot="store.snapshot"
            :project-channels="projectChannels"
            @action="action"
            @main-agent-changed="refreshProjectContext"
            @canonical-changed="refreshProjectContext"
            @approvals="
              () => {
                navigate('conversation');
                contextOpen = false;
              }
            "
          />
        </SheetContent>
      </Sheet>
    </div>
    <div
      v-else
      class="flex flex-1 items-center justify-center gap-2 text-sm text-muted-foreground"
    >
      <BrailleSpinner label="Opening workspace" />
      Opening workspace…
    </div>
    <StatusLine
      v-if="store.snapshot"
      :project="store.snapshot.project"
      :branch="statusBranch"
      :working-agents="workingAgents"
      :pending-approvals="selectedChannelApprovals(store.snapshot)"
    />
    <AddAgentDialog
      v-model:open="addAgentOpen"
      @created="
        (id) => {
          profileId = id;
          navigate('direct-messages');
        }
      "
      @connections="openProviderConnections"
    />
    <AgentProfileDialog
      :agent-id="profileId"
      @busy="profileBusy = $event"
      @close="profileId = null"
      @message="action(`message_agent:${$event}`)"
    />
    <ThreadActionDialogs :messages="store.history" />
    <UiInspector v-if="inspector" @close="inspector = false" />
    <Toaster
      :theme="store.snapshot?.theme.mode === 'light' ? 'light' : 'dark'"
      position="bottom-center"
      :duration="3000"
      close-button
    />
  </main>
</template>
