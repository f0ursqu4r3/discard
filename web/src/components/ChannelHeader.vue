<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { PanelRight, PanelLeft, Hash, AtSign, UserRound, Search } from '@lucide/vue'
import { InputGroup, InputGroupInput, InputGroupAddon, InputGroupButton } from './ui/input-group'
import { Button } from './ui/button'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from './ui/tooltip'
const props = withDefaults(
  defineProps<{
    channel: string
    memberListVisible: boolean
    subtitle?: string
    compact?: boolean
    directMessage?: boolean
    agentName?: string
    agentId?: string
    sidebarAvailable?: boolean
  }>(),
  { sidebarAvailable: true },
)
const emit = defineEmits<{ toggleMembers: []; toggleNavigation: []; search: [query: string] }>()
const query = ref('')
watch(
  () => props.agentId || props.agentName,
  () => {
    query.value = ''
  },
)
const label = computed(() =>
  props.directMessage
    ? props.memberListVisible
      ? 'Hide short profile'
      : 'Show short profile'
    : props.memberListVisible
      ? 'Hide member list'
      : 'Show member list',
)
function search() {
  if (query.value.trim()) emit('search', query.value.trim())
}
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
      <TooltipProvider v-if="sidebarAvailable !== false">
        <Tooltip>
          <TooltipTrigger as-child>
            <Button
              variant="ghost"
              size="icon-sm"
              :aria-label="label"
              :aria-pressed="memberListVisible"
              @click="emit('toggleMembers')"
            >
              <UserRound v-if="directMessage" />
              <PanelRight v-else />
            </Button>
          </TooltipTrigger>
          <TooltipContent>{{ label }}</TooltipContent>
        </Tooltip>
      </TooltipProvider>
      <form
        v-if="directMessage && agentName"
        role="search"
        class="flex items-center gap-1"
        @submit.prevent="search"
      >
        <InputGroup class="h-8 w-28 sm:w-44">
          <InputGroupInput
            v-model="query"
            class="h-full"
            :aria-label="`Search history with ${agentName}`"
            placeholder="Search history"
          />
          <InputGroupAddon align="inline-end">
            <InputGroupButton
              type="submit"
              size="icon-xs"
              aria-label="Search history"
              :disabled="!query.trim()"
            >
              <Search />
            </InputGroupButton>
          </InputGroupAddon>
        </InputGroup>
      </form>
    </div>
  </header>
</template>
