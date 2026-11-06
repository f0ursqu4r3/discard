<script setup lang="ts">
import { computed, useId } from 'vue'
import type { ProviderModel } from '@/bridge/api'
import { Field, FieldDescription, FieldLabel } from '@/components/ui/field'
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'

const props = defineProps<{ model?: ProviderModel; disabled?: boolean; loading?: boolean }>()
const effort = defineModel<string | null>()
const id = useId()
const selection = computed({
  get: () => effort.value || 'default',
  set: (value: string) => {
    effort.value = value === 'default' ? null : value
  },
})
const options = computed(() => props.model?.reasoningEfforts ?? [])
const savedUnlisted = computed(
  () => effort.value && !options.value.some((option) => option.value === effort.value),
)
const description = computed(
  () => options.value.find((option) => option.value === effort.value)?.description,
)
function label(value: string) {
  return (
    (
      {
        none: 'None',
        minimal: 'Minimal',
        low: 'Low',
        medium: 'Medium',
        high: 'High',
        xhigh: 'Extra high',
        max: 'Max',
        ultra: 'Ultra',
      } as Record<string, string>
    )[value] ?? value
  )
}
</script>

<template>
  <Field>
    <FieldLabel :for="id">Reasoning level</FieldLabel>
    <Select v-model="selection" :disabled="disabled || loading || (!options.length && !effort)">
      <SelectTrigger :id="id" class="w-full" :aria-busy="loading">
        <SelectValue placeholder="Provider default" />
      </SelectTrigger>
      <SelectContent>
        <SelectGroup>
          <SelectItem value="default">Provider default</SelectItem>
          <SelectItem v-if="savedUnlisted && effort" :value="effort">
            {{ label(effort) }} (saved)
          </SelectItem>
          <SelectItem
            v-for="option in options"
            :key="option.value"
            :value="option.value"
            :title="option.description"
          >
            {{ label(option.value) }}
          </SelectItem>
        </SelectGroup>
      </SelectContent>
    </Select>
    <FieldDescription>
      <template v-if="loading">Loading reasoning levels…</template>
      <template v-else-if="savedUnlisted">
        Saved level. Available levels could not be confirmed for this model.
      </template>
      <template v-else-if="!model">Choose a model with available reasoning levels.</template>
      <template v-else-if="!options.length">
        This model does not report adjustable reasoning levels.
      </template>
      <template v-else>
        {{ description || 'Use the provider default or choose how much reasoning the agent uses.' }}
      </template>
    </FieldDescription>
  </Field>
</template>
