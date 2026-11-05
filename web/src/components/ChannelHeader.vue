<script setup lang="ts">
import { computed } from 'vue'
import { PanelRight, PanelLeft, Hash, AtSign } from '@lucide/vue'
import { Button } from './ui/button'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from './ui/tooltip'
const props = defineProps<{
  channel: string
  memberListVisible: boolean
  subtitle?: string
  compact?: boolean
  directMessage?: boolean
}>()
const emit = defineEmits<{ toggleMembers: []; toggleNavigation: [] }>()
const label = computed(() => (props.memberListVisible ? 'Hide member list' : 'Show member list'))
</script>
<template>
  <header
    data-ui-id="channel-header"
    class="flex h-16 shrink-0 items-stretch overflow-hidden border-b border-border/60"
  >
    <Button
      v-if="compact"
      variant="ghost"
      size="icon-sm"
      class="my-auto ml-2"
      aria-label="Open navigation"
      @click="emit('toggleNavigation')"
    >
      <PanelLeft />
    </Button>
    <div
      data-testid="header-slot"
      class="h-full max-h-16 min-h-0 min-w-0 flex-1 overflow-x-hidden overflow-y-auto overscroll-contain"
    >
      <slot>
        <div class="flex h-full items-center gap-3 px-3 sm:px-5">
          <component
            :is="directMessage ? AtSign : Hash"
            class="size-5 shrink-0 text-muted-foreground"
          />
          <div class="min-w-0">
            <h2 class="truncate text-sm font-semibold">{{ channel }}</h2>
            <p v-if="subtitle" class="truncate text-xs text-muted-foreground">{{ subtitle }}</p>
          </div>
        </div>
      </slot>
    </div>
    <div class="flex items-center px-2">
      <TooltipProvider>
        <Tooltip>
          <TooltipTrigger as-child>
            <Button
              variant="ghost"
              size="icon-sm"
              :aria-label="label"
              @click="emit('toggleMembers')"
            >
              <PanelRight />
            </Button>
          </TooltipTrigger>
          <TooltipContent>{{ label }}</TooltipContent>
        </Tooltip>
      </TooltipProvider>
    </div>
  </header>
</template>
