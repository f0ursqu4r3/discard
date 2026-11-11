<script setup lang="ts">
import { ref, watch } from 'vue'
import { setProjectShowArchived, type ProjectEntry } from '@/bridge/api'
import { Card, CardHeader, CardTitle, CardContent } from '../ui/card'
import { Field, FieldContent, FieldDescription, FieldLabel } from '../ui/field'
import { Switch } from '../ui/switch'

const props = defineProps<{ project: ProjectEntry }>()
const emit = defineEmits<{ changed: [] }>()
const checked = ref(props.project.showArchived ?? false)
const saving = ref(false)
const error = ref('')
watch(
  () => props.project.showArchived,
  (value) => {
    checked.value = value ?? false
  },
)
async function change(value: boolean) {
  if (saving.value) return
  const previous = checked.value
  checked.value = value
  saving.value = true
  error.value = ''
  try {
    await setProjectShowArchived(props.project.id, value)
    emit('changed')
  } catch (cause) {
    checked.value = previous
    error.value = String(cause)
  } finally {
    saving.value = false
  }
}
</script>
<template>
  <Card data-ui-id="project-channel-list-settings">
    <CardHeader><CardTitle>Channel list</CardTitle></CardHeader>
    <CardContent class="flex flex-col gap-3">
      <Field orientation="horizontal">
        <FieldContent>
          <FieldLabel for="project-show-archived">Show archived channels</FieldLabel>
          <FieldDescription id="project-show-archived-description">
            Show the Archived section in the channel list. It holds archived channels and channels
            of deleted branches that have messages.
          </FieldDescription>
        </FieldContent>
        <Switch
          id="project-show-archived"
          :model-value="checked"
          :disabled="saving"
          aria-describedby="project-show-archived-description"
          @update:model-value="change($event === true)"
        />
      </Field>
      <p v-if="error" role="alert" class="text-sm text-destructive">{{ error }}</p>
    </CardContent>
  </Card>
</template>
