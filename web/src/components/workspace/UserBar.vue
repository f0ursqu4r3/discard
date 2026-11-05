<script setup lang="ts">
import { Settings } from '@lucide/vue'
import { Button } from '../ui/button'
import { Avatar, AvatarFallback } from '../ui/avatar'
import { Badge } from '../ui/badge'
defineProps<{ reviewCount: number; compact?: boolean }>()
const emit = defineEmits<{ settings: [] }>()
</script>
<template>
  <footer
    data-ui-id="user-bar"
    class="flex h-14 shrink-0 items-center gap-2 border-t border-border/60 bg-canvas px-3"
  >
    <Avatar v-if="!compact" class="size-8"
      ><AvatarFallback class="bg-secondary text-xs">Y</AvatarFallback></Avatar
    >
    <div v-if="!compact" class="min-w-0 flex-1">
      <p class="truncate text-xs font-semibold">You</p>
      <p class="truncate text-xs text-muted-foreground">Local workspace</p>
    </div>
    <Button
      variant="ghost"
      size="icon-sm"
      class="relative shrink-0"
      aria-label="Application settings"
      title="Application settings"
      @click="emit('settings')"
      ><Settings /><Badge v-if="reviewCount" variant="secondary" class="absolute -right-1 -top-1">{{
        reviewCount
      }}</Badge></Button
    >
  </footer>
</template>
