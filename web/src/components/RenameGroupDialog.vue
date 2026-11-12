<script setup lang="ts">
import { ref, watch } from 'vue'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from './ui/dialog'
import { Input } from './ui/input'
import { Field, FieldGroup, FieldLabel } from './ui/field'
import { Button } from './ui/button'

const open = defineModel<boolean>('open', { default: false })
const props = defineProps<{ name: string; placeholder: string }>()
const emit = defineEmits<{ save: [name: string] }>()
const draft = ref('')
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
  const field = document.getElementById('rename-group-name') as HTMLInputElement | null
  field?.focus()
  field?.select()
}
function submit() {
  emit('save', draft.value.trim())
  open.value = false
}
</script>
<template>
  <Dialog v-model:open="open">
    <DialogContent
      data-ui-id="rename-group-dialog"
      class="sm:max-w-md"
      @open-auto-focus="focusField"
    >
      <DialogHeader>
        <DialogTitle>Rename group</DialogTitle>
        <DialogDescription>
          A group name is optional. An empty name shows the names of the projects in the group.
        </DialogDescription>
      </DialogHeader>
      <form id="rename-group-form" class="flex flex-col gap-3" @submit.prevent="submit">
        <FieldGroup>
          <Field>
            <FieldLabel for="rename-group-name">Group name</FieldLabel>
            <Input
              id="rename-group-name"
              v-model="draft"
              data-ui-id="rename-group-input"
              autocomplete="off"
              maxlength="100"
              :placeholder="placeholder"
            />
          </Field>
        </FieldGroup>
      </form>
      <DialogFooter>
        <Button variant="ghost" @click="open = false">Cancel</Button>
        <Button type="submit" form="rename-group-form" data-ui-id="rename-group-submit">
          Save
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
