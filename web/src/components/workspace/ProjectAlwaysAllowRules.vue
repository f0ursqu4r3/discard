<script setup lang="ts">
import { ref, watch } from 'vue'
import { getAlwaysAllowRules, revokeAlwaysAllowRule, type RuleDto } from '@/bridge/api'
import { Button } from '../ui/button'
import { Card, CardHeader, CardTitle, CardContent } from '../ui/card'
const props = defineProps<{ projectId: string }>()
const rules = ref<RuleDto[]>([]),
  error = ref(''),
  loading = ref(true),
  revoking = ref<string | null>(null)
watch(
  () => props.projectId,
  async (project, _, onCleanup) => {
    let active = true
    onCleanup(() => {
      active = false
    })
    loading.value = true
    error.value = ''
    try {
      const rows = await getAlwaysAllowRules(project)
      if (active) rules.value = rows
    } catch (cause) {
      if (active) error.value = String(cause)
    } finally {
      if (active) loading.value = false
    }
  },
  { immediate: true },
)
async function revoke(id: string) {
  if (revoking.value) return
  const project = props.projectId
  revoking.value = id
  error.value = ''
  try {
    await revokeAlwaysAllowRule(project, id)
    if (props.projectId === project) rules.value = rules.value.filter((rule) => rule.id !== id)
  } catch (cause) {
    if (props.projectId === project) error.value = String(cause)
  } finally {
    revoking.value = null
  }
}
</script>
<template>
  <Card data-ui-id="project-always-allow-rules">
    <CardHeader><CardTitle>Always-allow rules</CardTitle></CardHeader>
    <CardContent class="space-y-3">
      <p v-if="loading" class="text-sm text-muted-foreground">Loading rules…</p>
      <p v-else-if="!rules.length && !error" class="text-sm text-muted-foreground">
        No Always-allow rules yet. Choose Always allow on an approval card to add one.
      </p>
      <div v-for="rule in rules" :key="rule.id" class="flex flex-wrap items-center gap-2 text-sm">
        <span>{{ rule.agentName }}</span>
        <span>{{ rule.kind === 'command' ? 'Command' : 'Network' }}</span>
        <code class="break-all">{{ rule.text }}</code>
        <span class="text-muted-foreground">
          {{ new Date(rule.createdAt * 1000).toLocaleDateString() }}
        </span>
        <Button variant="outline" size="sm" :disabled="revoking !== null" @click="revoke(rule.id)">
          Revoke
        </Button>
      </div>
      <p v-if="error" role="alert" class="text-sm text-destructive">{{ error }}</p>
    </CardContent>
  </Card>
</template>
