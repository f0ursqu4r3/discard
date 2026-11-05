<script setup lang="ts">
import { computed } from 'vue'
import { PanelRight } from '@lucide/vue'
import { Button } from './ui/button'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from './ui/tooltip'
const props = defineProps<{ channel: string; memberListVisible: boolean }>()
const emit = defineEmits<{ toggleMembers: [] }>()
const label = computed(() => props.memberListVisible ? 'Hide member list' : 'Show member list')
</script>
<template>
  <header class="flex max-h-16 shrink-0 items-stretch overflow-hidden">
    <div data-testid="header-slot" class="max-h-16 min-w-0 flex-1 overflow-y-auto"><slot><strong class="block truncate border-b border-border px-4 py-3"># {{ channel }}</strong></slot></div>
    <div class="flex items-center border-b border-border px-2"><TooltipProvider><Tooltip><TooltipTrigger as-child><Button variant="ghost" size="icon" :aria-label="label" @click="emit('toggleMembers')"><PanelRight /></Button></TooltipTrigger><TooltipContent>{{ label }}</TooltipContent></Tooltip></TooltipProvider></div>
  </header>
</template>
