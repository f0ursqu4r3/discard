<script setup lang="ts">
import { computed, type HTMLAttributes } from 'vue'
import { cn } from '@/lib/utils'
import { activeTheme } from '@/bridge/theme'
import { serverInitials } from '../layout'
import { iconComponent, iconForeground, parseAgentIcon, resolveColor } from '../agents/agent-icon'

const props = defineProps<{
  name: string
  avatar?: string | null
  class?: HTMLAttributes['class']
}>()
const color = computed(() =>
  appearance.value ? resolveColor(appearance.value.color, activeTheme.value?.tokens ?? {}) : '',
)
const appearance = computed(() => parseAgentIcon(props.avatar))
</script>
<template>
  <span
    data-ui-id="project-avatar"
    :data-project-icon="appearance?.icon"
    :data-project-color="appearance?.color"
    :class="
      cn(
        'inline-flex size-8 shrink-0 items-center justify-center overflow-hidden rounded-lg bg-muted text-sm font-semibold text-muted-foreground',
        props.class,
      )
    "
    :style="appearance ? { backgroundColor: color, color: iconForeground(color) } : undefined"
    aria-hidden="true"
  >
    <component v-if="appearance?.icon" :is="iconComponent(appearance.icon)" class="size-3/5" />
    <span v-else>{{ serverInitials(name) }}</span>
  </span>
</template>
