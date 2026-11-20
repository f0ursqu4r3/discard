import { shallowMount, flushPromises } from "@vue/test-utils";
import { invoke } from "@tauri-apps/api/core";
import { beforeEach, afterEach, expect, test, vi } from "vite-plus/test";
import fixture from "../bridge/fixtures/snapshot.json";
import type { ProjectChannelDto, SnapshotDto } from "../bridge/types";
const mocks = vi.hoisted(() => ({
  store: {
    snapshot: null as SnapshotDto | null,
    history: [] as unknown[],
    refresh: vi.fn(async () => {}),
  },
  live: null as { history: unknown[] } | null,
}));
const catalogHandlers = new Map<string, Set<() => void>>();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  isTauri: () => false,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, handler: () => void) => {
    if (!catalogHandlers.has(event)) catalogHandlers.set(event, new Set());
    catalogHandlers.get(event)!.add(handler);
    return () => catalogHandlers.get(event)?.delete(handler);
  }),
}));
vi.mock("../bridge/store", async () => {
  const { reactive } = await import("vue");
  mocks.live = reactive(mocks.store);
  return { useStore: () => mocks.live };
});
vi.mock("../bridge/shortcuts", () => ({ useWebShortcuts: vi.fn() }));
vi.mock("../bridge/api", async (original) => ({
  ...(await original<object>()),
  getProjects: vi.fn(async () => []),
  getProjectChannels: vi.fn(async () => []),
}));
vi.mock("vue-sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));
import App from "../App.vue";
import ReviewView from "../components/review/ReviewView.vue";
import { Badge } from "../components/ui/badge";
import { Tabs, TabsTrigger } from "../components/ui/tabs";
import { getProjectChannels } from "../bridge/api";

let view: ReturnType<typeof shallowMount> | undefined;
function channel(kind: ProjectChannelDto["kind"]): ProjectChannelDto {
  return {
    id: "channel",
    projectId: "project",
    name: "channel",
    agentId: "agent",
    workspaceMode: "main_checkout",
    kind,
    branch: null,
    archived: false,
    archivedInto: null,
    hasMessages: false,
  };
}
beforeEach(() => {
  vi.clearAllMocks();
  catalogHandlers.clear();
  mocks.store.snapshot = {
    ...structuredClone(fixture),
    projectId: "project",
    activeProjectId: "project",
    composerDisabled: false,
    turnRunning: false,
    liveBlocks: [],
  };
  mocks.store.history = [];
  vi.mocked(invoke).mockResolvedValue(undefined);
  vi.stubGlobal(
    "matchMedia",
    vi.fn(() => ({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })),
  );
});
afterEach(() => {
  view?.unmount();
  vi.unstubAllGlobals();
});
async function app(projectId: string | null = "project") {
  mocks.store.snapshot!.activeProjectId = projectId;
  vi.mocked(getProjectChannels).mockResolvedValue([channel("text")]);
  view = shallowMount(App, {
    global: {
      renderStubDefaultSlot: true,
      stubs: {
        ResizablePanelGroup: { template: "<div><slot /></div>" },
        ResizablePanel: { template: "<div><slot /></div>" },
      },
    },
  });
  await flushPromises();
  return view;
}
const tabs = (wrapper: NonNullable<typeof view>) =>
  wrapper
    .findAllComponents(Tabs)
    .find((item) => item.attributes("data-ui-id") === "conversation-tabs")!;

const review = (wrapper: NonNullable<typeof view>) =>
  wrapper
    .findAllComponents(TabsTrigger)
    .find((item) => item.props("value") === "review")!;
async function open(wrapper: NonNullable<typeof view>) {
  tabs(wrapper).vm.$emit("update:modelValue", "review");
  await flushPromises();
}

test("the Merge review tab is enabled and has no Soon label", async () => {
  const trigger = review(await app());
  expect(trigger.props("disabled")).toBeFalsy();
  expect(trigger.text()).toBe("Merge review");
});

test("opening the tab mounts ReviewView for the active channel", async () => {
  const wrapper = await app();
  expect(wrapper.findComponent(ReviewView).exists()).toBe(false);
  await open(wrapper);
  const view = wrapper.findComponent(ReviewView);
  expect(view.props("channelId")).toBe(mocks.store.snapshot!.channelId);
  expect(view.props("turnRunning")).toBe(false);
});

test.each([
  ["approved", "Approved"],
  ["changes_requested", "Changes requested"],
  ["changes_requested_new_commits", "Changes requested"],
  ["approval_out_of_date", "Out of date"],
  ["merged", "Merged"],
])("the badge shows for %s", async (state, label) => {
  const wrapper = await app();
  await open(wrapper);
  wrapper.findComponent(ReviewView).vm.$emit("state", state);
  await flushPromises();
  expect(review(wrapper).findComponent(Badge).text()).toBe(label);
});

test.each(["not_reviewed", "nothing_to_review", null])(
  "no badge for %s",
  async (state) => {
    const wrapper = await app();
    await open(wrapper);
    wrapper.findComponent(ReviewView).vm.$emit("state", "approved");
    wrapper.findComponent(ReviewView).vm.$emit("state", state);
    await flushPromises();
    expect(review(wrapper).findComponent(Badge).exists()).toBe(false);
  },
);

test("a channel change clears the badge and returns to Conversation", async () => {
  const wrapper = await app();
  await open(wrapper);
  wrapper.findComponent(ReviewView).vm.$emit("state", "merged");
  await flushPromises();
  (mocks.live as unknown as { snapshot: SnapshotDto }).snapshot.channelId =
    "other";
  await flushPromises();
  expect(tabs(wrapper).props("modelValue")).toBe("conversation");
  expect(review(wrapper).findComponent(Badge).exists()).toBe(false);
});

test("a conversation outside a project shows no tabs", async () => {
  const wrapper = await app(null);
  expect(wrapper.findComponent(Tabs).exists()).toBe(false);
  expect(wrapper.findComponent(ReviewView).exists()).toBe(false);
});
