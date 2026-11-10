<script setup lang="ts">
import type { PromptDto } from '../bridge/types'
import { decideApproval } from '../bridge/api'
import { computed, ref, watch } from 'vue'
import { Button } from './ui/button'
const props = defineProps<{ channelId: string; prompt: PromptDto }>()
const cut = ref(props.prompt.defaultCut ?? 1)
watch(
  () => props.prompt.approvalId,
  () => {
    cut.value = props.prompt.defaultCut ?? 1
  },
)
const preview = computed(() =>
  props.prompt.ruleHost
    ? `Always allow network access to \`${props.prompt.ruleHost}\``
    : props.prompt.ruleWords
      ? `Always allow \`${props.prompt.ruleWords.slice(0, cut.value).join(' ')}\` commands`
      : '',
)
function decide(decision: string) {
  if (decision === 'always_allow' && props.prompt.ruleWords) {
    return decideApproval(props.channelId, props.prompt.approvalId, decision, cut.value)
  }
  return decideApproval(props.channelId, props.prompt.approvalId, decision)
}
</script>
<template>
  <div
    data-ui-id="approval-card"
    data-protected="true"
    class="space-y-2 rounded-md border border-[var(--protected-border)] bg-[var(--protected-bg)] p-3 text-[var(--protected-fg)]"
  >
    <p class="font-semibold">
      {{
        prompt.kind === 'sandbox_escape'
          ? 'Runs outside the sandbox: full access as you'
          : prompt.kind === 'network_domain'
            ? `Allow network access to ${prompt.summary}`
            : `${prompt.tool} wants to run`
      }}
    </p>
    <p v-if="prompt.kind === 'sandbox_escape'">
      This command can read and change any file you can, and use the network.
    </p>
    <pre class="whitespace-pre-wrap text-sm">{{ prompt.summary }}</pre>
    <p v-if="prompt.alwaysAllowNote" class="text-sm">{{ prompt.alwaysAllowNote }}</p>
    <div v-if="prompt.view === 'pending' && prompt.alwaysAllow === 'available'" class="space-y-2">
      <div v-if="prompt.ruleWords" class="flex flex-wrap gap-1" aria-label="Command prefix">
        <button
          v-for="(word, index) in prompt.ruleWords"
          :key="index"
          type="button"
          :aria-pressed="index < cut"
          class="rounded px-2 py-1 font-mono text-sm"
          :class="
            index < cut ? 'bg-primary text-primary-foreground' : 'bg-muted text-muted-foreground'
          "
          @click="cut = index + 1"
        >
          {{ word }}
        </button>
      </div>
      <p v-if="preview" class="text-sm">{{ preview }}</p>
    </div>
    <div v-if="prompt.view === 'pending'" class="flex gap-2">
      <Button
        v-for="decision in prompt.alwaysAllow === 'available'
          ? ['approve', 'deny', 'always_allow']
          : ['approve', 'deny']"
        :key="decision"
        variant="outline"
        size="sm"
        class="border-[var(--protected-border)] bg-[var(--protected-bg)] text-[var(--protected-fg)] hover:bg-[var(--protected-bg)] hover:text-[var(--protected-fg)] focus-visible:border-[var(--protected-border)] focus-visible:ring-[var(--protected-border)] aria-invalid:border-[var(--protected-border)] aria-invalid:ring-[var(--protected-border)] dark:aria-invalid:border-[var(--protected-border)] dark:aria-invalid:ring-[var(--protected-border)]"
        @click="decide(decision)"
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
