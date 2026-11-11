<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { toast } from 'vue-sonner'
import { renameTextChannel } from '@/bridge/api'
import { normalizeTextChannelName } from '../text-channel'
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
const props = defineProps<{ projectId: string; channelId: string; name: string }>()
const draft = ref('')
const busy = ref(false)
const preview = computed(() => normalizeTextChannelName(draft.value))
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
  const field = document.getElementById('rename-channel-name') as HTMLInputElement | null
  field?.focus()
  field?.select()
}
async function submit() {
  if (busy.value || !preview.value) return
  // An unchanged name closes the dialog with no write.
  if (preview.value === props.name) {
    open.value = false
    return
  }
  busy.value = true
  try {
    await renameTextChannel(props.projectId, props.channelId, draft.value)
    open.value = false
  } catch (error) {
    // The dialog stays open with the name intact, so the user can fix it.
    toast.error('Could not rename channel', { description: String(error) })
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
      data-ui-id="rename-channel-dialog"
      :show-close-button="!busy"
      class="sm:max-w-md"
      @open-auto-focus="focusField"
    >
      <DialogHeader>
        <DialogTitle>Rename channel</DialogTitle>
        <DialogDescription>
          Use letters, numbers, hyphens, and underscores. The name must be new in this project.
        </DialogDescription>
      </DialogHeader>
      <form id="rename-channel-form" class="flex flex-col gap-3" @submit.prevent="submit">
        <FieldGroup>
          <Field>
            <FieldLabel for="rename-channel-name">Channel name</FieldLabel>
            <Input
              id="rename-channel-name"
              v-model="draft"
              data-ui-id="rename-channel-input"
              autocomplete="off"
              maxlength="100"
              :disabled="busy"
              required
            />
            <FieldDescription data-ui-id="rename-channel-preview" class="font-mono">
              {{ preview ? `#${preview}` : '#' }}
            </FieldDescription>
          </Field>
        </FieldGroup>
      </form>
      <DialogFooter>
        <Button variant="ghost" :disabled="busy" @click="open = false">Cancel</Button>
        <Button
          type="submit"
          form="rename-channel-form"
          data-ui-id="rename-channel-submit"
          :disabled="busy || !preview"
        >
          {{ busy ? 'Renaming…' : 'Rename channel' }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
