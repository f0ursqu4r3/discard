<script setup lang="ts">
import { provideAppearance, type AppearancePreferences } from '@/bridge/appearance'
import type { MessageDto } from '@/bridge/types'
import ServerRail from '@/components/ServerRail.vue'
import MessageRow from '@/components/MessageRow.vue'

const props = defineProps<{ preferences: AppearancePreferences }>()
provideAppearance(() => props.preferences)
const messages: MessageDto[] = [
  {
    id: 'preview-user',
    role: 'user',
    blocks: [{ type: 'markdown', text: 'Can you review this change?' }],
    continuesGroup: false,
  },
  {
    id: 'preview-agent',
    role: 'agent',
    blocks: [{ type: 'markdown', text: 'The change is ready. All checks passed.' }],
    continuesGroup: false,
  },
  {
    id: 'preview-followup',
    role: 'agent',
    blocks: [{ type: 'markdown', text: 'I also updated the documentation.' }],
    continuesGroup: true,
  },
]
</script>

<template>
  <figure class="space-y-2" aria-label="Appearance preview">
    <figcaption class="text-sm text-muted-foreground">
      Live preview · Sample conversation
    </figcaption>
    <div
      inert
      class="pointer-events-none flex overflow-hidden rounded-xl border border-border bg-background"
      data-testid="appearance-preview"
    >
      <ServerRail project="Sample project" :unread="true" :pending-approvals="1" />
      <div
        class="flex min-w-0 flex-1 flex-col py-2"
        :class="preferences.messageDensity === 'compact' ? 'gap-0' : 'gap-1'"
      >
        <MessageRow
          v-for="message in messages"
          :key="message.id"
          channel-id=""
          :message="message"
          agent="Assistant"
          preview
        />
      </div>
    </div>
  </figure>
</template>
