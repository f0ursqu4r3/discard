<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { toast } from 'vue-sonner'
import { renameThread } from '@/bridge/api'
import { THREAD_NAME_MAX, normalizeThreadName } from '../threads'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from '../ui/dialog'
import { Input } from '../ui/input'
import { Field, FieldDescription, FieldGroup, FieldLabel } from '../ui/field'
import { Button } from '../ui/button'

const open = defineModel<boolean>('open', { default: false })
const props = defineProps<{ threadId: string; name: string }>()
const draft = ref('')
const busy = ref(false)
const saved = computed(() => normalizeThreadName(draft.value))
const length = computed(() => [...draft.value.trim()].length)
watch(
  open,
  (value) => {
    if (value) draft.value = props.name
  },
  { immediate: true },
)
// Focus the field with its text selected, so typing replaces the name.
function focusField(event: Event) {
  event.preventDefault()
  const field = document.getElementById('rename-thread-name') as HTMLInputElement | null
  field?.focus()
  field?.select()
}
async function submit() {
  if (busy.value || saved.value === null) return
  // An unchanged name needs no write.
  if (saved.value === props.name) {
    open.value = false
    return
  }
  busy.value = true
  try {
    await renameThread(props.threadId, draft.value)
    open.value = false
  } catch (error) {
    // The dialog stays open with the name intact, so the user can fix it.
    toast.error('Could not rename thread', { description: String(error) })
  } finally {
    busy.value = false
  }
}
</script>
<template>
  <Dialog
    :open="open"
    @update:open="
      (value) => {
        if (!busy) open = value
      }
    "
  >
    <DialogContent
      data-ui-id="rename-thread-dialog"
      :show-close-button="!busy"
      class="sm:max-w-md"
      @open-auto-focus="focusField"
    >
      <DialogHeader>
        <DialogTitle>Rename thread</DialogTitle>
        <DialogDescription>The new name shows wherever the thread shows.</DialogDescription>
      </DialogHeader>
      <form id="rename-thread-form" class="flex flex-col gap-3" @submit.prevent="submit">
        <FieldGroup>
          <Field>
            <FieldLabel for="rename-thread-name">Thread name</FieldLabel>
            <Input
              id="rename-thread-name"
              v-model="draft"
              data-ui-id="rename-thread-input"
              autocomplete="off"
              :aria-invalid="saved === null"
              :disabled="busy"
            />
            <FieldDescription
              data-ui-id="rename-thread-count"
              class="font-mono"
              :class="{ 'text-destructive': saved === null }"
            >
              {{ length }} / {{ THREAD_NAME_MAX }}
            </FieldDescription>
          </Field>
        </FieldGroup>
      </form>
      <DialogFooter>
        <Button variant="ghost" :disabled="busy" @click="open = false">Cancel</Button>
        <Button
          type="submit"
          form="rename-thread-form"
          data-ui-id="rename-thread-submit"
          :disabled="busy || saved === null"
        >
          {{ busy ? 'Renaming…' : 'Rename' }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
