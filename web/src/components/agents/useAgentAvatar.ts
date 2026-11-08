import { onMounted, onUnmounted, ref, shallowRef, watch } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getAgents } from '@/bridge/api'

export const agentAvatars = shallowRef<Record<string, string | null>>({})

// One subscription per workspace, shared by the transcript and member panel.
export function useAgentAvatar(agentId: () => string | undefined) {
  const avatar = ref<string | null>(null)
  let generation = 0,
    disposed = false,
    stop: UnlistenFn | undefined
  async function reload() {
    const current = ++generation
    const id = agentId()
    if (!id) {
      avatar.value = null
      return
    }
    try {
      const agents = await getAgents()
      if (!disposed && current === generation) {
        agentAvatars.value = Object.fromEntries(agents.map((agent) => [agent.id, agent.avatar]))
        avatar.value = agents.find((agent) => agent.id === id)?.avatar ?? null
      }
    } catch {
      // Identity text remains available when the optional avatar cannot load.
    }
  }
  watch(
    agentId,
    () => {
      avatar.value = null
      void reload()
    },
    { immediate: true },
  )
  onMounted(async () => {
    try {
      const unlisten = await listen('discard://agents', () => void reload())
      if (disposed) unlisten()
      else {
        stop = unlisten
        void reload()
      }
    } catch {
      // The initial avatar remains usable if event subscription fails.
    }
  })
  onUnmounted(() => {
    disposed = true
    generation++
    stop?.()
  })
  return avatar
}
