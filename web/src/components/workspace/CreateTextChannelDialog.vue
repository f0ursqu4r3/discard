<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { createTextChannel } from '@/bridge/api'
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
const props = defineProps<{ projectId?: string | null }>()
const emit = defineEmits<{ created: [id: string] }>()
const name = ref('')
const error = ref('')
const busy = ref(false)
const preview = computed(() => normalizeTextChannelName(name.value))
watch(open, (value) => {
  if (!value) return
  name.value = ''
  error.value = ''
})
async function submit() {
  if (busy.value || !preview.value || !props.projectId) return
  busy.value = true
  error.value = ''
  try {
    const id = await createTextChannel(props.projectId, name.value)
    open.value = false
    emit('created', id)
  } catch (cause) {
    error.value = String(cause)
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
      data-ui-id="create-text-channel-dialog"
      :show-close-button="!busy"
      class="sm:max-w-md"
    >
      <DialogHeader>
        <DialogTitle>New text channel</DialogTitle>
        <DialogDescription>
          Use a text channel for a project conversation that has no branch.
        </DialogDescription>
      </DialogHeader>
      <form id="create-text-channel-form" class="flex flex-col gap-3" @submit.prevent="submit">
        <FieldGroup>
          <Field>
            <FieldLabel for="text-channel-name">Channel name</FieldLabel>
            <Input
              id="text-channel-name"
              v-model="name"
              data-ui-id="text-channel-name-input"
              autocomplete="off"
              maxlength="100"
              :disabled="busy"
              required
            />
            <FieldDescription data-ui-id="text-channel-preview" class="font-mono">
              {{ preview ? `#${preview}` : '#' }}
            </FieldDescription>
          </Field>
        </FieldGroup>
        <p v-if="error" role="alert" class="text-sm text-destructive">{{ error }}</p>
      </form>
      <DialogFooter>
        <Button variant="ghost" :disabled="busy" @click="open = false">Cancel</Button>
        <Button
          type="submit"
          form="create-text-channel-form"
          data-ui-id="create-text-channel-submit"
          :disabled="busy || !preview || !projectId"
        >
          {{ busy ? 'Creating…' : 'Create channel' }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
