<script setup lang="ts">
import { computed } from 'vue'
import { Avatar, AvatarFallback, AvatarImage } from '@/components/ui/avatar'
import { serverInitials } from '@/components/layout'
import { iconComponent, iconForeground, parseAgentIcon } from './agent-icon'
const props = defineProps<{ name: string; avatar?: string | null }>()
const appearance = computed(() => parseAgentIcon(props.avatar))
</script>
<template>
  <Avatar data-ui-id="agent-avatar">
    <span
      v-if="appearance"
      :data-agent-icon="appearance.icon"
      :data-agent-color="appearance.color"
      class="flex size-full items-center justify-center rounded-full"
      :style="{ backgroundColor: appearance.color, color: iconForeground(appearance.color) }"
      aria-hidden="true"
    >
      <component v-if="appearance.icon" :is="iconComponent(appearance.icon)" class="size-3/5" />
      <span v-else>{{ serverInitials(name) }}</span>
    </span>
    <template v-else>
      <AvatarImage v-if="avatar && !avatar.startsWith('lucide:')" :src="avatar" :alt="name" />
      <AvatarFallback>{{ serverInitials(name) }}</AvatarFallback>
    </template>
  </Avatar>
</template>
