<script setup lang="ts">
import type { PromptDto } from '../bridge/types'
import { decideApproval } from '../bridge/api'
import { Button } from './ui/button'
defineProps<{ channelId: string; prompt: PromptDto }>()
</script>
<template>
  <div
    data-ui-id="approval-card"
    data-protected="true"
    class="space-y-2 rounded-md border border-[var(--protected-border)] bg-[var(--protected-bg)] p-3 text-[var(--protected-fg)]"
  >
    <p class="font-semibold">{{ prompt.tool }} wants to run</p>
    <pre class="whitespace-pre-wrap text-sm">{{ prompt.summary }}</pre>
    <div v-if="prompt.view === 'pending'" class="flex gap-2">
      <Button
        v-for="decision in ['approve', 'deny', 'always_allow']"
        :key="decision"
        variant="outline"
        size="sm"
        class="border-[var(--protected-border)] bg-[var(--protected-bg)] text-[var(--protected-fg)] hover:bg-[var(--protected-bg)] hover:text-[var(--protected-fg)] focus-visible:border-[var(--protected-border)] focus-visible:ring-[var(--protected-border)] aria-invalid:border-[var(--protected-border)] aria-invalid:ring-[var(--protected-border)] dark:aria-invalid:border-[var(--protected-border)] dark:aria-invalid:ring-[var(--protected-border)]"
        @click="decideApproval(channelId, prompt.approvalId, decision)"
      >
        {{
          decision === 'always_allow' ? 'Always allow' : decision === 'approve' ? 'Approve' : 'Deny'
        }}
      </Button>
    </div>
    <p v-else class="text-sm">
      {{
        prompt.view === 'stale'
          ? 'Expired: the turn ended before this was answered'
          : prompt.view === 'approved'
            ? 'Approved'
            : prompt.view === 'denied'
              ? 'Denied'
              : 'Answered'
      }}
    </p>
  </div>
</template>
