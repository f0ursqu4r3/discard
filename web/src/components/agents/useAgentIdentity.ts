import { computed, toValue, type MaybeRefOrGetter } from 'vue'
import { activeTheme } from '@/bridge/theme'
import { agentIdentity } from './agent-icon'

export function useAgentIdentity(
  agentId: MaybeRefOrGetter<string | undefined>,
  avatar: MaybeRefOrGetter<string | null | undefined>,
) {
  return computed(() =>
    agentIdentity(toValue(avatar), toValue(agentId) ?? '', activeTheme.value?.tokens ?? {}),
  )
}
