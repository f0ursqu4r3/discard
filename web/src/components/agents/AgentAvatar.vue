<script setup lang="ts">
import { computed } from 'vue'
import { Avatar, AvatarFallback, AvatarImage } from '@/components/ui/avatar'
import { serverInitials } from '@/components/layout'
import { iconComponent, parseAgentIcon } from './agent-icon'
import { useAgentIdentity } from './useAgentIdentity'
const props = defineProps<{ name: string; agentId?: string; avatar?: string | null }>()
const identity = useAgentIdentity(
  () => props.agentId,
  () => props.avatar,
)
const markStyle = computed(() => ({
  background: `color-mix(in srgb, ${identity.value.mark} 15%, var(--background))`,
  color: identity.value.avatarMark,
  boxShadow: `inset 0 0 0 1px color-mix(in srgb, ${identity.value.mark} 40%, transparent)`,
}))
const appearance = computed(() => parseAgentIcon(props.avatar))
</script>
<template>
  <Avatar data-ui-id="agent-avatar">
    <span
      v-if="appearance"
      data-agent-mark
      :data-agent-icon="appearance.icon"
      :data-agent-color="appearance.color"
      class="flex size-full items-center justify-center rounded-full"
      :style="markStyle"
      aria-hidden="true"
    >
      <component v-if="appearance.icon" :is="iconComponent(appearance.icon)" class="size-3/5" />
      <span v-else>{{ serverInitials(name) }}</span>
    </span>
    <template v-else>
      <AvatarImage v-if="avatar && !avatar.startsWith('lucide:')" :src="avatar" :alt="name" />
      <AvatarFallback data-agent-mark class="rounded-full" :style="markStyle">
        {{ serverInitials(name) }}
      </AvatarFallback>
    </template>
  </Avatar>
</template>
